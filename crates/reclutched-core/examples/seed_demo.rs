//! Writes a few made-up matches into a database, so the app can be tried without playing.
//!
//!     cargo run -p reclutched-core --example seed_demo -- /tmp/demo.db
//!     cargo run -p reclutched-core --example seed_demo -- /tmp/demo.db 500   # plus 500 more
//!     cargo run -p reclutched-core --example seed_demo -- /tmp/demo.db history   # 14 months of play
//!
//! Everything here is invented (names, numbers); it goes through the real `Store`, so it also
//! checks that the schema accepts realistic data.

use reclutched_core::rocketleague::storage::Store;
use reclutched_core::rocketleague::tracker::*;

fn clock(remaining: i64) -> Clock {
    Clock {
        remaining,
        overtime: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn player(
    slot: usize,
    name: &str,
    team: i64,
    is_me: bool,
    stats: [i64; 8],
    left: Option<Clock>,
) -> PlayerSummary {
    let [score, goals, shots, assists, saves, touches, car_touches, demos] = stats;
    PlayerSummary {
        slot,
        name: name.to_string(),
        primary_id: Some(format!("Epic|demo{slot}|0")),
        team,
        is_me,
        score,
        goals,
        shots,
        assists,
        saves,
        touches,
        car_touches,
        demos,
        joined: None,
        left,
    }
}

fn movement(slot: usize, boost: f64, empty: f64, speed: f64) -> MovementSummary {
    MovementSummary {
        slot,
        samples: 2400,
        avg_boost: boost,
        pct_boost_empty: empty,
        pct_boost_low: 22.0,
        pct_boost_mid: 28.0,
        pct_boost_high: 100.0 - empty - 50.0,
        pct_boosting: 21.0,
        boost_used: 1500 + slot as i64 * 90,
        boost_gained: 1480 + slot as i64 * 100,
        avg_speed: speed,
        max_speed: 83.0,
        pct_supersonic: 9.0 + slot as f64,
        pct_powersliding: 1.5,
        pct_on_ground: 62.0,
        pct_on_wall: 6.0,
        pct_airborne: 32.0,
        pct_demolished: 1.0,
        times_demolished: slot as i64 % 2,
    }
}

fn touches(slot: usize, n: i64) -> TouchSummary {
    TouchSummary {
        slot,
        touches: n,
        def_third: n * 3 / 10,
        mid_third: n * 4 / 10,
        att_third: n - n * 3 / 10 - n * 4 / 10,
        avg_post_speed: 62.0 + slot as f64 * 4.0,
        max_post_speed: 130.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn base(
    guid: &str,
    at: i64,
    playlist: i64,
    arena: &str,
    result: MatchResult,
    goals: [i64; 2],
    overtime: bool,
    players: Vec<PlayerSummary>,
) -> FinishedMatch {
    let my_slot = players.iter().position(|p| p.is_me).unwrap_or(0);
    FinishedMatch {
        guid: Some(guid.to_string()),
        playlist_id: playlist,
        arena: arena.to_string(),
        started_at: at - 330,
        ended_at: at,
        my_slot,
        my_team: players[my_slot].team,
        team_goals: goals,
        overtime,
        result,
        abandoned_clock: None,
        my_possession_pct: Some(52.0),
        ball_avg_speed: Some(58.0),
        ball_max_speed: Some(171.0),
        team_empty_overlap: Some(7.0),
        players,
        goals: vec![],
        stat_events: vec![],
        event_stats: vec![],
        touches: vec![],
        crossbar: vec![],
        pickups: vec![],
        movement: vec![],
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: seed_demo <database file>");
    let _ = std::fs::remove_file(&path);
    let mut store = Store::open(std::path::Path::new(&path)).expect("open the database");

    // "now" minus a few hours/days, so the app shows different dates.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // 1. A 3v3 win with goals, assists and full movement data.
    let mut m = base(
        "DEMO0000000000000000000000000001",
        now - 20 * 60,
        3,
        "Stadium_P",
        MatchResult::Win,
        [4, 2],
        false,
        vec![
            player(0, "Zed", 0, true, [640, 2, 6, 1, 2, 41, 20, 1], None),
            player(1, "Nova", 0, false, [410, 1, 4, 1, 3, 33, 15, 0], None),
            player(2, "Pixel", 0, false, [300, 1, 3, 0, 1, 28, 12, 2], None),
            player(3, "Rook", 1, false, [350, 1, 5, 1, 2, 36, 18, 1], None),
            player(
                4,
                "Kite",
                1,
                false,
                [220, 1, 3, 0, 4, 30, 14, 0],
                Some(clock(35)),
            ),
            player(5, "Moth", 1, false, [180, 0, 2, 1, 1, 22, 9, 1], None),
        ],
    );
    let goal = |remaining, scorer, assister, speed, boost, after: [i64; 2]| GoalRow {
        clock: clock(remaining),
        scorer,
        assister,
        goal_speed: speed,
        last_touch_speed: speed + 8.0,
        x: 120.0,
        y: if scorer < 3 { 5120.0 } else { -5120.0 },
        z: 150.0,
        scorer_boost: Some(boost),
        scorer_speed: Some(61.0),
        score_after: Some(after),
    };
    m.goals = vec![
        goal(262, 0, Some(1), 74.0, 38, [1, 0]),
        goal(231, 3, Some(5), 66.0, 12, [1, 1]),
        goal(160, 1, Some(0), 81.0, 55, [2, 1]),
        goal(96, 4, None, 59.0, 0, [2, 2]),
        goal(70, 0, None, 88.0, 71, [3, 2]),
        goal(21, 2, Some(0), 70.0, 24, [4, 2]),
    ];
    let ev = |remaining, name: &str, main, boost| StatEventRow {
        clock: clock(remaining),
        name: name.to_string(),
        main,
        secondary: None,
        boost: Some(boost),
        speed: Some(48.0),
    };
    m.stat_events = vec![
        ev(280, "Shot", 0, 60),
        ev(255, "Save", 0, 9),
        ev(150, "Save", 0, 14),
        ev(120, "Shot", 0, 33),
    ];
    m.event_stats = vec![
        EventStat {
            slot: 0,
            event: "Save".into(),
            count: 2,
            avg_boost: Some(11.5),
            avg_speed: Some(46.0),
        },
        EventStat {
            slot: 0,
            event: "Shot".into(),
            count: 6,
            avg_boost: Some(48.0),
            avg_speed: Some(55.0),
        },
    ];
    m.movement = vec![
        movement(0, 46.0, 16.0, 49.0),
        movement(1, 38.0, 22.0, 51.0),
        movement(2, 55.0, 11.0, 47.0),
    ];
    m.touches = (0..6).map(|s| touches(s, 41 - s as i64 * 4)).collect();
    m.crossbar = vec![CrossbarSummary {
        slot: 0,
        count: 1,
        max_impact: 120.0,
    }];
    store.save_match(&m).unwrap();

    // 2. A 1v1 loss, a bit worse numbers.
    let mut m = base(
        "DEMO0000000000000000000000000002",
        now - 3 * 3600,
        1,
        "Mannfield_P",
        MatchResult::Loss,
        [1, 3],
        false,
        vec![
            player(0, "Zed", 0, true, [210, 1, 4, 0, 1, 24, 11, 0], None),
            player(1, "Vex", 1, false, [480, 3, 7, 0, 2, 27, 13, 1], None),
        ],
    );
    m.my_possession_pct = Some(44.0);
    m.team_empty_overlap = None;
    m.goals = vec![
        goal(240, 1, None, 70.0, 20, [0, 1]),
        goal(180, 0, None, 65.0, 40, [1, 1]),
    ];
    m.movement = vec![movement(0, 38.0, 24.0, 47.0)];
    m.touches = vec![touches(0, 24), touches(1, 27)];
    store.save_match(&m).unwrap();

    // 3. Overtime win.
    let mut m = base(
        "DEMO0000000000000000000000000003",
        now - 26 * 3600,
        11,
        "Stadium_P",
        MatchResult::Win,
        [3, 2],
        true,
        vec![
            player(0, "Zed", 0, true, [520, 2, 5, 0, 1, 30, 14, 1], None),
            player(1, "Nova", 0, false, [300, 1, 3, 1, 2, 26, 12, 0], None),
            player(2, "Rook", 1, false, [330, 1, 4, 1, 1, 28, 13, 1], None),
            player(3, "Kite", 1, false, [310, 1, 3, 0, 2, 25, 10, 0], None),
        ],
    );
    m.movement = vec![movement(0, 52.0, 12.0, 50.0), movement(1, 47.0, 15.0, 49.0)];
    m.touches = (0..4).map(|s| touches(s, 30 - s as i64 * 2)).collect();
    store.save_match(&m).unwrap();

    // 4. A match left early.
    let mut m = base(
        "DEMO0000000000000000000000000004",
        now - 2 * 86400,
        13,
        "Mannfield_P",
        MatchResult::Abandoned,
        [1, 2],
        false,
        vec![
            player(0, "Zed", 0, true, [150, 1, 2, 0, 0, 12, 6, 0], None),
            player(1, "Nova", 0, false, [90, 0, 1, 0, 1, 10, 5, 0], None),
            player(2, "Pixel", 0, false, [60, 0, 1, 0, 0, 8, 4, 1], None),
            player(3, "Rook", 1, false, [230, 2, 4, 0, 1, 15, 7, 0], None),
            player(4, "Kite", 1, false, [100, 0, 1, 1, 2, 11, 5, 0], None),
            player(5, "Moth", 1, false, [80, 0, 1, 0, 0, 9, 4, 0], None),
        ],
    );
    m.abandoned_clock = Some(clock(132));
    store.save_match(&m).unwrap();

    // 5-7: more history so the list scrolls.
    for (i, (playlist, result, goals)) in [
        (3, MatchResult::Win, [3, 1]),
        (3, MatchResult::Loss, [0, 2]),
        (11, MatchResult::Win, [2, 1]),
    ]
    .into_iter()
    .enumerate()
    {
        let m = base(
            &format!("DEMO00000000000000000000000000{:02}", 10 + i),
            now - (3 + i as i64) * 86400,
            playlist,
            "Stadium_P",
            result,
            goals,
            false,
            vec![
                player(0, "Zed", 0, true, [400, 1, 4, 1, 1, 30, 14, 0], None),
                player(1, "Nova", 0, false, [250, 1, 2, 0, 1, 20, 10, 1], None),
                player(2, "Rook", 1, false, [300, 1, 3, 1, 2, 25, 12, 0], None),
                player(3, "Kite", 1, false, [200, 0, 2, 0, 2, 22, 11, 0], None),
            ],
        );
        store.save_match(&m).unwrap();
    }
    // 8. Awkward data on purpose: long and unusual names, and a match without stored names or
    //    goals (as if those switches had been off).
    let mut m = base(
        "DEMO0000000000000000000000000020",
        now - 8 * 86400,
        3,
        "Stadium_P",
        MatchResult::Win,
        [2, 1],
        false,
        vec![
            player(0, "Zed", 0, true, [500, 2, 3, 0, 1, 20, 9, 0], None),
            player(
                1,
                "A_Very_Long_Gamertag_That_Goes_On_And_On_1234567890",
                0,
                false,
                [200, 0, 1, 1, 1, 15, 7, 0],
                None,
            ),
            player(
                2,
                "\u{0418}\u{0433}\u{0440}\u{0438}\u{0441} \u{30A2}\u{30EA}\u{30B9} \u{1F680}",
                1,
                false,
                [300, 1, 2, 0, 2, 18, 8, 1],
                None,
            ),
            player(
                3,
                "<b>Markup & \"quotes\"</b>",
                1,
                false,
                [150, 0, 1, 0, 0, 12, 6, 0],
                None,
            ),
        ],
    );
    m.goals = vec![goal(200, 1, Some(0), 70.0, 33, [1, 0])];
    store.save_match(&m).unwrap();
    let mut m = base(
        "DEMO0000000000000000000000000021",
        now - 9 * 86400,
        3,
        "Stadium_P",
        MatchResult::Loss,
        [0, 1],
        false,
        vec![
            player(0, "", 0, true, [0; 8], None),
            player(1, "", 1, false, [0; 8], None),
        ],
    );
    m.my_possession_pct = None;
    m.ball_avg_speed = None;
    m.ball_max_speed = None;
    m.team_empty_overlap = None;
    store.save_match(&m).unwrap();

    // `history`: about 14 months of play, busy and quiet days, with stats that slowly improve
    // (to try the day/month/year list, the calendar and the graphs).
    if std::env::args().nth(2).as_deref() == Some("history") {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let days = 430;
        let mut n = 0;
        for d in (1..days).rev() {
            // 0 = quiet, up to 1 = a very long day.
            let roll = next();
            let count = if roll < 0.40 {
                0
            } else if roll < 0.75 {
                3 + (next() * 6.0) as i64
            } else if roll < 0.95 {
                10 + (next() * 20.0) as i64
            } else {
                60 + (next() * 40.0) as i64
            };
            // How far along the 14 months this day is: 0 at the start, 1 today.
            let progress = 1.0 - d as f64 / days as f64;
            let day_start = (now - d * 86_400) / 86_400 * 86_400 + 9 * 3_600;
            for k in 0..count {
                let length = 300 + (next() * 120.0) as i64;
                let ended = day_start + k * 520;
                let win_chance = 0.42 + 0.22 * progress;
                let result = if next() < 0.08 {
                    MatchResult::Abandoned
                } else if next() < win_chance {
                    MatchResult::Win
                } else {
                    MatchResult::Loss
                };
                let goals = (0.6 + 1.0 * progress + next() * 1.4) as i64;
                let mut m = base(
                    &format!("HIST{n:028}"),
                    ended,
                    [3, 11, 13, 1][(k % 4) as usize],
                    "Stadium_P",
                    result,
                    match result {
                        MatchResult::Win => [2 + goals % 3, goals % 2],
                        MatchResult::Loss => [goals % 2, 2 + goals % 3],
                        MatchResult::Abandoned => [1, 1],
                    },
                    k % 9 == 0,
                    vec![
                        player(
                            0,
                            "Zed",
                            0,
                            true,
                            [
                                (200.0 + 260.0 * progress + next() * 150.0) as i64,
                                goals,
                                goals + 1 + (next() * 3.0) as i64,
                                (next() * 2.0) as i64,
                                (next() * 2.0 + 0.8 * progress) as i64,
                                20,
                                9,
                                (next() * 2.0) as i64,
                            ],
                            None,
                        ),
                        player(1, "Nova", 1, false, [200, 1, 2, 0, 1, 15, 7, 0], None),
                    ],
                );
                m.started_at = ended - length;
                m.my_possession_pct = Some(46.0 + 8.0 * progress + next() * 8.0);
                m.movement = vec![movement(
                    0,
                    38.0 + 12.0 * progress + next() * 8.0,
                    30.0 - 10.0 * progress + next() * 6.0,
                    46.0 + 6.0 * progress + next() * 4.0,
                )];
                store.save_match(&m).unwrap();
                n += 1;
            }
        }
        println!("... plus {n} matches over {days} days (history)");
    }

    // Optionally lots of history, to check the list stays fast.
    if let Some(extra) = std::env::args().nth(2).and_then(|n| n.parse::<i64>().ok()) {
        for i in 0..extra {
            let result = [
                MatchResult::Win,
                MatchResult::Loss,
                MatchResult::Win,
                MatchResult::Abandoned,
            ][(i % 4) as usize];
            let m = base(
                &format!("BULK{i:028}"),
                now - (10 + i) * 3600,
                [3, 11, 13, 1][(i % 4) as usize],
                "Stadium_P",
                result,
                match result {
                    MatchResult::Win => [2 + i % 3, i % 2],
                    MatchResult::Loss => [i % 2, 2 + i % 3],
                    MatchResult::Abandoned => [1, 1],
                },
                i % 7 == 0,
                vec![
                    player(0, "Zed", 0, true, [300, 1, 3, 0, 1, 20, 9, 0], None),
                    player(1, "Nova", 1, false, [200, 1, 2, 0, 1, 15, 7, 0], None),
                ],
            );
            store.save_match(&m).unwrap();
        }
        println!("... plus {extra} bulk matches");
    }
    println!("Wrote demo matches to {path}");
}
