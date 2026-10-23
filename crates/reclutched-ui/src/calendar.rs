//! The calendar page: a year at a glance, twelve months, every day coloured by how long you
//! played (gray for nothing, a gradient of reds up to a flashing red at eight hours or more).

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::dates::{days_in_month, duration_text, Day, MONTHS};
use reclutched_core::rocketleague::periods::{Grain, PeriodSummary};
use reclutched_core::rocketleague::storage::Store;

use crate::widgets::label;

/// Hours played for the hottest colour.
const MAX_HOURS: i64 = 8;

/// How hot a day looks: 0 for no play, then 1 to 8 (one step per started hour; 8 hours or more
/// is the top step).
pub fn heat_level(seconds: i64) -> u8 {
    if seconds <= 0 {
        return 0;
    }
    let hours = (seconds + 3599) / 3600;
    hours.min(MAX_HOURS) as u8
}

/// How many empty cells come before the 1st (weeks start on Monday) and how many days follow.
pub fn month_shape(year: i32, month: u32) -> (u32, u32) {
    let lead = Day::new(year, month, 1).map_or(0, |d| d.weekday());
    (lead, days_in_month(year, month))
}

pub struct Hooks {
    pub on_back: Rc<dyn Fn()>,
    /// A played day was clicked ("2026-10-23").
    pub on_open: Rc<dyn Fn(&str)>,
}

struct State {
    days: HashMap<String, PeriodSummary>,
    years: Vec<i32>,
    year: Cell<i32>,
    today: Day,
    holder: gtk::Box,
    title: gtk::Label,
    prev: gtk::Button,
    next: gtk::Button,
    on_open: Rc<dyn Fn(&str)>,
}

/// Builds the page. `today` is the current local day and `utc_offset` the local offset.
pub fn build(db_path: PathBuf, utc_offset: i64, today: Day, hooks: Hooks) -> gtk::Widget {
    let days: Vec<PeriodSummary> = Store::open(&db_path)
        .and_then(|s| s.period_summaries(Grain::Day, utc_offset))
        .unwrap_or_default();
    let first_year = days
        .iter()
        .filter_map(|d| d.key.get(..4)?.parse::<i32>().ok())
        .min()
        .unwrap_or(today.year)
        .min(today.year);
    let years: Vec<i32> = (first_year..=today.year).collect();

    let title = gtk::Label::builder().css_classes(["cal-year"]).build();
    let prev = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .tooltip_text("Previous year")
        .css_classes(["flat", "circular"])
        .build();
    let next = gtk::Button::builder()
        .icon_name("go-next-symbolic")
        .tooltip_text("Next year")
        .css_classes(["flat", "circular"])
        .build();
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let state = Rc::new(State {
        days: days.into_iter().map(|d| (d.key.clone(), d)).collect(),
        years,
        year: Cell::new(today.year),
        today,
        holder,
        title,
        prev: prev.clone(),
        next: next.clone(),
        on_open: hooks.on_open,
    });
    {
        let state = state.clone();
        prev.connect_clicked(move |_| step(&state, -1));
    }
    {
        let state = state.clone();
        next.connect_clicked(move |_| step(&state, 1));
    }
    render(&state);

    let nav = gtk::Box::builder()
        .spacing(14)
        .halign(gtk::Align::Center)
        .build();
    nav.append(&prev);
    nav.append(&state.title);
    nav.append(&next);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.append(&nav);
    body.append(&state.holder);
    body.append(&legend());
    body.set_margin_top(20);
    body.set_margin_bottom(28);
    body.set_margin_start(28);
    body.set_margin_end(28);
    let clamp = adw::Clamp::builder()
        .maximum_size(1100)
        .tightening_threshold(700)
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
    header.set_title_widget(Some(&adw::WindowTitle::new("Calendar", "")));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroller));
    view.upcast()
}

fn step(state: &Rc<State>, by: i32) {
    let year = state.year.get() + by;
    if state.years.contains(&year) {
        state.year.set(year);
        render(state);
    }
}

fn render(state: &Rc<State>) {
    let year = state.year.get();
    state.title.set_label(&year.to_string());
    state.prev.set_sensitive(state.years.contains(&(year - 1)));
    state.next.set_sensitive(state.years.contains(&(year + 1)));

    while let Some(child) = state.holder.first_child() {
        state.holder.remove(&child);
    }
    let grid = gtk::Grid::builder()
        .column_spacing(26)
        .row_spacing(22)
        .column_homogeneous(true)
        .row_homogeneous(true)
        .build();
    for month in 1..=12u32 {
        let card = month_card(state, year, month);
        grid.attach(
            &card,
            ((month - 1) % 3) as i32,
            ((month - 1) / 3) as i32,
            1,
            1,
        );
    }
    state.holder.append(&grid);
}

fn month_card(state: &Rc<State>, year: i32, month: u32) -> gtk::Box {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .css_classes(["cal-month"])
        .build();
    card.append(&label(MONTHS[month as usize - 1], &["cal-month-name"]));

    let grid = gtk::Grid::builder()
        .column_spacing(4)
        .row_spacing(4)
        .column_homogeneous(true)
        .halign(gtk::Align::Fill)
        .build();
    for (i, w) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
        let l = gtk::Label::builder()
            .label(*w)
            .css_classes(["cal-weekday"])
            .build();
        grid.attach(&l, i as i32, 0, 1, 1);
    }
    let (lead, count) = month_shape(year, month);
    for day in 1..=count {
        let slot = lead + day - 1;
        let key = format!("{year:04}-{month:02}-{day:02}");
        let summary = state.days.get(&key);
        let seconds = summary.map_or(0, |s| s.seconds_played);
        let level = heat_level(seconds);
        let is_today = state.today == Day { year, month, day };
        let mut classes = vec!["cal-day".to_string(), format!("heat{level}")];
        if is_today {
            classes.push("cal-today".into());
        }
        let text = day.to_string();
        let cell: gtk::Widget = match summary {
            Some(s) => {
                let b = gtk::Button::builder()
                    .label(&text)
                    .css_classes(classes)
                    .tooltip_text(format!(
                        "{} {}: {}, {} match{}",
                        MONTHS[month as usize - 1],
                        day,
                        duration_text(s.seconds_played),
                        s.matches,
                        if s.matches == 1 { "" } else { "es" }
                    ))
                    .build();
                let on_open = state.on_open.clone();
                b.connect_clicked(move |_| on_open(&key));
                b.upcast()
            }
            None => gtk::Label::builder()
                .label(&text)
                .css_classes(classes)
                .build()
                .upcast(),
        };
        grid.attach(&cell, (slot % 7) as i32, (slot / 7) as i32 + 1, 1, 1);
    }
    card.append(&grid);
    card
}

/// "Less ▪▪▪▪▪▪▪▪ 8 h or more"
fn legend() -> gtk::Box {
    let row = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    row.append(&label("Not played", &["cal-legend"]));
    for level in 0..=MAX_HOURS as u8 {
        row.append(
            &gtk::Box::builder()
                .css_classes(["cal-swatch".to_string(), format!("heat{level}")])
                .build(),
        );
    }
    row.append(&label("8 h or more", &["cal-legend"]));
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_follows_started_hours_and_tops_out_at_eight() {
        assert_eq!(heat_level(0), 0);
        assert_eq!(heat_level(1), 1);
        assert_eq!(heat_level(3600), 1);
        assert_eq!(heat_level(3601), 2);
        assert_eq!(heat_level(7 * 3600 + 1), 8);
        assert_eq!(heat_level(8 * 3600), 8);
        assert_eq!(heat_level(20 * 3600), 8);
    }

    #[test]
    fn months_start_on_the_right_weekday() {
        // 1 Oct 2026 is a Thursday; February 2024 has 29 days and starts on a Thursday.
        assert_eq!(month_shape(2026, 10), (3, 31));
        assert_eq!(month_shape(2024, 2), (3, 29));
        // 1 Jan 2024 was a Monday.
        assert_eq!(month_shape(2024, 1), (0, 31));
    }
}
