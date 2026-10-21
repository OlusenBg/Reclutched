//! The live window: score, clock, possession, your boost and a scoreboard while a match is on.
//! It is meant for a second screen. It only shows what the tracker already knows; nothing here
//! is stored.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::playlist::{default_map_path, PlaylistMap};
use reclutched_core::rocketleague::tracker::{LiveEvent, LiveKind, LivePlayer, LiveSnapshot};

use crate::format::{clock_text, mode_name, pct};
use crate::widgets::{bar, clear, empty_note, label, name_label, section, split_bar, vbox};

/// Team colours for the bars (the game's blue and orange, a little softened).
const BLUE: (f64, f64, f64) = (0.31, 0.56, 0.88);
const ORANGE: (f64, f64, f64) = (0.91, 0.54, 0.23);

fn team_class(team: i64) -> &'static str {
    if team == 0 {
        "team-blue"
    } else {
        "team-orange"
    }
}

fn team_name(snapshot: &LiveSnapshot, team: i64) -> String {
    let given = usize::try_from(team)
        .ok()
        .and_then(|i| snapshot.team_names.get(i))
        .filter(|n| !n.is_empty());
    match (given, team) {
        (Some(n), _) => n.clone(),
        (None, 0) => "Blue".to_string(),
        (None, _) => "Orange".to_string(),
    }
}

pub struct LiveWindow {
    window: adw::ApplicationWindow,
    names: PlaylistMap,
    body: gtk::Box,
    /// What is on screen now, so an unchanged update does not rebuild anything.
    shown: RefCell<Option<LiveSnapshot>>,
    seq: std::cell::Cell<u64>,
}

impl LiveWindow {
    /// `on_close` runs when the user closes the window (with the number of the match it showed).
    pub fn new(app: &adw::Application, on_close: impl Fn(u64) + 'static) -> Rc<Self> {
        let body = vbox(0);
        body.set_margin_top(22);
        body.set_margin_bottom(26);
        body.set_margin_start(24);
        body.set_margin_end(24);
        let clamp = adw::Clamp::builder().maximum_size(620).child(&body).build();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&clamp)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();

        let header = adw::HeaderBar::builder().css_classes(["flat"]).build();
        header.set_title_widget(Some(&label("Live", &["live-title"])));
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&scroll));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Reclutched \u{2013} Live")
            .default_width(560)
            .default_height(880)
            .width_request(380)
            .content(&view)
            .build();
        window.set_icon_name(Some("dev.reclutched.Reclutched"));

        let names = default_map_path()
            .and_then(|p| PlaylistMap::load(&p).ok())
            .unwrap_or_else(PlaylistMap::with_builtin);
        let live = Rc::new(Self {
            window,
            names,
            body,
            shown: RefCell::new(None),
            seq: std::cell::Cell::new(0),
        });
        let weak = Rc::downgrade(&live);
        live.window.connect_close_request(move |_| {
            if let Some(live) = weak.upgrade() {
                on_close(live.seq.get());
            }
            gtk::glib::Propagation::Proceed
        });
        live
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn close(&self) {
        self.window.destroy();
    }

    /// Shows the newest state of the match.
    pub fn update(&self, snapshot: &LiveSnapshot) {
        if self.shown.borrow().as_ref() == Some(snapshot) {
            return;
        }
        self.seq.set(snapshot.seq);
        clear(&self.body);
        self.build(snapshot);
        *self.shown.borrow_mut() = Some(snapshot.clone());
    }

    /// The tracker has no match any more (it ended, or you left): keep the last numbers, but
    /// say so.
    pub fn ended(&self) {
        let shown = self.shown.borrow().clone();
        if let Some(mut snapshot) = shown {
            // Marking it as over would invent a result, so only the header text changes.
            snapshot.replay = false;
            clear(&self.body);
            self.build(&snapshot);
            self.body.prepend(&label("MATCH ENDED", &["live-ended"]));
        }
    }

    fn build(&self, s: &LiveSnapshot) {
        let b = &self.body;

        // Top line: LIVE and the mode.
        let top = gtk::Box::builder().spacing(10).build();
        let live = if s.winner.is_some() {
            label("FINAL", &["live-flag", "final"])
        } else {
            label("\u{25CF} LIVE", &["live-flag"])
        };
        top.append(&live);
        top.append(&label(
            &mode_name(&self.names, s.playlist_id),
            &["head-sub"],
        ));
        b.append(&top);

        // Score and clock.
        b.append(&score_row(s));

        // Possession.
        if let Some([blue, orange]) = s.possession {
            b.append(&section("Possession"));
            let row = gtk::Box::builder().spacing(10).margin_top(6).build();
            let l = label(&pct(blue), &["stat-small", "team-blue"]);
            let r = label(&pct(orange), &["stat-small", "team-orange"]);
            r.set_xalign(1.0);
            row.append(&l);
            let bar = split_bar(blue, BLUE, ORANGE);
            bar.set_valign(gtk::Align::Center);
            row.append(&bar);
            row.append(&r);
            b.append(&row);
        }

        // You.
        if let Some(me) = s.players.iter().find(|p| p.is_me) {
            if me.boost.is_some() {
                b.append(&section("You"));
                b.append(&you_card(me, s.ball_speed));
            }
        }

        // Scoreboard, your team first.
        let first = s.my_team.unwrap_or(0);
        for team in [first, 1 - first] {
            let players: Vec<&LivePlayer> = s.players.iter().filter(|p| p.team == team).collect();
            if players.is_empty() {
                continue;
            }
            b.append(&section(&format!(
                "{}  \u{00B7}  {}",
                team_name(s, team),
                s.team_goals.get(team as usize).copied().unwrap_or(0)
            )));
            b.append(&team_card(&players, team));
        }

        // Recent events, newest first.
        b.append(&section("Recent"));
        if s.feed.is_empty() {
            b.append(&empty_note("Goals, saves and demolitions show up here."));
        } else {
            let list = vbox(6);
            list.set_margin_top(8);
            for e in s.feed.iter().rev() {
                list.append(&feed_row(e));
            }
            b.append(&list);
        }
    }
}

/// "BLUE 2   3:41   1 ORANGE", with the winner's word when the match is over.
fn score_row(s: &LiveSnapshot) -> gtk::Box {
    let row = gtk::Box::builder().homogeneous(true).margin_top(12).build();
    let side = |team: i64, align: gtk::Align| {
        let c = vbox(0);
        c.set_halign(align);
        // No ellipsis here: the wide letter spacing makes GTK cut short names too early.
        let shown: String = team_name(s, team).to_uppercase().chars().take(14).collect();
        let name = label(&shown, &["live-team", team_class(team)]);
        name.set_halign(align);
        name.set_xalign(if align == gtk::Align::End { 1.0 } else { 0.0 });
        c.append(&name);
        let score = label(
            &s.team_goals
                .get(team as usize)
                .copied()
                .unwrap_or(0)
                .to_string(),
            &["live-score", team_class(team)],
        );
        score.set_halign(align);
        c.append(&score);
        c
    };
    row.append(&side(0, gtk::Align::Start));

    let mid = vbox(2);
    mid.set_valign(gtk::Align::Center);
    mid.set_halign(gtk::Align::Center);
    let clock = label(&clock_text(s.clock), &["live-clock"]);
    clock.set_halign(gtk::Align::Center);
    mid.append(&clock);
    let note = match (s.winner, s.my_team, s.replay) {
        (Some(w), Some(me), _) if w == me => Some(("WIN", "win")),
        (Some(_), Some(_), _) => Some(("LOSS", "loss")),
        (Some(w), None, _) => Some((if w == 0 { "BLUE WINS" } else { "ORANGE WINS" }, "")),
        (None, _, true) => Some(("REPLAY", "")),
        _ => None,
    };
    if let Some((text, class)) = note {
        let l = label(text, &["big-result", class]);
        l.set_halign(gtk::Align::Center);
        l.set_margin_top(0);
        mid.append(&l);
    }
    row.append(&mid);

    row.append(&side(1, gtk::Align::End));
    row
}

/// Your boost as a big bar and your speed.
fn you_card(me: &LivePlayer, ball_speed: f64) -> gtk::Box {
    let card = vbox(14);
    card.add_css_class("player-card");
    card.set_margin_top(8);

    let boost = me.boost.unwrap_or(0);
    let top = gtk::Box::builder().spacing(14).build();
    top.append(&label("BOOST", &["tile-key"]));
    let value = label(&boost.to_string(), &["live-boost"]);
    value.set_hexpand(true);
    value.set_halign(gtk::Align::End);
    top.append(&value);
    card.append(&top);
    let boost_bar = bar(boost as f64 / 100.0);
    boost_bar.add_css_class("big-bar");
    card.append(&boost_bar);

    let cells = gtk::Grid::builder()
        .column_spacing(30)
        .column_homogeneous(true)
        .build();
    let speed = me
        .speed
        .map_or("\u{2013}".to_string(), |v| format!("{v:.0} km/h"));
    for (i, (key, value)) in [
        ("Speed", speed),
        ("Ball", format!("{ball_speed:.0} km/h")),
        ("Demos", me.demos.to_string()),
    ]
    .into_iter()
    .enumerate()
    {
        let cell = vbox(1);
        cell.append(&label(&key.to_uppercase(), &["tile-key"]));
        cell.append(&label(&value, &["stat-small"]));
        cells.attach(&cell, i as i32, 0, 1, 1);
    }
    card.append(&cells);
    card
}

/// One team's players: a small boost bar for the ones the game shows it for, and the numbers.
fn team_card(players: &[&LivePlayer], team: i64) -> gtk::Box {
    let card = vbox(0);
    card.add_css_class("player-card");
    card.set_margin_top(8);
    let grid = gtk::Grid::builder()
        .column_spacing(16)
        .row_spacing(9)
        .build();
    for (c, head) in ["", "Boost", "Score", "G", "Sv", "Sh", "D"]
        .into_iter()
        .enumerate()
    {
        let l = label(head, &["table-head"]);
        if c >= 2 {
            l.set_halign(gtk::Align::End);
            l.set_xalign(1.0);
        }
        grid.attach(&l, c as i32, 0, 1, 1);
    }
    for (i, p) in players.iter().enumerate() {
        let r = i as i32 + 1;
        let classes: Vec<&str> = if p.is_me {
            vec!["cell", "cell-me"]
        } else if p.left || p.demolished {
            vec!["cell", "cell-dim"]
        } else {
            vec!["cell"]
        };
        let mut name = p.name.clone();
        if p.left {
            name.push_str("  (left)");
        } else if p.demolished {
            name.push_str("  \u{2716}");
        }
        let n = name_label(&name, &classes);
        n.set_hexpand(true);
        grid.attach(&n, 0, r, 1, 1);
        match p.boost {
            Some(boost) if !p.left => {
                let b = bar(boost as f64 / 100.0);
                b.set_width_request(64);
                b.add_css_class(team_class(team));
                grid.attach(&b, 1, r, 1, 1);
            }
            _ => grid.attach(&gtk::Box::new(gtk::Orientation::Horizontal, 0), 1, r, 1, 1),
        }
        for (c, n) in [p.score, p.goals, p.saves, p.shots, p.demos]
            .into_iter()
            .enumerate()
        {
            let l = label(&n.to_string(), &classes);
            l.set_halign(gtk::Align::End);
            l.set_xalign(1.0);
            grid.attach(&l, c as i32 + 2, r, 1, 1);
        }
    }
    card.append(&grid);
    card
}

fn feed_row(e: &LiveEvent) -> gtk::Box {
    let row = gtk::Box::builder().spacing(12).build();
    row.append(&label(&clock_text(e.clock), &["goal-time"]));
    row.append(&label("\u{25CF}", &[team_class(e.team)]));
    let text = match e.kind {
        LiveKind::Goal => format!("Goal \u{2013} {}", e.name),
        LiveKind::Save => format!("Save \u{2013} {}", e.name),
        LiveKind::EpicSave => format!("Epic save \u{2013} {}", e.name),
        LiveKind::Demolish => match &e.other {
            Some(victim) => format!("Demolition \u{2013} {} \u{2192} {victim}", e.name),
            None => format!("Demolition \u{2013} {}", e.name),
        },
    };
    let l = name_label(&text, &["cell"]);
    row.append(&l);
    row
}
