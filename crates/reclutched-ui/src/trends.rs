//! The numbers behind the Main Menu graphs: one value per day over the last 30 days, a verdict
//! (better, worse or steady) and the colour and wording that go with it. The maths is plain code
//! so it can be tested without a window.

use gtk::prelude::*;
use reclutched_core::rocketleague::dates::{Day, MONTHS};
use reclutched_core::rocketleague::periods::PeriodSummary;

/// How a number is doing, which sets the colour of its line and its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Going up: green.
    Better,
    /// Going down: red.
    Worse,
    /// Going down, for the numbers we gently nudge instead of alarm about: orange.
    Work,
    Steady,
}

impl Tone {
    pub fn css_class(self) -> &'static str {
        match self {
            Tone::Better => "better",
            Tone::Worse => "worse",
            Tone::Work => "work",
            Tone::Steady => "steady",
        }
    }

    pub fn rgb(self) -> (f64, f64, f64) {
        match self {
            Tone::Better => (0.37, 0.64, 0.44),
            Tone::Worse => (0.85, 0.31, 0.35),
            Tone::Work => (0.93, 0.58, 0.22),
            Tone::Steady => (0.62, 0.57, 0.55),
        }
    }
}

/// One number we chart.
pub struct Metric {
    pub name: &'static str,
    pub decimals: usize,
    /// Added to a value when it is shown ("%").
    pub suffix: &'static str,
    /// Added to a change ("pts" for percentages), with its leading space.
    pub change_unit: &'static str,
    /// What going down looks like: red ([`Tone::Worse`]) or orange ([`Tone::Work`]).
    pub down: Tone,
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

/// The four graphs of the Main Menu.
pub const METRICS: [Metric; 4] = [
    Metric {
        name: "Win rate",
        decimals: 0,
        suffix: "%",
        change_unit: " pts",
        down: Tone::Worse,
        of_day: |s| {
            let n = (s.wins + s.losses) as f64;
            (n > 0.0).then(|| (s.wins as f64 / n * 100.0, n))
        },
    },
    Metric {
        name: "Assists",
        decimals: 1,
        suffix: "",
        change_unit: "",
        down: Tone::Work,
        of_day: |s| per_match(s, s.avg_assists),
    },
    Metric {
        name: "Average boost",
        decimals: 0,
        suffix: "",
        change_unit: " pts",
        down: Tone::Worse,
        of_day: |s| optional(s, s.avg_boost),
    },
    Metric {
        name: "Possession",
        decimals: 0,
        suffix: "%",
        change_unit: " pts",
        down: Tone::Worse,
        of_day: |s| optional(s, s.avg_possession),
    },
];

/// One point of a chart: the day (days since 1970) and its value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub at: i64,
    pub value: f64,
}

/// How many days the graphs look back, today included.
pub const DAYS: i64 = 30;

/// The first day the graphs show.
pub fn first_day(today: Day) -> Day {
    today.plus_days(-(DAYS - 1))
}

/// One metric per played day over the last [`DAYS`] days, oldest first.
pub fn series(days: &[PeriodSummary], metric: &Metric, today: Day) -> Vec<Point> {
    let from = first_day(today).to_days();
    let mut points: Vec<Point> = days
        .iter()
        .filter_map(|s| {
            let at = Day::parse(&s.key)?.to_days();
            if at < from || at > today.to_days() {
                return None;
            }
            let (value, _) = (metric.of_day)(s)?;
            Some(Point { at, value })
        })
        .collect();
    points.sort_by_key(|p| p.at);
    points
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Better,
    Worse,
    Steady,
}

/// How the last third of the points compares with the first third: the change in the metric's
/// own units and whether it counts as better, worse or steady. `None` with fewer than 3 points.
pub fn trend(points: &[Point]) -> Option<(f64, Verdict)> {
    if points.len() < 3 {
        return None;
    }
    let k = (points.len() / 3).max(1);
    let mean = |p: &[Point]| p.iter().map(|p| p.value).sum::<f64>() / p.len() as f64;
    let first = mean(&points[..k]);
    let last = mean(&points[points.len() - k..]);
    let change = last - first;
    // Under 3% of the level is noise.
    let level = ((first.abs() + last.abs()) / 2.0).max(1.0);
    let verdict = if change.abs() < level * 0.03 {
        Verdict::Steady
    } else if change > 0.0 {
        Verdict::Better
    } else {
        Verdict::Worse
    };
    Some((change, verdict))
}

/// The words under a graph and its tone: "+4 pts, improving", "\u{2212}4 pts, slipping" or, for the
/// numbers marked as such, "\u{2212}0.5, To work on".
pub fn describe(metric: &Metric, points: &[Point]) -> (String, Tone) {
    let Some((change, verdict)) = trend(points) else {
        return ("Not enough matches yet".to_string(), Tone::Steady);
    };
    let sign = if change >= 0.0 { "+" } else { "\u{2212}" };
    let amount = format!(
        "{sign}{:.*}{}",
        metric.decimals.max(1),
        change.abs(),
        metric.change_unit
    );
    match verdict {
        Verdict::Better => (format!("{amount}, improving"), Tone::Better),
        Verdict::Steady => (format!("{amount}, steady"), Tone::Steady),
        Verdict::Worse if metric.down == Tone::Work => {
            (format!("{amount}, To work on"), Tone::Work)
        }
        Verdict::Worse => (format!("{amount}, slipping"), Tone::Worse),
    }
}

/// A value as shown next to the graph name.
pub fn format_value(metric: &Metric, value: f64) -> String {
    format!("{value:.*}{}", metric.decimals, metric.suffix)
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
pub fn chart(
    points: Vec<Point>,
    from: i64,
    to: i64,
    decimals: usize,
    tone: Tone,
) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_height(130)
        .hexpand(true)
        .build();
    let line = tone.rgb();
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
        fill.add_color_stop_rgba(0.0, line.0, line.1, line.2, 0.28);
        fill.add_color_stop_rgba(1.0, line.0, line.1, line.2, 0.0);
        let _ = cr.set_source(&fill);
        let _ = cr.fill();

        cr.set_line_join(gtk::cairo::LineJoin::Round);
        cr.set_source_rgb(line.0, line.1, line.2);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn day(key: &str, matches: i64, wins: i64, assists: f64) -> PeriodSummary {
        PeriodSummary {
            key: key.to_string(),
            matches,
            wins,
            losses: matches - wins,
            abandoned: 0,
            seconds_played: matches * 300,
            avg_score: 300.0,
            avg_goals: 1.0,
            avg_assists: assists,
            avg_saves: 0.0,
            avg_shots: 0.0,
            avg_demos: 0.0,
            avg_possession: Some(50.0),
            avg_boost: Some(40.0),
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
    fn only_the_last_thirty_days_count_and_they_come_oldest_first() {
        let today = Day::new(2026, 10, 23).unwrap();
        let days = vec![
            day("2026-10-22", 2, 1, 1.0),
            day("2026-09-24", 2, 2, 0.5), // 29 days back: the first day shown
            day("2026-09-23", 2, 0, 9.0), // 30 days back: too old
            day("2026-10-23", 4, 3, 2.0),
        ];
        let win = series(&days, &METRICS[0], today);
        assert_eq!(
            win.iter().map(|p| p.value).collect::<Vec<_>>(),
            [100.0, 50.0, 75.0]
        );
        assert!(win.windows(2).all(|w| w[0].at < w[1].at));
        let assists = series(&days, &METRICS[1], today);
        assert_eq!(assists.len(), 3);
        assert!(assists.iter().all(|p| p.value < 3.0));
    }

    #[test]
    fn a_day_without_a_value_is_skipped() {
        let today = Day::new(2026, 10, 23).unwrap();
        let mut d = day("2026-10-23", 2, 1, 1.0);
        d.avg_boost = None;
        assert!(series(&[d], &METRICS[2], today).is_empty());
    }

    #[test]
    fn trends_say_better_worse_or_steady() {
        let rising = pts(&[40.0, 42.0, 41.0, 50.0, 55.0, 58.0]);
        assert_eq!(trend(&rising).unwrap().1, Verdict::Better);
        let falling = pts(&[58.0, 55.0, 50.0, 41.0, 42.0, 40.0]);
        assert_eq!(trend(&falling).unwrap().1, Verdict::Worse);
        let flat = pts(&[50.0, 50.5, 49.5, 50.0, 50.2, 49.9]);
        assert_eq!(trend(&flat).unwrap().1, Verdict::Steady);
        assert!(trend(&pts(&[1.0, 2.0])).is_none());
    }

    #[test]
    fn colours_and_words_follow_the_rules() {
        let falling = pts(&[60.0, 58.0, 55.0, 41.0, 40.0, 39.0]);
        // Win rate going down: red and "slipping".
        let (text, tone) = describe(&METRICS[0], &falling);
        assert_eq!(tone, Tone::Worse);
        assert!(
            text.ends_with("pts, slipping") && text.starts_with('\u{2212}'),
            "{text}"
        );
        // Assists going down: orange and "To work on", no unit.
        let down = pts(&[1.5, 1.4, 1.2, 0.8, 0.7, 0.6]);
        let (text, tone) = describe(&METRICS[1], &down);
        assert_eq!(tone, Tone::Work);
        assert_eq!(text, "\u{2212}0.8, To work on");
        // Going up is green for everything.
        let up = pts(&[0.6, 0.7, 0.8, 1.2, 1.4, 1.5]);
        assert_eq!(describe(&METRICS[1], &up).1, Tone::Better);
        assert_eq!(
            describe(&METRICS[3], &pts(&[40.0, 41.0, 45.0, 50.0, 52.0, 53.0])).1,
            Tone::Better
        );
        // Not enough days yet.
        assert_eq!(describe(&METRICS[0], &pts(&[50.0])).1, Tone::Steady);
    }

    #[test]
    fn values_are_formatted_with_their_unit() {
        assert_eq!(format_value(&METRICS[0], 58.4), "58%");
        assert_eq!(format_value(&METRICS[1], 1.25), "1.2");
        assert_eq!(format_value(&METRICS[2], 41.6), "42");
    }
}
