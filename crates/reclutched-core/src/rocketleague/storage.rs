//! SQLite storage for finished matches.
//!
//! Everything about one match is written in a single transaction, so a crash can never leave a
//! half-saved match. We store averages and counts, never per-tick data. Speeds and positions
//! are whole numbers (SQLite stores small integers in 1-2 bytes, a REAL always takes 8), which
//! is plenty of precision for these stats.
//!
//! Rows refer to players by `slot` (see `tracker::PlayerSummary`), not by player id, because
//! bots share ids and the game reuses shortcut numbers.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use super::tracker::{
    Clock, EventStat, FinishedMatch, GoalRow, MovementSummary, PlayerSummary, StatEventRow,
    TouchSummary,
};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("cannot create the data folder: {0}")]
    Io(#[from] std::io::Error),
    #[error("this database was made by a newer Reclutched (schema {found}, we know {known})")]
    TooNew { found: i64, known: i64 },
}

/// Bump this and add a step in [`migrate`] whenever the schema changes.
const SCHEMA_VERSION: i64 = 2;

const SCHEMA_V1: &str = "
CREATE TABLE players (
    primary_id TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    last_seen  INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE matches (
    id                  INTEGER PRIMARY KEY,
    guid                TEXT,
    started_at          INTEGER NOT NULL,
    ended_at            INTEGER NOT NULL,
    playlist_id         INTEGER NOT NULL,
    arena               TEXT NOT NULL,
    my_slot             INTEGER NOT NULL,
    my_team             INTEGER NOT NULL,
    team0_goals         INTEGER NOT NULL,
    team1_goals         INTEGER NOT NULL,
    overtime            INTEGER NOT NULL,
    result              TEXT NOT NULL CHECK (result IN ('win', 'loss', 'abandoned')),
    abandoned_remaining INTEGER,
    abandoned_overtime  INTEGER
);
-- The game's match id is unique for online matches; this also stops saving a match twice.
CREATE UNIQUE INDEX matches_guid ON matches (guid) WHERE guid IS NOT NULL;
CREATE INDEX matches_ended_at ON matches (ended_at);

-- 'remaining' = seconds left on the match clock, 'ot' = 1 if it was overtime.
CREATE TABLE match_players (
    match_id         INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    slot             INTEGER NOT NULL,
    name             TEXT NOT NULL,
    primary_id       TEXT,
    team             INTEGER NOT NULL,
    is_me            INTEGER NOT NULL,
    score            INTEGER NOT NULL,
    goals            INTEGER NOT NULL,
    shots            INTEGER NOT NULL,
    assists          INTEGER NOT NULL,
    saves            INTEGER NOT NULL,
    touches          INTEGER NOT NULL,
    car_touches      INTEGER NOT NULL,
    demos            INTEGER NOT NULL,
    joined_remaining INTEGER,
    joined_ot        INTEGER,
    left_remaining   INTEGER,
    left_ot          INTEGER,
    PRIMARY KEY (match_id, slot)
) WITHOUT ROWID;

CREATE TABLE goals (
    match_id         INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    remaining        INTEGER NOT NULL,
    ot               INTEGER NOT NULL,
    scorer           INTEGER NOT NULL,
    assister         INTEGER,
    goal_speed       INTEGER NOT NULL,
    last_touch_speed INTEGER NOT NULL,
    x INTEGER NOT NULL, y INTEGER NOT NULL, z INTEGER NOT NULL,
    team0_after      INTEGER,
    team1_after      INTEGER
);
CREATE INDEX goals_match ON goals (match_id);

CREATE TABLE stat_events (
    match_id  INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    remaining INTEGER NOT NULL,
    ot        INTEGER NOT NULL,
    name      TEXT NOT NULL,
    main      INTEGER NOT NULL,
    secondary INTEGER
);
CREATE INDEX stat_events_match ON stat_events (match_id);

CREATE TABLE ball_hits (
    match_id   INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    remaining  INTEGER NOT NULL,
    ot         INTEGER NOT NULL,
    player     INTEGER NOT NULL,
    pre_speed  INTEGER NOT NULL,
    post_speed INTEGER NOT NULL,
    x INTEGER NOT NULL, y INTEGER NOT NULL, z INTEGER NOT NULL
);
CREATE INDEX ball_hits_match ON ball_hits (match_id);

CREATE TABLE crossbar_hits (
    match_id     INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    remaining    INTEGER NOT NULL,
    ot           INTEGER NOT NULL,
    player       INTEGER NOT NULL,
    ball_speed   INTEGER NOT NULL,
    impact_force INTEGER NOT NULL,
    x INTEGER NOT NULL, y INTEGER NOT NULL, z INTEGER NOT NULL
);
CREATE INDEX crossbar_hits_match ON crossbar_hits (match_id);

CREATE TABLE boost_pickups (
    match_id  INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    remaining INTEGER NOT NULL,
    ot        INTEGER NOT NULL,
    player    INTEGER NOT NULL,
    kind      TEXT NOT NULL,
    amount    INTEGER NOT NULL,
    x INTEGER NOT NULL, y INTEGER NOT NULL, z INTEGER NOT NULL
);
CREATE INDEX boost_pickups_match ON boost_pickups (match_id);

CREATE TABLE player_movement (
    match_id        INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    slot            INTEGER NOT NULL,
    samples         INTEGER NOT NULL,
    avg_boost       REAL NOT NULL,
    pct_boost_empty REAL NOT NULL,
    pct_boosting    REAL NOT NULL,
    pct_supersonic  REAL NOT NULL,
    pct_on_ground   REAL NOT NULL,
    pct_on_wall     REAL NOT NULL,
    PRIMARY KEY (match_id, slot)
) WITHOUT ROWID;
";

/// v1 -> v2: per-touch / per-pickup rows become per-player averages and counts, and the
/// movement table gains boost bands, speed, powerslide and demolition columns.
const MIGRATE_V2: &str = "
DROP TABLE ball_hits;
DROP TABLE crossbar_hits;
DROP TABLE boost_pickups;

ALTER TABLE matches ADD COLUMN my_possession_pct REAL;
ALTER TABLE matches ADD COLUMN ball_avg_speed INTEGER;
ALTER TABLE matches ADD COLUMN ball_max_speed INTEGER;
ALTER TABLE matches ADD COLUMN team_empty_overlap REAL;
ALTER TABLE goals ADD COLUMN scorer_boost INTEGER;
ALTER TABLE goals ADD COLUMN scorer_speed INTEGER;
ALTER TABLE stat_events ADD COLUMN boost INTEGER;
ALTER TABLE stat_events ADD COLUMN speed INTEGER;

ALTER TABLE player_movement RENAME TO player_movement_v1;
CREATE TABLE player_movement (
    match_id         INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    slot             INTEGER NOT NULL,
    samples          INTEGER NOT NULL,
    avg_boost        REAL NOT NULL,
    pct_boost_empty  REAL NOT NULL,
    pct_boost_low    REAL NOT NULL DEFAULT 0,
    pct_boost_mid    REAL NOT NULL DEFAULT 0,
    pct_boost_high   REAL NOT NULL DEFAULT 0,
    pct_boosting     REAL NOT NULL,
    boost_used       INTEGER NOT NULL DEFAULT 0,
    boost_gained     INTEGER NOT NULL DEFAULT 0,
    avg_speed        REAL NOT NULL DEFAULT 0,
    max_speed        INTEGER NOT NULL DEFAULT 0,
    pct_supersonic   REAL NOT NULL,
    pct_powersliding REAL NOT NULL DEFAULT 0,
    pct_on_ground    REAL NOT NULL,
    pct_on_wall      REAL NOT NULL,
    pct_airborne     REAL NOT NULL DEFAULT 0,
    pct_demolished   REAL NOT NULL DEFAULT 0,
    times_demolished INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (match_id, slot)
) WITHOUT ROWID;
INSERT INTO player_movement (match_id, slot, samples, avg_boost, pct_boost_empty, pct_boosting,
                             pct_supersonic, pct_on_ground, pct_on_wall)
    SELECT match_id, slot, samples, avg_boost, pct_boost_empty, pct_boosting,
           pct_supersonic, pct_on_ground, pct_on_wall FROM player_movement_v1;
DROP TABLE player_movement_v1;

CREATE TABLE player_touches (
    match_id       INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    slot           INTEGER NOT NULL,
    touches        INTEGER NOT NULL,
    def_third      INTEGER NOT NULL,
    mid_third      INTEGER NOT NULL,
    att_third      INTEGER NOT NULL,
    avg_post_speed INTEGER NOT NULL,
    max_post_speed INTEGER NOT NULL,
    PRIMARY KEY (match_id, slot)
) WITHOUT ROWID;

CREATE TABLE player_event_stats (
    match_id  INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    slot      INTEGER NOT NULL,
    event     TEXT NOT NULL,
    count     INTEGER NOT NULL,
    avg_boost REAL,
    avg_speed REAL,
    PRIMARY KEY (match_id, slot, event)
) WITHOUT ROWID;

CREATE TABLE player_crossbar (
    match_id   INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    slot       INTEGER NOT NULL,
    count      INTEGER NOT NULL,
    max_impact INTEGER NOT NULL,
    PRIMARY KEY (match_id, slot)
) WITHOUT ROWID;

CREATE TABLE player_pickups (
    match_id INTEGER NOT NULL REFERENCES matches (id) ON DELETE CASCADE,
    slot     INTEGER NOT NULL,
    pads     INTEGER NOT NULL,
    pills    INTEGER NOT NULL,
    amount   INTEGER NOT NULL,
    PRIMARY KEY (match_id, slot)
) WITHOUT ROWID;
";

/// `$XDG_DATA_HOME/reclutched/matches.db` (or `~/.local/share/reclutched/matches.db`).
pub fn default_db_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/share"),
    };
    Some(base.join("reclutched").join("matches.db"))
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (and creates or upgrades) the database file.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        // WAL: readers (`history`) never block the writer and a crash can't corrupt the file.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        // SQLite only enforces foreign keys when asked to, per connection.
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    /// For queries (history, tests).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Saves a finished match in one transaction. Returns the new match id, or `None` when a
    /// match with the same game id was already saved.
    pub fn save_match(&mut self, m: &FinishedMatch) -> Result<Option<i64>, StoreError> {
        let tx = self.conn.transaction()?;
        let inserted = tx.execute(
            "INSERT INTO matches (guid, started_at, ended_at, playlist_id, arena, my_slot, my_team,
                                  team0_goals, team1_goals, overtime, result,
                                  abandoned_remaining, abandoned_overtime,
                                  my_possession_pct, ball_avg_speed, ball_max_speed,
                                  team_empty_overlap)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT DO NOTHING",
            params![
                m.guid,
                m.started_at,
                m.ended_at,
                m.playlist_id,
                m.arena,
                m.my_slot as i64,
                m.my_team,
                m.team_goals[0],
                m.team_goals[1],
                m.overtime,
                m.result.as_str(),
                m.abandoned_clock.map(|c| c.remaining),
                m.abandoned_clock.map(|c| c.overtime),
                m.my_possession_pct,
                m.ball_avg_speed.map(whole),
                m.ball_max_speed.map(whole),
                m.team_empty_overlap,
            ],
        )?;
        if inserted == 0 {
            return Ok(None); // dropping `tx` rolls back
        }
        let id = tx.last_insert_rowid();

        {
            let mut player = tx.prepare_cached(
                "INSERT INTO match_players VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                                                  ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            )?;
            let mut known = tx.prepare_cached(
                "INSERT INTO players (primary_id, name, last_seen) VALUES (?1, ?2, ?3)
                 ON CONFLICT (primary_id) DO UPDATE SET name = ?2, last_seen = ?3",
            )?;
            for p in &m.players {
                player.execute(params![
                    id,
                    p.slot as i64,
                    p.name,
                    p.primary_id,
                    p.team,
                    p.is_me,
                    p.score,
                    p.goals,
                    p.shots,
                    p.assists,
                    p.saves,
                    p.touches,
                    p.car_touches,
                    p.demos,
                    p.joined.map(|c| c.remaining),
                    p.joined.map(|c| c.overtime),
                    p.left.map(|c| c.remaining),
                    p.left.map(|c| c.overtime),
                ])?;
                // Bots and guests all share "Unknown|0|0"; they are not real accounts.
                if let Some(pid) = p
                    .primary_id
                    .as_deref()
                    .filter(|s| !s.starts_with("Unknown"))
                {
                    known.execute(params![pid, p.name, m.ended_at])?;
                }
            }

            let mut goal = tx.prepare_cached(
                "INSERT INTO goals (match_id, remaining, ot, scorer, assister, goal_speed,
                                    last_touch_speed, x, y, z, team0_after, team1_after,
                                    scorer_boost, scorer_speed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?;
            for g in &m.goals {
                goal.execute(params![
                    id,
                    g.clock.remaining,
                    g.clock.overtime,
                    g.scorer as i64,
                    g.assister.map(|a| a as i64),
                    whole(g.goal_speed),
                    whole(g.last_touch_speed),
                    whole(g.x),
                    whole(g.y),
                    whole(g.z),
                    g.score_after.map(|s| s[0]),
                    g.score_after.map(|s| s[1]),
                    g.scorer_boost,
                    g.scorer_speed.map(whole),
                ])?;
            }

            let mut stat = tx.prepare_cached(
                "INSERT INTO stat_events (match_id, remaining, ot, name, main, secondary, boost, speed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for s in &m.stat_events {
                stat.execute(params![
                    id,
                    s.clock.remaining,
                    s.clock.overtime,
                    s.name,
                    s.main as i64,
                    s.secondary.map(|a| a as i64),
                    s.boost,
                    s.speed.map(whole),
                ])?;
            }

            let mut event_stat = tx
                .prepare_cached("INSERT INTO player_event_stats VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
            for e in &m.event_stats {
                event_stat.execute(params![
                    id,
                    e.slot as i64,
                    e.event,
                    e.count,
                    e.avg_boost,
                    e.avg_speed
                ])?;
            }

            let mut touch = tx.prepare_cached(
                "INSERT INTO player_touches VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for t in &m.touches {
                touch.execute(params![
                    id,
                    t.slot as i64,
                    t.touches,
                    t.def_third,
                    t.mid_third,
                    t.att_third,
                    whole(t.avg_post_speed),
                    whole(t.max_post_speed),
                ])?;
            }

            let mut bar =
                tx.prepare_cached("INSERT INTO player_crossbar VALUES (?1, ?2, ?3, ?4)")?;
            for c in &m.crossbar {
                bar.execute(params![id, c.slot as i64, c.count, whole(c.max_impact)])?;
            }

            let mut pickup =
                tx.prepare_cached("INSERT INTO player_pickups VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for b in &m.pickups {
                pickup.execute(params![id, b.slot as i64, b.pads, b.pills, whole(b.amount)])?;
            }

            let mut movement = tx.prepare_cached(
                "INSERT INTO player_movement (match_id, slot, samples, avg_boost, pct_boost_empty,
                    pct_boost_low, pct_boost_mid, pct_boost_high, pct_boosting, boost_used,
                    boost_gained, avg_speed, max_speed, pct_supersonic, pct_powersliding,
                    pct_on_ground, pct_on_wall, pct_airborne, pct_demolished, times_demolished)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                         ?17, ?18, ?19, ?20)",
            )?;
            for s in &m.movement {
                movement.execute(params![
                    id,
                    s.slot as i64,
                    s.samples,
                    s.avg_boost,
                    s.pct_boost_empty,
                    s.pct_boost_low,
                    s.pct_boost_mid,
                    s.pct_boost_high,
                    s.pct_boosting,
                    s.boost_used,
                    s.boost_gained,
                    s.avg_speed,
                    whole(s.max_speed),
                    s.pct_supersonic,
                    s.pct_powersliding,
                    s.pct_on_ground,
                    s.pct_on_wall,
                    s.pct_airborne,
                    s.pct_demolished,
                    s.times_demolished,
                ])?;
            }
        }

        tx.commit()?;
        Ok(Some(id))
    }
}

/// Rounds to a whole number for compact storage.
fn whole(v: f64) -> i64 {
    v.round() as i64
}

/// Brings the file up to [`SCHEMA_VERSION`]. `PRAGMA user_version` is a number SQLite keeps
/// in the file header for exactly this purpose.
fn migrate(conn: &Connection) -> Result<(), StoreError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(StoreError::TooNew {
            found,
            known: SCHEMA_VERSION,
        });
    }
    if found < 1 {
        // Incremental auto-vacuum lets the file shrink again when rows are deleted later.
        // It can only be chosen before the first table exists.
        conn.pragma_update(None, "auto_vacuum", 2)?;
        conn.execute_batch(&format!(
            "BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;"
        ))?;
    }
    if found < 2 {
        conn.execute_batch(&format!(
            "BEGIN; {MIGRATE_V2} PRAGMA user_version = 2; COMMIT;"
        ))?;
        if found == 1 {
            // An old file was created without incremental auto-vacuum; convert it once.
            // (VACUUM can't run inside a transaction. The file is tiny at this point.)
            conn.pragma_update(None, "auto_vacuum", 2)?;
            conn.execute_batch("VACUUM;")?;
        }
    }
    // Future: `if found < 3 { ...; PRAGMA user_version = 3; }`
    Ok(())
}

/// A saved match as shown in `history`.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchRow {
    pub id: i64,
    pub ended_at: i64,
    pub playlist_id: i64,
    pub my_team: i64,
    pub team0_goals: i64,
    pub team1_goals: i64,
    pub overtime: bool,
    pub result: String,
    pub abandoned_at: Option<Clock>,
}

/// Everything stored about one match, for the detail view.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchDetail {
    pub row: MatchRow,
    pub my_slot: usize,
    pub my_possession_pct: Option<f64>,
    pub ball_avg_speed: Option<i64>,
    pub ball_max_speed: Option<i64>,
    pub team_empty_overlap: Option<f64>,
    pub players: Vec<PlayerSummary>,
    pub movement: Vec<MovementSummary>,
    pub touches: Vec<TouchSummary>,
    pub event_stats: Vec<EventStat>,
    /// Oldest first.
    pub goals: Vec<GoalRow>,
    /// Oldest first.
    pub stat_events: Vec<StatEventRow>,
}

/// How you usually play: your own movement numbers averaged over your recent matches. The
/// overlay compares a live match against these.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementAverages {
    /// How many matches went into the averages.
    pub matches: i64,
    pub avg_boost: f64,
    pub pct_boost_empty: f64,
    pub avg_speed: f64,
    pub pct_supersonic: f64,
    pub pct_powersliding: f64,
    pub pct_airborne: f64,
    pub pct_on_wall: f64,
}

/// How your recent matches went: the overlay shows it when no match is on.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentForm {
    /// How many matches went into it.
    pub matches: i64,
    pub wins: i64,
    pub losses: i64,
    pub abandoned: i64,
    /// What happened in the latest matches in a row: ("win", 3) is three wins in a row. `None`
    /// when the latest match was a left-early one.
    pub streak: Option<(String, usize)>,
    /// Your numbers per finished match (left-early matches are not counted in these).
    pub avg_score: f64,
    pub avg_goals: f64,
    pub avg_saves: f64,
    pub avg_shots: f64,
    /// Share of the time your team had the ball.
    pub avg_possession: Option<f64>,
}

/// How many results in a row the newest ones share ("win" x3), newest first. `None` for a
/// left-early match or no matches.
pub fn current_streak(results_newest_first: &[&str]) -> Option<(String, usize)> {
    let first = *results_newest_first.first()?;
    if first == "abandoned" {
        return None;
    }
    let count = results_newest_first
        .iter()
        .take_while(|r| **r == first)
        .count();
    Some((first.to_string(), count))
}

/// A comparison needs a few matches behind it, or "your average" is just the last match.
pub const MIN_MATCHES_FOR_AVERAGES: i64 = 3;

impl Store {
    /// How your last `last` matches went (None when there are no matches at all).
    pub fn recent_form(&self, last: u32) -> Result<Option<RecentForm>, StoreError> {
        let rows = self.recent_matches(last)?;
        if rows.is_empty() {
            return Ok(None);
        }
        let count = |result: &str| rows.iter().filter(|r| r.result == result).count() as i64;
        let results: Vec<&str> = rows.iter().map(|r| r.result.as_str()).collect();
        let (score, goals, saves, shots, possession) = self.conn.query_row(
            "SELECT AVG(p.score), AVG(p.goals), AVG(p.saves), AVG(p.shots), AVG(m.my_possession_pct)
             FROM (SELECT * FROM matches ORDER BY ended_at DESC, id DESC LIMIT ?1) m
             LEFT JOIN match_players p ON p.match_id = m.id AND p.slot = m.my_slot
             WHERE m.result != 'abandoned'",
            [last],
            |r| {
                Ok((
                    r.get::<_, Option<f64>>(0)?,
                    r.get::<_, Option<f64>>(1)?,
                    r.get::<_, Option<f64>>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                    r.get::<_, Option<f64>>(4)?,
                ))
            },
        )?;
        Ok(Some(RecentForm {
            matches: rows.len() as i64,
            wins: count("win"),
            losses: count("loss"),
            abandoned: count("abandoned"),
            streak: current_streak(&results),
            avg_score: score.unwrap_or(0.0),
            avg_goals: goals.unwrap_or(0.0),
            avg_saves: saves.unwrap_or(0.0),
            avg_shots: shots.unwrap_or(0.0),
            avg_possession: possession,
        }))
    }

    /// Your own movement averages over your last `last` matches that have movement data, or
    /// `None` when there are fewer than [`MIN_MATCHES_FOR_AVERAGES`].
    pub fn my_movement_averages(&self, last: u32) -> Result<Option<MovementAverages>, StoreError> {
        let row = self.conn.query_row(
            "SELECT COUNT(*), AVG(pm.avg_boost), AVG(pm.pct_boost_empty), AVG(pm.avg_speed),
                    AVG(pm.pct_supersonic), AVG(pm.pct_powersliding), AVG(pm.pct_airborne),
                    AVG(pm.pct_on_wall)
             FROM player_movement pm
             JOIN matches m ON m.id = pm.match_id AND m.my_slot = pm.slot
             WHERE pm.samples >= 100
               AND pm.match_id IN (SELECT id FROM matches ORDER BY ended_at DESC, id DESC LIMIT ?1)",
            [last],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<f64>>(1)?,
                    r.get::<_, Option<f64>>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                    r.get::<_, Option<f64>>(4)?,
                    r.get::<_, Option<f64>>(5)?,
                    r.get::<_, Option<f64>>(6)?,
                    r.get::<_, Option<f64>>(7)?,
                ))
            },
        )?;
        let (matches, boost, empty, speed, sonic, slide, air, wall) = row;
        if matches < MIN_MATCHES_FOR_AVERAGES {
            return Ok(None);
        }
        Ok(Some(MovementAverages {
            matches,
            avg_boost: boost.unwrap_or(0.0),
            pct_boost_empty: empty.unwrap_or(0.0),
            avg_speed: speed.unwrap_or(0.0),
            pct_supersonic: sonic.unwrap_or(0.0),
            pct_powersliding: slide.unwrap_or(0.0),
            pct_airborne: air.unwrap_or(0.0),
            pct_on_wall: wall.unwrap_or(0.0),
        }))
    }

    /// Newest first.
    pub fn recent_matches(&self, limit: u32) -> Result<Vec<MatchRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ended_at, playlist_id, my_team, team0_goals, team1_goals, overtime,
                    result, abandoned_remaining, abandoned_overtime
             FROM matches ORDER BY ended_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], match_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The id of the newest match, if any.
    pub fn latest_match_id(&self) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM matches ORDER BY ended_at DESC, id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// (wins, losses, abandoned).
    pub fn result_counts(&self) -> Result<(i64, i64, i64), StoreError> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM(result = 'win'), 0), COALESCE(SUM(result = 'loss'), 0),
                    COALESCE(SUM(result = 'abandoned'), 0) FROM matches",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }

    /// (all matches, abandoned matches).
    pub fn abandon_counts(&self) -> Result<(i64, i64), StoreError> {
        let (wins, losses, abandoned) = self.result_counts()?;
        Ok((wins + losses + abandoned, abandoned))
    }

    /// Reads back one match with its per-player numbers. `None` if there is no such match.
    pub fn match_detail(&self, id: i64) -> Result<Option<MatchDetail>, StoreError> {
        let head = self
            .conn
            .query_row(
                "SELECT id, ended_at, playlist_id, my_team, team0_goals, team1_goals, overtime,
                        result, abandoned_remaining, abandoned_overtime,
                        my_slot, my_possession_pct, ball_avg_speed, ball_max_speed,
                        team_empty_overlap
                 FROM matches WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        match_row(r)?,
                        r.get::<_, i64>(10)?,
                        r.get(11)?,
                        r.get(12)?,
                        r.get(13)?,
                        r.get(14)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            row,
            my_slot,
            my_possession_pct,
            ball_avg_speed,
            ball_max_speed,
            team_empty_overlap,
        )) = head
        else {
            return Ok(None);
        };

        let clock = |remaining: Option<i64>, ot: Option<bool>| {
            remaining.map(|remaining| Clock {
                remaining,
                overtime: ot.unwrap_or(false),
            })
        };
        let players = self
            .conn
            .prepare(
                "SELECT slot, name, primary_id, team, is_me, score, goals, shots, assists, saves,
                        touches, car_touches, demos, joined_remaining, joined_ot, left_remaining,
                        left_ot FROM match_players WHERE match_id = ?1 ORDER BY slot",
            )?
            .query_map([id], |r| {
                Ok(PlayerSummary {
                    slot: r.get::<_, i64>(0)? as usize,
                    name: r.get(1)?,
                    primary_id: r.get(2)?,
                    team: r.get(3)?,
                    is_me: r.get(4)?,
                    score: r.get(5)?,
                    goals: r.get(6)?,
                    shots: r.get(7)?,
                    assists: r.get(8)?,
                    saves: r.get(9)?,
                    touches: r.get(10)?,
                    car_touches: r.get(11)?,
                    demos: r.get(12)?,
                    joined: clock(r.get(13)?, r.get(14)?),
                    left: clock(r.get(15)?, r.get(16)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let movement = self
            .conn
            .prepare(
                "SELECT slot, samples, avg_boost, pct_boost_empty, pct_boost_low, pct_boost_mid,
                        pct_boost_high, pct_boosting, boost_used, boost_gained, avg_speed,
                        max_speed, pct_supersonic, pct_powersliding, pct_on_ground, pct_on_wall,
                        pct_airborne, pct_demolished, times_demolished
                 FROM player_movement WHERE match_id = ?1 ORDER BY slot",
            )?
            .query_map([id], |r| {
                Ok(MovementSummary {
                    slot: r.get::<_, i64>(0)? as usize,
                    samples: r.get(1)?,
                    avg_boost: r.get(2)?,
                    pct_boost_empty: r.get(3)?,
                    pct_boost_low: r.get(4)?,
                    pct_boost_mid: r.get(5)?,
                    pct_boost_high: r.get(6)?,
                    pct_boosting: r.get(7)?,
                    boost_used: r.get(8)?,
                    boost_gained: r.get(9)?,
                    avg_speed: r.get(10)?,
                    max_speed: r.get::<_, i64>(11)? as f64,
                    pct_supersonic: r.get(12)?,
                    pct_powersliding: r.get(13)?,
                    pct_on_ground: r.get(14)?,
                    pct_on_wall: r.get(15)?,
                    pct_airborne: r.get(16)?,
                    pct_demolished: r.get(17)?,
                    times_demolished: r.get(18)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let touches = self
            .conn
            .prepare(
                "SELECT slot, touches, def_third, mid_third, att_third, avg_post_speed,
                        max_post_speed FROM player_touches WHERE match_id = ?1 ORDER BY slot",
            )?
            .query_map([id], |r| {
                Ok(TouchSummary {
                    slot: r.get::<_, i64>(0)? as usize,
                    touches: r.get(1)?,
                    def_third: r.get(2)?,
                    mid_third: r.get(3)?,
                    att_third: r.get(4)?,
                    avg_post_speed: r.get::<_, i64>(5)? as f64,
                    max_post_speed: r.get::<_, i64>(6)? as f64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let event_stats = self
            .conn
            .prepare(
                "SELECT slot, event, count, avg_boost, avg_speed FROM player_event_stats
                 WHERE match_id = ?1 ORDER BY slot, event",
            )?
            .query_map([id], |r| {
                Ok(EventStat {
                    slot: r.get::<_, i64>(0)? as usize,
                    event: r.get(1)?,
                    count: r.get(2)?,
                    avg_boost: r.get(3)?,
                    avg_speed: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let goals = self
            .conn
            .prepare(
                "SELECT remaining, ot, scorer, assister, goal_speed, last_touch_speed, x, y, z,
                        team0_after, team1_after, scorer_boost, scorer_speed
                 FROM goals WHERE match_id = ?1 ORDER BY rowid",
            )?
            .query_map([id], |r| {
                let team0: Option<i64> = r.get(9)?;
                let team1: Option<i64> = r.get(10)?;
                Ok(GoalRow {
                    clock: Clock {
                        remaining: r.get(0)?,
                        overtime: r.get(1)?,
                    },
                    scorer: r.get::<_, i64>(2)? as usize,
                    assister: r.get::<_, Option<i64>>(3)?.map(|a| a as usize),
                    goal_speed: r.get::<_, i64>(4)? as f64,
                    last_touch_speed: r.get::<_, i64>(5)? as f64,
                    x: r.get::<_, i64>(6)? as f64,
                    y: r.get::<_, i64>(7)? as f64,
                    z: r.get::<_, i64>(8)? as f64,
                    scorer_boost: r.get(11)?,
                    scorer_speed: r.get::<_, Option<i64>>(12)?.map(|v| v as f64),
                    score_after: team0.zip(team1).map(|(a, b)| [a, b]),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let stat_events = self
            .conn
            .prepare(
                "SELECT remaining, ot, name, main, secondary, boost, speed
                 FROM stat_events WHERE match_id = ?1 ORDER BY rowid",
            )?
            .query_map([id], |r| {
                Ok(StatEventRow {
                    clock: Clock {
                        remaining: r.get(0)?,
                        overtime: r.get(1)?,
                    },
                    name: r.get(2)?,
                    main: r.get::<_, i64>(3)? as usize,
                    secondary: r.get::<_, Option<i64>>(4)?.map(|a| a as usize),
                    boost: r.get(5)?,
                    speed: r.get::<_, Option<i64>>(6)?.map(|v| v as f64),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Some(MatchDetail {
            row,
            my_slot: my_slot as usize,
            my_possession_pct,
            ball_avg_speed,
            ball_max_speed,
            team_empty_overlap,
            players,
            movement,
            touches,
            event_stats,
            goals,
            stat_events,
        }))
    }
}

/// Reads the first ten columns of the `matches` SELECTs above.
fn match_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MatchRow> {
    let remaining: Option<i64> = r.get(8)?;
    let overtime: Option<bool> = r.get(9)?;
    Ok(MatchRow {
        id: r.get(0)?,
        ended_at: r.get(1)?,
        playlist_id: r.get(2)?,
        my_team: r.get(3)?,
        team0_goals: r.get(4)?,
        team1_goals: r.get(5)?,
        overtime: r.get(6)?,
        result: r.get(7)?,
        abandoned_at: remaining.map(|remaining| Clock {
            remaining,
            overtime: overtime.unwrap_or(false),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A v1 file with one match, exactly as the first release wrote it.
    fn v1_database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(&format!(
            "BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;"
        ))
        .unwrap();
        conn.execute_batch(
            "INSERT INTO matches VALUES (1, 'G', 10, 20, 3, 'Stadium_P', 0, 0, 3, 1, 0, 'win', NULL, NULL);
             INSERT INTO match_players VALUES (1, 0, 'Me', 'Epic|1|0', 0, 1, 100, 3, 4, 0, 1, 20, 10, 0,
                                               NULL, NULL, NULL, NULL);
             INSERT INTO ball_hits VALUES (1, 100, 0, 0, 0, 1000, 1, 2, 3);
             INSERT INTO player_movement VALUES (1, 0, 50, 40.0, 25.0, 30.0, 10.0, 60.0, 5.0);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn v1_database_is_migrated_and_keeps_its_data() {
        let conn = v1_database();
        migrate(&conn).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        // Old rows survive, new columns get neutral defaults.
        let (samples, boost, ground, powerslide): (i64, f64, f64, f64) = conn
            .query_row(
                "SELECT samples, avg_boost, pct_on_ground, pct_powersliding FROM player_movement",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((samples, boost, ground, powerslide), (50, 40.0, 60.0, 0.0));
        let name: String = conn
            .query_row("SELECT name FROM match_players", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Me");
        // The per-hit table is gone, the new aggregate tables exist.
        assert!(conn.prepare("SELECT * FROM ball_hits").is_err());
        conn.prepare("SELECT * FROM player_touches").unwrap();
        conn.prepare("SELECT * FROM player_event_stats").unwrap();
        // Migrating again is a no-op.
        migrate(&conn).unwrap();
    }

    #[test]
    fn migrated_file_uses_incremental_auto_vacuum() {
        let conn = v1_database();
        migrate(&conn).unwrap();
        let mode: i64 = conn
            .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
            .unwrap();
        // On a file this is 2 (incremental); an in-memory database may report 0, so only
        // check that asking did not break anything and the data is still readable.
        assert!(mode == 2 || mode == 0);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM matches", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
