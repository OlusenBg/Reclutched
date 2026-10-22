//! The overlay panel you get while holding the overlay key: possession on top, your habits as
//! bars on the left, the players on the right. Everything is drawn on cards over the game; the
//! space between them stays see-through.

use gtk::prelude::*;
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::storage::{MatchRow, MovementAverages, RecentForm};
use reclutched_core::rocketleague::tracker::{LivePlayer, LiveSnapshot, MovementSummary};

use crate::format::{mode_name, pct, result_class, score_text, when_label};
use crate::widgets::{label, name_label, split_bar, stripe, vbox};

const BLUE: (f64, f64, f64) = (0.31, 0.56, 0.88);
const ORANGE: (f64, f64, f64) = (0.91, 0.54, 0.23);

/// Samples (ten per second) before a bar is judged against your average; before that the
/// numbers jump around too much.
const WARM_UP_SAMPLES: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Better,
    Same,
    Worse,
    /// No average to compare with, or this bar is not good or bad by itself.
    None,
}

/// How a live value compares with your usual one. `higher_is_better` is `None` for numbers
/// that are a style, not a quality (how much you powerslide).
pub fn verdict(value: f64, usual: Option<f64>, higher_is_better: Option<bool>) -> Verdict {
    let (Some(usual), Some(higher_is_better)) = (usual, higher_is_better) else {
        return Verdict::None;
    };
    // Within a tenth of your usual value (and at least 1.5 units) counts as the same.
    let tolerance = (usual.abs() * 0.10).max(1.5);
    let delta = value - usual;
    if delta.abs() <= tolerance {
        Verdict::Same
    } else if (delta > 0.0) == higher_is_better {
        Verdict::Better
    } else {
        Verdict::Worse
    }
}

/// One line of the "you" card.
struct Metric {
    name: &'static str,
    value: f64,
    shown: String,
    /// Where a full bar is.
    scale: f64,
    usual: Option<f64>,
    higher_is_better: Option<bool>,
}

fn metrics(me: &MovementSummary, usual: Option<&MovementAverages>) -> Vec<Metric> {
    vec![
        Metric {
            name: "Out of boost",
            value: me.pct_boost_empty,
            shown: pct(me.pct_boost_empty),
            scale: 50.0,
            usual: usual.map(|u| u.pct_boost_empty),
            higher_is_better: Some(false),
        },
        Metric {
            name: "Average boost",
            value: me.avg_boost,
            shown: format!("{:.0}", me.avg_boost),
            scale: 100.0,
            usual: usual.map(|u| u.avg_boost),
            higher_is_better: Some(true),
        },
        Metric {
            name: "Average speed",
            value: me.avg_speed,
            shown: format!("{:.0} km/h", me.avg_speed),
            scale: 100.0,
            usual: usual.map(|u| u.avg_speed),
            higher_is_better: Some(true),
        },
        Metric {
            name: "Supersonic",
            value: me.pct_supersonic,
            shown: pct(me.pct_supersonic),
            scale: 50.0,
            usual: usual.map(|u| u.pct_supersonic),
            higher_is_better: Some(true),
        },
        Metric {
            name: "Powersliding",
            value: me.pct_powersliding,
            shown: pct(me.pct_powersliding),
            scale: 15.0,
            usual: usual.map(|u| u.pct_powersliding),
            higher_is_better: None,
        },
        Metric {
            name: "In the air",
            value: me.pct_airborne,
            shown: pct(me.pct_airborne),
            scale: 100.0,
            usual: usual.map(|u| u.pct_airborne),
            higher_is_better: None,
        },
        Metric {
            name: "On walls",
            value: me.pct_on_wall,
            shown: pct(me.pct_on_wall),
            scale: 30.0,
            usual: usual.map(|u| u.pct_on_wall),
            higher_is_better: None,
        },
    ]
}

/// A thin bar with a tick where your usual value is.
fn metric_bar(fraction: f64, marker: Option<f64>, verdict: Verdict) -> gtk::DrawingArea {
    let fraction = fraction.clamp(0.0, 1.0);
    let area = gtk::DrawingArea::builder()
        .content_height(10)
        .hexpand(true)
        .build();
    area.set_draw_func(move |_, cr, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.09);
        cr.rectangle(0.0, 2.0, w, h - 4.0);
        let _ = cr.fill();
        let (r, g, b) = match verdict {
            Verdict::Better => (0.37, 0.64, 0.44),
            Verdict::Worse => (0.85, 0.33, 0.31),
            Verdict::Same => (0.80, 0.78, 0.77),
            Verdict::None => (0.64, 0.15, 0.19),
        };
        cr.set_source_rgb(r, g, b);
        cr.rectangle(0.0, 2.0, w * fraction, h - 4.0);
        let _ = cr.fill();
        if let Some(m) = marker {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            cr.rectangle((w * m.clamp(0.0, 1.0) - 1.0).max(0.0), 0.0, 2.0, h);
            let _ = cr.fill();
        }
    });
    area
}

fn card() -> gtk::Box {
    let b = vbox(0);
    b.add_css_class("overlay-card");
    b
}

/// The "you" card: one bar per habit, judged against your own average.
fn you_card(me: &MovementSummary, usual: Option<&MovementAverages>) -> gtk::Box {
    let c = card();
    c.set_width_request(330);
    c.set_halign(gtk::Align::Start);
    c.set_valign(gtk::Align::Center);
    c.set_margin_start(40);

    let head = gtk::Box::builder().spacing(8).build();
    head.append(&label("YOU", &["overlay-key"]));
    let note = match usual {
        Some(u) => format!("tick = your average of {} matches", u.matches),
        None => "play a few matches to compare".to_string(),
    };
    let note = label(&note, &["overlay-note"]);
    note.set_hexpand(true);
    note.set_halign(gtk::Align::End);
    head.append(&note);
    c.append(&head);

    let warmed_up = me.samples >= WARM_UP_SAMPLES;
    for m in metrics(me, usual) {
        let judged = if warmed_up {
            verdict(m.value, m.usual, m.higher_is_better)
        } else {
            Verdict::None
        };
        let line = vbox(4);
        line.set_margin_top(14);
        let top = gtk::Box::builder().spacing(10).build();
        top.append(&label(m.name, &["overlay-label"]));
        let value = label(&m.shown, &["overlay-value"]);
        value.set_hexpand(true);
        value.set_halign(gtk::Align::End);
        top.append(&value);
        line.append(&top);
        line.append(&metric_bar(
            m.value / m.scale,
            m.usual.map(|u| u / m.scale),
            judged,
        ));
        c.append(&line);
    }
    c
}

fn possession_bar(snapshot: &LiveSnapshot) -> Option<gtk::Box> {
    let [blue, orange] = snapshot.possession?;
    let c = card();
    c.set_width_request(560);
    c.set_halign(gtk::Align::Center);
    c.set_valign(gtk::Align::Start);
    c.set_margin_top(36);
    let row = gtk::Box::builder().spacing(14).build();
    row.append(&label(&pct(blue), &["overlay-big", "team-blue"]));
    let bar = split_bar(blue, BLUE, ORANGE);
    bar.set_valign(gtk::Align::Center);
    row.append(&bar);
    row.append(&label(&pct(orange), &["overlay-big", "team-orange"]));
    c.append(&label("POSSESSION", &["overlay-key"]));
    c.append(&row);
    Some(c)
}

fn players_card(snapshot: &LiveSnapshot) -> gtk::Box {
    let c = card();
    c.set_width_request(340);
    c.set_halign(gtk::Align::End);
    c.set_valign(gtk::Align::Center);
    c.set_margin_end(40);

    let first = snapshot.my_team.unwrap_or(0);
    for (i, team) in [first, 1 - first].into_iter().enumerate() {
        let players: Vec<&LivePlayer> =
            snapshot.players.iter().filter(|p| p.team == team).collect();
        if players.is_empty() {
            continue;
        }
        let goals = usize::try_from(team)
            .ok()
            .and_then(|t| snapshot.team_goals.get(t))
            .copied()
            .unwrap_or(0);
        let head = gtk::Box::builder().spacing(8).build();
        head.set_margin_top(if i == 0 { 0 } else { 18 });
        let team_class = if team == 0 {
            "team-blue"
        } else {
            "team-orange"
        };
        head.append(&label("\u{25CF}", &[team_class]));
        head.append(&label(
            if team == 0 { "BLUE" } else { "ORANGE" },
            &["overlay-key"],
        ));
        let g = label(&goals.to_string(), &["overlay-big", team_class]);
        g.set_hexpand(true);
        g.set_halign(gtk::Align::End);
        head.append(&g);
        c.append(&head);

        for p in players {
            let row = gtk::Box::builder().spacing(10).build();
            row.set_margin_top(8);
            let classes: &[&str] = if p.is_me {
                &["overlay-name", "me"]
            } else if p.left || p.demolished {
                &["overlay-name", "dim"]
            } else {
                &["overlay-name"]
            };
            let n = name_label(&p.name, classes);
            n.set_hexpand(true);
            row.append(&n);
            row.append(&label(&format!("{} G", p.goals), &["overlay-note"]));
            let score = label(&p.score.to_string(), &["overlay-value"]);
            score.set_width_chars(5);
            score.set_xalign(1.0);
            row.append(&score);
            c.append(&row);
        }
    }
    c
}

fn waiting_card(text: &str) -> gtk::Box {
    let c = card();
    c.set_halign(gtk::Align::Center);
    c.set_valign(gtk::Align::Start);
    c.set_margin_top(36);
    c.append(&label(text, &["overlay-label"]));
    c
}

/// Your recent matches, for the panel when no match is on.
pub struct Recent {
    /// Newest first (up to 20).
    pub matches: Vec<MatchRow>,
    pub form: RecentForm,
}

/// How many matches the list on the left shows.
const LIST_LEN: usize = 8;

fn result_color(result: &str) -> (f64, f64, f64) {
    match result {
        "win" => (0.37, 0.64, 0.44),
        "loss" => (0.85, 0.33, 0.31),
        _ => (0.79, 0.60, 0.23),
    }
}

/// One small square per match, newest first: green won, red lost, amber left early.
fn form_strip(matches: &[MatchRow]) -> gtk::DrawingArea {
    let results: Vec<(f64, f64, f64)> = matches.iter().map(|m| result_color(&m.result)).collect();
    let (size, gap) = (20, 6);
    let width = results.len() as i32 * (size + gap) - gap;
    let area = gtk::DrawingArea::builder()
        .content_width(width.max(1))
        .content_height(size)
        .halign(gtk::Align::Start)
        .build();
    area.set_draw_func(move |_, cr, _, _| {
        for (i, (r, g, b)) in results.iter().enumerate() {
            let x = f64::from(i as i32 * (size + gap));
            let side = f64::from(size);
            // A rounded square.
            let radius = 5.0;
            cr.new_sub_path();
            cr.arc(
                x + side - radius,
                radius,
                radius,
                -std::f64::consts::FRAC_PI_2,
                0.0,
            );
            cr.arc(
                x + side - radius,
                side - radius,
                radius,
                0.0,
                std::f64::consts::FRAC_PI_2,
            );
            cr.arc(
                x + radius,
                side - radius,
                radius,
                std::f64::consts::FRAC_PI_2,
                std::f64::consts::PI,
            );
            cr.arc(
                x + radius,
                radius,
                radius,
                std::f64::consts::PI,
                3.0 * std::f64::consts::FRAC_PI_2,
            );
            cr.close_path();
            cr.set_source_rgb(*r, *g, *b);
            let _ = cr.fill();
        }
    });
    area
}

/// "3 wins in a row", "2 losses in a row" or nothing.
fn streak_text(form: &RecentForm) -> Option<(String, &'static str)> {
    let (kind, count) = form.streak.as_ref()?;
    if *count < 2 {
        return None;
    }
    let (word, class) = match kind.as_str() {
        "win" => ("wins", "win"),
        "loss" => ("losses", "loss"),
        _ => return None,
    };
    Some((format!("{count} {word} in a row"), class))
}

/// The card on top: the strip of results, the record and the win rate.
fn form_card(recent: &Recent) -> gtk::Box {
    let form = &recent.form;
    let c = card();
    c.set_width_request(560);
    c.set_halign(gtk::Align::Center);
    c.set_valign(gtk::Align::Start);
    c.set_margin_top(36);

    let head = gtk::Box::builder().spacing(8).build();
    head.append(&label("RECENT FORM", &["overlay-key"]));
    let note = label(
        &format!("last {} matches, newest first", form.matches),
        &["overlay-note"],
    );
    note.set_hexpand(true);
    note.set_halign(gtk::Align::End);
    head.append(&note);
    c.append(&head);

    let strip = form_strip(&recent.matches);
    strip.set_margin_top(12);
    c.append(&strip);

    let numbers = gtk::Box::builder().spacing(16).build();
    numbers.set_margin_top(12);
    numbers.append(&label(
        &format!(
            "{} won  \u{00B7}  {} lost  \u{00B7}  {} left early",
            form.wins, form.losses, form.abandoned
        ),
        &["overlay-label"],
    ));
    if let Some((text, class)) = streak_text(form) {
        numbers.append(&label(&text, &["overlay-value", class]));
    }
    let played = form.wins + form.losses;
    if played > 0 {
        let rate = form.wins as f64 * 100.0 / played as f64;
        let l = label(
            &format!("{} wins", pct(rate)),
            &["overlay-big", if rate >= 50.0 { "win" } else { "loss" }],
        );
        l.set_hexpand(true);
        l.set_halign(gtk::Align::End);
        numbers.append(&l);
    }
    c.append(&numbers);
    c
}

/// The list on the left: your last matches with their colour, score and when.
fn matches_card(recent: &Recent, names: &PlaylistMap) -> gtk::Box {
    let c = card();
    c.set_width_request(360);
    c.set_halign(gtk::Align::Start);
    c.set_valign(gtk::Align::Center);
    c.set_margin_start(40);
    c.append(&label("RECENT MATCHES", &["overlay-key"]));
    for m in recent.matches.iter().take(LIST_LEN) {
        let row = gtk::Box::builder().spacing(12).build();
        row.set_margin_top(12);
        row.append(&stripe(result_class(&m.result)));
        let text = vbox(1);
        text.set_hexpand(true);
        text.append(&name_label(
            &mode_name(names, m.playlist_id),
            &["overlay-name"],
        ));
        let mut when = when_label(m.ended_at);
        if m.result == "abandoned" {
            when.push_str("  \u{00B7}  left early");
        } else if m.overtime {
            when.push_str("  \u{00B7}  overtime");
        }
        text.append(&label(&when, &["overlay-note"]));
        row.append(&text);
        let score = label(&score_text(m), &["row-score", result_class(&m.result)]);
        score.set_valign(gtk::Align::Center);
        row.append(&score);
        c.append(&row);
    }
    c
}

/// A caption and a value over a bar (no tick).
fn average_row(name: &str, shown: &str, fraction: f64) -> gtk::Box {
    let line = vbox(4);
    line.set_margin_top(14);
    let top = gtk::Box::builder().spacing(10).build();
    top.append(&label(name, &["overlay-label"]));
    let value = label(shown, &["overlay-value"]);
    value.set_hexpand(true);
    value.set_halign(gtk::Align::End);
    top.append(&value);
    line.append(&top);
    line.append(&metric_bar(fraction, None, Verdict::None));
    line
}

/// The card on the right: what you usually do per match.
fn averages_card(recent: &Recent, usual: Option<&MovementAverages>) -> gtk::Box {
    let form = &recent.form;
    let c = card();
    c.set_width_request(340);
    c.set_halign(gtk::Align::End);
    c.set_valign(gtk::Align::Center);
    c.set_margin_end(40);
    c.append(&label("YOUR AVERAGES", &["overlay-key"]));

    let played = form.wins + form.losses;
    if played > 0 {
        let rate = form.wins as f64 * 100.0 / played as f64;
        c.append(&average_row("Win rate", &pct(rate), rate / 100.0));
    }
    if let Some(p) = form.avg_possession {
        let line = vbox(4);
        line.set_margin_top(14);
        let top = gtk::Box::builder().spacing(10).build();
        top.append(&label("Possession", &["overlay-label"]));
        let value = label(&pct(p), &["overlay-value"]);
        value.set_hexpand(true);
        value.set_halign(gtk::Align::End);
        top.append(&value);
        line.append(&top);
        line.append(&split_bar(p, (0.64, 0.15, 0.19), (0.36, 0.31, 0.31)));
        c.append(&line);
    }
    c.append(&average_row(
        "Goals per match",
        &format!("{:.1}", form.avg_goals),
        form.avg_goals / 3.0,
    ));
    c.append(&average_row(
        "Saves per match",
        &format!("{:.1}", form.avg_saves),
        form.avg_saves / 3.0,
    ));
    c.append(&average_row(
        "Shots per match",
        &format!("{:.1}", form.avg_shots),
        form.avg_shots / 6.0,
    ));
    c.append(&average_row(
        "Score per match",
        &format!("{:.0}", form.avg_score),
        form.avg_score / 600.0,
    ));
    if let Some(u) = usual {
        c.append(&average_row(
            "Average boost",
            &format!("{:.0}", u.avg_boost),
            u.avg_boost / 100.0,
        ));
    }
    c
}

/// What the panel shows when no match is on: how your recent matches went.
fn idle_pieces(
    recent: Option<&Recent>,
    usual: Option<&MovementAverages>,
    names: &PlaylistMap,
) -> Vec<gtk::Widget> {
    let Some(recent) = recent.filter(|r| !r.matches.is_empty()) else {
        return vec![waiting_card("No matches yet. Play one and it shows up here.").upcast()];
    };
    vec![
        form_card(recent).upcast(),
        matches_card(recent, names).upcast(),
        averages_card(recent, usual).upcast(),
    ]
}

/// The pieces of the panel, ready to be placed on a `gtk::Overlay`. With no match on, it is your
/// recent form instead.
pub fn build(
    snapshot: Option<&LiveSnapshot>,
    usual: Option<&MovementAverages>,
    recent: Option<&Recent>,
    names: &PlaylistMap,
) -> Vec<gtk::Widget> {
    let Some(snapshot) = snapshot else {
        return idle_pieces(recent, usual, names);
    };
    let mut pieces: Vec<gtk::Widget> = Vec::new();
    if let Some(bar) = possession_bar(snapshot) {
        pieces.push(bar.upcast());
    }
    if let Some(me) = &snapshot.me_movement {
        pieces.push(you_card(me, usual).upcast());
    }
    pieces.push(players_card(snapshot).upcast());
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_follow_the_direction_and_a_tolerance() {
        // Lower is better (out of boost): 30% when you usually have 20% is worse.
        assert_eq!(verdict(30.0, Some(20.0), Some(false)), Verdict::Worse);
        assert_eq!(verdict(12.0, Some(20.0), Some(false)), Verdict::Better);
        // Higher is better (average boost).
        assert_eq!(verdict(60.0, Some(45.0), Some(true)), Verdict::Better);
        assert_eq!(verdict(30.0, Some(45.0), Some(true)), Verdict::Worse);
        // Within a tenth (at least 1.5) of your usual value is the same.
        assert_eq!(verdict(46.0, Some(45.0), Some(true)), Verdict::Same);
        assert_eq!(verdict(1.0, Some(0.0), Some(true)), Verdict::Same);
        // Nothing to compare with, or a style rather than a quality.
        assert_eq!(verdict(50.0, None, Some(true)), Verdict::None);
        assert_eq!(verdict(50.0, Some(10.0), None), Verdict::None);
    }
}
