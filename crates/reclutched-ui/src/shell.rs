//! The frame of the app: three tabs at the top (Tracking, Main Menu, Capturing), the tracking
//! light next to them and the settings gear in the corner.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use reclutched_core::rocketleague::storage::Store;

use crate::mainview::MainView;
use crate::menu::MainMenu;
use crate::tracking::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Tracking,
    Menu,
    Capturing,
}

const TABS: [(Tab, &str, &str); 3] = [
    (Tab::Tracking, "tracking", "Tracking"),
    (Tab::Menu, "menu", "Main Menu"),
    (Tab::Capturing, "capturing", "Capturing"),
];

pub struct Shell {
    pub widget: adw::ToolbarView,
    stack: gtk::Stack,
    tabs: Vec<(Tab, gtk::ToggleButton)>,
    light: gtk::Button,
    dot: gtk::Box,
    tracking: Rc<MainView>,
    menu: Rc<MainMenu>,
    /// Set while we switch tabs ourselves, so the buttons do not answer twice.
    switching: Cell<bool>,
}

impl Shell {
    pub fn new(
        window: &adw::ApplicationWindow,
        store: Rc<Store>,
        names_path: Option<PathBuf>,
        settings_path: PathBuf,
        on_settings: Rc<dyn Fn()>,
        on_calendar: Rc<dyn Fn()>,
        on_toggle_tracking: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let tracking = MainView::new(
            window,
            store.clone(),
            names_path.clone(),
            on_settings.clone(),
        );
        let menu = MainMenu::new(store, settings_path, names_path, on_calendar);

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(160)
            .build();
        stack.add_named(&tracking.widget, Some("tracking"));
        stack.add_named(&menu.widget, Some("menu"));
        let capturing = adw::StatusPage::builder()
            .icon_name("camera-video-symbolic")
            .title("Capturing")
            .description("This page is coming soon. Screenshots already work from Settings.")
            .build();
        stack.add_named(&capturing, Some("capturing"));

        // --- the top bar ---
        let dot = gtk::Box::builder()
            .css_classes(["status-dot", "off"])
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .width_request(10)
            .height_request(10)
            .build();
        let light = gtk::Button::builder()
            .child(&dot)
            .css_classes(["flat", "status-button"])
            .valign(gtk::Align::Center)
            .build();
        light.connect_clicked(move |_| on_toggle_tracking());

        let tab_box = gtk::Box::builder()
            .css_classes(["linked", "top-tabs"])
            .build();
        let mut tabs = Vec::new();
        let mut first: Option<gtk::ToggleButton> = None;
        for (tab, _, title) in TABS {
            let button = gtk::ToggleButton::builder().label(title).build();
            match &first {
                Some(group) => button.set_group(Some(group)),
                None => first = Some(button.clone()),
            }
            tab_box.append(&button);
            tabs.push((tab, button));
        }
        let title_row = gtk::Box::builder().spacing(10).build();
        title_row.append(&light);
        title_row.append(&tab_box);

        let gear = gtk::Button::builder()
            .icon_name("emblem-system-symbolic")
            .tooltip_text("Settings")
            .css_classes(["flat"])
            .build();
        gear.connect_clicked(move |_| on_settings());

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title_row));
        header.pack_end(&gear);
        let widget = adw::ToolbarView::new();
        widget.add_top_bar(&header);
        widget.set_content(Some(&stack));

        let this = Rc::new(Self {
            widget,
            stack,
            tabs,
            light,
            dot,
            tracking,
            menu,
            switching: Cell::new(false),
        });
        for (tab, button) in &this.tabs {
            let (weak, tab) = (Rc::downgrade(&this), *tab);
            button.connect_toggled(move |b| {
                if let Some(shell) = weak.upgrade() {
                    if b.is_active() && !shell.switching.get() {
                        shell.show(tab);
                    }
                }
            });
        }
        this.set_status(Status::Off);
        this.show(Tab::Menu);
        this
    }

    /// Switches to a tab.
    pub fn show(&self, tab: Tab) {
        self.switching.set(true);
        for (t, button) in &self.tabs {
            button.set_active(*t == tab);
        }
        self.switching.set(false);
        if let Some((_, name, _)) = TABS.iter().find(|(t, _, _)| *t == tab) {
            self.stack.set_visible_child_name(name);
        }
        if tab == Tab::Menu {
            self.menu.refresh_if_changed();
        }
    }

    /// Updates the tracking light.
    pub fn set_status(&self, status: Status) {
        self.light
            .set_tooltip_text(Some(&format!("{}. {}", status.label(), status.tooltip())));
        for class in ["off", "waiting", "active"] {
            self.dot.remove_css_class(class);
        }
        self.dot.add_css_class(status.css_class());
    }

    /// Reads everything again.
    pub fn reload(&self) {
        self.tracking.reload();
        self.menu.refresh();
    }

    /// Opens a day, month or year on the Tracking tab (used by the calendar).
    pub fn open_period(&self, key: &str) {
        self.show(Tab::Tracking);
        self.tracking.open_period(key);
    }

    /// Developer helper for screenshots: the newest match with this detail tab.
    pub fn show_match_tab(&self, name: &str) {
        self.show(Tab::Tracking);
        self.tracking.select_newest_match();
        self.tracking.show_tab(name);
    }
}
