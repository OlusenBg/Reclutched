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
    assert_eq!(version, 2);
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
