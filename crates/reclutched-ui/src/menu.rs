//! The Main Menu tab: a welcome, a few numbers that rotate, your latest screenshots and clips
//! and four small graphs of how the last 30 days went.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::dates::{duration_text, Day};
use reclutched_core::rocketleague::periods::Grain;
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::settings::Settings;
use reclutched_core::rocketleague::storage::Store;

use crate::carousel::carousel;
use crate::format::{mode_name, pct};
use crate::gallery;
use crate::history::day_title;
use crate::insights::{busiest, last_days, merge, Merged};
use crate::mainview::local_now;
use crate::trends;
use crate::widgets::{label, section, tile, vbox};

/// Seconds each card of the rotating section stays.
const CARD_SECONDS: u32 = 10;
const DASH: &str = "\u{2013}";

/// What the page was built from; when it changes, the page is built again.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Fingerprint {
    matches: (i64, i64),
    media: (usize, u64),
    day: String,
    username: String,
}

pub struct MainMenu {
    pub widget: gtk::ScrolledWindow,
    body: gtk::Box,
    store: Rc<Store>,
    settings_path: PathBuf,
    names_path: Option<PathBuf>,
    on_calendar: Rc<dyn Fn()>,
    seen: RefCell<Fingerprint>,
    built: Cell<bool>,
}

impl MainMenu {
    pub fn new(
        store: Rc<Store>,
        settings_path: PathBuf,
        names_path: Option<PathBuf>,
        on_calendar: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let body = vbox(0);
        body.set_margin_top(30);
        body.set_margin_bottom(36);
        body.set_margin_start(36);
        body.set_margin_end(36);
        let clamp = adw::Clamp::builder()
            .maximum_size(980)
            .tightening_threshold(700)
            .child(&body)
            .build();
        let widget = gtk::ScrolledWindow::builder()
            .child(&clamp)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let this = Rc::new(Self {
            widget,
            body,
            store,
            settings_path,
            names_path,
            on_calendar,
            seen: RefCell::new(Fingerprint::default()),
            built: Cell::new(false),
        });
        this.refresh();
        // Look for new matches and pictures every few seconds. The timer stops by itself once
        // the page is gone.
        let weak = Rc::downgrade(&this);
        gtk::glib::timeout_add_seconds_local(3, move || match weak.upgrade() {
            Some(menu) => {
                menu.refresh_if_changed();
                gtk::glib::ControlFlow::Continue
            }
            None => gtk::glib::ControlFlow::Break,
        });
        this
    }

    fn fingerprint(&self, settings: &Settings) -> Fingerprint {
        let count = self.store.abandon_counts().map_or(0, |(all, _)| all);
        let newest = self.store.latest_match_id().ok().flatten().unwrap_or(0);
        let media = settings
            .capture_folder()
            .map(|f| gallery::recent_media(&f))
            .unwrap_or_default();
        let newest_media = media
            .iter()
            .filter_map(|m| m.modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .max()
            .unwrap_or(0);
        Fingerprint {
            matches: (count, newest),
            media: (media.len(), newest_media),
            day: local_now().1.key(),
            username: settings.username.clone().unwrap_or_default(),
        }
    }

    pub fn refresh_if_changed(&self) {
        let settings = Settings::load(&self.settings_path).unwrap_or_default();
        if !self.built.get() || self.fingerprint(&settings) != *self.seen.borrow() {
            self.refresh();
        }
    }

    /// Builds the page again from the database and the capture folder.
    pub fn refresh(&self) {
        let settings = Settings::load(&self.settings_path).unwrap_or_default();
        *self.seen.borrow_mut() = self.fingerprint(&settings);
        self.built.set(true);
        let body = &self.body;
        while let Some(child) = body.first_child() {
            body.remove(&child);
        }

        // --- welcome, and the calendar ---
        let welcome = match settings.username.as_deref().filter(|n| !n.is_empty()) {
            Some(name) => format!("Welcome back, {name}!"),
            None => "Welcome back!".to_string(),
        };
        let top = gtk::Box::builder().spacing(12).build();
        let title = label(&welcome, &["period-title"]);
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        top.append(&title);
        top.append(&self.calendar_button());
        body.append(&top);

        let (wins, losses, abandoned) = self.store.result_counts().unwrap_or((0, 0, 0));
        let media = settings
            .capture_folder()
            .map(|f| gallery::recent_media(&f))
            .unwrap_or_default();
        if wins + losses + abandoned == 0 {
            let empty = label(
                "Very empty here, go play a game and come back.",
                &["welcome-empty"],
            );
            empty.set_margin_top(8);
            body.append(&empty);
            if !media.is_empty() {
                body.append(&section("Recent clips"));
                let show = gallery::slideshow(&media);
                show.set_margin_top(10);
                body.append(&show);
            }
            return;
        }

        let (offset, today) = local_now();
        let days = self
            .store
            .period_summaries(Grain::Day, offset)
            .unwrap_or_default();
        let recent = last_days(&days, today, trends::DAYS);

        // --- the rotating numbers ---
        let names = self
            .names_path
            .as_deref()
            .and_then(|p| PlaylistMap::load(p).ok())
            .unwrap_or_else(PlaylistMap::with_builtin);
        let since = today.plus_days(-(trends::DAYS - 1)).to_days() * 86_400 - offset;
        let favourite = self
            .store
            .favourite_playlist_since(since)
            .ok()
            .flatten()
            .map(|(id, n)| (mode_name(&names, id), n));
        let streak = self
            .store
            .recent_form(50)
            .ok()
            .flatten()
            .and_then(|f| f.streak);
        let pages = cards(
            merge(&recent).as_ref(),
            streak,
            favourite,
            busiest(&recent).map(|d| (d.key.clone(), d.seconds_played)),
        );
        let rotating = carousel(&pages, CARD_SECONDS);
        rotating.set_margin_top(22);
        body.append(&rotating);

        // --- recent clips ---
        body.append(&section("Recent clips"));
        let show = gallery::slideshow(&media);
        show.set_margin_top(10);
        body.append(&show);

        // --- the four graphs ---
        body.append(&section("Last 30 days"));
        let grid = gtk::Grid::builder()
            .column_spacing(16)
            .row_spacing(16)
            .column_homogeneous(true)
            .margin_top(10)
            .build();
        for (i, metric) in trends::METRICS.iter().enumerate() {
            let points = trends::series(&days, metric, today);
            grid.attach(
                &graph_card(metric, &points, today),
                (i % 2) as i32,
                (i / 2) as i32,
                1,
                1,
            );
        }
        body.append(&grid);
    }

    fn calendar_button(&self) -> gtk::Button {
        let row = gtk::Box::builder().spacing(8).build();
        row.append(&gtk::Image::from_icon_name("x-office-calendar-symbolic"));
        row.append(&gtk::Label::new(Some("Calendar")));
        let button = gtk::Button::builder()
            .child(&row)
            .valign(gtk::Align::Center)
            .tooltip_text("Which days you played")
            .build();
        let on_calendar = self.on_calendar.clone();
        button.connect_clicked(move |_| on_calendar());
        button
    }
}

/// One graph with its name, the latest value and how it is going.
fn graph_card(metric: &trends::Metric, points: &[trends::Point], today: Day) -> gtk::Box {
    let (text, tone) = trends::describe(metric, points);
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["graph-card"])
        .build();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    head.append(&label(&metric.name.to_uppercase(), &["graph-name"]));
    let value = label(
        &points
            .last()
            .map_or(DASH.to_string(), |p| trends::format_value(metric, p.value)),
        &["graph-value"],
    );
    value.set_hexpand(true);
    value.set_halign(gtk::Align::End);
    value.set_xalign(1.0);
    head.append(&value);
    card.append(&head);
    card.append(&label(&text, &["graph-trend", tone.css_class()]));
    card.append(&trends::chart(
        points.to_vec(),
        trends::first_day(today).to_days(),
        today.to_days(),
        metric.decimals,
        tone,
    ));
    card
}

fn number(value: f64, decimals: usize) -> String {
    format!("{value:.decimals$}")
}

fn optional(value: Option<f64>, show: impl Fn(f64) -> String) -> String {
    value.map_or(DASH.to_string(), show)
}

/// One card of the rotating section: a title and three numbers.
fn card(title: &str, tiles: [(&str, String, &str); 3]) -> gtk::Widget {
    let page = vbox(14);
    page.add_css_class("info-card");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let name = label(title, &["card-title"]);
    name.set_hexpand(true);
    head.append(&name);
    head.append(&label("Last 30 days", &["card-sub"]));
    page.append(&head);
    let row = gtk::Box::builder().spacing(14).homogeneous(true).build();
    for (key, value, hint) in &tiles {
        row.append(&tile(key, value, hint));
    }
    page.append(&row);
    page.upcast()
}

/// The five rotating cards (a single "nothing yet" card when the last 30 days are empty).
fn cards(
    merged: Option<&Merged>,
    streak: Option<(String, usize)>,
    favourite: Option<(String, i64)>,
    busiest_day: Option<(String, i64)>,
) -> Vec<gtk::Widget> {
    let Some(m) = merged else {
        let page = vbox(8);
        page.add_css_class("info-card");
        page.append(&label("Nothing in the last 30 days", &["card-title"]));
        page.append(&label(
            "Play a match and your numbers show up here.",
            &["muted-note"],
        ));
        return vec![page.upcast()];
    };
    let streak_tile = match streak {
        Some((result, n)) => (
            n.to_string(),
            format!("{}{} in a row", result, if n == 1 { "" } else { "s" }),
        ),
        None => (DASH.to_string(), "no streak right now".to_string()),
    };
    let busiest_tile = busiest_day.map_or((DASH.to_string(), String::new()), |(key, secs)| {
        (
            Day::parse(&key).map_or(key.clone(), day_title),
            duration_text(secs),
        )
    });
    let favourite_tile = favourite.map_or((DASH.to_string(), String::new()), |(name, n)| {
        (name, format!("{n} matches"))
    });
    vec![
        card(
            "How you move",
            [
                (
                    "Average boost",
                    optional(m.boost, |v| number(v, 0)),
                    "out of 100",
                ),
                ("Out of boost", optional(m.boost_empty, pct), "of the time"),
                (
                    "Average speed",
                    optional(m.speed, |v| format!("{v:.0} km/h")),
                    "while moving",
                ),
            ],
        ),
        card(
            "How you score",
            [
                ("Score", number(m.score, 0), "per match"),
                ("Goals", number(m.goals, 1), "per match"),
                ("Shots", number(m.shots, 1), "per match"),
            ],
        ),
        card(
            "How you defend",
            [
                ("Saves", number(m.saves, 1), "per match"),
                ("Assists", number(m.assists, 1), "per match"),
                ("Demos", number(m.demos, 1), "per match"),
            ],
        ),
        card(
            "Your record",
            [
                ("Wins", m.wins.to_string(), "won"),
                ("Losses", m.losses.to_string(), "lost"),
                ("Streak", streak_tile.0, &streak_tile.1),
            ],
        ),
        card(
            "Highlights",
            [
                ("Favourite mode", favourite_tile.0, &favourite_tile.1),
                ("Busiest day", busiest_tile.0, &busiest_tile.1),
                ("Time played", duration_text(m.seconds_played), "in matches"),
            ],
        ),
    ]
}
