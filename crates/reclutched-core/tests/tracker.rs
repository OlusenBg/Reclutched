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

fn player(shortcut: i64, name: &str, team: i64, score: i64, boost: Option<i64>) -> Value {
    let mut p = json!({
        "Name": name, "PrimaryId": format!("Epic|{name}|0"), "Shortcut": shortcut,
        "TeamNum": team, "Score": score, "Goals": 0, "Shots": 0, "Assists": 0, "Saves": 0,
        "Touches": 0, "CarTouches": 0, "Demos": 0
    });
    if let Some(b) = boost {
        p["Boost"] = json!(b);
        p["bBoosting"] = json!(b > 0);
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
                "Ball": {"Speed": 0.0, "TeamNum": 255}, "bReplay": false, "bHasWinner": false,
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
    assert!(!m.ball_hits.is_empty());
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
        movement: false,
        roster: false,
        ..Settings::default()
    };
    let m = run(settings, &events).0.unwrap();
    assert!(m.goals.is_empty() && m.stat_events.is_empty() && m.ball_hits.is_empty());
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
        state(playlist, 300, players([100, 0]), "Me", [0, 0]),
        event("RoundStarted", guid()),
        state(playlist, 299, players([0, 0]), "Me", [0, 0]),
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
    assert_eq!(m.abandoned_clock.unwrap().remaining, 299);
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
    assert_eq!(foe.left.unwrap().remaining, 299);
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
