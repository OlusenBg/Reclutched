//! Your matches boiled down by day, month and year. The matches themselves are all kept; these
//! numbers are worked out from them when you ask, so nothing can get out of step.
//!
//! A "day" is a calendar day in your time zone. SQLite does not know your time zone, so the
//! caller passes the offset from UTC in seconds (the app asks the desktop).

use super::storage::{match_row, MatchRow, Store, StoreError};

/// How coarse a summary is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grain {
    Day,
    Month,
    Year,
}

impl Grain {
    /// The `strftime` pattern that turns a time into this grain's key.
    fn pattern(self) -> &'static str {
        match self {
            Grain::Day => "%Y-%m-%d",
            Grain::Month => "%Y-%m",
            Grain::Year => "%Y",
        }
    }

    /// The grain of a key like "2026-10-23", "2026-10" or "2026".
    pub fn of_key(key: &str) -> Option<Self> {
        match key.len() {
            10 => Some(Grain::Day),
            7 => Some(Grain::Month),
            4 => Some(Grain::Year),
            _ => None,
        }
    }
}

/// One day, month or year in numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct PeriodSummary {
    /// "2026-10-23", "2026-10" or "2026".
    pub key: String,
    pub matches: i64,
    pub wins: i64,
    pub losses: i64,
    pub abandoned: i64,
    /// Time spent in matches (each match counts at most an hour).
    pub seconds_played: i64,
    /// Your numbers per finished match (matches you left early are not counted in these).
    pub avg_score: f64,
    pub avg_goals: f64,
    pub avg_assists: f64,
    pub avg_saves: f64,
    pub avg_shots: f64,
    pub avg_demos: f64,
    /// Share of the time your team had the ball.
    pub avg_possession: Option<f64>,
    pub avg_boost: Option<f64>,
    pub avg_boost_empty: Option<f64>,
    pub avg_speed: Option<f64>,
}

impl PeriodSummary {
    /// Wins out of the matches that were played to the end, in percent.
    pub fn win_rate(&self) -> Option<f64> {
        let played = self.wins + self.losses;
        (played > 0).then(|| self.wins as f64 * 100.0 / played as f64)
    }

    pub fn hours_played(&self) -> f64 {
        self.seconds_played as f64 / 3_600.0
    }
}

/// A match counts for at most this long, so a match left open overnight does not count as a day.
const MAX_MATCH_SECONDS: i64 = 3_600;
/// Movement numbers need at least this many samples (ten seconds of play) to count.
const MIN_MOVEMENT_SAMPLES: i64 = 100;

fn forget_unused_players(conn: &rusqlite::Connection) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM players WHERE primary_id NOT IN
         (SELECT primary_id FROM match_players WHERE primary_id IS NOT NULL)",
        [],
    )?;
    Ok(())
}

impl Store {
    /// Every day, month or year that has matches, newest first.
    pub fn period_summaries(
        &self,
        grain: Grain,
        utc_offset: i64,
    ) -> Result<Vec<PeriodSummary>, StoreError> {
        self.summaries(grain, utc_offset, None)
    }

    /// One day, month or year by its key; `None` when nothing was played then.
    pub fn period_summary(
        &self,
        key: &str,
        utc_offset: i64,
    ) -> Result<Option<PeriodSummary>, StoreError> {
        let Some(grain) = Grain::of_key(key) else {
            return Ok(None);
        };
        Ok(self
            .summaries(grain, utc_offset, Some(key))?
            .into_iter()
            .next())
    }

    /// The matches of one day, month or year, newest first.
    pub fn matches_in_period(
        &self,
        key: &str,
        utc_offset: i64,
    ) -> Result<Vec<MatchRow>, StoreError> {
        let Some(grain) = Grain::of_key(key) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.connection().prepare(&format!(
            "SELECT id, ended_at, playlist_id, my_team, team0_goals, team1_goals, overtime,
                    result, abandoned_remaining, abandoned_overtime
             FROM matches
             WHERE strftime('{}', ended_at + ?1, 'unixepoch') = ?2
             ORDER BY ended_at DESC, id DESC",
            grain.pattern()
        ))?;
        let rows = stmt.query_map(rusqlite::params![utc_offset, key], match_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The mode you played most since this time (unix seconds): (playlist id, matches).
    pub fn favourite_playlist_since(&self, since: i64) -> Result<Option<(i64, i64)>, StoreError> {
        let found = self.connection().query_row(
            "SELECT playlist_id, COUNT(*) AS n FROM matches WHERE ended_at >= ?1
             GROUP BY playlist_id ORDER BY n DESC, playlist_id LIMIT 1",
            [since],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        );
        match found {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Deletes every match of one day, month or year (with its players, goals and the rest) and
    /// returns how many matches went. Names of players you no longer share a match with go too.
    pub fn delete_period(&mut self, key: &str, utc_offset: i64) -> Result<usize, StoreError> {
        let Some(grain) = Grain::of_key(key) else {
            return Ok(0);
        };
        let tx = self.conn.transaction()?;
        let gone = tx.execute(
            &format!(
                "DELETE FROM matches
                 WHERE strftime('{}', ended_at + ?1, 'unixepoch') = ?2",
                grain.pattern()
            ),
            rusqlite::params![utc_offset, key],
        )?;
        forget_unused_players(&tx)?;
        tx.commit()?;
        Ok(gone)
    }

    /// Deletes every match and every remembered player. Returns how many matches went.
    pub fn delete_everything(&mut self) -> Result<usize, StoreError> {
        let tx = self.conn.transaction()?;
        let gone = tx.execute("DELETE FROM matches", [])?;
        tx.execute("DELETE FROM players", [])?;
        tx.commit()?;
        Ok(gone)
    }

    fn summaries(
        &self,
        grain: Grain,
        utc_offset: i64,
        only: Option<&str>,
    ) -> Result<Vec<PeriodSummary>, StoreError> {
        // Averages skip matches that were left early and movement rows with too little data;
        // AVG ignores the NULLs the CASE expressions produce for them.
        let sql = format!(
            "SELECT strftime('{pattern}', m.ended_at + ?1, 'unixepoch') AS k,
                    COUNT(*),
                    COALESCE(SUM(m.result = 'win'), 0),
                    COALESCE(SUM(m.result = 'loss'), 0),
                    COALESCE(SUM(m.result = 'abandoned'), 0),
                    COALESCE(SUM(MIN(MAX(m.ended_at - m.started_at, 0), {max_seconds})), 0),
                    AVG(CASE WHEN m.result != 'abandoned' THEN p.score END),
                    AVG(CASE WHEN m.result != 'abandoned' THEN p.goals END),
                    AVG(CASE WHEN m.result != 'abandoned' THEN p.assists END),
                    AVG(CASE WHEN m.result != 'abandoned' THEN p.saves END),
                    AVG(CASE WHEN m.result != 'abandoned' THEN p.shots END),
                    AVG(CASE WHEN m.result != 'abandoned' THEN m.my_possession_pct END),
                    AVG(CASE WHEN pm.samples >= {min_samples} THEN pm.avg_boost END),
                    AVG(CASE WHEN pm.samples >= {min_samples} THEN pm.pct_boost_empty END),
                    AVG(CASE WHEN pm.samples >= {min_samples} THEN pm.avg_speed END),
                    AVG(CASE WHEN m.result != 'abandoned' THEN p.demos END)
             FROM matches m
             LEFT JOIN match_players p ON p.match_id = m.id AND p.slot = m.my_slot
             LEFT JOIN player_movement pm ON pm.match_id = m.id AND pm.slot = m.my_slot
             GROUP BY k
             HAVING (?2 IS NULL OR k = ?2)
             ORDER BY k DESC",
            pattern = grain.pattern(),
            max_seconds = MAX_MATCH_SECONDS,
            min_samples = MIN_MOVEMENT_SAMPLES,
        );
        let mut stmt = self.connection().prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![utc_offset, only], |r| {
            let float = |i: usize| -> rusqlite::Result<f64> {
                Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0))
            };
            Ok(PeriodSummary {
                key: r.get(0)?,
                matches: r.get(1)?,
                wins: r.get(2)?,
                losses: r.get(3)?,
                abandoned: r.get(4)?,
                seconds_played: r.get(5)?,
                avg_score: float(6)?,
                avg_goals: float(7)?,
                avg_assists: float(8)?,
                avg_saves: float(9)?,
                avg_shots: float(10)?,
                avg_demos: float(15)?,
                avg_possession: r.get(11)?,
                avg_boost: r.get(12)?,
                avg_boost_empty: r.get(13)?,
                avg_speed: r.get(14)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
