//! The main window content: match list on the left, details on the right.
//! It reads the database; `reclutched track` writes it. A small timer notices new matches.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::storage::Store;

use crate::detail::DetailView;
use crate::format::summary_text;
use crate::list::MatchList;
use crate::tracking::Status;

/// How many matches the list loads. Plenty; each row is tiny.
const LIST_LIMIT: u32 = 500;
const POLL_SECONDS: u32 = 2;

pub struct MainView {
    pub widget: gtk::Stack,
    store: Rc<Store>,
    names: RefCell<PlaylistMap>,
    names_path: Option<PathBuf>,
    list: Rc<MatchList>,
    detail: DetailView,
    title: adw::WindowTitle,
    light: StatusLight,
    empty_light: StatusLight,
    /// (number of matches, newest match id) at the last reload: if it changes, we reload.
    seen: Cell<(i64, i64)>,
    selected: Cell<Option<i64>>,
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
        store: Rc<Store>,
        names_path: Option<PathBuf>,
        on_settings: Rc<dyn Fn()>,
        on_toggle_tracking: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let names = names_path
            .as_deref()
            .and_then(|p| PlaylistMap::load(p).ok())
            .unwrap_or_else(PlaylistMap::with_builtin);

        // --- the split view: list | details ---
        let weak_self: Rc<RefCell<Option<std::rc::Weak<MainView>>>> = Rc::new(RefCell::new(None));
        let list = {
            let weak_self = weak_self.clone();
            MatchList::new(move |id| {
                if let Some(this) = weak_self.borrow().as_ref().and_then(std::rc::Weak::upgrade) {
                    this.show_match(id);
                }
            })
        };
        let detail = DetailView::new();

        let title = adw::WindowTitle::new("Matches", "");
        let light = StatusLight::new(on_toggle_tracking.clone());
        let empty_light = StatusLight::new(on_toggle_tracking);
        let side_header = adw::HeaderBar::new();
        side_header.set_title_widget(Some(&title));
        side_header.pack_start(&light.button);
        let side = adw::ToolbarView::new();
        side.add_top_bar(&side_header);
        side.set_content(Some(&list.widget));
        let side_page = adw::NavigationPage::builder()
            .title("Matches")
            .child(&side)
            .build();

        let switcher = gtk::StackSwitcher::builder()
            .stack(&detail.stack)
            .css_classes(["tabs"])
            .build();
        let main_header = adw::HeaderBar::new();
        main_header.set_title_widget(Some(&switcher));
        main_header.pack_end(&gear_button(&on_settings));
        let main = adw::ToolbarView::new();
        main.add_top_bar(&main_header);
        main.set_content(Some(&detail.stack));
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

        // --- the empty state: nothing saved yet ---
        let status = adw::StatusPage::builder()
            .title("No matches yet")
            .description(
                "Play a match with Rocket League running. It appears here when the match ends.\nTracking has to be on (top left), and the game's Stats API enabled.",
            )
            .build();
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
            store,
            names: RefCell::new(names),
            names_path,
            list,
            detail,
            title,
            light,
            empty_light,
            seen: Cell::new((-1, -1)),
            selected: Cell::new(None),
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
        if self.state() != self.seen.get() {
            self.reload();
        }
    }

    /// Reads the database again and rebuilds the list, keeping your selection when possible.
    pub fn reload(&self) {
        if let Some(path) = &self.names_path {
            if let Ok(fresh) = PlaylistMap::load(path) {
                *self.names.borrow_mut() = fresh;
            }
        }
        self.seen.set(self.state());
        let rows = self.store.recent_matches(LIST_LIMIT).unwrap_or_default();
        let (wins, losses, abandoned) = self.store.result_counts().unwrap_or((0, 0, 0));
        self.title
            .set_subtitle(&summary_text(wins, losses, abandoned));

        if rows.is_empty() {
            self.widget.set_visible_child_name("empty");
            self.selected.set(None);
            return;
        }
        self.widget.set_visible_child_name("split");
        let names = self.names.borrow();
        let id = self.list.set_rows(&rows, &names, self.selected.get());
        drop(names);
        match id {
            Some(id) => self.show_match(id),
            None => self.selected.set(None),
        }
    }

    fn show_match(&self, id: i64) {
        self.selected.set(Some(id));
        let detail = self.store.match_detail(id).ok().flatten();
        self.detail.set(detail.as_ref(), &self.names.borrow());
    }
}
