//! `reclutched history`: a table of saved matches, green for wins and red for losses.

use anyhow::Result;
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::storage::{MatchDetail, MatchRow, Store};
use reclutched_core::rocketleague::tracker::PlayerSummary;

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

/// Prints every number we stored for one match (`None` = the newest match).
pub fn run_detail(store: &Store, names: &PlaylistMap, id: Option<i64>, color: bool) -> Result<()> {
    let Some(id) = id.map_or_else(|| store.latest_match_id(), |id| Ok(Some(id)))? else {
        println!("No matches saved yet. Run `reclutched track` while you play.");
        return Ok(());
    };
    let Some(detail) = store.match_detail(id)? else {
        println!("There is no match #{id}.");
        return Ok(());
    };
    for line in format_detail(&detail, names, color) {
        println!("{line}");
    }
    Ok(())
}

/// "You", the player's name, or "Player 3" when names are not stored (roster switch off).
fn who(players: &[PlayerSummary], slot: usize) -> String {
    match players.iter().find(|p| p.slot == slot) {
        Some(p) if p.is_me && !p.name.is_empty() => format!("{} (you)", p.name),
        Some(p) if p.is_me => "You".to_string(),
        Some(p) if !p.name.is_empty() => p.name.clone(),
        _ => format!("Player {}", slot + 1),
    }
}

pub fn format_detail(d: &MatchDetail, names: &PlaylistMap, color: bool) -> Vec<String> {
    let (body, tint) = row_body(&d.row, names);
    let mut out = vec![paint(format!("Match #{}  {body}", d.row.id), tint, color)];

    let mut facts = Vec::new();
    if let Some(p) = d.my_possession_pct {
        facts.push(format!("possession {p:.0}%"));
    }
    if let (Some(avg), Some(max)) = (d.ball_avg_speed, d.ball_max_speed) {
        facts.push(format!("ball speed avg {avg} / max {max}"));
    }
    if let Some(o) = d.team_empty_overlap {
        facts.push(format!(
            "you and a teammate both out of boost {o:.0}% of the time"
        ));
    }
    if !facts.is_empty() {
        out.push(format!("  {}", facts.join("   ")));
    }

    if !d.movement.is_empty() {
        out.push(String::new());
        out.push("Boost and movement (you and your teammates), % of live time".into());
        out.push(format!(
            "  {:<22} {:>5} {:>5} {:>5} {:>5} {:>5} {:>6} {:>6} {:>6} {:>5} {:>5} {:>5} {:>5} {:>5}",
            "", "boost", "empty", "low", "mid", "high", "used", "gained", "km/h", "super", "slide", "air", "wall", "dead"
        ));
        for m in &d.movement {
            out.push(format!(
                "  {:<22} {:>5.0} {:>4.0}% {:>4.0}% {:>4.0}% {:>4.0}% {:>6} {:>6} {:>6.0} {:>4.0}% {:>4.0}% {:>4.0}% {:>4.0}% {:>4.0}%",
                who(&d.players, m.slot),
                m.avg_boost,
                m.pct_boost_empty,
                m.pct_boost_low,
                m.pct_boost_mid,
                m.pct_boost_high,
                m.boost_used,
                m.boost_gained,
                m.avg_speed,
                m.pct_supersonic,
                m.pct_powersliding,
                m.pct_airborne,
                m.pct_on_wall,
                m.pct_demolished
            ));
        }
    }

    if !d.touches.is_empty() {
        out.push(String::new());
        out.push("Ball touches".into());
        out.push(format!(
            "  {:<22} {:>7} {:>6} {:>6} {:>6} {:>9}",
            "", "touches", "own 3rd", "middle", "enemy 3rd", "avg hit"
        ));
        for t in &d.touches {
            out.push(format!(
                "  {:<22} {:>7} {:>6} {:>6} {:>9} {:>9}",
                who(&d.players, t.slot),
                t.touches,
                t.def_third,
                t.mid_third,
                t.att_third,
                t.avg_post_speed
            ));
        }
    }

    if !d.event_stats.is_empty() {
        out.push(String::new());
        out.push("Your boost and speed when it happened".into());
        for e in d.event_stats.iter().filter(|e| e.slot == d.my_slot) {
            let ctx = match (e.avg_boost, e.avg_speed) {
                (Some(b), Some(v)) => format!("avg boost {b:.0}, avg speed {v:.0}"),
                (Some(b), None) => format!("avg boost {b:.0}"),
                _ => String::new(),
            };
            out.push(format!("  {:<12} x{:<3} {ctx}", e.event, e.count));
        }
    }
    out
}

/// The part of a row after the id: date, mode, result and score. Also returns the color.
fn row_body(r: &MatchRow, names: &PlaylistMap) -> (String, &'static str) {
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
    let body = format!(
        "{:<16}  {:<24}  {:<9}  {mine}-{theirs}{note}",
        format_utc(r.ended_at),
        names.display(r.playlist_id),
        label
    );
    (body, tint)
}

fn paint(text: String, tint: &str, color: bool) -> String {
    if color {
        format!("{tint}{text}{RESET}")
    } else {
        text
    }
}

pub fn format_rows(rows: &[MatchRow], names: &PlaylistMap, color: bool) -> Vec<String> {
    let mut lines = vec![format!(
        "{:>5}  {:<16}  {:<24}  {:<9}  {}",
        "#", "Date (UTC)", "Mode", "Result", "Score"
    )];
    for r in rows {
        let (body, tint) = row_body(r, names);
        lines.push(paint(format!("{:>5}  {body}", r.id), tint, color));
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
    fn detail_view_lists_the_numbers_and_names_you() {
        use reclutched_core::rocketleague::tracker::{EventStat, MovementSummary, TouchSummary};
        let me = PlayerSummary {
            slot: 0,
            name: "Zed".into(),
            primary_id: None,
            team: 0,
            is_me: true,
            score: 0,
            goals: 0,
            shots: 0,
            assists: 0,
            saves: 0,
            touches: 0,
            car_touches: 0,
            demos: 0,
            joined: None,
            left: None,
        };
        let mate = PlayerSummary {
            slot: 1,
            name: String::new(),
            is_me: false,
            ..me.clone()
        };
        let d = MatchDetail {
            row: row(5, 0, [3, 1], "win"),
            my_slot: 0,
            my_possession_pct: Some(54.4),
            ball_avg_speed: Some(1120),
            ball_max_speed: Some(3010),
            team_empty_overlap: Some(6.0),
            players: vec![me, mate],
            movement: vec![MovementSummary {
                slot: 0,
                samples: 100,
                avg_boost: 41.7,
                pct_boost_empty: 12.0,
                pct_boost_low: 20.0,
                pct_boost_mid: 30.0,
                pct_boost_high: 38.0,
                pct_boosting: 22.0,
                boost_used: 800,
                boost_gained: 900,
                avg_speed: 1350.0,
                max_speed: 2300.0,
                pct_supersonic: 18.0,
                pct_powersliding: 3.0,
                pct_on_ground: 60.0,
                pct_on_wall: 5.0,
                pct_airborne: 35.0,
                pct_demolished: 2.0,
                times_demolished: 1,
            }],
            touches: vec![TouchSummary {
                slot: 1,
                touches: 9,
                def_third: 3,
                mid_third: 4,
                att_third: 2,
                avg_post_speed: 1500.0,
                max_post_speed: 2500.0,
            }],
            event_stats: vec![EventStat {
                slot: 0,
                event: "Save".into(),
                count: 2,
                avg_boost: Some(20.4),
                avg_speed: Some(1000.0),
            }],
        };
        let text = format_detail(&d, &PlaylistMap::with_builtin(), false).join("\n");
        assert!(text.starts_with("Match #5  2023-"), "{text}");
        assert!(text.contains("Casual Standard"));
        assert!(text.contains("possession 54%") && text.contains("ball speed avg 1120 / max 3010"));
        assert!(text.contains("Zed (you)") && text.contains("Player 2"));
        assert!(text.contains("Save") && text.contains("avg boost 20"));
        assert!(!text.contains('\x1b'));
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
