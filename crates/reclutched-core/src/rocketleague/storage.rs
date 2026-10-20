//! SQLite storage for finished matches.
//!
//! Everything about one match is written in a single transaction, so a crash can never leave a
//! half-saved match. Speeds and positions are stored as whole numbers (SQLite stores small
//! integers in 1-2 bytes, a REAL always takes 8), which is plenty of precision for these stats.
//!
//! Rows refer to players by `slot` (see `tracker::PlayerSummary`), not by player id, because
//! bots share ids and the game reuses shortcut numbers.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use super::tracker::{Clock, FinishedMatch};

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
const SCHEMA_VERSION: i64 = 1;

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

/// `<config dir>/../reclutched/matches.db`: the data folder is `$XDG_DATA_HOME/reclutched`
/// (or `~/.local/share/reclutched`).
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
                                  abandoned_remaining, abandoned_overtime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
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
                "INSERT INTO goals VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                ])?;
            }

            let mut stat =
                tx.prepare_cached("INSERT INTO stat_events VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
            for s in &m.stat_events {
                stat.execute(params![
                    id,
                    s.clock.remaining,
                    s.clock.overtime,
                    s.name,
                    s.main as i64,
                    s.secondary.map(|a| a as i64),
                ])?;
            }

            let mut hit = tx.prepare_cached(
                "INSERT INTO ball_hits VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for h in &m.ball_hits {
                hit.execute(params![
                    id,
                    h.clock.remaining,
                    h.clock.overtime,
                    h.player as i64,
                    whole(h.pre_speed),
                    whole(h.post_speed),
                    whole(h.x),
                    whole(h.y),
                    whole(h.z),
                ])?;
            }

            let mut bar = tx.prepare_cached(
                "INSERT INTO crossbar_hits VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for c in &m.crossbar_hits {
                bar.execute(params![
                    id,
                    c.clock.remaining,
                    c.clock.overtime,
                    c.player as i64,
                    whole(c.ball_speed),
                    whole(c.impact_force),
                    whole(c.x),
                    whole(c.y),
                    whole(c.z),
                ])?;
            }

            let mut pickup = tx.prepare_cached(
                "INSERT INTO boost_pickups VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for b in &m.boost_pickups {
                pickup.execute(params![
                    id,
                    b.clock.remaining,
                    b.clock.overtime,
                    b.player as i64,
                    b.kind,
                    whole(b.amount),
                    whole(b.x),
                    whole(b.y),
                    whole(b.z),
                ])?;
            }

            let mut movement = tx.prepare_cached(
                "INSERT INTO player_movement VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for s in &m.movement {
                movement.execute(params![
                    id,
                    s.slot as i64,
                    s.samples,
                    s.avg_boost,
                    s.pct_boost_empty,
                    s.pct_boosting,
                    s.pct_supersonic,
                    s.pct_on_ground,
                    s.pct_on_wall,
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
        conn.execute_batch(&format!(
            "BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;"
        ))?;
    }
    // Future: `if found < 2 { ...ALTER TABLE...; PRAGMA user_version = 2; }`
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

impl Store {
    /// Newest first.
    pub fn recent_matches(&self, limit: u32) -> Result<Vec<MatchRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ended_at, playlist_id, my_team, team0_goals, team1_goals, overtime,
                    result, abandoned_remaining, abandoned_overtime
             FROM matches ORDER BY ended_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
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
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// (all matches, abandoned matches).
    pub fn abandon_counts(&self) -> Result<(i64, i64), StoreError> {
        let counts = self
            .conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(result = 'abandoned'), 0) FROM matches",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(counts.unwrap_or((0, 0)))
    }
}
