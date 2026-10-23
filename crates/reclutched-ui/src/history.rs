//! What the left side of the window lists, as plain data (no widgets), so the rules can be tested:
//!
//! - **Home** first: today's numbers.
//! - **Today**: every match on its own row.
//! - **Earlier this month**: one row per day.
//! - **Earlier months of this year**: one group per month, with its days inside.
//! - **Earlier years**: one group per year, with only its months inside (no days).
//!
//! Every match is still in the database; days, months and years only summarise them, and you
//! can open any of them to see its matches.

use std::collections::HashSet;

use reclutched_core::rocketleague::dates::{Day, MONTHS, WEEKDAYS};
use reclutched_core::rocketleague::periods::PeriodSummary;
use reclutched_core::rocketleague::storage::MatchRow;

use crate::format::{clock_text, result_class, score_text};

/// What a row opens on the right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Home,
    Match(i64),
    /// A day ("2026-10-23"), a month ("2026-10") or a year ("2026").
    Period(String),
}

/// How a row is coloured (the thin stripe on its left and its right-hand text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Win,
    Loss,
    Abandoned,
    Neutral,
}

impl Tone {
    pub fn css_class(self) -> &'static str {
        match self {
            Tone::Win => "win",
            Tone::Loss => "loss",
            Tone::Abandoned => "abandoned",
            Tone::Neutral => "neutral",
        }
    }
}

/// A row that can be opened and closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub key: String,
    pub expanded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub target: Target,
    /// 0 for top-level rows, 1 for rows inside a month or year.
    pub depth: u8,
    pub title: String,
    pub subtitle: String,
    /// The score of a match, or the record ("3 – 2") of a period.
    pub right: String,
    pub tone: Tone,
    pub group: Option<Group>,
}

/// Everything the list is built from.
pub struct Inputs<'a> {
    pub today: Day,
    pub today_matches: &'a [MatchRow],
    /// Newest first.
    pub days: &'a [PeriodSummary],
    pub months: &'a [PeriodSummary],
    pub years: &'a [PeriodSummary],
    /// Keys of the groups that are open ("m:2026-09", "y:2025").
    pub expanded: &'a HashSet<String>,
    /// Name of a mode, and the time of a match, as text.
    pub mode_name: &'a dyn Fn(i64) -> String,
    pub when: &'a dyn Fn(i64) -> String,
}

/// "Wed, Oct 21".
pub fn day_title(day: Day) -> String {
    format!(
        "{}, {} {}",
        &WEEKDAYS[day.weekday() as usize][..3],
        &MONTHS[day.month as usize - 1][..3],
        day.day
    )
}

/// "Yesterday" or "Wed, Oct 21".
fn day_label(day: Day, today: Day) -> String {
    if day == today.plus_days(-1) {
        "Yesterday".to_string()
    } else {
        day_title(day)
    }
}

/// "September 2026".
pub fn month_title(year: i32, month: u32) -> String {
    format!("{} {year}", MONTHS[month as usize - 1])
}

/// What to call a period in a heading: "Today", "Wed, Oct 21", "September 2026" or "2025".
pub fn period_title(key: &str, today: Day) -> String {
    if let Some(day) = Day::parse(key) {
        return if day == today {
            "Today".to_string()
        } else {
            day_label(day, today)
        };
    }
    if let Some((year, month)) = reclutched_core::rocketleague::dates::parse_month(key) {
        return month_title(year, month);
    }
    key.to_string()
}

/// The line under a period's heading: the full date of a day, or how much is inside.
pub fn period_subtitle(key: &str, children: usize) -> String {
    if let Some(day) = Day::parse(key) {
        return format!(
            "{}, {} {}, {}",
            WEEKDAYS[day.weekday() as usize],
            MONTHS[day.month as usize - 1],
            day.day,
            day.year
        );
    }
    match key.len() {
        7 => format!("{} played", days_word(children)),
        4 => months_word(children),
        _ => String::new(),
    }
}

pub fn tone_of(summary: &PeriodSummary) -> Tone {
    match summary.wins.cmp(&summary.losses) {
        std::cmp::Ordering::Greater => Tone::Win,
        std::cmp::Ordering::Less => Tone::Loss,
        std::cmp::Ordering::Equal if summary.wins > 0 => Tone::Abandoned,
        std::cmp::Ordering::Equal => Tone::Neutral,
    }
}

fn record(summary: &PeriodSummary) -> String {
    format!("{} \u{2013} {}", summary.wins, summary.losses)
}

fn matches_word(n: i64) -> String {
    if n == 1 {
        "1 match".to_string()
    } else {
        format!("{n} matches")
    }
}

/// One match on its own row.
pub fn match_entry(
    row: &MatchRow,
    mode_name: &dyn Fn(i64) -> String,
    when: &dyn Fn(i64) -> String,
) -> Entry {
    let mut subtitle = when(row.ended_at);
    if let Some(clock) = row.abandoned_at {
        subtitle.push_str(&format!("  \u{00B7}  left with {}", clock_text(clock)));
    } else if row.overtime {
        subtitle.push_str("  \u{00B7}  overtime");
    }
    Entry {
        target: Target::Match(row.id),
        depth: 0,
        title: mode_name(row.playlist_id),
        subtitle,
        right: score_text(row),
        tone: match result_class(&row.result) {
            "win" => Tone::Win,
            "loss" => Tone::Loss,
            _ => Tone::Abandoned,
        },
        group: None,
    }
}

fn day_entry(summary: &PeriodSummary, day: Day, today: Day, depth: u8) -> Entry {
    Entry {
        target: Target::Period(summary.key.clone()),
        depth,
        title: day_label(day, today),
        subtitle: matches_word(summary.matches),
        right: record(summary),
        tone: tone_of(summary),
        group: None,
    }
}

/// The whole list, top to bottom.
pub fn build_entries(input: &Inputs<'_>) -> Vec<Entry> {
    let today = input.today;
    let mut out = Vec::new();

    // Home: today in one line.
    let (won, lost) =
        input
            .today_matches
            .iter()
            .fold((0, 0), |(w, l), m| match m.result.as_str() {
                "win" => (w + 1, l),
                "loss" => (w, l + 1),
                _ => (w, l),
            });
    out.push(Entry {
        target: Target::Home,
        depth: 0,
        title: "Home".to_string(),
        subtitle: if input.today_matches.is_empty() {
            "Nothing played today yet".to_string()
        } else {
            format!(
                "Today  \u{00B7}  {}",
                matches_word(input.today_matches.len() as i64)
            )
        },
        right: if input.today_matches.is_empty() {
            String::new()
        } else {
            format!("{won} \u{2013} {lost}")
        },
        tone: match won.cmp(&lost) {
            std::cmp::Ordering::Greater => Tone::Win,
            std::cmp::Ordering::Less => Tone::Loss,
            std::cmp::Ordering::Equal if won > 0 => Tone::Abandoned,
            std::cmp::Ordering::Equal => Tone::Neutral,
        },
        group: None,
    });

    // Today: every match.
    for row in input.today_matches {
        out.push(match_entry(row, input.mode_name, input.when));
    }

    let today_key = today.key();
    let this_month = today.month_key();
    let this_year = format!("{:04}", today.year);

    // Earlier days of this month: one row each.
    for summary in input.days {
        let Some(day) = Day::parse(&summary.key) else {
            continue;
        };
        if summary.key < today_key && summary.key.starts_with(&this_month) {
            out.push(day_entry(summary, day, today, 0));
        }
    }

    // Earlier months of this year: a group each, with its days.
    for month in input.months {
        if !(month.key.starts_with(&this_year) && month.key < this_month) {
            continue;
        }
        let Some((year, number)) = reclutched_core::rocketleague::dates::parse_month(&month.key)
        else {
            continue;
        };
        let key = format!("m:{}", month.key);
        let expanded = input.expanded.contains(&key);
        let days_of_month: Vec<&PeriodSummary> = input
            .days
            .iter()
            .filter(|d| d.key.starts_with(&month.key))
            .collect();
        out.push(Entry {
            target: Target::Period(month.key.clone()),
            depth: 0,
            title: month_title(year, number),
            subtitle: format!(
                "{}  \u{00B7}  {}",
                days_word(days_of_month.len()),
                matches_word(month.matches)
            ),
            right: record(month),
            tone: tone_of(month),
            group: Some(Group { key, expanded }),
        });
        if expanded {
            for summary in days_of_month {
                if let Some(day) = Day::parse(&summary.key) {
                    out.push(day_entry(summary, day, today, 1));
                }
            }
        }
    }

    // Earlier years: a group each, with only its months.
    for year in input.years {
        if year.key >= this_year {
            continue;
        }
        let key = format!("y:{}", year.key);
        let expanded = input.expanded.contains(&key);
        let months_of_year: Vec<&PeriodSummary> = input
            .months
            .iter()
            .filter(|m| m.key.starts_with(&year.key))
            .collect();
        out.push(Entry {
            target: Target::Period(year.key.clone()),
            depth: 0,
            title: year.key.clone(),
            subtitle: format!(
                "{}  \u{00B7}  {}",
                months_word(months_of_year.len()),
                matches_word(year.matches)
            ),
            right: record(year),
            tone: tone_of(year),
            group: Some(Group { key, expanded }),
        });
        if expanded {
            for month in months_of_year {
                let Some((y, number)) =
                    reclutched_core::rocketleague::dates::parse_month(&month.key)
                else {
                    continue;
                };
                out.push(Entry {
                    target: Target::Period(month.key.clone()),
                    depth: 1,
                    title: MONTHS[number as usize - 1].to_string(),
                    subtitle: matches_word(month.matches),
                    right: record(month),
                    tone: tone_of(month),
                    group: None,
                });
                let _ = y;
            }
        }
    }
    out
}

fn days_word(n: usize) -> String {
    if n == 1 {
        "1 day".to_string()
    } else {
        format!("{n} days")
    }
}

fn months_word(n: usize) -> String {
    if n == 1 {
        "1 month".to_string()
    } else {
        format!("{n} months")
    }
}

/// The rows listed inside a period's page: a day's matches, a month's days, a year's months.
pub fn children_of(
    key: &str,
    today: Day,
    matches: &[MatchRow],
    days: &[PeriodSummary],
    months: &[PeriodSummary],
    mode_name: &dyn Fn(i64) -> String,
    when: &dyn Fn(i64) -> String,
) -> Vec<Entry> {
    match key.len() {
        // A day: its matches.
        10 => matches
            .iter()
            .map(|m| match_entry(m, mode_name, when))
            .collect(),
        // A month: its days.
        7 => days
            .iter()
            .filter(|d| d.key.starts_with(key))
            .filter_map(|d| Day::parse(&d.key).map(|day| day_entry(d, day, today, 0)))
            .collect(),
        // A year: its months.
        4 => months
            .iter()
            .filter(|m| m.key.starts_with(key))
            .filter_map(|m| {
                let (_, number) = reclutched_core::rocketleague::dates::parse_month(&m.key)?;
                Some(Entry {
                    target: Target::Period(m.key.clone()),
                    depth: 0,
                    title: MONTHS[number as usize - 1].to_string(),
                    subtitle: matches_word(m.matches),
                    right: record(m),
                    tone: tone_of(m),
                    group: None,
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(key: &str, wins: i64, losses: i64, abandoned: i64) -> PeriodSummary {
        PeriodSummary {
            key: key.to_string(),
            matches: wins + losses + abandoned,
            wins,
            losses,
            abandoned,
            seconds_played: 0,
            avg_score: 0.0,
            avg_goals: 0.0,
            avg_assists: 0.0,
            avg_saves: 0.0,
            avg_shots: 0.0,
            avg_possession: None,
            avg_boost: None,
            avg_boost_empty: None,
            avg_speed: None,
        }
    }

    fn match_row(id: i64, result: &str) -> MatchRow {
        MatchRow {
            id,
            ended_at: 1_000 + id,
            playlist_id: 3,
            my_team: 0,
            team0_goals: 3,
            team1_goals: 1,
            overtime: false,
            result: result.to_string(),
            abandoned_at: None,
        }
    }

    /// Today is Friday 2026-10-23. The data has: two matches today; the 22nd, 21st and 3rd of
    /// this month; August and September (this year); and two earlier years.
    fn titles(expanded: &[&str]) -> Vec<(u8, String)> {
        let today = Day::parse("2026-10-23").unwrap();
        let today_matches = [match_row(11, "win"), match_row(10, "loss")];
        let days = [
            summary("2026-10-22", 3, 2, 0),
            summary("2026-10-21", 1, 4, 1),
            summary("2026-10-03", 2, 2, 0),
            summary("2026-09-30", 5, 1, 0),
            summary("2026-09-02", 1, 0, 0),
            summary("2026-08-15", 0, 0, 2),
            summary("2025-12-31", 4, 0, 0),
            summary("2024-06-01", 1, 1, 0),
        ];
        let months = [
            summary("2026-10", 6, 8, 1),
            summary("2026-09", 6, 1, 0),
            summary("2026-08", 0, 0, 2),
            summary("2025-12", 4, 0, 0),
            summary("2024-06", 1, 1, 0),
        ];
        let years = [
            summary("2026", 12, 10, 3),
            summary("2025", 4, 0, 0),
            summary("2024", 1, 1, 0),
        ];
        let expanded: HashSet<String> = expanded.iter().map(|s| s.to_string()).collect();
        build_entries(&Inputs {
            today,
            today_matches: &today_matches,
            days: &days,
            months: &months,
            years: &years,
            expanded: &expanded,
            mode_name: &|_| "Casual".to_string(),
            when: &|t| format!("t{t}"),
        })
        .into_iter()
        .map(|e| (e.depth, e.title))
        .collect()
    }

    fn t(depth: u8, title: &str) -> (u8, String) {
        (depth, title.to_string())
    }

    #[test]
    fn the_list_has_home_today_then_days_then_month_and_year_groups() {
        assert_eq!(
            titles(&[]),
            vec![
                t(0, "Home"),
                t(0, "Casual"),
                t(0, "Casual"),
                t(0, "Yesterday"),
                t(0, "Wed, Oct 21"),
                t(0, "Sat, Oct 3"),
                t(0, "September 2026"),
                t(0, "August 2026"),
                t(0, "2025"),
                t(0, "2024"),
            ]
        );
    }

    #[test]
    fn an_open_month_shows_its_days_and_an_open_year_only_its_months() {
        let rows = titles(&["m:2026-09", "y:2025", "y:2024"]);
        assert_eq!(
            &rows[6..],
            &[
                t(0, "September 2026"),
                t(1, "Wed, Sep 30"),
                t(1, "Wed, Sep 2"),
                t(0, "August 2026"),
                t(0, "2025"),
                t(1, "December"),
                t(0, "2024"),
                t(1, "June"),
            ]
        );
        // No day rows inside a year.
        let first_year = rows.iter().position(|(_, title)| title == "2025").unwrap();
        assert!(!rows[first_year..]
            .iter()
            .any(|(_, title)| title.contains(',')));
    }

    #[test]
    fn rows_know_their_target_tone_and_group() {
        let today = Day::parse("2026-10-23").unwrap();
        let days = [summary("2026-10-22", 3, 2, 0)];
        let empty = HashSet::new();
        let entries = build_entries(&Inputs {
            today,
            today_matches: &[],
            days: &days,
            months: &[],
            years: &[],
            expanded: &empty,
            mode_name: &|_| String::new(),
            when: &|_| String::new(),
        });
        assert_eq!(entries[0].target, Target::Home);
        assert_eq!(entries[0].subtitle, "Nothing played today yet");
        let yesterday = &entries[1];
        assert_eq!(yesterday.target, Target::Period("2026-10-22".into()));
        assert_eq!(
            (yesterday.right.as_str(), yesterday.tone),
            ("3 \u{2013} 2", Tone::Win)
        );
        assert_eq!(yesterday.subtitle, "5 matches");
        assert!(yesterday.group.is_none());
    }

    #[test]
    fn tones_follow_wins_and_losses() {
        assert_eq!(tone_of(&summary("x", 3, 1, 0)), Tone::Win);
        assert_eq!(tone_of(&summary("x", 1, 3, 0)), Tone::Loss);
        assert_eq!(tone_of(&summary("x", 2, 2, 0)), Tone::Abandoned);
        assert_eq!(tone_of(&summary("x", 0, 0, 3)), Tone::Neutral);
    }

    #[test]
    fn a_period_page_lists_its_matches_days_or_months() {
        let today = Day::parse("2026-10-23").unwrap();
        let rows = [match_row(2, "win"), match_row(1, "loss")];
        let days = [
            summary("2026-09-30", 1, 0, 0),
            summary("2026-09-02", 0, 1, 0),
            summary("2026-08-15", 1, 0, 0),
        ];
        let months = [
            summary("2026-09", 1, 1, 0),
            summary("2026-08", 1, 0, 0),
            summary("2025-12", 1, 0, 0),
        ];
        let names = |_: i64| "Casual".to_string();
        let when = |t: i64| format!("t{t}");
        let of = |key: &str| {
            children_of(key, today, &rows, &days, &months, &names, &when)
                .into_iter()
                .map(|e| e.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(of("2026-10-22"), ["Casual", "Casual"]);
        assert_eq!(of("2026-09"), ["Wed, Sep 30", "Wed, Sep 2"]);
        assert_eq!(of("2026"), ["September", "August"]);
        assert!(of("nonsense").is_empty());
    }

    #[test]
    fn subtitles_say_the_date_or_how_much_is_inside() {
        assert_eq!(period_subtitle("2026-10-23", 2), "Friday, October 23, 2026");
        assert_eq!(period_subtitle("2026-09", 12), "12 days played");
        assert_eq!(period_subtitle("2026-09", 1), "1 day played");
        assert_eq!(period_subtitle("2025", 3), "3 months");
    }

    #[test]
    fn headings_name_the_period() {
        let today = Day::parse("2026-10-23").unwrap();
        assert_eq!(period_title("2026-10-23", today), "Today");
        assert_eq!(period_title("2026-10-22", today), "Yesterday");
        assert_eq!(period_title("2026-10-03", today), "Sat, Oct 3");
        assert_eq!(period_title("2026-09", today), "September 2026");
        assert_eq!(period_title("2025", today), "2025");
    }
}
