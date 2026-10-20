//! Feeds saved and hand-made events through the tracker.

use reclutched_core::rocketleague::settings::Settings;
use reclutched_core::rocketleague::tracker::{FinishedMatch, MatchResult, Tracker};
use reclutched_core::rocketleague::{parse_message, StatsEvent};
use serde_json::{json, Value};

const T0: i64 = 1_000;
const T1: i64 = 1_300;

fn fixture(name: &str) -> Vec<StatsEvent> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| parse_message(l).unwrap())
        .collect()
}

/// Runs events through a tracker; returns the first finished match (or what is left at the end).
fn run(settings: Settings, events: &[StatsEvent]) -> (Option<FinishedMatch>, Tracker) {
    let mut tracker = Tracker::new(settings);
    for ev in events {
        if let Some(done) = tracker.on_event_at(ev, T0) {
            return (Some(done), tracker);
        }
    }
    let done = tracker.finish_pending(T1);
    (done, tracker)
}

fn event(name: &str, data: Value) -> StatsEvent {
    parse_message(&json!({ "Event": name, "Data": data }).to_string()).unwrap()
}

/// A player as the game sends them. `boost: Some(_)` means "we can see this player's movement"
/// (you and teammates); like the real game, `Boost` is left out at 0 and `bBoosting` when false.
fn player(shortcut: i64, name: &str, team: i64, score: i64, boost: Option<i64>) -> Value {
    let mut p = json!({
        "Name": name, "PrimaryId": format!("Epic|{name}|0"), "Shortcut": shortcut,
        "TeamNum": team, "Score": score, "Goals": 0, "Shots": 0, "Assists": 0, "Saves": 0,
        "Touches": 0, "CarTouches": 0, "Demos": 0
    });
    if let Some(b) = boost {
        p["bHasCar"] = json!(true);
        p["Speed"] = json!(0.0);
        if b > 0 {
            p["Boost"] = json!(b);
            p["bBoosting"] = json!(true);
        }
    }
    p
}

/// Adds or overrides fields of a player JSON (e.g. `Speed`, `bPowersliding`).
fn with(mut p: Value, fields: Value) -> Value {
    for (k, v) in fields.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

fn state(
    playlist: i64,
    remaining: i64,
    players: Vec<Value>,
    me: &str,
    scores: [i64; 2],
) -> StatsEvent {
    state_ball(playlist, remaining, players, me, scores, (0.0, 255))
}

/// Like `state`, with the ball's speed and the team that touched it last.
fn state_ball(
    playlist: i64,
    remaining: i64,
    players: Vec<Value>,
    me: &str,
    scores: [i64; 2],
    ball: (f64, i64),
) -> StatsEvent {
    let target = players.iter().find(|p| p["Name"] == me).unwrap();
    event(
        "UpdateState",
        json!({
            "MatchGuid": "G1", "Players": players,
            "Game": {
                "Teams": [
                    {"Name":"Blue","TeamNum":0,"Score":scores[0],"ColorPrimary":"0","ColorSecondary":"0"},
                    {"Name":"Orange","TeamNum":1,"Score":scores[1],"ColorPrimary":"0","ColorSecondary":"0"}
                ],
                "PlaylistId": playlist, "TimeSeconds": remaining, "bOvertime": false,
                "Ball": {"Speed": ball.0, "TeamNum": ball.1}, "bReplay": false, "bHasWinner": false,
                "Winner": "", "Arena": "Stadium_P", "bHasTarget": true,
                "Target": {"Name": me, "Shortcut": target["Shortcut"], "TeamNum": target["TeamNum"]}
            }
        }),
    )
}

fn guid() -> Value {
    json!({ "MatchGuid": "G1" })
}

#[test]
fn real_online_match_becomes_a_win_with_one_goal() {
    let (done, mut tracker) = run(Settings::default(), &fixture("casual_2v2.jsonl"));
    let m = done.expect("the match is recorded");

    assert_eq!(m.result, MatchResult::Win);
    assert_eq!(m.playlist_id, 3);
    assert!(m.guid.is_some());
    assert!(m.players.len() >= 4);
    assert_eq!(m.players[m.my_slot].team, m.my_team);
    assert!(m.players[m.my_slot].is_me);

    // The fixture has one live goal plus its duplicate sent during the replay.
    assert_eq!(m.goals.len(), 1, "replay duplicate must be ignored");
    let goal = &m.goals[0];
    assert!(goal.assister.is_some());
    assert!(goal.goal_speed > 0.0);
    assert!(goal.score_after.is_some());

    assert!(m.stat_events.iter().any(|s| s.name == "Shot"));
    assert!(!m.touches.is_empty());
    assert!(!m.movement.is_empty(), "teammates and you have boost data");
    // We learned who we are (a real id, not a bot id).
    assert!(tracker.take_learned_id().unwrap().starts_with("Epic|"));
}

#[test]
fn offline_match_is_skipped_unless_enabled_and_then_counts_as_abandoned() {
    let events = fixture("offline_bots.jsonl");
    assert!(run(Settings::default(), &events).0.is_none());

    let settings = Settings {
        offline_matches: true,
        ..Settings::default()
    };
    let m = run(settings, &events).0.expect("recorded when enabled");
    // The capture never reached MatchEnded.
    assert_eq!(m.result, MatchResult::Abandoned);
    assert!(m.abandoned_clock.is_some());
    assert_eq!(m.guid, None);
}

#[test]
fn switches_turn_collectors_off() {
    let events = fixture("casual_2v2.jsonl");
    let settings = Settings {
        goals: false,
        stat_events: false,
        ball_hits: false,
        possession: false,
        movement: false,
        roster: false,
        ..Settings::default()
    };
    let m = run(settings, &events).0.unwrap();
    assert!(m.goals.is_empty() && m.stat_events.is_empty() && m.event_stats.is_empty());
    assert!(m.touches.is_empty() && m.crossbar.is_empty() && m.pickups.is_empty());
    assert!(m.my_possession_pct.is_none() && m.team_empty_overlap.is_none());
    assert!(m.movement.is_empty());
    // Roster off: slots and teams remain, but no names, ids or stats.
    assert!(m
        .players
        .iter()
        .all(|p| p.name.is_empty() && p.primary_id.is_none() && p.score == 0));
    assert_eq!(
        m.result,
        MatchResult::Win,
        "the result itself is a separate switch"
    );

    let nothing = Settings {
        matches: false,
        ..Settings::default()
    };
    assert!(run(nothing, &events).0.is_none());
}

#[test]
fn watching_someone_else_is_not_recorded() {
    let settings = Settings {
        my_primary_id: Some("Epic|somebody-else|0".into()),
        ..Settings::default()
    };
    assert!(run(settings, &fixture("casual_2v2.jsonl")).0.is_none());
}

/// A 2v2 start: countdown (not sampled), then two live ticks: boost 100/0, then 0/0.
fn two_v_two(playlist: i64) -> Vec<StatsEvent> {
    let players = |boosts: [i64; 2]| {
        vec![
            player(1, "Me", 0, 100, Some(boosts[0])),
            player(2, "Mate", 0, 50, Some(boosts[1])),
            player(3, "Foe1", 1, 20, None),
            player(4, "Foe2", 1, 10, None),
        ]
    };
    vec![
        event("MatchCreated", guid()),
        event("CountdownBegin", guid()),
        state(playlist, 300, players([33, 33]), "Me", [0, 0]),
        event("RoundStarted", guid()),
        state(playlist, 299, players([100, 0]), "Me", [0, 0]),
        state(playlist, 298, players([0, 0]), "Me", [0, 0]),
    ]
}

#[test]
fn training_sessions_are_not_matches() {
    let mut events = two_v_two(9);
    events.push(event("MatchDestroyed", guid()));
    assert!(run(Settings::default(), &events).0.is_none());
}

#[test]
fn loss_when_the_other_team_wins() {
    let mut events = two_v_two(11);
    events.push(event(
        "MatchEnded",
        json!({ "MatchGuid": "G1", "WinnerTeamNum": 1 }),
    ));
    events.push(event("MatchDestroyed", guid()));
    let m = run(Settings::default(), &events).0.unwrap();
    assert_eq!(m.result, MatchResult::Loss);
    assert_eq!(m.my_team, 0);
}

#[test]
fn leaving_without_match_ended_is_an_abandon() {
    let mut events = two_v_two(11);
    events.push(event("MatchDestroyed", guid()));
    let m = run(Settings::default(), &events).0.unwrap();
    assert_eq!(m.result, MatchResult::Abandoned);
    assert_eq!(m.abandoned_clock.unwrap().remaining, 298);
}

#[test]
fn leaver_keeps_last_seen_stats_and_leave_time() {
    let mut events = two_v_two(11);
    events.push(event(
        "PlayerLeft",
        json!({ "MatchGuid": "G1", "PlayerName": "Foe1", "PrimaryId": "Epic|Foe1|0" }),
    ));
    // Foe1 is gone from the next update.
    events.push(state(
        11,
        280,
        vec![
            player(1, "Me", 0, 100, Some(50)),
            player(2, "Mate", 0, 50, Some(50)),
            player(4, "Foe2", 1, 10, None),
        ],
        "Me",
        [0, 0],
    ));
    events.push(event(
        "MatchEnded",
        json!({ "MatchGuid": "G1", "WinnerTeamNum": 0 }),
    ));
    events.push(event("MatchDestroyed", guid()));
    let m = run(Settings::default(), &events).0.unwrap();
    let foe = m.players.iter().find(|p| p.name == "Foe1").unwrap();
    assert_eq!(foe.score, 20);
    assert_eq!(foe.left.unwrap().remaining, 298);
}

#[test]
fn movement_percentages_are_computed_per_player() {
    let mut events = two_v_two(11);
    events.push(event(
        "MatchEnded",
        json!({ "MatchGuid": "G1", "WinnerTeamNum": 0 }),
    ));
    events.push(event("MatchDestroyed", guid()));
    let m = run(Settings::default(), &events).0.unwrap();
    // "Me": boost 100 then 0 -> average 50, empty half the time, boosting half the time.
    let me = m.movement.iter().find(|s| s.slot == m.my_slot).unwrap();
    assert_eq!(me.samples, 2);
    assert!((me.avg_boost - 50.0).abs() < 1e-9);
    assert!((me.pct_boost_empty - 50.0).abs() < 1e-9);
    assert!((me.pct_boosting - 50.0).abs() < 1e-9);
    // Opponents have no boost data, so no movement row.
    assert_eq!(m.movement.len(), 2);
}

#[test]
fn a_new_match_closes_the_previous_one() {
    let mut events = two_v_two(11);
    events.push(event(
        "MatchEnded",
        json!({ "MatchGuid": "G1", "WinnerTeamNum": 0 }),
    ));
    // No MatchDestroyed: the next MatchCreated must close the first match.
    events.push(event("MatchCreated", json!({ "MatchGuid": "G2" })));
    let mut tracker = Tracker::new(Settings::default());
    let finished: Vec<_> = events
        .iter()
        .filter_map(|e| tracker.on_event_at(e, T0))
        .collect();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].result, MatchResult::Win);
}

/// Runs synthetic events and returns the finished match (the last event must end the match).
fn finish(mut events: Vec<StatsEvent>) -> FinishedMatch {
    events.push(event(
        "MatchEnded",
        json!({ "MatchGuid": "G1", "WinnerTeamNum": 0 }),
    ));
    events.push(event("MatchDestroyed", guid()));
    run(Settings::default(), &events).0.expect("recorded")
}

fn me_movement(m: &FinishedMatch) -> &reclutched_core::rocketleague::tracker::MovementSummary {
    m.movement.iter().find(|s| s.slot == m.my_slot).unwrap()
}

fn pair(me: Value, mate: Value) -> Vec<Value> {
    vec![
        me,
        mate,
        player(3, "Foe1", 1, 0, None),
        player(4, "Foe2", 1, 0, None),
    ]
}

#[test]
fn countdown_and_replay_ticks_are_not_sampled() {
    let live = |boost: i64| {
        state(
            11,
            100,
            pair(
                player(1, "Me", 0, 0, Some(boost)),
                player(2, "Mate", 0, 0, Some(boost)),
            ),
            "Me",
            [0, 0],
        )
    };
    let m = finish(vec![
        event("MatchCreated", guid()),
        event("CountdownBegin", guid()),
        live(33), // countdown: ignored
        event("RoundStarted", guid()),
        live(100),
        event("GoalReplayStart", guid()),
        live(0), // replay: ignored
        event("GoalReplayEnd", guid()),
        event("CountdownBegin", guid()),
        live(33), // next countdown: ignored
        event("RoundStarted", guid()),
        live(50),
    ]);
    let me = me_movement(&m);
    assert_eq!(me.samples, 2);
    assert!((me.avg_boost - 75.0).abs() < 1e-9);
}

#[test]
fn boost_used_and_gained_come_from_changes_between_live_ticks() {
    let tick = |boost: i64| {
        state(
            11,
            100,
            pair(
                player(1, "Me", 0, 0, Some(boost)),
                player(2, "Mate", 0, 0, Some(50)),
            ),
            "Me",
            [0, 0],
        )
    };
    let m = finish(vec![
        event("MatchCreated", guid()),
        event("RoundStarted", guid()),
        tick(100),
        tick(60), // used 40
        tick(90), // gained 30
        tick(90),
        // A goal ends the round; the jump to the kickoff boost must not count as a pickup.
        event("GoalReplayStart", guid()),
        event("GoalReplayEnd", guid()),
        event("CountdownBegin", guid()),
        event("RoundStarted", guid()),
        tick(33),
    ]);
    let me = me_movement(&m);
    assert_eq!((me.boost_used, me.boost_gained), (40, 30));
    assert_eq!(me.samples, 5);
}

#[test]
fn speed_powerslide_air_and_boost_bands() {
    let tick = |boost: i64, extra: Value| {
        let me = with(player(1, "Me", 0, 0, Some(boost)), extra);
        state(
            11,
            100,
            pair(me, player(2, "Mate", 0, 0, Some(10))),
            "Me",
            [0, 0],
        )
    };
    let m = finish(vec![
        event("MatchCreated", guid()),
        event("RoundStarted", guid()),
        tick(
            0,
            json!({"Speed": 1000.0, "bOnGround": true, "bOnWall": false, "bPowersliding": true}),
        ),
        tick(
            20,
            json!({"Speed": 2300.0, "bSupersonic": true, "bOnGround": false, "bOnWall": false}),
        ),
        tick(
            50,
            json!({"Speed": 1500.0, "bOnGround": false, "bOnWall": true}),
        ),
        tick(
            90,
            json!({"Speed": 1200.0, "bOnGround": true, "bOnWall": false}),
        ),
    ]);
    let me = me_movement(&m);
    assert!((me.avg_speed - 1500.0).abs() < 1e-9);
    assert_eq!(me.max_speed, 2300.0);
    for (name, got, want) in [
        ("empty", me.pct_boost_empty, 25.0),
        ("low", me.pct_boost_low, 25.0),
        ("mid", me.pct_boost_mid, 25.0),
        ("high", me.pct_boost_high, 25.0),
        ("supersonic", me.pct_supersonic, 25.0),
        ("powerslide", me.pct_powersliding, 25.0),
        ("ground", me.pct_on_ground, 50.0),
        ("wall", me.pct_on_wall, 25.0),
        ("airborne", me.pct_airborne, 25.0),
    ] {
        assert!((got - want).abs() < 1e-9, "{name}: {got}");
    }
}

#[test]
fn demolished_time_and_count_and_no_fake_pickup_on_respawn() {
    let alive = |boost: i64| {
        state(
            11,
            100,
            pair(
                player(1, "Me", 0, 0, Some(boost)),
                player(2, "Mate", 0, 0, Some(50)),
            ),
            "Me",
            [0, 0],
        )
    };
    let dead = || {
        let me = with(
            player(1, "Me", 0, 0, Some(0)),
            json!({"bDemolished": true, "bHasCar": false}),
        );
        state(
            11,
            100,
            pair(me, player(2, "Mate", 0, 0, Some(50))),
            "Me",
            [0, 0],
        )
    };
    let m = finish(vec![
        event("MatchCreated", guid()),
        event("RoundStarted", guid()),
        alive(50),
        dead(),
        dead(),
        alive(33), // respawn with the starting boost
    ]);
    let me = me_movement(&m);
    assert_eq!(me.times_demolished, 1);
    assert!((me.pct_demolished - 50.0).abs() < 1e-9);
    assert_eq!((me.boost_used, me.boost_gained), (0, 0));
}

#[test]
fn possession_ignores_untouched_ball_and_uses_my_team() {
    let tick = |team: i64, speed: f64| {
        state_ball(
            11,
            100,
            pair(
                player(1, "Me", 0, 0, Some(50)),
                player(2, "Mate", 0, 0, Some(50)),
            ),
            "Me",
            [0, 0],
            (speed, team),
        )
    };
    let m = finish(vec![
        event("MatchCreated", guid()),
        event("RoundStarted", guid()),
        tick(255, 0.0),
        tick(1, 1000.0),
        tick(1, 2000.0),
        tick(0, 3000.0),
    ]);
    assert!((m.my_possession_pct.unwrap() - 100.0 / 3.0).abs() < 1e-9);
    assert!((m.ball_avg_speed.unwrap() - 1500.0).abs() < 1e-9);
    assert_eq!(m.ball_max_speed, Some(3000.0));
}

#[test]
fn touches_are_split_in_thirds_from_each_teams_point_of_view() {
    let hit = |name: &str, shortcut: i64, team: i64, y: f64, speed: f64| {
        event(
            "BallHit",
            json!({
                "MatchGuid": "G1",
                "Players": [{"Name": name, "Shortcut": shortcut, "TeamNum": team}],
                "Ball": {"PreHitSpeed": 0.0, "PostHitSpeed": speed, "Location": {"X": 0.0, "Y": y, "Z": 90.0}}
            }),
        )
    };
    let mut events = two_v_two(11);
    events.extend([
        hit("Me", 1, 0, 3000.0, 1000.0),  // blue attacks +Y: attacking third
        hit("Me", 1, 0, -3000.0, 2000.0), // defensive third
        hit("Me", 1, 0, 0.0, 3000.0),     // middle
        hit("Foe1", 3, 1, 3000.0, 500.0), // orange attacks -Y: +Y is their defensive third
        event("GoalReplayStart", guid()),
        hit("Me", 1, 0, 0.0, 9999.0), // during a replay: ignored
    ]);
    let m = finish(events);
    let me = m.touches.iter().find(|t| t.slot == m.my_slot).unwrap();
    assert_eq!(
        (me.touches, me.def_third, me.mid_third, me.att_third),
        (3, 1, 1, 1)
    );
    assert!((me.avg_post_speed - 2000.0).abs() < 1e-9);
    assert_eq!(me.max_post_speed, 3000.0);
    let foe = m.touches.iter().find(|t| t.slot != m.my_slot).unwrap();
    assert_eq!((foe.touches, foe.def_third, foe.att_third), (1, 1, 0));
}

#[test]
fn stat_events_remember_boost_and_speed_and_average_them() {
    let tick = |boost: i64, speed: f64| {
        state(
            11,
            100,
            pair(
                with(player(1, "Me", 0, 0, Some(boost)), json!({"Speed": speed})),
                player(2, "Mate", 0, 0, Some(50)),
            ),
            "Me",
            [0, 0],
        )
    };
    let save = || {
        event(
            "StatfeedEvent",
            json!({"MatchGuid": "G1", "EventName": "Save", "Type": "Save",
                   "MainTarget": {"Name": "Me", "Shortcut": 1, "TeamNum": 0}}),
        )
    };
    let m = finish(vec![
        event("MatchCreated", guid()),
        event("RoundStarted", guid()),
        tick(20, 1000.0),
        save(),
        tick(40, 1400.0),
        save(),
    ]);
    assert_eq!(m.stat_events.len(), 2);
    assert_eq!(m.stat_events[0].boost, Some(20));
    let stat = m.event_stats.iter().find(|e| e.event == "Save").unwrap();
    assert_eq!(stat.slot, m.my_slot);
    assert_eq!(stat.count, 2);
    assert!((stat.avg_boost.unwrap() - 30.0).abs() < 1e-9);
    assert!((stat.avg_speed.unwrap() - 1200.0).abs() < 1e-9);
}

#[test]
fn goal_rows_carry_the_scorers_boost() {
    let m = finish({
        let mut events = two_v_two(11);
        events.push(event(
            "GoalScored",
            json!({"MatchGuid": "G1", "GoalSpeed": 80.0, "GoalTime": 10.0,
                   "ImpactLocation": {"X": 0.0, "Y": 5000.0, "Z": 100.0},
                   "Scorer": {"Name": "Me", "Shortcut": 1, "TeamNum": 0},
                   "BallLastTouch": {"Player": {"Name": "Me", "Shortcut": 1, "TeamNum": 0}, "Speed": 90.0}}),
        ));
        events
    });
    assert_eq!(m.goals.len(), 1);
    assert_eq!(m.goals[0].scorer_boost, Some(0)); // the last live tick had boost 0
}

#[test]
fn crossbar_and_pickups_are_counted_not_listed() {
    let bar = |impact: f64| {
        event(
            "CrossbarHit",
            json!({"MatchGuid": "G1", "BallLocation": {"X": 0.0, "Y": 5000.0, "Z": 320.0},
                   "BallSpeed": 800.0, "ImpactForce": impact,
                   "BallLastTouch": {"Player": {"Name": "Me", "Shortcut": 1, "TeamNum": 0}, "Speed": 100.0}}),
        )
    };
    let pickup = |kind: &str, amount: f64, replay: bool| {
        event(
            "BoostPickup",
            json!({"MatchGuid": "G1", "Player": {"Name": "Me", "Shortcut": 1, "TeamNum": 0},
                   "Location": {"X": 0.0, "Y": 0.0, "Z": 70.0}, "BoostAmount": amount,
                   "BoostType": kind, "bReplay": replay}),
        )
    };
    let mut events = two_v_two(11);
    events.extend([
        bar(100.0),
        bar(300.0),
        pickup("BoostType_Pad", 12.0, false),
        pickup("BoostType_Pill", 100.0, false),
        pickup("BoostType_Pad", 12.0, true), // replay: ignored
    ]);
    let m = finish(events);
    assert_eq!((m.crossbar[0].count, m.crossbar[0].max_impact), (2, 300.0));
    let p = &m.pickups[0];
    assert_eq!((p.pads, p.pills, p.amount), (1, 1, 112.0));
}

#[test]
fn both_of_us_empty_is_measured_as_overlap() {
    let tick = |me: i64, mate: i64| {
        state(
            11,
            100,
            pair(
                player(1, "Me", 0, 0, Some(me)),
                player(2, "Mate", 0, 0, Some(mate)),
            ),
            "Me",
            [0, 0],
        )
    };
    let m = finish(vec![
        event("MatchCreated", guid()),
        event("RoundStarted", guid()),
        tick(0, 0),  // both empty
        tick(0, 80), // only me
        tick(70, 0), // only mate
        tick(60, 60),
    ]);
    assert!((m.team_empty_overlap.unwrap() - 25.0).abs() < 1e-9);
}

#[test]
fn a_missing_boost_key_means_zero_boost_but_opponents_are_not_visible() {
    // Real game behavior: "Boost" is omitted at 0. Opponents have no movement fields at all.
    let mut me = player(1, "Me", 0, 0, Some(40));
    let mut mate = player(2, "Mate", 0, 0, Some(40));
    me.as_object_mut().unwrap().remove("Boost");
    me.as_object_mut().unwrap().remove("bBoosting");
    mate["Boost"] = json!(80);
    let tick = state(11, 100, pair(me, mate), "Me", [0, 0]);
    let m = finish(vec![
        event("MatchCreated", guid()),
        event("RoundStarted", guid()),
        tick.clone(),
        tick,
    ]);
    let me = me_movement(&m);
    assert_eq!(me.samples, 2);
    assert!((me.pct_boost_empty - 100.0).abs() < 1e-9);
    assert!((me.avg_boost - 0.0).abs() < 1e-9);
    assert_eq!(
        m.movement.len(),
        2,
        "only you and your teammate; opponents have no data"
    );
    // Both of us at zero... except the mate has 80, so no overlap.
    assert!((m.team_empty_overlap.unwrap() - 0.0).abs() < 1e-9);
}
