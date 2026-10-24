//! Saves matches (from the real fixtures) into an in-memory database and reads them back.

use reclutched_core::rocketleague::parse_message;
use reclutched_core::rocketleague::settings::Settings;
use reclutched_core::rocketleague::storage::{Store, StoreError};
use reclutched_core::rocketleague::tracker::{FinishedMatch, Tracker};

fn real_match() -> FinishedMatch {
    let path = format!(
        "{}/tests/fixtures/casual_2v2.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut tracker = Tracker::new(Settings::default());
    for line in std::fs::read_to_string(path).unwrap().lines() {
        tracker.on_event_at(&parse_message(line).unwrap(), 1_000);
    }
    tracker
        .finish_pending(1_300)
        .expect("fixture is a full match")
}

fn count(store: &Store, table: &str) -> i64 {
    store
        .connection()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn saved_match_can_be_read_back() {
    let m = real_match();
    let mut store = Store::open_in_memory().unwrap();
    let id = store.save_match(&m).unwrap().expect("first save inserts");

    assert_eq!(count(&store, "matches"), 1);
    assert_eq!(count(&store, "match_players"), m.players.len() as i64);
    assert_eq!(count(&store, "goals"), m.goals.len() as i64);
    assert_eq!(count(&store, "stat_events"), m.stat_events.len() as i64);
    assert_eq!(count(&store, "player_touches"), m.touches.len() as i64);
    assert_eq!(
        count(&store, "player_event_stats"),
        m.event_stats.len() as i64
    );
    assert_eq!(count(&store, "player_movement"), m.movement.len() as i64);

    let rows = store.recent_matches(10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].result, "win");
    assert_eq!(rows[0].playlist_id, 3);
    assert_eq!(store.abandon_counts().unwrap(), (1, 0));

    // Real accounts are remembered, bots and guests ("Unknown|0|0") are not.
    let bad: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM players WHERE primary_id LIKE 'Unknown%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bad, 0);
}

#[test]
fn saving_the_same_online_match_twice_is_ignored() {
    let m = real_match();
    let mut store = Store::open_in_memory().unwrap();
    assert!(store.save_match(&m).unwrap().is_some());
    assert!(store.save_match(&m).unwrap().is_none());
    assert_eq!(count(&store, "matches"), 1);
    assert_eq!(
        count(&store, "goals"),
        m.goals.len() as i64,
        "no rows from the second try"
    );
}

#[test]
fn deleting_a_match_deletes_its_rows() {
    let mut store = Store::open_in_memory().unwrap();
    store.save_match(&real_match()).unwrap();
    store
        .connection()
        .execute("DELETE FROM matches", [])
        .unwrap();
    for table in [
        "match_players",
        "goals",
        "stat_events",
        "player_touches",
        "player_event_stats",
        "player_crossbar",
        "player_pickups",
        "player_movement",
    ] {
        assert_eq!(count(&store, table), 0, "{table}");
    }
}

#[test]
fn file_database_reopens_and_refuses_a_newer_schema() {
    let dir = std::env::temp_dir().join(format!("reclutched-db-{}", std::process::id()));
    let path = dir.join("matches.db");
    {
        let mut store = Store::open(&path).unwrap();
        store.save_match(&real_match()).unwrap();
    }
    // Reopening keeps the data and does not recreate tables.
    let store = Store::open(&path).unwrap();
    assert_eq!(count(&store, "matches"), 1);
    let version: i64 = store
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 3);
    drop(store);

    // A database from a future version must not be touched.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);
    assert!(matches!(
        Store::open(&path),
        Err(StoreError::TooNew { found: 99, .. })
    ));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_match_is_small_on_disk() {
    let dir = std::env::temp_dir().join(format!("reclutched-size-{}", std::process::id()));
    let path = dir.join("matches.db");
    let mut store = Store::open(&path).unwrap();
    store.save_match(&real_match()).unwrap();
    store
        .connection()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    // The empty schema alone is a few dozen KB (one page per table and index); one match
    // must not add anything remotely like the raw 13 MB capture.
    assert!(size < 200_000, "database is {size} bytes");
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn match_detail_reads_back_what_was_saved() {
    let m = real_match();
    let mut store = Store::open_in_memory().unwrap();
    assert_eq!(store.latest_match_id().unwrap(), None);
    assert!(store.match_detail(1).unwrap().is_none());

    let id = store.save_match(&m).unwrap().unwrap();
    assert_eq!(store.latest_match_id().unwrap(), Some(id));
    let d = store.match_detail(id).unwrap().unwrap();

    assert_eq!(d.row.result, "win");
    assert_eq!(d.my_slot, m.my_slot);
    assert_eq!(d.players.len(), m.players.len());
    assert!(d.players[m.my_slot].is_me);
    assert_eq!(d.movement.len(), m.movement.len());
    assert_eq!(d.touches.len(), m.touches.len());
    assert_eq!(d.event_stats.len(), m.event_stats.len());
    assert_eq!(d.goals.len(), m.goals.len());
    assert_eq!(d.stat_events.len(), m.stat_events.len());
    if let (Some(a), Some(b)) = (d.goals.first(), m.goals.first()) {
        assert_eq!(
            (a.scorer, a.assister, a.clock),
            (b.scorer, b.assister, b.clock)
        );
        assert_eq!(a.score_after, b.score_after);
        assert_eq!(a.scorer_boost, b.scorer_boost);
    }
    assert_eq!(d.my_possession_pct, m.my_possession_pct);
    // Whole-number storage: speeds come back rounded.
    assert_eq!(d.ball_max_speed, m.ball_max_speed.map(|v| v.round() as i64));
    if let (Some(a), Some(b)) = (d.movement.first(), m.movement.first()) {
        assert!((a.avg_boost - b.avg_boost).abs() < 1e-9);
        assert_eq!(a.times_demolished, b.times_demolished);
    }
}

#[test]
fn my_movement_averages_need_a_few_matches_and_use_only_my_rows() {
    let mut base = real_match();
    // The fixture is a few seconds long; a real match has thousands of samples.
    for m in &mut base.movement {
        m.samples = 5_000;
    }
    let mine = base
        .movement
        .iter()
        .find(|m| m.slot == base.my_slot)
        .expect("the fixture has movement data for me")
        .clone();
    let mut store = Store::open_in_memory().unwrap();
    for i in 1..=3 {
        assert_eq!(
            store.my_movement_averages(20).unwrap(),
            None,
            "{} saved matches are too few",
            i - 1
        );
        let mut m = base.clone();
        m.guid = Some(format!("guid-{i}"));
        m.ended_at += i;
        store.save_match(&m).unwrap().expect("a new match");
    }
    // Only the samples of my own row count (the fixture's other players have movement rows too).
    let avg = store
        .my_movement_averages(20)
        .unwrap()
        .expect("three matches");
    assert_eq!(avg.matches, 3);
    assert!((avg.avg_boost - mine.avg_boost).abs() < 1e-9);
    assert!((avg.pct_supersonic - mine.pct_supersonic).abs() < 1e-9);
}

#[test]
fn recent_form_counts_results_streaks_and_averages() {
    use reclutched_core::rocketleague::storage::current_streak;
    use reclutched_core::rocketleague::tracker::MatchResult;

    let mut store = Store::open_in_memory().unwrap();
    assert_eq!(store.recent_form(20).unwrap(), None, "no matches yet");

    let base = real_match();
    let me = base
        .players
        .iter()
        .find(|p| p.slot == base.my_slot)
        .unwrap()
        .clone();
    // Oldest to newest: loss, win, win, left early, win, win.
    let results = [
        MatchResult::Loss,
        MatchResult::Win,
        MatchResult::Win,
        MatchResult::Abandoned,
        MatchResult::Win,
        MatchResult::Win,
    ];
    for (i, result) in results.into_iter().enumerate() {
        let mut m = base.clone();
        m.guid = Some(format!("form-{i}"));
        m.ended_at += i as i64;
        m.result = result;
        store.save_match(&m).unwrap().expect("a new match");
    }

    let form = store.recent_form(20).unwrap().expect("matches exist");
    assert_eq!(form.matches, 6);
    assert_eq!((form.wins, form.losses, form.abandoned), (4, 1, 1));
    // The two newest are wins; the left-early match ends the run behind them.
    assert_eq!(form.streak, Some(("win".to_string(), 2)));
    // Left-early matches are not part of the averages; the others are all the same match.
    assert!((form.avg_goals - me.goals as f64).abs() < 1e-9);
    assert!((form.avg_saves - me.saves as f64).abs() < 1e-9);
    assert!((form.avg_score - me.score as f64).abs() < 1e-9);

    // Only the last `n` matches count.
    let last3 = store.recent_form(3).unwrap().unwrap();
    assert_eq!((last3.matches, last3.wins, last3.abandoned), (3, 2, 1));

    assert_eq!(current_streak(&[]), None);
    assert_eq!(current_streak(&["abandoned", "win"]), None);
    assert_eq!(
        current_streak(&["loss", "loss", "loss", "win"]),
        Some(("loss".to_string(), 3))
    );
}

#[test]
fn periods_group_by_local_day_month_and_year_and_keep_every_match() {
    use reclutched_core::rocketleague::dates::Day;
    use reclutched_core::rocketleague::periods::Grain;
    use reclutched_core::rocketleague::tracker::MatchResult;

    let at = |day: &str, hour: i64, minute: i64| {
        Day::parse(day).unwrap().to_days() * 86_400 + hour * 3_600 + minute * 60
    };
    let base = real_match();
    let me = base
        .players
        .iter()
        .find(|p| p.slot == base.my_slot)
        .unwrap()
        .clone();
    let mut store = Store::open_in_memory().unwrap();
    // (end time, length in seconds, result)
    let plays = [
        (at("2026-10-22", 12, 0), 1_200, MatchResult::Win),
        (at("2026-10-22", 13, 0), 600, MatchResult::Loss),
        (at("2026-10-23", 12, 0), 900, MatchResult::Win),
        // 23:30 UTC on the last day of September: already October 1st at UTC+2.
        (at("2026-09-30", 23, 30), 300, MatchResult::Win),
        (at("2025-12-31", 12, 0), 5 * 3_600, MatchResult::Abandoned),
    ];
    for (i, (ended, length, result)) in plays.into_iter().enumerate() {
        let mut m = base.clone();
        m.guid = Some(format!("period-{i}"));
        m.ended_at = ended;
        m.started_at = ended - length;
        m.result = result;
        store.save_match(&m).unwrap().expect("a new match");
    }

    // In UTC: the two matches of the 22nd are one day, newest day first.
    let days = store.period_summaries(Grain::Day, 0).unwrap();
    let keys: Vec<_> = days.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(
        keys,
        ["2026-10-23", "2026-10-22", "2026-09-30", "2025-12-31"]
    );
    let the22 = &days[1];
    assert_eq!((the22.matches, the22.wins, the22.losses), (2, 1, 1));
    assert_eq!(the22.seconds_played, 1_800);
    assert_eq!(the22.win_rate(), Some(50.0));
    assert!((the22.avg_goals - me.goals as f64).abs() < 1e-9);

    // A match counts for at most an hour, and a left-early match has no averages.
    let left_early = days.last().unwrap();
    assert_eq!((left_early.matches, left_early.abandoned), (1, 1));
    assert_eq!(left_early.seconds_played, 3_600);
    assert_eq!(left_early.avg_score, 0.0);
    assert_eq!(left_early.win_rate(), None);

    // Months and years add the days up.
    let months = store.period_summaries(Grain::Month, 0).unwrap();
    assert_eq!(
        months
            .iter()
            .map(|m| (m.key.as_str(), m.matches))
            .collect::<Vec<_>>(),
        [("2026-10", 3), ("2026-09", 1), ("2025-12", 1)]
    );
    let years = store.period_summaries(Grain::Year, 0).unwrap();
    assert_eq!(
        years
            .iter()
            .map(|y| (y.key.as_str(), y.matches))
            .collect::<Vec<_>>(),
        [("2026", 4), ("2025", 1)]
    );

    // The time zone moves the late September match into October.
    let october = store.period_summary("2026-10", 2 * 3_600).unwrap().unwrap();
    assert_eq!(october.matches, 4);
    assert_eq!(store.period_summary("2026-09", 2 * 3_600).unwrap(), None);

    // Every match is still there, newest first, for any period.
    let on_the_22nd = store.matches_in_period("2026-10-22", 0).unwrap();
    assert_eq!(on_the_22nd.len(), 2);
    assert!(on_the_22nd[0].ended_at > on_the_22nd[1].ended_at);
    assert_eq!(store.matches_in_period("2026-10", 0).unwrap().len(), 3);
    assert_eq!(store.matches_in_period("2026", 0).unwrap().len(), 4);
    assert!(store.matches_in_period("nonsense", 0).unwrap().is_empty());
    assert_eq!(store.period_summary("2030-01-01", 0).unwrap(), None);
    assert_eq!(count(&store, "matches"), 5, "summarising deletes nothing");
}

#[test]
fn wiping_a_period_removes_its_matches_and_everything_attached() {
    use reclutched_core::rocketleague::dates::Day;
    let at = |day: &str| Day::parse(day).unwrap().to_days() * 86_400 + 12 * 3_600;
    let base = real_match();
    let mut store = Store::open_in_memory().unwrap();
    for (i, day) in ["2026-10-23", "2026-10-22", "2026-09-30", "2025-12-31"]
        .into_iter()
        .enumerate()
    {
        let mut m = base.clone();
        m.guid = Some(format!("wipe-{i}"));
        m.ended_at = at(day);
        m.started_at = m.ended_at - 300;
        store.save_match(&m).unwrap().expect("a new match");
    }
    let players = m_players(&store);

    assert_eq!(store.delete_period("2026-10-23", 0).unwrap(), 1);
    assert_eq!(count(&store, "matches"), 3);
    assert_eq!(
        count(&store, "match_players"),
        3 * base.players.len() as i64
    );
    // October is the 22nd now.
    assert_eq!(store.delete_period("2026-10", 0).unwrap(), 1);
    assert_eq!(store.delete_period("2026-10", 0).unwrap(), 0);
    assert_eq!(count(&store, "matches"), 2);
    // A whole year, and a key that means nothing.
    assert_eq!(store.delete_period("2026", 0).unwrap(), 1);
    assert_eq!(store.delete_period("nonsense", 0).unwrap(), 0);
    assert_eq!(count(&store, "matches"), 1);
    // The player names of the last match are still remembered, those of deleted ones are not
    // (here they are the same people, so they stay).
    assert_eq!(count(&store, "players"), players);

    assert_eq!(store.delete_everything().unwrap(), 1);
    for table in [
        "matches",
        "match_players",
        "goals",
        "player_touches",
        "players",
    ] {
        assert_eq!(count(&store, table), 0, "{table}");
    }
}

fn m_players(store: &Store) -> i64 {
    count(store, "players")
}

#[test]
fn clips_are_remembered_attached_to_a_match_and_survive_its_deletion() {
    use reclutched_core::rocketleague::storage::NewClip;
    let mut store = Store::open_in_memory().unwrap();
    let id = store.save_match(&real_match()).unwrap().expect("saved");
    let clip = |path: &str, kind: &str, at: i64| NewClip {
        path: path.into(),
        kind: kind.into(),
        trigger: "key".into(),
        taken_at: at,
        seconds: 30,
        thumb: None,
    };
    let a = store.add_clip(&clip("/c/a.mp4", "clip", 100)).unwrap();
    let b = store
        .add_clip(&clip("/c/b.png", "screenshot", 200))
        .unwrap();
    let outside = store.add_clip(&clip("/c/c.mp4", "clip", 300)).unwrap();
    assert!(store.clips_for_match(id).unwrap().is_empty());
    store.attach_clips(&[a, b], id).unwrap();
    let mine = store.clips_for_match(id).unwrap();
    assert_eq!(
        mine.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        ["/c/a.mp4", "/c/b.png"]
    );
    assert_eq!(store.recent_clips(2).unwrap()[0].id, outside);
    // Deleting the match keeps the entries, just no longer tied to it.
    store
        .connection()
        .execute("DELETE FROM matches WHERE id = ?1", [id])
        .unwrap();
    assert_eq!(store.recent_clips(10).unwrap().len(), 3);
    assert!(store
        .recent_clips(10)
        .unwrap()
        .iter()
        .all(|c| c.match_id.is_none()));
    store.forget_clip(a).unwrap();
    assert_eq!(store.recent_clips(10).unwrap().len(), 2);
    // Wiping everything also forgets them.
    store.delete_everything().unwrap();
    assert!(store.recent_clips(10).unwrap().is_empty());
}
