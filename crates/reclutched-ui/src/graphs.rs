//! The graphs page: how your numbers change over time, one small chart per number, with a
//! plain "better / worse / steady" note. The maths (bucketing and trends) is plain code so it
//! can be tested without a window.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::dates::{Day, MONTHS};
use reclutched_core::rocketleague::periods::{Grain, PeriodSummary};
use reclutched_core::rocketleague::storage::Store;

use crate::widgets::label;

/// How far back the charts look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Days30,
    Days90,
    ThisYear,
    All,
}

pub const RANGES: [(Range, &str); 4] = [
    (Range::Days30, "30 days"),
    (Range::Days90, "90 days"),
    (Range::ThisYear, "This year"),
    (Range::All, "All time"),
];

/// How points are grouped: one per day, per week or per month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Day,
    Week,
    Month,
}

impl Range {
    pub fn step(self) -> Step {
        match self {
            Range::Days30 => Step::Day,
            Range::Days90 | Range::ThisYear => Step::Week,
            Range::All => Step::Month,
        }
    }

    /// First day shown; `None` means "from your first match".
    pub fn start(self, today: Day) -> Option<Day> {
        match self {
            Range::Days30 => Some(today.plus_days(-29)),
            Range::Days90 => Some(today.plus_days(-89)),
            Range::ThisYear => Day::new(today.year, 1, 1),
            Range::All => None,
        }
    }
}

/// One number we chart.
pub struct Metric {
    pub name: &'static str,
    /// Higher is better (false for "out of boost").
    pub up_is_good: bool,
    /// Shown as a percentage (changes are in points) or as a plain number.
    pub percent: bool,
    pub decimals: usize,
    pub suffix: &'static str,
    /// The day's value and how many matches it is based on.
    pub of_day: fn(&PeriodSummary) -> Option<(f64, f64)>,
}

fn finished(s: &PeriodSummary) -> f64 {
    (s.matches - s.abandoned) as f64
}

fn per_match(s: &PeriodSummary, v: f64) -> Option<(f64, f64)> {
    (finished(s) > 0.0).then_some((v, finished(s)))
}

fn optional(s: &PeriodSummary, v: Option<f64>) -> Option<(f64, f64)> {
    v.and_then(|v| per_match(s, v))
}

pub const METRICS: [Metric; 10] = [
    Metric {
        name: "Win rate",
        up_is_good: true,
        percent: true,
        decimals: 0,
        suffix: "%",
        of_day: |s| {
            let n = (s.wins + s.losses) as f64;
            (n > 0.0).then(|| (s.wins as f64 / n * 100.0, n))
        },
    },
    Metric {
        name: "Score",
        up_is_good: true,
        percent: false,
        decimals: 0,
        suffix: "",
        of_day: |s| per_match(s, s.avg_score),
    },
    Metric {
        name: "Goals",
        up_is_good: true,
        percent: false,
        decimals: 1,
        suffix: "",
        of_day: |s| per_match(s, s.avg_goals),
    },
    Metric {
        name: "Assists",
        up_is_good: true,
        percent: false,
        decimals: 1,
        suffix: "",
        of_day: |s| per_match(s, s.avg_assists),
    },
    Metric {
        name: "Saves",
        up_is_good: true,
        percent: false,
        decimals: 1,
        suffix: "",
        of_day: |s| per_match(s, s.avg_saves),
    },
    Metric {
        name: "Shots",
        up_is_good: true,
        percent: false,
        decimals: 1,
        suffix: "",
        of_day: |s| per_match(s, s.avg_shots),
    },
    Metric {
        name: "Possession",
        up_is_good: true,
        percent: true,
        decimals: 0,
        suffix: "%",
        of_day: |s| optional(s, s.avg_possession),
    },
    Metric {
        name: "Average boost",
        up_is_good: true,
        percent: false,
        decimals: 0,
        suffix: "",
        of_day: |s| optional(s, s.avg_boost),
    },
    Metric {
        name: "Out of boost",
        up_is_good: false,
        percent: true,
        decimals: 0,
        suffix: "%",
        of_day: |s| optional(s, s.avg_boost_empty),
    },
    Metric {
        name: "Average speed",
        up_is_good: true,
        percent: false,
        decimals: 0,
        suffix: " km/h",
        of_day: |s| optional(s, s.avg_speed),
    },
];

/// One point of a chart: where it starts (days since 1970) and its value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub at: i64,
    pub value: f64,
}

/// First day of the group a day belongs to.
fn bucket_start(day: Day, step: Step) -> i64 {
    match step {
        Step::Day => day.to_days(),
        Step::Week => day.to_days() - day.weekday() as i64,
        Step::Month => Day { day: 1, ..day }.to_days(),
    }
}

/// The points of one metric: days inside the range, grouped by `range.step()` and averaged,
/// each day weighted by the matches behind it. Oldest first.
pub fn series(days: &[PeriodSummary], metric: &Metric, range: Range, today: Day) -> Vec<Point> {
    let from = range.start(today).map(|d| d.to_days());
    let mut sums: std::collections::BTreeMap<i64, (f64, f64)> = Default::default();
    for s in days {
        let Some(day) = Day::parse(&s.key) else {
            continue;
        };
        if from.is_some_and(|f| day.to_days() < f) || day.to_days() > today.to_days() {
            continue;
        }
        let Some((value, weight)) = (metric.of_day)(s) else {
            continue;
        };
        let entry = sums
            .entry(bucket_start(day, range.step()))
            .or_insert((0.0, 0.0));
        entry.0 += value * weight;
        entry.1 += weight;
    }
    sums.into_iter()
        .filter(|(_, (_, w))| *w > 0.0)
        .map(|(at, (sum, w))| Point { at, value: sum / w })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Better,
    Worse,
    Steady,
}

/// How the last third of the points compares with the first third: the change in the metric's
/// own units and whether it counts as better, worse or steady. `None` with fewer than 3 points.
pub fn trend(points: &[Point], up_is_good: bool) -> Option<(f64, Verdict)> {
    if points.len() < 3 {
        return None;
    }
    let k = (points.len() / 3).max(1);
    let mean = |p: &[Point]| p.iter().map(|p| p.value).sum::<f64>() / p.len() as f64;
    let first = mean(&points[..k]);
    let last = mean(&points[points.len() - k..]);
    let change = last - first;
    // Under 3% of the level (or a tiny absolute change) is noise.
    let level = ((first.abs() + last.abs()) / 2.0).max(1.0);
    let verdict = if change.abs() < level * 0.03 {
        Verdict::Steady
    } else if (change > 0.0) == up_is_good {
        Verdict::Better
    } else {
        Verdict::Worse
    };
    Some((change, verdict))
}

fn format_value(m: &Metric, v: f64) -> String {
    format!("{v:.*}{}", m.decimals, m.suffix)
}

fn trend_text(m: &Metric, points: &[Point]) -> (String, &'static str) {
    match trend(points, m.up_is_good) {
        None => ("not enough data yet".to_string(), "steady"),
        Some((change, verdict)) => {
            let arrow = if change >= 0.0 { "+" } else { "\u{2212}" };
            let unit = if m.percent { " pts" } else { m.suffix };
            let amount = format!("{arrow}{:.*}{unit}", m.decimals.max(1), change.abs());
            match verdict {
                Verdict::Better => (format!("{amount}, improving"), "better"),
                Verdict::Worse => (format!("{amount}, slipping"), "worse"),
                Verdict::Steady => (format!("{amount}, steady"), "steady"),
            }
        }
    }
}

fn day_text(days_since_epoch: i64) -> String {
    let d = Day::from_days(days_since_epoch);
    format!(
        "{} {} {}",
        d.day,
        &MONTHS[d.month as usize - 1][..3],
        d.year
    )
}

/// The line chart itself.
fn chart(points: Vec<Point>, from: i64, to: i64, decimals: usize) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_height(130)
        .hexpand(true)
        .build();
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let (left, right, top, bottom) = (6.0, 6.0, 8.0, 18.0);
        let (pw, ph) = (w - left - right, h - top - bottom);
        // Faint guide lines.
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.06);
        cr.set_line_width(1.0);
        for i in 0..=3 {
            let y = top + ph * i as f64 / 3.0;
            cr.move_to(left, y.round() + 0.5);
            cr.line_to(w - right, y.round() + 0.5);
            let _ = cr.stroke();
        }
        if points.len() < 2 {
            cr.set_source_rgba(0.55, 0.51, 0.49, 1.0);
            cr.move_to(left + 8.0, top + ph / 2.0);
            let _ = cr.show_text("Not enough matches in this time yet");
            return;
        }
        let lo = points.iter().map(|p| p.value).fold(f64::MAX, f64::min);
        let hi = points.iter().map(|p| p.value).fold(f64::MIN, f64::max);
        let pad = ((hi - lo) * 0.15).max(if hi.abs() > 10.0 { 1.0 } else { 0.1 });
        let (lo, hi) = (lo - pad, hi + pad);
        let span = (to - from).max(1) as f64;
        let x = |at: i64| left + pw * (at - from) as f64 / span;
        let y = |v: f64| top + ph * (1.0 - (v - lo) / (hi - lo));

        // Area under the line, then the line.
        cr.move_to(x(points[0].at), top + ph);
        for p in &points {
            cr.line_to(x(p.at), y(p.value));
        }
        cr.line_to(x(points[points.len() - 1].at), top + ph);
        cr.close_path();
        let fill = gtk::cairo::LinearGradient::new(0.0, top, 0.0, top + ph);
        fill.add_color_stop_rgba(0.0, 0.85, 0.2, 0.25, 0.30);
        fill.add_color_stop_rgba(1.0, 0.85, 0.2, 0.25, 0.0);
        let _ = cr.set_source(&fill);
        let _ = cr.fill();

        cr.set_line_join(gtk::cairo::LineJoin::Round);
        cr.set_source_rgb(0.85, 0.31, 0.35);
        cr.set_line_width(2.0);
        for (i, p) in points.iter().enumerate() {
            if i == 0 {
                cr.move_to(x(p.at), y(p.value));
            } else {
                cr.line_to(x(p.at), y(p.value));
            }
        }
        let _ = cr.stroke();
        if points.len() <= 45 {
            for p in &points {
                cr.arc(x(p.at), y(p.value), 2.6, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        }
        // Range labels and the extremes.
        cr.set_source_rgba(0.55, 0.51, 0.49, 1.0);
        cr.set_font_size(10.0);
        cr.move_to(left, h - 4.0);
        let _ = cr.show_text(&day_text(from));
        let end = day_text(to);
        if let Ok(ext) = cr.text_extents(&end) {
            cr.move_to(w - right - ext.width(), h - 4.0);
            let _ = cr.show_text(&end);
        }
        let top_label = format!("{:.*}", decimals, hi - pad);
        cr.move_to(left + 2.0, top + 10.0);
        let _ = cr.show_text(&top_label);
    });
    area
}

struct State {
    days: Vec<PeriodSummary>,
    today: Day,
    range: Cell<Range>,
    holder: gtk::Box,
    chips: RefCell<Vec<(Range, gtk::ToggleButton)>>,
}

fn render(state: &Rc<State>) {
    while let Some(child) = state.holder.first_child() {
        state.holder.remove(&child);
    }
    let range = state.range.get();
    if state.days.is_empty() {
        state.holder.append(&label(
            "Nothing to chart yet. Play a few matches and come back.",
            &["muted-note"],
        ));
        return;
    }
    let grid = gtk::Grid::builder()
        .column_spacing(18)
        .row_spacing(18)
        .column_homogeneous(true)
        .build();
    for (i, m) in METRICS.iter().enumerate() {
        let points = series(&state.days, m, range, state.today);
        let first_day = points.first().map_or(state.today.to_days(), |p| p.at);
        let from = range.start(state.today).map_or(first_day, |d| d.to_days());
        let to = state.today.to_days();

        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["graph-card"])
            .build();
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        head.append(&label(&m.name.to_uppercase(), &["graph-name"]));
        let now = points
            .last()
            .map_or("\u{2013}".to_string(), |p| format_value(m, p.value));
        let value = label(&now, &["graph-value"]);
        value.set_hexpand(true);
        value.set_halign(gtk::Align::End);
        value.set_xalign(1.0);
        head.append(&value);
        card.append(&head);
        let (text, tone) = trend_text(m, &points);
        card.append(&label(&text, &["graph-trend", tone]));
        card.append(&chart(points, from, to, m.decimals));
        grid.attach(&card, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    state.holder.append(&grid);
}

pub struct Hooks {
    pub on_back: Rc<dyn Fn()>,
}

pub fn build(db_path: PathBuf, utc_offset: i64, today: Day, hooks: Hooks) -> gtk::Widget {
    let days: Vec<PeriodSummary> = Store::open(&db_path)
        .and_then(|s| s.period_summaries(Grain::Day, utc_offset))
        .unwrap_or_default();
    let state = Rc::new(State {
        days,
        today,
        range: Cell::new(Range::Days90),
        holder: gtk::Box::new(gtk::Orientation::Vertical, 0),
        chips: RefCell::new(Vec::new()),
    });

    let chips = gtk::Box::builder()
        .css_classes(["linked"])
        .halign(gtk::Align::Center)
        .build();
    let mut first: Option<gtk::ToggleButton> = None;
    for (range, name) in RANGES {
        let chip = gtk::ToggleButton::builder().label(name).build();
        match &first {
            Some(group) => chip.set_group(Some(group)),
            None => first = Some(chip.clone()),
        }
        chip.set_active(range == state.range.get());
        let state2 = state.clone();
        chip.connect_toggled(move |c| {
            if c.is_active() {
                state2.range.set(range);
                render(&state2);
            }
        });
        chips.append(&chip);
        state.chips.borrow_mut().push((range, chip));
    }
    render(&state);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.append(&chips);
    body.append(&state.holder);
    body.set_margin_top(20);
    body.set_margin_bottom(28);
    body.set_margin_start(28);
    body.set_margin_end(28);
    let clamp = adw::Clamp::builder()
        .maximum_size(1000)
        .tightening_threshold(640)
        .child(&body)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&clamp)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_width(0)
        .vexpand(true)
        .build();

    let header = adw::HeaderBar::new();
    let back = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text("Back to your matches")
        .build();
    {
        let on_back = hooks.on_back;
        back.connect_clicked(move |_| on_back());
    }
    header.pack_start(&back);
    header.set_title_widget(Some(&adw::WindowTitle::new(
        "Graphs",
        "How you are doing over time",
    )));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroller));
    view.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(key: &str, matches: i64, wins: i64, goals: f64) -> PeriodSummary {
        PeriodSummary {
            key: key.to_string(),
            matches,
            wins,
            losses: matches - wins,
            abandoned: 0,
            seconds_played: matches * 300,
            avg_score: 300.0,
            avg_goals: goals,
            avg_assists: 0.0,
            avg_saves: 0.0,
            avg_shots: 0.0,
            avg_possession: None,
            avg_boost: None,
            avg_boost_empty: None,
            avg_speed: None,
        }
    }

    fn pts(values: &[f64]) -> Vec<Point> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| Point {
                at: i as i64,
                value: *v,
            })
            .collect()
    }

    #[test]
    fn days_are_weighted_by_their_matches_and_grouped_by_week() {
        let today = Day::new(2026, 10, 23).unwrap();
        // Mon 19 and Tue 20 Oct are one week; 4 + 1 matches, goals 2.0 and 0.0.
        let days = vec![
            day("2026-10-19", 4, 2, 2.0),
            day("2026-10-20", 1, 1, 0.0),
            day("2026-10-12", 2, 1, 1.0),
        ];
        let goals = &METRICS[2];
        let weekly = series(&days, goals, Range::ThisYear, today);
        assert_eq!(weekly.len(), 2);
        assert!((weekly[1].value - 1.6).abs() < 1e-9);
        assert_eq!(Day::from_days(weekly[1].at).day, 19);
        let daily = series(&days, goals, Range::Days30, today);
        assert_eq!(daily.len(), 3);
        assert_eq!(series(&days, goals, Range::Days90, today).len(), 2);
    }

    #[test]
    fn the_range_cuts_off_old_days() {
        let today = Day::new(2026, 10, 23).unwrap();
        let days = vec![day("2026-01-05", 3, 1, 1.0), day("2026-10-22", 3, 2, 1.0)];
        let win = &METRICS[0];
        assert_eq!(series(&days, win, Range::Days30, today).len(), 1);
        assert_eq!(series(&days, win, Range::ThisYear, today).len(), 2);
        let monthly = series(&days, win, Range::All, today);
        assert_eq!(monthly.len(), 2);
        assert_eq!(Day::from_days(monthly[0].at).month, 1);
    }

    #[test]
    fn trends_say_better_worse_or_steady() {
        let rising = pts(&[40.0, 42.0, 41.0, 50.0, 55.0, 58.0]);
        assert_eq!(trend(&rising, true).unwrap().1, Verdict::Better);
        assert_eq!(trend(&rising, false).unwrap().1, Verdict::Worse);
        let flat = pts(&[50.0, 50.5, 49.5, 50.0, 50.2, 49.9]);
        assert_eq!(trend(&flat, true).unwrap().1, Verdict::Steady);
        assert!(trend(&pts(&[1.0, 2.0]), true).is_none());
    }
}
