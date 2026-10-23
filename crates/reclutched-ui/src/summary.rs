//! The page for a day, a month, a year or Home (today): the numbers in a calm layout and, below,
//! what is inside it (a day's matches, a month's days, a year's months) to click through.

use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::dates::duration_text;
use reclutched_core::rocketleague::periods::PeriodSummary;

use crate::format::pct;
use crate::history::{Entry, Target, Tone};
use crate::list::entry_row;
use crate::widgets::{clear, empty_note, label, meter, scroll_page, section, tile, vbox};

/// Everything one page shows.
pub struct Page {
    pub heading: String,
    pub subtitle: String,
    /// `None` when nothing was played then.
    pub summary: Option<PeriodSummary>,
    pub empty_text: String,
    /// "Matches", "Days" or "Months".
    pub children_title: String,
    /// Newest first.
    pub children: Vec<Entry>,
}

pub struct SummaryView {
    pub widget: gtk::ScrolledWindow,
    body: gtk::Box,
    on_open: Rc<dyn Fn(Target)>,
}

fn tone_color(tone: Tone) -> (f64, f64, f64) {
    match tone {
        Tone::Win => (0.37, 0.64, 0.44),
        Tone::Loss => (0.85, 0.33, 0.31),
        Tone::Abandoned => (0.79, 0.60, 0.23),
        Tone::Neutral => (0.29, 0.25, 0.25),
    }
}

/// A row of small squares, oldest first, coloured by how each match or day went.
fn strip(tones: Vec<Tone>) -> gtk::DrawingArea {
    let (size, gap) = (18, 5);
    let area = gtk::DrawingArea::builder()
        .content_width((tones.len() as i32 * (size + gap) - gap).max(1))
        .content_height(size)
        .halign(gtk::Align::Start)
        .build();
    area.set_draw_func(move |_, cr, _, _| {
        for (i, tone) in tones.iter().enumerate() {
            let (r, g, b) = tone_color(*tone);
            let (x, side, radius) = (f64::from(i as i32 * (size + gap)), f64::from(size), 4.5);
            use std::f64::consts::{FRAC_PI_2, PI};
            cr.new_sub_path();
            cr.arc(x + side - radius, radius, radius, -FRAC_PI_2, 0.0);
            cr.arc(x + side - radius, side - radius, radius, 0.0, FRAC_PI_2);
            cr.arc(x + radius, side - radius, radius, FRAC_PI_2, PI);
            cr.arc(x + radius, radius, radius, PI, 3.0 * FRAC_PI_2);
            cr.close_path();
            cr.set_source_rgb(r, g, b);
            let _ = cr.fill();
        }
    });
    area
}

impl SummaryView {
    pub fn new(on_open: impl Fn(Target) + 'static) -> Self {
        let body = vbox(0);
        let widget = scroll_page(&body);
        Self {
            widget,
            body,
            on_open: Rc::new(on_open),
        }
    }

    pub fn set(&self, page: &Page) {
        let b = &self.body;
        clear(b);
        b.append(&label(&page.heading, &["period-title"]));
        if !page.subtitle.is_empty() {
            b.append(&label(&page.subtitle, &["period-sub"]));
        }
        let Some(s) = &page.summary else {
            b.append(&empty_note(&page.empty_text));
            return;
        };

        // How each match or day went, oldest first.
        if page.children.len() > 1 {
            let tones: Vec<Tone> = page.children.iter().rev().map(|e| e.tone).collect();
            let strip = strip(tones);
            strip.set_margin_top(16);
            b.append(&strip);
        }

        // Four quiet tiles.
        let tiles = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(2)
            .max_children_per_line(4)
            .column_spacing(12)
            .row_spacing(12)
            .homogeneous(true)
            .margin_top(22)
            .build();
        let mut record = format!("{} won, {} lost", s.wins, s.losses);
        if s.abandoned > 0 {
            record.push_str(&format!(", {} left early", s.abandoned));
        }
        tiles.append(&tile("Matches", &s.matches.to_string(), &record));
        tiles.append(&tile(
            "Time played",
            &duration_text(s.seconds_played),
            "in matches",
        ));
        tiles.append(&tile(
            "Win rate",
            &s.win_rate().map_or("\u{2013}".to_string(), pct),
            "of finished matches",
        ));
        tiles.append(&tile(
            "Possession",
            &s.avg_possession.map_or("\u{2013}".to_string(), pct),
            "time with the ball",
        ));
        b.append(&tiles);

        // Per match, as bars.
        if s.wins + s.losses > 0 {
            b.append(&section("Per match"));
            let card = vbox(16);
            card.add_css_class("player-card");
            card.set_margin_top(8);
            for (name, value, full) in [
                ("Score", s.avg_score, 600.0),
                ("Goals", s.avg_goals, 3.0),
                ("Assists", s.avg_assists, 2.0),
                ("Saves", s.avg_saves, 3.0),
                ("Shots", s.avg_shots, 6.0),
            ] {
                let shown = if name == "Score" {
                    format!("{value:.0}")
                } else {
                    format!("{value:.1}")
                };
                card.append(&meter(name, &shown, value / full));
            }
            b.append(&card);
        }

        // Boost and speed, when we have them.
        if s.avg_boost.is_some() || s.avg_speed.is_some() {
            b.append(&section("How you move"));
            let card = vbox(16);
            card.add_css_class("player-card");
            card.set_margin_top(8);
            if let Some(v) = s.avg_boost {
                card.append(&meter("Average boost", &format!("{v:.0}"), v / 100.0));
            }
            if let Some(v) = s.avg_boost_empty {
                card.append(&meter("Out of boost", &pct(v), v / 50.0));
            }
            if let Some(v) = s.avg_speed {
                card.append(&meter("Average speed", &format!("{v:.0} km/h"), v / 100.0));
            }
            b.append(&card);
        }

        // What is inside, to click through.
        if !page.children.is_empty() {
            b.append(&section(&page.children_title));
            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .css_classes(["children-list"])
                .margin_top(8)
                .build();
            for entry in &page.children {
                list.append(&entry_row(entry));
            }
            let (entries, on_open) = (page.children.clone(), self.on_open.clone());
            list.connect_row_activated(move |_, row| {
                let target = usize::try_from(row.index())
                    .ok()
                    .and_then(|i| entries.get(i))
                    .map(|e| e.target.clone());
                if let Some(target) = target {
                    on_open(target);
                }
            });
            b.append(&list);
        }
    }
}
