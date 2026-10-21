//! The right side: everything we stored about one match, in four calm tabs.
//! Overview first; the other tabs are where you go looking for more.

use adw::prelude::*;
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::storage::MatchDetail;
use reclutched_core::rocketleague::tracker::{
    GoalRow, MovementSummary, PlayerSummary, TouchSummary,
};

use crate::format::{
    clock_text, mode_name, pct, player_name, result_class, result_word, score_from_my_side,
    score_text, when_label,
};
use crate::widgets::{
    bar, clear, empty_note, label, meter, name_label, scroll_page, section, small_tile, split_bar,
    stripe, thirds_bar, tile, vbox,
};

pub struct DetailView {
    pub stack: gtk::Stack,
    overview: gtk::Box,
    players: gtk::Box,
    goals: gtk::Box,
    movement: gtk::Box,
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
        // A flow box: four tiles in a row when there is room, two by two when there is not.
        let tiles = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(2)
            .max_children_per_line(4)
            .column_spacing(12)
            .row_spacing(12)
            .homogeneous(true)
            .build();
        tiles.set_margin_top(24);
        tiles.append(&tile(
            "Possession",
            &d.my_possession_pct.map_or("\u{2013}".into(), pct),
            "time with the ball",
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
            "of team touches",
        ));
        b.append(&tiles);

        // Your numbers, as small tiles.
        b.append(&section("Your match"));
        if let Some(me) = d.players.iter().find(|p| p.slot == d.my_slot) {
            let stats = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .min_children_per_line(3)
                .max_children_per_line(6)
                .column_spacing(10)
                .row_spacing(10)
                .homogeneous(true)
                .margin_top(8)
                .build();
            for (key, value) in [
                ("Score", me.score),
                ("Goals", me.goals),
                ("Assists", me.assists),
                ("Saves", me.saves),
                ("Shots", me.shots),
                ("Demos", me.demos),
            ] {
                stats.append(&small_tile(key, &value.to_string()));
            }
            b.append(&stats);
        } else {
            b.append(&empty_note("Player stats were not stored for this match."));
        }

        // How the match went, as bars. Only what we have.
        let card = vbox(16);
        card.add_css_class("player-card");
        card.set_margin_top(16);
        let mut rows = 0;
        if let Some(p) = d.my_possession_pct {
            let block = vbox(6);
            let top = gtk::Box::builder().spacing(12).build();
            top.append(&label("POSSESSION", &["tile-key"]));
            let v = label(
                &format!("You {}  \u{00B7}  Them {}", pct(p), pct(100.0 - p)),
                &["stat-small"],
            );
            v.set_hexpand(true);
            v.set_halign(gtk::Align::End);
            top.append(&v);
            block.append(&top);
            block.append(&split_bar(p, MINE, THEIRS));
            card.append(&block);
            rows += 1;
        }
        if let Some(me) = d.players.iter().find(|p| p.slot == d.my_slot) {
            if me.shots > 0 {
                card.append(&meter(
                    "Shot accuracy",
                    &format!("{} of {} shots scored", me.goals, me.shots),
                    me.goals as f64 / me.shots as f64,
                ));
                rows += 1;
            }
        }
        if let Some(o) = d.team_empty_overlap {
            card.append(&meter(
                "You and a teammate both out of boost",
                &pct(o),
                o / 100.0,
            ));
            rows += 1;
        }
        if let (Some(avg), Some(max)) = (d.ball_avg_speed, d.ball_max_speed) {
            card.append(&meter(
                "Ball speed (average / top)",
                &format!("{avg} / {max} km/h"),
                if max > 0 {
                    avg as f64 / max as f64
                } else {
                    0.0
                },
            ));
            rows += 1;
        }
        if rows > 0 {
            b.append(&card);
        }
    }

    fn fill_players(&self, d: &MatchDetail) {
        let b = &self.players;
        if d.players.is_empty() {
            b.append(&empty_note("Players were not stored for this match."));
            return;
        }
        let my_team = d.row.my_team;
        let max_score = d.players.iter().map(|p| p.score).max().unwrap_or(0).max(1);
        let mut first = true;
        for (title, mine) in [("Your team", true), ("Opponents", false)] {
            let group: Vec<&PlayerSummary> = d
                .players
                .iter()
                .filter(|p| (p.team == my_team) == mine)
                .collect();
            if group.is_empty() {
                continue;
            }
            let (team_goals, other_goals) = if my_team == 0 {
                (d.row.team0_goals, d.row.team1_goals)
            } else {
                (d.row.team1_goals, d.row.team0_goals)
            };
            let goals = if mine { team_goals } else { other_goals };
            let heading = section(&format!("{title}  \u{00B7}  {goals} goals"));
            heading.set_margin_top(if first { 0 } else { 30 });
            first = false;
            b.append(&heading);
            for p in group {
                b.append(&player_card(p, &player_name(&d.players, p.slot), max_score));
            }
        }
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
        let max_speed = d.goals.iter().map(|g| g.goal_speed).fold(1.0, f64::max);
        b.append(&label(
            "Goals in the order they happened. The time is the match clock.",
            &["head-sub"],
        ));
        for g in &d.goals {
            b.append(&goal_card(d, g, max_speed));
        }
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

/// Team colours for the two-part bars: yours in the app's red, the others in grey.
const MINE: (f64, f64, f64) = (0.64, 0.15, 0.19);
const THEIRS: (f64, f64, f64) = (0.36, 0.31, 0.31);

/// One player as a card: name, a bar for the score compared to the best score of the match,
/// and the other numbers in a row.
fn player_card(p: &PlayerSummary, name: &str, max_score: i64) -> gtk::Box {
    let card = vbox(10);
    card.add_css_class("player-card");
    if p.is_me {
        card.add_css_class("me");
    }
    card.set_margin_top(10);

    let top = gtk::Box::builder().spacing(14).build();
    top.append(&name_label(
        name,
        &["cell", if p.is_me { "cell-me" } else { "cell" }],
    ));
    if let Some(left) = p.left {
        top.append(&label(
            &format!("left at {}", clock_text(left)),
            &["row-note"],
        ));
    }
    let score = gtk::Box::builder()
        .spacing(10)
        .hexpand(true)
        .halign(gtk::Align::End)
        .build();
    score.append(&label("SCORE", &["tile-key"]));
    let b = bar(p.score as f64 / max_score as f64);
    b.set_width_request(120);
    score.append(&b);
    score.append(&label(&p.score.to_string(), &["stat-val"]));
    top.append(&score);
    card.append(&top);

    let grid = gtk::Grid::builder()
        .column_spacing(30)
        .row_spacing(10)
        .column_homogeneous(true)
        .build();
    for (i, (key, value)) in [
        ("Goals", p.goals),
        ("Assists", p.assists),
        ("Saves", p.saves),
        ("Shots", p.shots),
        ("Demos", p.demos),
        ("Touches", p.touches),
    ]
    .into_iter()
    .enumerate()
    {
        let cell = vbox(1);
        cell.append(&label(&key.to_uppercase(), &["tile-key"]));
        cell.append(&label(&value.to_string(), &["stat-small"]));
        grid.attach(&cell, i as i32, 0, 1, 1);
    }
    card.append(&grid);
    card
}

/// One goal as a card: green edge for yours, red for theirs; the shot speed and the scorer's
/// boost as bars.
fn goal_card(d: &MatchDetail, g: &GoalRow, max_speed: f64) -> gtk::Box {
    let scorer_team = d
        .players
        .iter()
        .find(|p| p.slot == g.scorer)
        .map(|p| p.team);
    let mine = scorer_team == Some(d.row.my_team);

    let outer = gtk::Box::builder().spacing(14).build();
    outer.add_css_class("player-card");
    outer.set_margin_top(10);
    outer.append(&stripe(if mine { "win" } else { "loss" }));

    let inner = vbox(12);
    inner.set_hexpand(true);

    let top = gtk::Box::builder().spacing(16).build();
    let time = label(&clock_text(g.clock), &["goal-time"]);
    time.set_valign(gtk::Align::Start);
    top.append(&time);
    let who = vbox(1);
    who.append(&name_label(
        &player_name(&d.players, g.scorer),
        &["cell", if mine { "goal-mine" } else { "goal-theirs" }],
    ));
    if let Some(a) = g.assister {
        who.append(&name_label(
            &format!("assist {}", player_name(&d.players, a)),
            &["row-note"],
        ));
    }
    top.append(&who);
    if let Some(after) = g.score_after {
        let s = label(&score_from_my_side(after, d.row.my_team), &["stat-val"]);
        s.set_hexpand(true);
        s.set_halign(gtk::Align::End);
        top.append(&s);
    }
    inner.append(&top);

    let meters = gtk::Box::builder().spacing(28).homogeneous(true).build();
    meters.append(&meter(
        "Shot speed",
        &format!("{:.0} km/h", g.goal_speed),
        g.goal_speed / max_speed,
    ));
    if let Some(boost) = g.scorer_boost {
        meters.append(&meter(
            "Scorer's boost",
            &boost.to_string(),
            boost as f64 / 100.0,
        ));
    }
    inner.append(&meters);

    outer.append(&inner);
    outer
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
    top.append(&name_label(
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
    top.append(&name_label(
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
