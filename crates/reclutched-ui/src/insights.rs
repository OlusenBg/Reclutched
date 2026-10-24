//! Adds several days together for the Main Menu cards ("How you move", "How you score", ...).
//! Plain code, no widgets, so it can be tested.

use reclutched_core::rocketleague::dates::Day;
use reclutched_core::rocketleague::periods::PeriodSummary;

/// Several days as one: counts added up, averages weighted by the matches behind each day.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub matches: i64,
    pub wins: i64,
    pub losses: i64,
    pub abandoned: i64,
    pub seconds_played: i64,
    pub score: f64,
    pub goals: f64,
    pub assists: f64,
    pub saves: f64,
    pub shots: f64,
    pub demos: f64,
    pub possession: Option<f64>,
    pub boost: Option<f64>,
    pub boost_empty: Option<f64>,
    pub speed: Option<f64>,
}

/// The days from `today` back `n` days (today included).
pub fn last_days(days: &[PeriodSummary], today: Day, n: i64) -> Vec<&PeriodSummary> {
    let from = today.plus_days(-(n - 1)).to_days();
    days.iter()
        .filter(|d| {
            Day::parse(&d.key).is_some_and(|day| (from..=today.to_days()).contains(&day.to_days()))
        })
        .collect()
}

/// A weighted average of the days that have the value, or `None` when none does.
fn weighted(days: &[&PeriodSummary], value: impl Fn(&PeriodSummary) -> Option<f64>) -> Option<f64> {
    let (sum, weight) = days.iter().fold((0.0, 0.0), |(sum, weight), d| {
        let w = (d.matches - d.abandoned) as f64;
        match value(d) {
            Some(v) if w > 0.0 => (sum + v * w, weight + w),
            _ => (sum, weight),
        }
    });
    (weight > 0.0).then(|| sum / weight)
}

/// `None` when there are no matches at all in these days.
pub fn merge(days: &[&PeriodSummary]) -> Option<Merged> {
    let matches: i64 = days.iter().map(|d| d.matches).sum();
    if matches == 0 {
        return None;
    }
    let sum = |f: fn(&PeriodSummary) -> i64| days.iter().map(|d| f(d)).sum::<i64>();
    let avg = |f: fn(&PeriodSummary) -> f64| weighted(days, |d| Some(f(d))).unwrap_or(0.0);
    Some(Merged {
        matches,
        wins: sum(|d| d.wins),
        losses: sum(|d| d.losses),
        abandoned: sum(|d| d.abandoned),
        seconds_played: sum(|d| d.seconds_played),
        score: avg(|d| d.avg_score),
        goals: avg(|d| d.avg_goals),
        assists: avg(|d| d.avg_assists),
        saves: avg(|d| d.avg_saves),
        shots: avg(|d| d.avg_shots),
        demos: avg(|d| d.avg_demos),
        possession: weighted(days, |d| d.avg_possession),
        boost: weighted(days, |d| d.avg_boost),
        boost_empty: weighted(days, |d| d.avg_boost_empty),
        speed: weighted(days, |d| d.avg_speed),
    })
}

/// The day you played the longest.
pub fn busiest<'a>(days: &[&'a PeriodSummary]) -> Option<&'a PeriodSummary> {
    days.iter()
        .copied()
        .filter(|d| d.seconds_played > 0)
        .max_by_key(|d| (d.seconds_played, d.key.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(
        key: &str,
        matches: i64,
        abandoned: i64,
        goals: f64,
        boost: Option<f64>,
        secs: i64,
    ) -> PeriodSummary {
        PeriodSummary {
            key: key.to_string(),
            matches,
            wins: matches - abandoned,
            losses: 0,
            abandoned,
            seconds_played: secs,
            avg_score: 100.0,
            avg_goals: goals,
            avg_assists: 0.0,
            avg_saves: 0.0,
            avg_shots: 0.0,
            avg_demos: 0.0,
            avg_possession: None,
            avg_boost: boost,
            avg_boost_empty: None,
            avg_speed: None,
        }
    }

    #[test]
    fn the_last_days_include_today_and_stop_at_the_limit() {
        let today = Day::new(2026, 10, 23).unwrap();
        let days = [
            day("2026-10-23", 1, 0, 1.0, None, 10),
            day("2026-09-24", 1, 0, 1.0, None, 10),
            day("2026-09-23", 1, 0, 1.0, None, 10),
        ];
        let keys: Vec<_> = last_days(&days, today, 30)
            .iter()
            .map(|d| d.key.as_str())
            .collect();
        assert_eq!(keys, ["2026-10-23", "2026-09-24"]);
    }

    #[test]
    fn averages_are_weighted_by_finished_matches() {
        // 3 matches averaging 1 goal and 1 match averaging 5: (3 + 5) / 4 = 2 goals.
        let a = day("2026-10-22", 3, 0, 1.0, Some(30.0), 900);
        let b = day("2026-10-23", 2, 1, 5.0, Some(50.0), 600);
        let merged = merge(&[&a, &b]).unwrap();
        assert_eq!(merged.matches, 5);
        assert_eq!(merged.abandoned, 1);
        assert!((merged.goals - 2.0).abs() < 1e-9);
        assert!((merged.boost.unwrap() - 35.0).abs() < 1e-9);
        assert_eq!(merged.seconds_played, 1_500);
        assert_eq!(merged.possession, None);
    }

    #[test]
    fn nothing_played_merges_to_nothing() {
        assert!(merge(&[]).is_none());
        assert!(busiest(&[]).is_none());
    }

    #[test]
    fn the_busiest_day_is_the_longest() {
        let a = day("2026-10-22", 1, 0, 1.0, None, 900);
        let b = day("2026-10-23", 1, 0, 1.0, None, 3_000);
        assert_eq!(busiest(&[&a, &b]).unwrap().key, "2026-10-23");
    }
}
