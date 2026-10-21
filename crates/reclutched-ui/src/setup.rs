//! The first-launch setup wizard: the pages and the navigation between them.
//! The rules (which step is next, what the answers mean) live in `flow.rs`.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::gdk;
use reclutched_core::rocketleague::settings::{DisplayMode, Settings};

use crate::flow::{
    username_error, SetupKind, SetupState, Step, ADV_EVENTS, ADV_HABITS, ADV_MATCHES,
};
use crate::ready::fact;

/// The wizard. Widgets call back into it through `Weak` pointers so nothing keeps it alive
/// longer than the window needs it (a plain `Rc` in every closure would form a cycle).
pub struct Wizard {
    state: RefCell<SetupState>,
    step: Cell<Step>,
    stack: gtk::Stack,
    back: gtk::Button,
    next: gtk::Button,
    dots: gtk::Box,
    error: gtk::Label,
    mode_cards: Vec<gtk::Button>,
    kind_cards: Vec<gtk::Button>,
    username: gtk::Entry,
    summary: gtk::Box,
    /// Called with the finished settings when the user presses Finish.
    on_finish: Box<dyn Fn(Settings)>,
    root: gtk::Box,
}

fn name_of(step: Step) -> &'static str {
    match step {
        Step::Username => "username",
        Step::Mode => "mode",
        Step::Kind => "kind",
        Step::Regular => "regular",
        Step::AdvMatches => "adv-matches",
        Step::AdvEvents => "adv-events",
        Step::AdvHabits => "adv-habits",
        Step::Finish => "finish",
    }
}

/// Friendly titles for the switches (the descriptions come from `TOGGLES`).
fn switch_title(name: &str) -> &'static str {
    match name {
        "matches" => "Match results",
        "roster" => "Players and their stats",
        "offline_matches" => "Offline matches",
        "goals" => "Goals",
        "stat_events" => "Shots, saves and demolitions",
        "crossbar_hits" => "Crossbar hits",
        "boost_pickups" => "Boost pickups",
        "movement" => "Boost and speed habits",
        "ball_hits" => "Ball touches",
        "possession" => "Possession and ball speed",
        _ => "",
    }
}

/// One calm sentence per switch.
fn switch_subtitle(name: &str) -> &'static str {
    match name {
        "matches" => {
            "Win or loss, the score and matches you left early. Turning this off stores nothing."
        }
        "roster" => "Everyone who played, their final stats, and who joined or left and when.",
        "offline_matches" => "Also keep matches against bots, exhibition games and similar.",
        "goals" => "Who scored, who assisted, how fast the shot was and when it happened.",
        "stat_events" => {
            "Shots, saves, demolitions and MVPs, with your boost and speed at that moment."
        }
        "crossbar_hits" => "How often the ball hit the crossbar and the hardest hit.",
        "boost_pickups" => "Boost pad pickups. The game only reports these while spectating.",
        "movement" => "Boost use, speed, powerslides and time in the air, as averages per match.",
        "ball_hits" => "Ball touches per player: how many, where on the field and how hard.",
        "possession" => "How long your team had the ball, and the ball's speed.",
        _ => "",
    }
}

fn title(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .css_classes(["setup-title"])
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .wrap(true)
        .build()
}

fn subtitle(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .css_classes(["setup-sub"])
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .wrap(true)
        .build()
}

fn page_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build()
}

/// Puts a page in a fixed-width column that starts at the same height on every step, so
/// titles do not jump around when you move between pages.
fn centered(content: &gtk::Box) -> gtk::Widget {
    content.set_width_request(500);
    adw::Clamp::builder()
        .maximum_size(500)
        .child(content)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Start)
        .margin_top(96)
        .margin_start(24)
        .margin_end(24)
        .build()
        .upcast()
}

/// A big selectable card: number, title and one line of explanation.
fn card(number: &str, heading: &str, description: &str) -> gtk::Button {
    let num = gtk::Label::builder()
        .label(number)
        .css_classes(["choice-num"])
        .valign(gtk::Align::Start)
        .margin_top(3)
        .build();
    let head = gtk::Label::builder()
        .label(heading)
        .css_classes(["choice-title"])
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .build();
    let desc = gtk::Label::builder()
        .label(description)
        .css_classes(["choice-desc"])
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .hexpand(true)
        .max_width_chars(60)
        .wrap(true)
        .build();
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    text.append(&head);
    text.append(&desc);
    let row = gtk::Box::builder().spacing(14).build();
    row.append(&num);
    row.append(&text);
    gtk::Button::builder()
        .child(&row)
        .css_classes(["choice"])
        .build()
}

fn switch_row(name: &str, title: &str, subtitle: &str, on: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(title)
        .subtitle(subtitle)
        .active(on)
        .build();
    let _ = name;
    row
}

fn rows_list() -> gtk::ListBox {
    gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_top(10)
        .build()
}

impl Wizard {
    /// Builds the whole wizard, starting from the settings that are saved now.
    pub fn new(current: Settings, on_finish: impl Fn(Settings) + 'static) -> Rc<Self> {
        let state = SetupState::new(current);

        // The note in the top-left corner, on every page.
        let note = gtk::Label::builder()
            .label("*These can be changed in settings later")
            .css_classes(["setup-note"])
            .halign(gtk::Align::Start)
            .margin_top(18)
            .margin_start(24)
            .build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::SlideLeft)
            .transition_duration(260)
            .hexpand(true)
            .vexpand(true)
            .build();

        let back = gtk::Button::builder()
            .label("Back")
            .css_classes(["flat-nav"])
            .build();
        let next = gtk::Button::builder()
            .label("Next")
            .css_classes(["primary-nav"])
            .build();
        let dots = gtk::Box::builder()
            .spacing(6)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        let bar = gtk::CenterBox::builder()
            .margin_start(24)
            .margin_end(24)
            .margin_bottom(22)
            .build();
        bar.set_start_widget(Some(&back));
        bar.set_center_widget(Some(&dots));
        bar.set_end_widget(Some(&next));

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        root.append(&note);
        root.append(&stack);
        root.append(&bar);

        // --- pages that need to be reachable later (cards, entry, ...) ---
        let username = gtk::Entry::builder()
            .placeholder_text("Username")
            .max_length(64)
            .css_classes(["setup-entry"])
            .text(state.username.as_str())
            .build();
        let error = gtk::Label::builder()
            .css_classes(["setup-error"])
            .halign(gtk::Align::Start)
            .build();
        let mode_cards = vec![
            card("1", "Overlay", "Only the in-game overlay. No other window opens."),
            card(
                "2",
                "2 Screens",
                "Instead of an overlay, live stats appear in a window. Great if you have two screens.",
            ),
            card("3", "Both", "The in-game overlay and the live stats window."),
        ];
        let kind_cards = vec![
            card("1", "Regular", "For starting to track fast"),
            card("2", "Advanced", "Fully customize your experience"),
        ];
        let summary = page_box();
        summary.set_spacing(14);

        let wiz = Rc::new(Self {
            state: RefCell::new(state),
            step: Cell::new(Step::Username),
            stack,
            back,
            next,
            dots,
            error,
            mode_cards,
            kind_cards,
            username,
            summary,
            on_finish: Box::new(on_finish),
            root,
        });
        wiz.build_pages();
        wiz.wire_navigation();
        wiz.refresh_nav();
        wiz
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn weak(self: &Rc<Self>) -> Weak<Self> {
        Rc::downgrade(self)
    }

    fn build_pages(self: &Rc<Self>) {
        // 1. Username
        let page = page_box();
        page.append(&title("Welcome to Reclutched !"));
        page.append(&subtitle("Please enter your RL Username."));
        self.username.set_margin_top(14);
        page.append(&self.username);
        page.append(&self.error);
        page.append(
            &gtk::Label::builder()
                .label("* This will be used for showing your rank in the app and has nothing to do with accounts")
                .css_classes(["setup-hint"])
                .halign(gtk::Align::Start)
                .xalign(0.0)
                .wrap(true)
                .build(),
        );
        self.stack.add_named(&centered(&page), Some("username"));
        let w = self.weak();
        self.username.connect_changed(move |entry| {
            if let Some(w) = w.upgrade() {
                w.state.borrow_mut().username = entry.text().to_string();
                // Only complain about things that are wrong, not about an empty box.
                let text = entry.text();
                let msg = if text.trim().is_empty() {
                    None
                } else {
                    username_error(&text)
                };
                w.error.set_text(msg.unwrap_or(""));
                w.refresh_nav();
            }
        });
        let w = self.weak();
        self.username.connect_activate(move |_| {
            if let Some(w) = w.upgrade() {
                w.go_next();
            }
        });

        // 2. Mode
        let page = page_box();
        page.append(&title("Please choose a mode"));
        page.append(&subtitle(
            "How do you want to see your stats while you play?",
        ));
        let modes = [
            DisplayMode::Overlay,
            DisplayMode::TwoScreens,
            DisplayMode::Both,
        ];
        for (card, mode) in self.mode_cards.iter().zip(modes) {
            card.set_margin_top(4);
            page.append(card);
            let w = self.weak();
            card.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    w.choose_mode(mode);
                }
            });
        }
        self.stack.add_named(&centered(&page), Some("mode"));

        // 3. Kind
        let page = page_box();
        let heading = title("Choose your setup");
        heading.set_margin_bottom(10);
        page.append(&heading);
        let kinds = [SetupKind::Regular, SetupKind::Advanced];
        for (card, kind) in self.kind_cards.iter().zip(kinds) {
            card.set_margin_top(4);
            page.append(card);
            let w = self.weak();
            card.connect_clicked(move |_| {
                if let Some(w) = w.upgrade() {
                    w.choose_kind(kind);
                }
            });
        }
        self.stack.add_named(&centered(&page), Some("kind"));

        // 4. Regular: three yes/no questions
        let page = page_box();
        page.append(&title("A few quick questions"));
        page.append(&subtitle(
            "Everything else stays on its recommended default.",
        ));
        let list = rows_list();
        let regular = self.state.borrow().regular;
        let questions = [
            (
                "Track offline matches",
                "Matches against bots and exhibition games",
                regular.offline_matches,
            ),
            (
                "Collect detailed stats",
                "Boost habits, touches, possession and crossbar hits",
                regular.detailed_stats,
            ),
            (
                "Remember other players' names",
                "Show who you played with in your match history",
                regular.remember_names,
            ),
        ];
        for (i, (heading, desc, on)) in questions.into_iter().enumerate() {
            let row = switch_row("", heading, desc, on);
            let w = self.weak();
            row.connect_active_notify(move |row| {
                if let Some(w) = w.upgrade() {
                    let mut s = w.state.borrow_mut();
                    match i {
                        0 => s.regular.offline_matches = row.is_active(),
                        1 => s.regular.detailed_stats = row.is_active(),
                        _ => s.regular.remember_names = row.is_active(),
                    }
                }
            });
            list.append(&row);
        }
        page.append(&list);
        self.stack.add_named(&centered(&page), Some("regular"));

        // 5. Advanced: one page per group of switches
        let groups: [(&str, &str, &str, &[&str]); 3] = [
            (
                "adv-matches",
                "Matches and players",
                "What we keep about each match and who played in it.",
                &ADV_MATCHES,
            ),
            (
                "adv-events",
                "Events",
                "Things that happen during a match.",
                &ADV_EVENTS,
            ),
            (
                "adv-habits",
                "Habits",
                "How you play, kept as averages per match (no raw data).",
                &ADV_HABITS,
            ),
        ];
        for (name, heading, desc, switches) in groups {
            let page = page_box();
            page.append(&title(heading));
            page.append(&subtitle(desc));
            let list = rows_list();
            for switch in switches {
                let on = self.state.borrow().draft.get(switch).unwrap_or(true);
                let row = switch_row(switch, switch_title(switch), switch_subtitle(switch), on);
                let w = self.weak();
                let key = (*switch).to_string();
                row.connect_active_notify(move |row| {
                    if let Some(w) = w.upgrade() {
                        w.state.borrow_mut().draft.set(&key, row.is_active());
                    }
                });
                list.append(&row);
            }
            page.append(&list);
            self.stack.add_named(&centered(&page), Some(name));
        }

        // 6. Finish
        let page = page_box();
        page.append(&title("You're all set"));
        page.append(&subtitle("Here is what we will save."));
        self.summary.set_margin_top(20);
        page.append(&self.summary);
        self.stack.add_named(&centered(&page), Some("finish"));
    }

    fn wire_navigation(self: &Rc<Self>) {
        let w = self.weak();
        self.back.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.go_back();
            }
        });
        let w = self.weak();
        self.next.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.go_next();
            }
        });

        // Number keys pick a card on the two choice pages (1, 2, 3).
        let keys = gtk::EventControllerKey::new();
        let w = self.weak();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(w) = w.upgrade() else {
                return gtk::glib::Propagation::Proceed;
            };
            let index = match key {
                gdk::Key::_1 => 0,
                gdk::Key::_2 => 1,
                gdk::Key::_3 => 2,
                _ => return gtk::glib::Propagation::Proceed,
            };
            match w.step.get() {
                Step::Mode => {
                    let modes = [
                        DisplayMode::Overlay,
                        DisplayMode::TwoScreens,
                        DisplayMode::Both,
                    ];
                    if let Some(mode) = modes.get(index) {
                        w.choose_mode(*mode);
                        return gtk::glib::Propagation::Stop;
                    }
                }
                Step::Kind => {
                    let kinds = [SetupKind::Regular, SetupKind::Advanced];
                    if let Some(kind) = kinds.get(index) {
                        w.choose_kind(*kind);
                        return gtk::glib::Propagation::Stop;
                    }
                }
                _ => {}
            }
            gtk::glib::Propagation::Proceed
        });
        self.root.add_controller(keys);
    }

    fn choose_mode(self: &Rc<Self>, mode: DisplayMode) {
        self.state.borrow_mut().mode = Some(mode);
        self.refresh_nav();
        self.go_next();
    }

    fn choose_kind(self: &Rc<Self>, kind: SetupKind) {
        self.state.borrow_mut().kind = Some(kind);
        self.refresh_nav();
        self.go_next();
    }

    pub fn go_next(self: &Rc<Self>) {
        let at = self.step.get();
        if !self.state.borrow().can_continue(at) {
            return;
        }
        if at == Step::Finish {
            let settings = self.state.borrow().finish();
            (self.on_finish)(settings);
            return;
        }
        let next = self.state.borrow().next(at);
        if let Some(next) = next {
            self.show(next, true);
        }
    }

    pub fn go_back(self: &Rc<Self>) {
        let back = self.state.borrow().back(self.step.get());
        if let Some(back) = back {
            self.show(back, false);
        }
    }

    /// Shows a step. `forward` only decides which way the page slides.
    pub fn show(self: &Rc<Self>, step: Step, forward: bool) {
        self.stack.set_transition_type(if forward {
            gtk::StackTransitionType::SlideLeft
        } else {
            gtk::StackTransitionType::SlideRight
        });
        if step == Step::Finish {
            self.fill_summary();
        }
        self.step.set(step);
        self.stack.set_visible_child_name(name_of(step));
        self.refresh_nav();
        if step == Step::Username {
            self.username.grab_focus();
        }
    }

    /// For screenshots: opens a step directly, with the answers it depends on filled in.
    pub fn jump(self: &Rc<Self>, step: Step) {
        {
            let mut s = self.state.borrow_mut();
            if s.username.trim().is_empty() {
                s.username = "Zed".into();
            }
            s.mode.get_or_insert(DisplayMode::Both);
            s.kind = Some(match step {
                Step::AdvMatches | Step::AdvEvents | Step::AdvHabits => SetupKind::Advanced,
                _ => SetupKind::Regular,
            });
        }
        // Copy the text out first: setting it runs a handler that borrows the state again.
        let name = self.state.borrow().username.clone();
        self.username.set_text(&name);
        self.show(step, true);
    }

    fn fill_summary(&self) {
        while let Some(child) = self.summary.first_child() {
            self.summary.remove(&child);
        }
        for (key, value) in self.state.borrow().summary() {
            self.summary.append(&fact(&key.to_uppercase(), &value));
        }
    }

    /// Updates Back/Next, the highlighted cards and the dots for the current step.
    fn refresh_nav(&self) {
        let state = self.state.borrow();
        let at = self.step.get();
        self.back.set_visible(state.back(at).is_some());
        self.next
            .set_label(if at == Step::Finish { "Finish" } else { "Next" });
        self.next.set_sensitive(state.can_continue(at));
        // The two choice pages move on by themselves when a card is clicked.
        self.next
            .set_visible(!matches!(at, Step::Mode | Step::Kind) || state.can_continue(at));

        for (i, mode) in [
            DisplayMode::Overlay,
            DisplayMode::TwoScreens,
            DisplayMode::Both,
        ]
        .into_iter()
        .enumerate()
        {
            set_selected(&self.mode_cards[i], state.mode == Some(mode));
        }
        for (i, kind) in [SetupKind::Regular, SetupKind::Advanced]
            .into_iter()
            .enumerate()
        {
            set_selected(&self.kind_cards[i], state.kind == Some(kind));
        }

        while let Some(child) = self.dots.first_child() {
            self.dots.remove(&child);
        }
        let (position, total) = state.progress(at);
        for i in 0..total {
            let dot = gtk::Box::builder().css_classes(["dot"]).build();
            if i == position {
                dot.add_css_class("active");
            }
            self.dots.append(&dot);
        }
    }
}

fn set_selected(card: &gtk::Button, selected: bool) {
    if selected {
        card.add_css_class("selected");
    } else {
        card.remove_css_class("selected");
    }
}
