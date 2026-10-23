//! The main window content: match list on the left, details on the right.
//! It reads the database; `reclutched track` writes it. A small timer notices new matches.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::dates::Day;
use reclutched_core::rocketleague::periods::Grain;
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::storage::Store;

use crate::detail::DetailView;
use crate::format::{mode_name, summary_text, when_label};
use crate::history::{self, Inputs, Target};
use crate::list::HistoryList;
use crate::summary::{Page, SummaryView};
use crate::tracking::Status;

const POLL_SECONDS: u32 = 2;
/// Window width (in pixels) under which the list and the details are shown one at a time.
const NARROW_WIDTH: f64 = 1000.0;

pub struct MainView {
    pub widget: gtk::Stack,
    split: adw::NavigationSplitView,
    store: Rc<Store>,
    names: RefCell<PlaylistMap>,
    names_path: Option<PathBuf>,
    list: Rc<HistoryList>,
    detail: DetailView,
    /// The page for Home, a day, a month or a year.
    summary: SummaryView,
    /// The right side shows either a match ("match") or a period ("period").
    content: gtk::Stack,
    /// The header shows the match tabs, or the name of the period.
    header_title: gtk::Stack,
    period_title: adw::WindowTitle,
    title: adw::WindowTitle,
    light: StatusLight,
    empty_light: StatusLight,
    /// (number of matches, newest match id) at the last reload: if it changes, we reload.
    seen: Cell<(i64, i64)>,
    /// The day it was at the last reload: after midnight "today" is another day.
    seen_day: RefCell<String>,
    selection: RefCell<Target>,
    /// Keys of the months and years that are open in the list.
    expanded: RefCell<HashSet<String>>,
}

/// Your time zone's offset from UTC in seconds, and today's date there.
fn local_now() -> (i64, Day) {
    match gtk::glib::DateTime::now_local() {
        Ok(now) => {
            let offset = now.utc_offset().as_microseconds() / 1_000_000;
            let day = Day::new(now.year(), now.month() as u32, now.day_of_month() as u32);
            (
                offset,
                day.unwrap_or(Day {
                    year: 1970,
                    month: 1,
                    day: 1,
                }),
            )
        }
        Err(_) => (
            0,
            Day {
                year: 1970,
                month: 1,
                day: 1,
            },
        ),
    }
}

fn icon_button(icon: &str, tooltip: &str, on_click: &Rc<dyn Fn()>) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .css_classes(["flat"])
        .build();
    let on_click = on_click.clone();
    button.connect_clicked(move |_| on_click());
    button
}

fn gear_button(on_settings: &Rc<dyn Fn()>) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("emblem-system-symbolic")
        .tooltip_text("Settings")
        .css_classes(["flat"])
        .build();
    let on_settings = on_settings.clone();
    button.connect_clicked(move |_| on_settings());
    button
}

/// A coloured dot and a few words that show whether tracking is running. Clicking toggles it.
struct StatusLight {
    button: gtk::Button,
    dot: gtk::Box,
    label: gtk::Label,
}

impl StatusLight {
    fn new(on_toggle: Rc<dyn Fn()>) -> Self {
        let dot = gtk::Box::builder()
            .css_classes(["status-dot", "off"])
            .valign(gtk::Align::Center)
            .build();
        let label = gtk::Label::builder().css_classes(["status-text"]).build();
        let row = gtk::Box::builder().spacing(8).build();
        row.append(&dot);
        row.append(&label);
        let button = gtk::Button::builder()
            .child(&row)
            .css_classes(["flat", "status-button"])
            .build();
        button.connect_clicked(move |_| on_toggle());
        let light = Self { button, dot, label };
        light.set(Status::Off);
        light
    }

    fn set(&self, status: Status) {
        self.label.set_text(status.label());
        self.button.set_tooltip_text(Some(status.tooltip()));
        for class in ["off", "waiting", "active"] {
            self.dot.remove_css_class(class);
        }
        self.dot.add_css_class(status.css_class());
    }
}

impl MainView {
    pub fn new(
        window: &adw::ApplicationWindow,
        store: Rc<Store>,
        names_path: Option<PathBuf>,
        on_settings: Rc<dyn Fn()>,
        on_calendar: Rc<dyn Fn()>,
        on_graphs: Rc<dyn Fn()>,
        on_toggle_tracking: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let names = names_path
            .as_deref()
            .and_then(|p| PlaylistMap::load(p).ok())
            .unwrap_or_else(PlaylistMap::with_builtin);

        // --- the split view: list | details ---
        let weak_self: Rc<RefCell<Option<std::rc::Weak<MainView>>>> = Rc::new(RefCell::new(None));
        let this_of = |weak: &Rc<RefCell<Option<std::rc::Weak<MainView>>>>| {
            weak.borrow().as_ref().and_then(std::rc::Weak::upgrade)
        };
        let list = {
            let (on_select, on_toggle) = (weak_self.clone(), weak_self.clone());
            HistoryList::new(
                move |target| {
                    if let Some(this) = this_of(&on_select) {
                        this.select(target);
                    }
                },
                move |key| {
                    if let Some(this) = this_of(&on_toggle) {
                        this.toggle_group(&key);
                    }
                },
            )
        };
        let detail = DetailView::new();
        let summary = {
            let on_open = weak_self.clone();
            SummaryView::new(move |target| {
                if let Some(this) = this_of(&on_open) {
                    this.select(target);
                }
            })
        };
        let content = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(120)
            .build();
        content.add_named(&detail.stack, Some("match"));
        content.add_named(&summary.widget, Some("period"));
        content.set_visible_child_name("period");

        let title = adw::WindowTitle::new("Matches", "");
        let light = StatusLight::new(on_toggle_tracking.clone());
        let empty_light = StatusLight::new(on_toggle_tracking);
        let side_header = adw::HeaderBar::new();
        side_header.set_title_widget(Some(&title));
        let side = adw::ToolbarView::new();
        side.add_top_bar(&side_header);
        side.set_content(Some(&list.widget));
        // The tracking light lives under the list, so it never competes with the title.
        let footer = gtk::Box::builder().css_classes(["sidebar-footer"]).build();
        footer.append(&light.button);
        side.add_bottom_bar(&footer);
        let side_page = adw::NavigationPage::builder()
            .title("Matches")
            .child(&side)
            .build();

        let switcher = gtk::StackSwitcher::builder()
            .stack(&detail.stack)
            .css_classes(["tabs"])
            .build();
        let period_title = adw::WindowTitle::new("Today", "");
        let header_title = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        header_title.add_named(&switcher, Some("match"));
        header_title.add_named(&period_title, Some("period"));
        header_title.set_visible_child_name("period");
        let main_header = adw::HeaderBar::new();
        main_header.set_title_widget(Some(&header_title));
        // From the right: settings, the calendar, the graphs.
        main_header.pack_end(&gear_button(&on_settings));
        main_header.pack_end(&icon_button(
            "x-office-calendar-symbolic",
            "Calendar: which days you played",
            &on_calendar,
        ));
        main_header.pack_end(&icon_button(
            "utilities-system-monitor-symbolic",
            "Graphs: how you are improving",
            &on_graphs,
        ));
        let main = adw::ToolbarView::new();
        main.add_top_bar(&main_header);
        main.set_content(Some(&content));
        let main_page = adw::NavigationPage::builder()
            .title("Match")
            .child(&main)
            .build();

        let split = adw::NavigationSplitView::builder()
            .sidebar(&side_page)
            .content(&main_page)
            .min_sidebar_width(290.0)
            .max_sidebar_width(380.0)
            .sidebar_width_fraction(0.33)
            .build();

        // Below this width the window shows one screen at a time (the list, then the details
        // with a back button) instead of both side by side. The side-by-side layout needs about
        // 960 px, so the limit must be above that.
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            NARROW_WIDTH,
            adw::LengthUnit::Px,
        ));
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(narrow);

        // --- the empty state: nothing saved yet ---
        let status = adw::StatusPage::builder()
            .title("No matches yet")
            .description(
                "Play a match with Rocket League running. It appears here when the match ends.\nTracking has to be on (top left), and the game's Stats API enabled.",
            )
            .build();
        let setup_button = gtk::Button::builder()
            .label("Set up Rocket League\u{2026}")
            .css_classes(["pill"])
            .halign(gtk::Align::Center)
            .build();
        {
            let on_settings = on_settings.clone();
            setup_button.connect_clicked(move |_| on_settings());
        }
        status.set_child(Some(&setup_button));
        let empty_header = adw::HeaderBar::new();
        empty_header.pack_start(&empty_light.button);
        empty_header.pack_end(&gear_button(&on_settings));
        let empty = adw::ToolbarView::new();
        empty.add_top_bar(&empty_header);
        empty.set_content(Some(&status));

        let widget = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        widget.add_named(&split, Some("split"));
        widget.add_named(&empty, Some("empty"));

        let this = Rc::new(Self {
            widget,
            split,
            store,
            names: RefCell::new(names),
            names_path,
            list,
            detail,
            summary,
            content,
            header_title,
            period_title,
            title,
            light,
            empty_light,
            seen: Cell::new((-1, -1)),
            seen_day: RefCell::new(String::new()),
            selection: RefCell::new(Target::Home),
            expanded: RefCell::new(HashSet::new()),
        });
        *weak_self.borrow_mut() = Some(Rc::downgrade(&this));
        this.reload();

        // Look for new matches every couple of seconds. The timer stops by itself once the
        // view is gone (the weak pointer no longer upgrades).
        let weak = Rc::downgrade(&this);
        gtk::glib::timeout_add_seconds_local(POLL_SECONDS, move || match weak.upgrade() {
            Some(view) => {
                view.poll();
                gtk::glib::ControlFlow::Continue
            }
            None => gtk::glib::ControlFlow::Break,
        });
        this
    }

    /// Developer helper for screenshots: picks a detail tab by name.
    pub fn show_tab(&self, name: &str) {
        self.detail.stack.set_visible_child_name(name);
    }

    /// Updates the status light (in both headers).
    pub fn set_status(&self, status: Status) {
        self.light.set(status);
        self.empty_light.set(status);
    }

    fn state(&self) -> (i64, i64) {
        let count = self.store.abandon_counts().map_or(0, |(all, _)| all);
        let newest = self.store.latest_match_id().ok().flatten().unwrap_or(0);
        (count, newest)
    }

    fn poll(&self) {
        let (_, today) = local_now();
        if self.state() != self.seen.get() || today.key() != *self.seen_day.borrow() {
            self.reload();
        }
    }

    /// Reads the database again, rebuilds the list and shows the current selection again.
    pub fn reload(&self) {
        if let Some(path) = &self.names_path {
            if let Ok(fresh) = PlaylistMap::load(path) {
                *self.names.borrow_mut() = fresh;
            }
        }
        self.seen.set(self.state());
        let (wins, losses, abandoned) = self.store.result_counts().unwrap_or((0, 0, 0));
        self.title
            .set_subtitle(&summary_text(wins, losses, abandoned));

        if wins + losses + abandoned == 0 {
            self.widget.set_visible_child_name("empty");
            *self.selection.borrow_mut() = Target::Home;
            return;
        }
        self.widget.set_visible_child_name("split");
        self.rebuild_list();
        let target = self.selection.borrow().clone();
        self.show_target(&target);
    }

    /// Builds the left list from the database.
    fn rebuild_list(&self) {
        let (offset, today) = local_now();
        *self.seen_day.borrow_mut() = today.key();
        let today_matches = self
            .store
            .matches_in_period(&today.key(), offset)
            .unwrap_or_default();
        let days = self
            .store
            .period_summaries(Grain::Day, offset)
            .unwrap_or_default();
        let months = self
            .store
            .period_summaries(Grain::Month, offset)
            .unwrap_or_default();
        let years = self
            .store
            .period_summaries(Grain::Year, offset)
            .unwrap_or_default();
        let names = self.names.borrow();
        let expanded = self.expanded.borrow();
        let entries = history::build_entries(&Inputs {
            today,
            today_matches: &today_matches,
            days: &days,
            months: &months,
            years: &years,
            expanded: &expanded,
            mode_name: &|id| mode_name(&names, id),
            when: &when_label,
        });
        drop((names, expanded));
        self.list.set_entries(entries, &self.selection.borrow());
    }

    /// A row was picked (or opened from inside a page).
    fn select(&self, target: Target) {
        *self.selection.borrow_mut() = target.clone();
        // Keep the list's highlight in step when we got here from a page instead of the list.
        self.rebuild_list();
        self.show_target(&target);
        // In the narrow layout the list and the page are two screens; a click goes to the page.
        self.split.set_show_content(true);
    }

    /// Opens a day, month or year (used by the calendar).
    #[allow(dead_code)] // the calendar page opens days with this (next step)
    pub fn open_period(&self, key: &str) {
        self.select(Target::Period(key.to_string()));
    }

    fn toggle_group(&self, key: &str) {
        {
            let mut open = self.expanded.borrow_mut();
            if !open.remove(key) {
                open.insert(key.to_string());
            }
        }
        self.rebuild_list();
    }

    fn show_target(&self, target: &Target) {
        match target {
            Target::Match(id) => self.show_match(*id),
            Target::Home => {
                let (_, today) = local_now();
                self.show_period(&today.key(), true);
            }
            Target::Period(key) => self.show_period(key, false),
        }
    }

    fn show_match(&self, id: i64) {
        let detail = self.store.match_detail(id).ok().flatten();
        self.detail.set(detail.as_ref(), &self.names.borrow());
        self.content.set_visible_child_name("match");
        self.header_title.set_visible_child_name("match");
    }

    /// The page of a day, month or year; for Home it is today.
    fn show_period(&self, key: &str, home: bool) {
        let (offset, today) = local_now();
        let summary = self.store.period_summary(key, offset).ok().flatten();
        let names = self.names.borrow();
        let matches = if key.len() == 10 {
            self.store
                .matches_in_period(key, offset)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let days = if key.len() == 7 {
            self.store
                .period_summaries(Grain::Day, offset)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let months = if key.len() == 4 {
            self.store
                .period_summaries(Grain::Month, offset)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let children = history::children_of(
            key,
            today,
            &matches,
            &days,
            &months,
            &|id| mode_name(&names, id),
            &when_label,
        );
        drop(names);
        let heading = if home {
            "Today".to_string()
        } else {
            history::period_title(key, today)
        };
        let page = Page {
            heading: heading.clone(),
            subtitle: history::period_subtitle(key, children.len()),
            summary,
            empty_text: if home {
                "Nothing played today yet. Start a match and it shows up here.".to_string()
            } else {
                "Nothing was played then.".to_string()
            },
            children_title: match key.len() {
                10 => "Matches",
                7 => "Days",
                _ => "Months",
            }
            .to_string(),
            children,
        };
        self.summary.set(&page);
        self.period_title.set_title(&heading);
        self.content.set_visible_child_name("period");
        self.header_title.set_visible_child_name("period");
    }
}
