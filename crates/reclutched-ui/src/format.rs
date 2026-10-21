//! Small text helpers for the screens. Kept free of GTK (except the local-time one at the
//! bottom) so they can be tested.

use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::storage::MatchRow;
use reclutched_core::rocketleague::tracker::{Clock, PlayerSummary};

/// The CSS class that colours a result: win, loss or abandoned.
pub fn result_class(result: &str) -> &'static str {
    match result {
        "win" => "win",
        "loss" => "loss",
        _ => "abandoned",
    }
}

pub fn result_word(result: &str) -> &'static str {
    match result {
        "win" => "WIN",
        "loss" => "LOSS",
        _ => "LEFT EARLY",
    }
}

/// (my goals, their goals).
pub fn my_score(row: &MatchRow) -> (i64, i64) {
    if row.my_team == 0 {
        (row.team0_goals, row.team1_goals)
    } else {
        (row.team1_goals, row.team0_goals)
    }
}

/// "4 - 2" with a proper dash.
pub fn score_text(row: &MatchRow) -> String {
    let (mine, theirs) = my_score(row);
    format!("{mine} \u{2013} {theirs}")
}

/// A running score [team0, team1] seen from my team's side.
pub fn score_from_my_side(after: [i64; 2], my_team: i64) -> String {
    let (mine, theirs) = if my_team == 0 {
        (after[0], after[1])
    } else {
        (after[1], after[0])
    };
    format!("{mine} \u{2013} {theirs}")
}

/// The match clock: "4:22". In overtime the game counts up, which we show as "OT 0:45".
pub fn clock_text(clock: Clock) -> String {
    let secs = clock.remaining.max(0);
    let text = format!("{}:{:02}", secs / 60, secs % 60);
    if clock.overtime {
        format!("OT {text}")
    } else {
        text
    }
}

/// "7 matches: 4 won, 2 lost, 1 abandoned" (and the abandon share when there is one).
pub fn summary_text(wins: i64, losses: i64, abandoned: i64) -> String {
    let total = wins + losses + abandoned;
    match total {
        0 => "No matches yet".to_string(),
        1 => "1 match".to_string(),
        _ => {
            let mut text = format!("{total} matches: {wins} won, {losses} lost");
            if abandoned > 0 {
                let share = abandoned as f64 * 100.0 / total as f64;
                text.push_str(&format!(", {abandoned} left early ({share:.0}%)"));
            }
            text
        }
    }
}

pub fn pct(value: f64) -> String {
    format!("{value:.0}%")
}

/// "Zed (you)" / "Nova" / "Player 3" when names are not stored (the roster switch was off).
pub fn player_name(players: &[PlayerSummary], slot: usize) -> String {
    match players.iter().find(|p| p.slot == slot) {
        Some(p) if !p.name.is_empty() && p.is_me => format!("{} (you)", p.name),
        Some(p) if !p.name.is_empty() => p.name.clone(),
        Some(p) if p.is_me => "You".to_string(),
        _ => format!("Player {}", slot + 1),
    }
}

/// The mode shown in the list: the name from the table, or "Playlist 17".
pub fn mode_name(names: &PlaylistMap, playlist_id: i64) -> String {
    names.display(playlist_id)
}

/// A word for how long ago a day was, when it is recent enough to be worth a word.
pub fn relative_day(days_ago: i64) -> Option<&'static str> {
    match days_ago {
        0 => Some("Today"),
        1 => Some("Yesterday"),
        _ => None,
    }
}

/// Local date and time: "Today, 7:17 PM", "Yesterday, 9:40 PM" or "Oct 18, 7:17 PM".
/// GLib knows the time zone, so no extra crate is needed.
pub fn when_label(unix: i64) -> String {
    use gtk::glib::DateTime;
    let Ok(then) = DateTime::from_unix_local(unix) else {
        return String::new();
    };
    let clock = then
        .format("%-I:%M %p")
        .map(|s| s.to_string())
        .unwrap_or_default();
    let day = DateTime::now_local().ok().and_then(|now| {
        // Whole days between the two dates, ignoring the time of day.
        let (a, b) = (now.ymd(), then.ymd());
        let midnight = |(y, m, d): (i32, i32, i32)| {
            DateTime::from_local(y, m, d, 0, 0, 0.0)
                .map(|t| t.to_unix())
                .ok()
        };
        let (a, b) = (midnight(a)?, midnight(b)?);
        Some((a - b + 3_600) / 86_400)
    });
    match day.and_then(relative_day) {
        Some(word) => format!("{word}, {clock}"),
        None => {
            let date = then
                .format("%b %-d")
                .map(|s| s.to_string())
                .unwrap_or_default();
            format!("{date}, {clock}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(team: i64, goals: [i64; 2], result: &str) -> MatchRow {
        MatchRow {
            id: 1,
            ended_at: 0,
            playlist_id: 3,
            my_team: team,
            team0_goals: goals[0],
            team1_goals: goals[1],
            overtime: false,
            result: result.into(),
            abandoned_at: None,
        }
    }

    fn player(slot: usize, name: &str, is_me: bool) -> PlayerSummary {
        PlayerSummary {
            slot,
            name: name.into(),
            primary_id: None,
            team: 0,
            is_me,
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
        }
    }

    #[test]
    fn result_words_and_classes() {
        assert_eq!(result_class("win"), "win");
        assert_eq!(result_class("loss"), "loss");
        assert_eq!(result_class("abandoned"), "abandoned");
        assert_eq!(result_word("win"), "WIN");
        assert_eq!(result_word("abandoned"), "LEFT EARLY");
    }

    #[test]
    fn scores_are_shown_from_my_side() {
        assert_eq!(score_text(&row(0, [4, 2], "win")), "4 \u{2013} 2");
        assert_eq!(score_text(&row(1, [4, 2], "loss")), "2 \u{2013} 4");
        assert_eq!(score_from_my_side([1, 3], 1), "3 \u{2013} 1");
        assert_eq!(score_from_my_side([1, 3], 0), "1 \u{2013} 3");
    }

    #[test]
    fn clock_text_handles_minutes_overtime_and_negatives() {
        let c = |remaining, overtime| Clock {
            remaining,
            overtime,
        };
        assert_eq!(clock_text(c(262, false)), "4:22");
        assert_eq!(clock_text(c(5, false)), "0:05");
        assert_eq!(clock_text(c(0, false)), "0:00");
        assert_eq!(clock_text(c(45, true)), "OT 0:45");
        assert_eq!(clock_text(c(-3, false)), "0:00");
    }

    #[test]
    fn summary_wording() {
        assert_eq!(summary_text(0, 0, 0), "No matches yet");
        assert_eq!(summary_text(1, 0, 0), "1 match");
        assert_eq!(summary_text(4, 2, 0), "6 matches: 4 won, 2 lost");
        assert_eq!(
            summary_text(4, 2, 2),
            "8 matches: 4 won, 2 lost, 2 left early (25%)"
        );
    }

    #[test]
    fn player_names_fall_back_when_the_roster_was_off() {
        let players = vec![
            player(0, "Zed", true),
            player(1, "Nova", false),
            player(2, "", false),
            player(3, "", true),
        ];
        assert_eq!(player_name(&players, 0), "Zed (you)");
        assert_eq!(player_name(&players, 1), "Nova");
        assert_eq!(player_name(&players, 2), "Player 3");
        assert_eq!(player_name(&players, 3), "You");
        assert_eq!(player_name(&players, 9), "Player 10");
    }

    #[test]
    fn relative_days() {
        assert_eq!(relative_day(0), Some("Today"));
        assert_eq!(relative_day(1), Some("Yesterday"));
        assert_eq!(relative_day(2), None);
        assert_eq!(relative_day(-1), None);
        assert_eq!(pct(46.4), "46%");
    }

    #[test]
    fn unknown_playlists_show_their_number() {
        let names = PlaylistMap::with_builtin();
        assert_eq!(mode_name(&names, 3), "Casual Standard");
        assert_eq!(mode_name(&names, 12), "Playlist 12");
    }
}
