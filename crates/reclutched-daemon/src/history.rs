//! `reclutched history`: a table of saved matches, green for wins and red for losses.

use anyhow::Result;
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::storage::{MatchRow, Store};

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// Prints the newest `limit` matches and a summary. `color` is off when output is piped.
pub fn run(store: &Store, names: &PlaylistMap, limit: u32, color: bool) -> Result<()> {
    let rows = store.recent_matches(limit)?;
    if rows.is_empty() {
        println!("No matches saved yet. Run `reclutched track` while you play.");
        return Ok(());
    }
    for line in format_rows(&rows, names, color) {
        println!("{line}");
    }
    let (wins, losses, abandoned) = store.result_counts()?;
    println!("{}", format_summary(wins, losses, abandoned, color));
    Ok(())
}

pub fn format_rows(rows: &[MatchRow], names: &PlaylistMap, color: bool) -> Vec<String> {
    let mut lines = vec![format!(
        "{:>5}  {:<16}  {:<24}  {:<9}  {}",
        "#", "Date (UTC)", "Mode", "Result", "Score"
    )];
    for r in rows {
        let (mine, theirs) = if r.my_team == 0 {
            (r.team0_goals, r.team1_goals)
        } else {
            (r.team1_goals, r.team0_goals)
        };
        let (label, tint) = match r.result.as_str() {
            "win" => ("Win", GREEN),
            "loss" => ("Loss", RED),
            _ => ("Abandoned", YELLOW),
        };
        let mut note = String::new();
        if r.overtime {
            note.push_str(" (overtime)");
        }
        if let Some(clock) = r.abandoned_at {
            let ot = if clock.overtime { " overtime" } else { "" };
            note.push_str(&format!(
                " (left with {}:{:02} on the clock{ot})",
                clock.remaining / 60,
                clock.remaining % 60
            ));
        }
        let row = format!(
            "{:>5}  {:<16}  {:<24}  {:<9}  {mine}-{theirs}{note}",
            r.id,
            format_utc(r.ended_at),
            names.display(r.playlist_id),
            label
        );
        lines.push(if color {
            format!("{tint}{row}{RESET}")
        } else {
            row
        });
    }
    lines
}

pub fn format_summary(wins: i64, losses: i64, abandoned: i64, color: bool) -> String {
    let total = wins + losses + abandoned;
    let percent = |n: i64| {
        if total == 0 {
            0.0
        } else {
            n as f64 * 100.0 / total as f64
        }
    };
    let text = format!(
        "\n{total} matches: {wins} won, {losses} lost, {abandoned} abandoned ({:.0}% abandoned)",
        percent(abandoned)
    );
    if color {
        format!("{DIM}{text}{RESET}")
    } else {
        text
    }
}

/// "2026-10-19 22:41" from unix seconds, in UTC. (Local time would need a time-zone crate.)
pub fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Days since 1970-01-01 -> calendar date (Howard Hinnant's "civil from days" algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reclutched_core::rocketleague::tracker::Clock;

    fn row(id: i64, team: i64, goals: [i64; 2], result: &str) -> MatchRow {
        MatchRow {
            id,
            ended_at: 1_700_000_000,
            playlist_id: 3,
            my_team: team,
            team0_goals: goals[0],
            team1_goals: goals[1],
            overtime: false,
            result: result.into(),
            abandoned_at: None,
        }
    }

    #[test]
    fn utc_dates_are_right() {
        assert_eq!(format_utc(0), "1970-01-01 00:00");
        assert_eq!(format_utc(1_000_000_000), "2001-09-09 01:46");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13");
        // Leap day.
        assert_eq!(format_utc(1_709_164_800), "2024-02-29 00:00");
    }

    #[test]
    fn wins_are_green_losses_red_abandoned_yellow() {
        let names = PlaylistMap::with_builtin();
        let rows = [
            row(1, 0, [3, 1], "win"),
            row(2, 0, [1, 3], "loss"),
            row(3, 1, [0, 0], "abandoned"),
        ];
        let lines = format_rows(&rows, &names, true);
        assert!(lines[1].starts_with(GREEN) && lines[1].ends_with(RESET));
        assert!(lines[2].starts_with(RED));
        assert!(lines[3].starts_with(YELLOW));
    }

    #[test]
    fn score_is_shown_from_my_side_and_plain_when_color_is_off() {
        let names = PlaylistMap::with_builtin();
        // I am on team 1, team 1 scored 4: "4-2" is my view.
        let lines = format_rows(&[row(7, 1, [2, 4], "win")], &names, false);
        assert!(lines[1].contains("4-2") && lines[1].contains("Casual Standard"));
        assert!(!lines[1].contains('\x1b'));
    }

    #[test]
    fn abandoned_rows_say_when_you_left_and_overtime_is_marked() {
        let names = PlaylistMap::with_builtin();
        let mut r = row(9, 0, [1, 1], "abandoned");
        r.abandoned_at = Some(Clock {
            remaining: 125,
            overtime: false,
        });
        let mut o = row(10, 0, [2, 1], "win");
        o.overtime = true;
        let lines = format_rows(&[r, o], &names, false);
        assert!(lines[1].contains("left with 2:05 on the clock"));
        assert!(lines[2].contains("(overtime)"));
    }

    #[test]
    fn summary_shows_abandon_percentage() {
        let text = format_summary(6, 2, 2, false);
        assert!(text.contains("10 matches: 6 won, 2 lost, 2 abandoned (20% abandoned)"));
        assert!(format_summary(0, 0, 0, false).contains("0 matches"));
    }
}
