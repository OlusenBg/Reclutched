//! The right side: everything we stored about one match, in four calm tabs.
//! Overview first; the other tabs are where you go looking for more.

use adw::prelude::*;
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::storage::MatchDetail;
use reclutched_core::rocketleague::tracker::{MovementSummary, PlayerSummary, TouchSummary};

use crate::format::{
    clock_text, mode_name, pct, player_name, result_class, result_word, score_from_my_side,
    score_text, when_label,
};

pub struct DetailView {
    pub stack: gtk::Stack,
    overview: gtk::Box,
    players: gtk::Box,
    goals: gtk::Box,
    movement: gtk::Box,
}

// ---------- small building blocks ----------

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let wraps = classes
        .iter()
        .any(|c| matches!(*c, "head-sub" | "muted-note" | "fact"));
    gtk::Label::builder()
        .label(text)
        .css_classes(classes.iter().map(|c| c.to_string()).collect::<Vec<_>>())
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .wrap(wraps)
        .build()
}

fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(spacing)
        .build()
}

fn section(title: &str) -> gtk::Label {
    let l = label(&title.to_uppercase(), &["section-label"]);
    l.set_margin_top(26);
    l
}

/// A muted one-liner for things we don't have (a switch was off, or the game didn't send it).
fn empty_note(text: &str) -> gtk::Label {
    let l = label(text, &["muted-note"]);
    l.set_margin_top(6);
    l
}

/// A big quiet number with a small caption.
fn tile(key: &str, value: &str, hint: &str) -> gtk::Box {
    let b = vbox(2);
    b.add_css_class("tile");
    b.set_hexpand(true);
    b.append(&label(&key.to_uppercase(), &["tile-key"]));
    b.append(&label(value, &["tile-val"]));
    b.append(&label(hint, &["tile-hint"]));
    b
}

fn scroll_page(content: &gtk::Box) -> gtk::ScrolledWindow {
    content.set_margin_top(28);
    content.set_margin_bottom(32);
    content.set_margin_start(28);
    content.set_margin_end(28);
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .child(content)
        .build();
    gtk::ScrolledWindow::builder()
        .child(&clamp)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build()
}

fn clear(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

/// A thin horizontal bar for a value between 0 and 1.
fn bar(fraction: f64) -> gtk::ProgressBar {
    gtk::ProgressBar::builder()
        .fraction(fraction.clamp(0.0, 1.0))
        .css_classes(["thin-bar"])
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build()
}

/// A bar split in three parts (own third, middle, enemy third) in proportion to the numbers.
fn thirds_bar(def: i64, mid: i64, att: i64) -> gtk::DrawingArea {
    let total = (def + mid + att).max(1) as f64;
    let parts = [def as f64 / total, mid as f64 / total, att as f64 / total];
    let area = gtk::DrawingArea::builder()
        .content_height(8)
        .hexpand(true)
        .build();
    area.set_draw_func(move |_, cr, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        let colors = [(0.36, 0.30, 0.30), (0.55, 0.47, 0.47), (0.64, 0.15, 0.19)];
        let mut x = 0.0;
        for (part, (r, g, b)) in parts.iter().zip(colors) {
            let seg = (w * part - 2.0).max(0.0);
            if seg > 0.0 {
                cr.set_source_rgb(r, g, b);
                cr.rectangle(x, 0.0, seg, h);
                let _ = cr.fill();
            }
            x += w * part;
        }
    });
    area
}

// ---------- the view ----------

impl DetailView {
    pub fn new() -> Self {
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(160)
            .build();
        let pages = [
            ("overview", "Overview"),
            ("players", "Players"),
            ("goals", "Goals"),
            ("movement", "Movement"),
        ];
        let boxes: Vec<gtk::Box> = pages.iter().map(|_| vbox(0)).collect();
        for ((name, title), content) in pages.iter().zip(&boxes) {
            stack.add_titled(&scroll_page(content), Some(name), title);
        }
        let [overview, players, goals, movement]: [gtk::Box; 4] = boxes.try_into().unwrap();
        Self {
            stack,
            overview,
            players,
            goals,
            movement,
        }
    }

    pub fn set(&self, detail: Option<&MatchDetail>, names: &PlaylistMap) {
        for b in [&self.overview, &self.players, &self.goals, &self.movement] {
            clear(b);
        }
        let Some(d) = detail else { return };
        self.fill_overview(d, names);
        self.fill_players(d);
        self.fill_goals(d);
        self.fill_movement(d);
    }

    fn fill_overview(&self, d: &MatchDetail, names: &PlaylistMap) {
        let b = &self.overview;
        let row = &d.row;

        // The headline: the result word and the score.
        let head = gtk::Box::builder().spacing(18).build();
        head.append(&label(
            result_word(&row.result),
            &["big-result", result_class(&row.result)],
        ));
        let score = label(&score_text(row), &["big-score"]);
        score.set_valign(gtk::Align::Center);
        head.append(&score);
        b.append(&head);

        let mut sub = format!(
            "{}  \u{00B7}  {}",
            mode_name(names, row.playlist_id),
            when_label(row.ended_at)
        );
        if row.overtime {
            sub.push_str("  \u{00B7}  Overtime");
        }
        if let Some(clock) = row.abandoned_at {
            sub.push_str(&format!(
                "  \u{00B7}  Left with {} on the clock",
                clock_text(clock)
            ));
        }
        b.append(&label(&sub, &["head-sub"]));

        // Four quiet tiles.
        let me_move = d.movement.iter().find(|m| m.slot == d.my_slot);
        let tiles = gtk::Box::builder().spacing(12).homogeneous(true).build();
        tiles.set_margin_top(24);
        tiles.append(&tile(
            "Possession",
            &d.my_possession_pct.map_or("\u{2013}".into(), pct),
            "your team had the ball",
        ));
        tiles.append(&tile(
            "Avg boost",
            &me_move.map_or("\u{2013}".into(), |m| format!("{:.0}", m.avg_boost)),
            "out of 100",
        ));
        tiles.append(&tile(
            "Avg speed",
            &me_move.map_or("\u{2013}".into(), |m| format!("{:.0}", m.avg_speed)),
            "km/h",
        ));
        tiles.append(&tile(
            "Touches",
            &touch_share(d).map_or("\u{2013}".into(), pct),
            "of your team's touches",
        ));
        b.append(&tiles);

        // Your numbers.
        b.append(&section("Your match"));
        if let Some(me) = d.players.iter().find(|p| p.slot == d.my_slot) {
            let grid = gtk::Grid::builder()
                .column_spacing(34)
                .row_spacing(4)
                .build();
            let stats = [
                ("Score", me.score),
                ("Goals", me.goals),
                ("Assists", me.assists),
                ("Saves", me.saves),
                ("Shots", me.shots),
                ("Demos", me.demos),
            ];
            for (i, (key, value)) in stats.into_iter().enumerate() {
                let col = i as i32;
                grid.attach(&label(&key.to_uppercase(), &["tile-key"]), col, 0, 1, 1);
                grid.attach(&label(&value.to_string(), &["stat-val"]), col, 1, 1, 1);
            }
            b.append(&grid);
            // Shots that did not score.
            if me.shots > 0 {
                let missed = ((me.shots - me.goals).max(0) as f64) * 100.0 / me.shots as f64;
                b.append(&label(
                    &format!(
                        "{} of your {} shots missed ({}).",
                        me.shots - me.goals,
                        me.shots,
                        pct(missed)
                    ),
                    &["fact"],
                ));
            }
        } else {
            b.append(&empty_note("Player stats were not stored for this match."));
        }

        // A few more facts, only when we have them.
        let mut facts = Vec::new();
        if let (Some(avg), Some(max)) = (d.ball_avg_speed, d.ball_max_speed) {
            facts.push(format!(
                "The ball averaged {avg} km/h and peaked at {max} km/h."
            ));
        }
        if let Some(o) = d.team_empty_overlap {
            facts.push(format!(
                "You and a teammate were both out of boost {} of the time.",
                pct(o)
            ));
        }
        if !facts.is_empty() {
            b.append(&section("Also"));
            for f in facts {
                b.append(&label(&f, &["fact"]));
            }
        }
    }

    fn fill_players(&self, d: &MatchDetail) {
        let b = &self.players;
        if d.players.is_empty() {
            b.append(&empty_note("Players were not stored for this match."));
            return;
        }
        let my_team = d.row.my_team;
        // One grid for both teams, so the columns line up.
        let grid = gtk::Grid::builder()
            .column_spacing(24)
            .row_spacing(6)
            .build();
        let mut r = 0;
        for (title, mine) in [("Your team", true), ("Opponents", false)] {
            let group: Vec<&PlayerSummary> = d
                .players
                .iter()
                .filter(|p| (p.team == my_team) == mine)
                .collect();
            if group.is_empty() {
                continue;
            }
            let heading = section(title);
            heading.set_margin_top(if r == 0 { 0 } else { 30 });
            grid.attach(&heading, 0, r, 9, 1);
            r += 1;
            for (c, head) in [
                "Player", "Score", "Goals", "Assists", "Saves", "Shots", "Demos", "Touches",
            ]
            .into_iter()
            .enumerate()
            {
                let l = label(head, &["table-head"]);
                l.set_margin_top(8);
                if c > 0 {
                    l.set_halign(gtk::Align::End);
                    l.set_xalign(1.0);
                }
                grid.attach(&l, c as i32, r, 1, 1);
            }
            r += 1;
            for p in group {
                let classes: &[&str] = if p.is_me {
                    &["cell", "cell-me"]
                } else {
                    &["cell"]
                };
                grid.attach(
                    &label(&player_name(&d.players, p.slot), classes),
                    0,
                    r,
                    1,
                    1,
                );
                let numbers = [
                    p.score, p.goals, p.assists, p.saves, p.shots, p.demos, p.touches,
                ];
                for (c, n) in numbers.into_iter().enumerate() {
                    let l = label(&n.to_string(), classes);
                    l.set_halign(gtk::Align::End);
                    l.set_xalign(1.0);
                    grid.attach(&l, c as i32 + 1, r, 1, 1);
                }
                // Someone who left early gets a quiet note after the numbers.
                if let Some(left) = p.left {
                    grid.attach(
                        &label(&format!("left at {}", clock_text(left)), &["row-note"]),
                        8,
                        r,
                        1,
                        1,
                    );
                }
                r += 1;
            }
        }
        b.append(&grid);
    }

    fn fill_goals(&self, d: &MatchDetail) {
        let b = &self.goals;
        if d.goals.is_empty() {
            b.append(&empty_note(if d.row.team0_goals + d.row.team1_goals == 0 {
                "No goals in this match."
            } else {
                "The goals were not stored for this match."
            }));
            return;
        }
        b.append(&label(
            "Goals in the order they happened. The time is the match clock.",
            &["head-sub"],
        ));
        let grid = gtk::Grid::builder()
            .column_spacing(26)
            .row_spacing(10)
            .margin_top(16)
            .build();
        for (r, g) in d.goals.iter().enumerate() {
            let r = r as i32;
            let scorer_team = d
                .players
                .iter()
                .find(|p| p.slot == g.scorer)
                .map(|p| p.team);
            let mine = scorer_team == Some(d.row.my_team);
            let who = vbox(1);
            who.append(&label(
                &player_name(&d.players, g.scorer),
                &["cell", if mine { "goal-mine" } else { "goal-theirs" }],
            ));
            if let Some(a) = g.assister {
                who.append(&label(
                    &format!("assist {}", player_name(&d.players, a)),
                    &["row-note"],
                ));
            }
            let time = label(&clock_text(g.clock), &["goal-time"]);
            time.set_valign(gtk::Align::Start);
            grid.attach(&time, 0, r, 1, 1);
            grid.attach(&who, 1, r, 1, 1);

            let mut details = format!("{:.0} km/h shot", g.goal_speed);
            if let Some(boost) = g.scorer_boost {
                details.push_str(&format!("  \u{00B7}  {boost} boost"));
            }
            let l = label(&details, &["row-note"]);
            l.set_valign(gtk::Align::Start);
            grid.attach(&l, 2, r, 1, 1);

            if let Some(after) = g.score_after {
                let s = label(&score_from_my_side(after, d.row.my_team), &["goal-score"]);
                s.set_halign(gtk::Align::End);
                s.set_hexpand(true);
                s.set_valign(gtk::Align::Start);
                grid.attach(&s, 3, r, 1, 1);
            }
        }
        b.append(&grid);
    }

    fn fill_movement(&self, d: &MatchDetail) {
        let b = &self.movement;
        if d.movement.is_empty() && d.touches.is_empty() {
            b.append(&empty_note(
                "Boost, speed and touch habits were not collected for this match.",
            ));
            return;
        }

        if !d.movement.is_empty() {
            b.append(&label(
                "Boost and speed for you and your teammates (the game does not send these for opponents).",
                &["head-sub"],
            ));
            for m in &d.movement {
                b.append(&movement_card(
                    m,
                    &player_name(&d.players, m.slot),
                    m.slot == d.my_slot,
                ));
            }
        }

        // How the stats looked at key moments (your saves, shots, ...).
        let mine: Vec<_> = d
            .event_stats
            .iter()
            .filter(|e| e.slot == d.my_slot)
            .collect();
        if !mine.is_empty() {
            b.append(&section("When it happened (you)"));
            for e in mine {
                let ctx = match (e.avg_boost, e.avg_speed) {
                    (Some(boost), Some(speed)) => {
                        format!("boost {boost:.0}, speed {speed:.0} km/h on average")
                    }
                    (Some(boost), None) => format!("boost {boost:.0} on average"),
                    _ => String::new(),
                };
                let row = gtk::Box::builder().spacing(14).build();
                row.append(&label(
                    &format!("{} \u{00D7} {}", e.event, e.count),
                    &["cell"],
                ));
                row.append(&label(&ctx, &["row-note"]));
                b.append(&row);
            }
        }

        if !d.touches.is_empty() {
            b.append(&section("Ball touches"));
            b.append(&label(
                "Where on the field each player touched the ball: own third, middle, enemy third.",
                &["head-sub"],
            ));
            for t in &d.touches {
                b.append(&touch_row(
                    t,
                    &player_name(&d.players, t.slot),
                    t.slot == d.my_slot,
                ));
            }
        }
    }
}

/// Your share of your team's ball touches.
fn touch_share(d: &MatchDetail) -> Option<f64> {
    let me = d.touches.iter().find(|t| t.slot == d.my_slot)?;
    // Without teammates "your share of the team" would always be 100%.
    let has_teammates = d
        .players
        .iter()
        .any(|p| p.team == d.row.my_team && p.slot != d.my_slot);
    if !has_teammates {
        return None;
    }
    let team: i64 = d
        .touches
        .iter()
        .filter(|t| {
            d.players
                .iter()
                .find(|p| p.slot == t.slot)
                .is_some_and(|p| p.team == d.row.my_team)
        })
        .map(|t| t.touches)
        .sum();
    (team > 0).then(|| me.touches as f64 * 100.0 / team as f64)
}

fn movement_card(m: &MovementSummary, name: &str, is_me: bool) -> gtk::Box {
    let card = vbox(10);
    card.add_css_class("player-card");
    card.set_margin_top(14);

    let top = gtk::Box::builder().spacing(14).build();
    top.append(&label(
        name,
        &["cell", if is_me { "cell-me" } else { "cell" }],
    ));
    let boost = gtk::Box::builder()
        .spacing(10)
        .hexpand(true)
        .halign(gtk::Align::End)
        .build();
    boost.append(&label("AVG BOOST", &["tile-key"]));
    let b = bar(m.avg_boost / 100.0);
    b.set_width_request(160);
    boost.append(&b);
    boost.append(&label(&format!("{:.0}", m.avg_boost), &["stat-val"]));
    top.append(&boost);
    card.append(&top);

    let grid = gtk::Grid::builder()
        .column_spacing(30)
        .row_spacing(10)
        .column_homogeneous(true)
        .build();
    let cells = [
        ("Out of boost", pct(m.pct_boost_empty)),
        (
            "Used / picked up",
            format!("{} / {}", m.boost_used, m.boost_gained),
        ),
        (
            "Avg / top speed",
            format!("{:.0} / {:.0}", m.avg_speed, m.max_speed),
        ),
        ("Supersonic", pct(m.pct_supersonic)),
        ("Powersliding", pct(m.pct_powersliding)),
        ("In the air", pct(m.pct_airborne)),
        ("On walls", pct(m.pct_on_wall)),
        ("Demolished", format!("{} \u{00D7}", m.times_demolished)),
    ];
    for (i, (key, value)) in cells.into_iter().enumerate() {
        let cell = vbox(1);
        cell.append(&label(&key.to_uppercase(), &["tile-key"]));
        cell.append(&label(&value, &["stat-small"]));
        grid.attach(&cell, (i % 4) as i32, (i / 4) as i32, 1, 1);
    }
    card.append(&grid);
    card
}

fn touch_row(t: &TouchSummary, name: &str, is_me: bool) -> gtk::Box {
    let row = vbox(5);
    row.set_margin_top(12);
    let top = gtk::Box::builder().spacing(12).build();
    top.append(&label(
        name,
        &["cell", if is_me { "cell-me" } else { "cell" }],
    ));
    let right = label(&format!("{} touches", t.touches), &["row-note"]);
    right.set_hexpand(true);
    right.set_halign(gtk::Align::End);
    top.append(&right);
    row.append(&top);
    row.append(&thirds_bar(t.def_third, t.mid_third, t.att_third));
    row
}
