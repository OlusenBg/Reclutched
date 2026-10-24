//! Reclutched desktop app.
//!
//! First launch: a loading screen, then the setup wizard. After that the main window with your
//! match history, the details of each match and the settings.

mod calendar;
mod capture;
mod carousel;
mod detail;
mod flow;
mod focus;
mod format;
mod gallery;
mod history;
mod hotkeys;
mod insights;
mod keys;
mod list;
mod live;
mod mainview;
mod menu;
mod notify;
mod overlay;
mod panel;
mod recording;
mod settings_page;
mod setup;
mod shell;
mod splash;
mod startup;
mod summary;
mod tracking;
mod trends;
mod triggers;
mod widgets;
mod wipe;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use clap::Parser;
use reclutched_core::rocketleague::playlist::default_map_path;
use reclutched_core::rocketleague::settings::{DisplayMode, Settings};
use reclutched_core::rocketleague::storage::MovementAverages;
use reclutched_core::rocketleague::storage::{default_db_path, Store};
use reclutched_core::rocketleague::tracker::LiveSnapshot;
use reclutched_daemon::track::{Paths, TrackEvent};

use flow::Step;

const FIRST_LAUNCH_SPLASH_MS: u64 = 3000;
const LATER_LAUNCH_SPLASH_MS: u64 = 1000;

#[derive(Parser)]
#[command(name = "reclutched-ui", version, about = "Reclutched desktop app")]
struct Args {
    /// Settings file (default: ~/.config/reclutched/settings.json).
    #[arg(long)]
    settings: Option<PathBuf>,
    /// Match database (default: ~/.local/share/reclutched/matches.db).
    #[arg(long)]
    db: Option<PathBuf>,
    /// Address of Rocket League's Stats API websocket.
    #[arg(long, default_value = "127.0.0.1:49124")]
    addr: String,
    /// Run the first-launch setup again, even if it was finished.
    #[arg(long)]
    reset_setup: bool,
    /// Developer option: how long the loading screen lasts, in milliseconds.
    #[arg(long, hide = true)]
    splash_ms: Option<u64>,
    /// Developer option: window size as WIDTHxHEIGHT, e.g. 640x520 (for screenshots).
    #[arg(long, hide = true, value_parser = parse_size)]
    size: Option<(i32, i32)>,
    /// Developer option: open one page directly (for screenshots).
    #[arg(long, hide = true)]
    page: Option<String>,
    /// Developer option: act as if the overlay key were held down (for screenshots).
    #[arg(long, hide = true)]
    hold_key: bool,
    /// Start with the window minimized (tracking and the overlay still run).
    #[arg(long)]
    minimized: bool,
    /// Developer option: capture a moving test picture instead of the screen.
    #[arg(long, hide = true)]
    capture_test: bool,
    /// Developer option: show an example pop-up (win, loss or left) a moment after the start.
    #[arg(long, hide = true)]
    popup_demo: Option<String>,
    /// Developer option: save a clip this many seconds after the main page opens.
    #[arg(long, hide = true)]
    capture_clip_after: Option<u64>,
    /// Developer option: take a screenshot a few seconds after the main page opens.
    #[arg(long, hide = true)]
    capture_shot_at_start: bool,
}

fn parse_size(text: &str) -> Result<(i32, i32), String> {
    let (w, h) = text
        .split_once('x')
        .ok_or("use WIDTHxHEIGHT, e.g. 640x520")?;
    Ok((
        w.parse().map_err(|_| "bad width")?,
        h.parse().map_err(|_| "bad height")?,
    ))
}

/// Everything the window needs to move between its screens.
struct App {
    window: adw::ApplicationWindow,
    pages: gtk::Stack,
    toasts: adw::ToastOverlay,
    settings_path: PathBuf,
    db_path: PathBuf,
    addr: String,
    /// The tracker, while it is running.
    tracking: RefCell<Option<tracking::Tracking>>,
    /// Keep the current screens alive while they are shown.
    wizard: RefCell<Option<Rc<setup::Wizard>>>,
    main_view: RefCell<Option<Rc<shell::Shell>>>,
    /// True once the timer that reads tracker news is running.
    watching: Cell<bool>,
    /// The live window (modes "2 Screens" and "Both"), while a match is shown.
    live: RefCell<Option<Rc<live::LiveWindow>>>,
    /// The newest match whose live window the user closed; it is not opened again for it.
    live_dismissed: Cell<u64>,
    /// Is the game window focused? (Hyprland only for now.)
    focus: focus::GameFocus,
    /// The in-game overlay (modes "Overlay" and "Both"), once it has been created.
    overlay: RefCell<Option<Rc<overlay::Overlay>>>,
    /// We already tried to create it (so a desktop without layer-shell is only tried once).
    overlay_tried: Cell<bool>,
    overlay_wanted: Cell<bool>,
    /// What the status light shows now; the overlay's pill follows it.
    status: Cell<tracking::Status>,
    ticks: Cell<u32>,
    /// Watches the overlay key (hold to show the panel); started when the overlay is wanted.
    keys: RefCell<Option<keys::KeyWatcher>>,
    key_held: Cell<bool>,
    /// The controller button is held (either one shows the overlay).
    pad_held: Cell<bool>,
    key_hint_shown: Cell<bool>,
    /// Waiting for a controller button press from the settings page.
    capture_cb: RefCell<Option<settings_page::PadCallback>>,
    capture_gen: Cell<u32>,
    keys_timer_started: Cell<bool>,
    /// The match in progress, for the overlay panel.
    latest_live: RefCell<Option<LiveSnapshot>>,
    /// Your usual numbers, loaded when the panel needs them (and again after a match is saved).
    usual: RefCell<Option<MovementAverages>>,
    /// Your recent matches, for the panel between matches (loaded together with `usual`).
    recent: RefCell<Option<panel::Recent>>,
    usual_loaded: Cell<bool>,
    /// Screenshots (and later clips): the screen-share session and what it is doing.
    screen: capture::Capture,
    /// Recording, clips and the capture keys.
    rec: recording::RecState,
    shot_at_start: Cell<bool>,
    clip_after: Cell<Option<u64>>,
    popup_demo: RefCell<Option<String>>,
    /// The pop-up for saved screenshots, clips and matches (made when first needed).
    notifier: RefCell<Option<Rc<notify::Notifier>>>,
}

/// Wraps a page with a flat, empty header bar, so the window can be moved and closed
/// on desktops that draw their own window decorations (GNOME).
fn with_chrome(content: &gtk::Widget) -> gtk::Widget {
    let header = adw::HeaderBar::builder().css_classes(["flat"]).build();
    header.set_show_title(false);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(content));
    view.upcast()
}

impl App {
    fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Replaces the page with this name.
    fn set_page(&self, name: &str, widget: &gtk::Widget) {
        if let Some(old) = self.pages.child_by_name(name) {
            self.pages.remove(&old);
        }
        self.pages.add_named(widget, Some(name));
    }

    /// The main window: your matches. Needs the database.
    fn show_main(self: &Rc<Self>) {
        let store = match Store::open(&self.db_path) {
            Ok(store) => Rc::new(store),
            Err(err) => {
                let status = adw::StatusPage::builder()
                    .title("Could not open the match database")
                    .description(format!("{}\n{err}", self.db_path.display()))
                    .build();
                self.set_page("error", &with_chrome(status.upcast_ref()));
                self.pages.set_visible_child_name("error");
                return;
            }
        };
        let app = Rc::downgrade(self);
        let on_settings: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(app) = app.upgrade() {
                app.open_settings();
            }
        });
        let calendar = Rc::downgrade(self);
        let on_calendar: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(app) = calendar.upgrade() {
                app.open_calendar();
            }
        });
        let toggle = Rc::downgrade(self);
        let on_toggle: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(app) = toggle.upgrade() {
                app.toggle_tracking();
            }
        });
        let view = shell::Shell::new(
            &self.window,
            store,
            default_map_path(),
            self.settings_path.clone(),
            on_settings,
            on_calendar,
            on_toggle,
        );
        self.set_page("main", view.widget.upcast_ref());
        self.pages.set_visible_child_name("main");
        *self.main_view.borrow_mut() = Some(view);

        // Track by itself, unless the user turned that off.
        let wanted = Settings::load(&self.settings_path)
            .unwrap_or_default()
            .auto_track;
        if wanted && self.tracking.borrow().is_none() {
            self.start_tracking();
        } else {
            self.set_status(tracking::Status::Off);
        }
        self.watch_tracking();
        self.start_capture_if_wanted();
    }

    fn set_status(&self, status: tracking::Status) {
        self.status.set(status);
        if let Some(view) = self.main_view.borrow().as_ref() {
            view.set_status(status);
        }
    }

    fn start_tracking(&self) {
        let paths = Paths {
            db: self.db_path.clone(),
            settings: self.settings_path.clone(),
            playlist_map: default_map_path(),
        };
        *self.tracking.borrow_mut() = Some(tracking::Tracking::start(self.addr.clone(), paths));
        self.set_status(tracking::Status::Waiting);
    }

    fn stop_tracking(&self) {
        // Take it out first, so nothing can touch it while it saves and shuts down.
        let running = self.tracking.borrow_mut().take();
        if let Some(mut running) = running {
            running.stop();
        }
        self.close_live();
        self.set_status(tracking::Status::Off);
    }

    fn toggle_tracking(&self) {
        if self.tracking.borrow().is_some() {
            self.stop_tracking();
        } else {
            self.start_tracking();
        }
    }

    /// Checks the tracker for news twice a second and shows it. Runs for as long as the app lives.
    fn watch_tracking(self: &Rc<Self>) {
        if self.watching.replace(true) {
            return;
        }
        let app = Rc::downgrade(self);
        gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(app) = app.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            app.apply_updates();
            app.update_overlay();
            gtk::glib::ControlFlow::Continue
        });
    }

    fn apply_updates(self: &Rc<Self>) {
        let updates = match self.tracking.borrow().as_ref() {
            Some(t) => t.drain(),
            None => return,
        };
        let mut ended = None;
        // Only the newest live picture matters.
        let mut live_news: Option<Option<Box<LiveSnapshot>>> = None;
        let mut saved_match: Option<(i64, String)> = None;
        for update in updates {
            match update {
                tracking::Update::Event(TrackEvent::Connected) => {
                    self.set_status(tracking::Status::Active);
                }
                tracking::Update::Event(TrackEvent::Waiting) => {
                    self.set_status(tracking::Status::Waiting);
                }
                tracking::Update::Event(TrackEvent::MatchSaved { id, summary }) => {
                    self.usual_loaded.set(false);
                    saved_match = Some((id, summary));
                    if let Some(view) = self.main_view.borrow().as_ref() {
                        view.reload();
                    }
                }
                tracking::Update::Event(TrackEvent::PlayerIdLearned) => {}
                tracking::Update::Event(TrackEvent::Live(snapshot)) => live_news = Some(snapshot),
                tracking::Update::Event(TrackEvent::Error(message)) => self.toast(&message),
                tracking::Update::Ended(reason) => ended = Some(reason),
            }
        }
        if let Some(snapshot) = live_news {
            *self.latest_live.borrow_mut() = snapshot.as_deref().cloned();
            self.on_live(snapshot.as_deref());
            self.show_live(snapshot);
        }
        if let Some((id, summary)) = saved_match {
            self.on_match_saved(id, &summary);
        }
        // The tracker stopped on its own (for example it could not open the database).
        if let Some(reason) = ended {
            self.close_live();
            self.tracking.borrow_mut().take();
            self.set_status(tracking::Status::Off);
            if let Some(reason) = reason {
                self.toast(&format!("Tracking stopped: {reason}"));
            }
        }
    }

    /// Opens the live window by itself when a match starts (modes "2 Screens" and "Both"),
    /// and keeps it up to date. Closing it hides it for the rest of that match.
    fn show_live(self: &Rc<Self>, snapshot: Option<Box<LiveSnapshot>>) {
        let mode = Settings::load(&self.settings_path)
            .ok()
            .and_then(|s| s.display_mode);
        let wanted = matches!(mode, Some(DisplayMode::TwoScreens | DisplayMode::Both));
        let existing = self.live.borrow().clone();
        match snapshot {
            Some(snapshot) if wanted => {
                let window = match existing {
                    Some(window) => window,
                    None => {
                        if snapshot.seq <= self.live_dismissed.get() {
                            return;
                        }
                        let Some(application) = self
                            .window
                            .application()
                            .and_then(|a| a.downcast::<adw::Application>().ok())
                        else {
                            return;
                        };
                        let weak = Rc::downgrade(self);
                        let window = live::LiveWindow::new(&application, move |seq| {
                            if let Some(app) = weak.upgrade() {
                                app.live_dismissed.set(seq);
                                app.live.borrow_mut().take();
                            }
                        });
                        *self.live.borrow_mut() = Some(window.clone());
                        window.present();
                        window
                    }
                };
                window.update(&snapshot);
            }
            Some(_) => {}
            None => {
                if let Some(window) = existing {
                    window.ended();
                }
            }
        }
    }

    /// Creates the overlay when the chosen mode wants it and keeps its pill up to date.
    fn update_overlay(self: &Rc<Self>) {
        let ticks = self.ticks.get();
        self.ticks.set(ticks.wrapping_add(1));
        // The mode can change in the settings; look about every two seconds.
        if ticks.is_multiple_of(8) {
            let settings = Settings::load(&self.settings_path).unwrap_or_default();
            self.overlay_wanted.set(matches!(
                settings.display_mode,
                Some(DisplayMode::Overlay | DisplayMode::Both)
            ));
            self.sync_key_watcher(&settings);
            if let Some(overlay) = self.overlay.borrow().as_ref() {
                overlay.apply_look(&settings);
            }
        }
        if !self.overlay_wanted.get() {
            self.close_overlay();
            return;
        }
        let existing = self.overlay.borrow().clone();
        let overlay = match existing {
            Some(overlay) => overlay,
            None => {
                if self.overlay_tried.replace(true) {
                    return;
                }
                let Some(application) = self
                    .window
                    .application()
                    .and_then(|a| a.downcast::<adw::Application>().ok())
                else {
                    return;
                };
                let Some(created) = overlay::Overlay::new(&application) else {
                    eprintln!(
                        "The overlay needs a Wayland desktop with layer-shell (Hyprland, Sway, KDE); it is off here."
                    );
                    return;
                };
                created.apply_look(&Settings::load(&self.settings_path).unwrap_or_default());
                *self.overlay.borrow_mut() = Some(created.clone());
                created
            }
        };
        overlay.set_pill(overlay::pill_state(
            self.status.get(),
            self.focus.game_focused(),
        ));
        self.refresh_panel();
    }

    /// Makes sure the key and controller watcher runs (and its 25-times-a-second check).
    fn ensure_key_watcher(self: &Rc<Self>) {
        if self.keys.borrow().is_none() {
            let settings = Settings::load(&self.settings_path).unwrap_or_default();
            *self.keys.borrow_mut() = Some(keys::KeyWatcher::start(
                &settings.overlay_key,
                &settings.overlay_button,
            ));
        }
        if !self.keys_timer_started.replace(true) {
            self.watch_keys();
        }
    }

    /// Follows the saved key and button, and, once, says how to get the permission when we are
    /// not allowed to read the devices.
    fn sync_key_watcher(self: &Rc<Self>, settings: &Settings) {
        if !self.overlay_wanted.get() {
            return;
        }
        self.ensure_key_watcher();
        if let Some(watcher) = self.keys.borrow().as_ref() {
            watcher.set_bindings(&settings.overlay_key, &settings.overlay_button);
        }
        let denied = self
            .keys
            .borrow()
            .as_ref()
            .is_some_and(|k| k.access() == keys::KeyAccess::NoPermission);
        if denied && !self.key_hint_shown.replace(true) {
            self.toast(
                "The overlay key and controller button need permission to read your devices: run \"sudo usermod -aG input $USER\", then log out and back in.",
            );
        }
    }

    /// Checks the key and the controller about 25 times a second, so the panel follows your finger.
    fn watch_keys(self: &Rc<Self>) {
        let app = Rc::downgrade(self);
        gtk::glib::timeout_add_local(Duration::from_millis(40), move || {
            let Some(app) = app.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            let events = match app.keys.borrow().as_ref() {
                Some(keys) => keys.drain(),
                None => Vec::new(),
            };
            let mut changed = false;
            for event in events {
                match event {
                    keys::KeyEvent::Pressed(keys::Source::Keyboard) => app.key_held.set(true),
                    keys::KeyEvent::Released(keys::Source::Keyboard) => app.key_held.set(false),
                    keys::KeyEvent::Pressed(keys::Source::Controller) => app.pad_held.set(true),
                    keys::KeyEvent::Released(keys::Source::Controller) => app.pad_held.set(false),
                    keys::KeyEvent::Captured(code) => {
                        let done = app.capture_cb.borrow_mut().take();
                        if let Some(done) = done {
                            done(Some(code));
                        }
                    }
                }
                changed = true;
            }
            if changed {
                app.refresh_panel();
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    /// Says that something was saved: the pop-up over everything, or a toast when the desktop
    /// cannot draw one (or the pop-up is switched off in Settings, then nothing at all).
    fn announce(&self, popup: &notify::Popup) {
        let settings = Settings::load(&self.settings_path).unwrap_or_default();
        if !settings.notify_enabled {
            return;
        }
        if !notify::Notifier::available() {
            self.toast(&format!("{}: {}", popup.title, popup.subtitle));
            return;
        }
        let notifier = self.notifier.borrow().clone();
        let notifier = match notifier {
            Some(n) => n,
            None => {
                let Some(app) = self
                    .window
                    .application()
                    .and_then(|a| a.downcast::<adw::Application>().ok())
                else {
                    return;
                };
                let n = notify::Notifier::new(app);
                *self.notifier.borrow_mut() = Some(n.clone());
                n
            }
        };
        notifier.show(popup, &settings);
    }

    /// The pop-up for the match that was just saved (from the newest match in the database).
    fn match_popup(&self) -> Option<notify::Popup> {
        let row = Store::open(&self.db_path)
            .ok()?
            .recent_matches(1)
            .ok()?
            .into_iter()
            .next()?;
        let names = default_map_path()
            .and_then(|p| reclutched_core::rocketleague::playlist::PlaylistMap::load(&p).ok())
            .unwrap_or_else(reclutched_core::rocketleague::playlist::PlaylistMap::with_builtin);
        let outcome = match row.result.as_str() {
            "win" => notify::Outcome::Win,
            "loss" => notify::Outcome::Loss,
            _ => notify::Outcome::LeftEarly,
        };
        Some(notify::Popup::saved_match(
            &format::mode_name(&names, row.playlist_id),
            &format::score_text(&row),
            outcome,
        ))
    }

    /// Turns capturing on or off. On: the desktop may ask what to share.
    fn turn_capture(self: &Rc<Self>, on: bool) {
        if on {
            self.screen.turn_on();
            self.watch_capture();
        } else {
            self.screen.turn_off();
        }
    }

    /// Starts capturing at launch when it was left on, and the screenshot key watcher.
    fn start_capture_if_wanted(self: &Rc<Self>) {
        let settings = Settings::load(&self.settings_path).unwrap_or_default();
        if settings.capture_enabled {
            self.turn_capture(true);
        }
        if let Some(which) = self.popup_demo.take() {
            let app = Rc::downgrade(self);
            gtk::glib::timeout_add_local_once(Duration::from_secs(2), move || {
                let Some(app) = app.upgrade() else { return };
                let outcome = match which.as_str() {
                    "loss" => notify::Outcome::Loss,
                    "left" => notify::Outcome::LeftEarly,
                    _ => notify::Outcome::Win,
                };
                app.announce(&notify::Popup::saved_match(
                    "Casual Duel",
                    "4 \u{2013} 2",
                    outcome,
                ));
            });
        }
        if let Some(seconds) = self.clip_after.take() {
            let app = Rc::downgrade(self);
            gtk::glib::timeout_add_local_once(Duration::from_secs(seconds), move || {
                if let Some(app) = app.upgrade() {
                    app.make_clip(triggers::Trigger::Key);
                }
            });
        }
        if self.shot_at_start.replace(false) {
            let app = Rc::downgrade(self);
            gtk::glib::timeout_add_local_once(Duration::from_secs(2), move || {
                if let Some(app) = app.upgrade() {
                    app.take_screenshot();
                }
            });
        }
    }

    /// For the settings page: listens for the next controller button (up to 12 seconds).
    fn capture_pad(self: &Rc<Self>, done: settings_page::PadCallback) {
        self.ensure_key_watcher();
        *self.capture_cb.borrow_mut() = Some(done);
        if let Some(watcher) = self.keys.borrow().as_ref() {
            watcher.capture_button();
        }
        let generation = self.capture_gen.get().wrapping_add(1);
        self.capture_gen.set(generation);
        let app = Rc::downgrade(self);
        gtk::glib::timeout_add_local_once(Duration::from_secs(12), move || {
            let Some(app) = app.upgrade() else { return };
            if app.capture_gen.get() != generation {
                return;
            }
            let done = app.capture_cb.borrow_mut().take();
            if let Some(done) = done {
                if let Some(watcher) = app.keys.borrow().as_ref() {
                    watcher.cancel_capture();
                }
                done(None);
            }
        });
    }

    fn cancel_pad_capture(&self) {
        self.capture_gen.set(self.capture_gen.get().wrapping_add(1));
        self.capture_cb.borrow_mut().take();
        if let Some(watcher) = self.keys.borrow().as_ref() {
            watcher.cancel_capture();
        }
    }

    /// The devices we read right now, when the watcher runs.
    fn key_devices(&self) -> Option<(keys::Devices, keys::KeyAccess)> {
        self.keys
            .borrow()
            .as_ref()
            .map(|watcher| (watcher.devices(), watcher.access()))
    }

    /// Shows the panel while the key is held and the game has the focus, otherwise hides it.
    fn refresh_panel(&self) {
        let overlay = self.overlay.borrow().clone();
        let Some(overlay) = overlay else { return };
        let game_here = match self.focus.game_focused() {
            Some(focused) => focused,
            // Without focus information: the game being connected is the best sign we have.
            None => self.status.get() == tracking::Status::Active,
        };
        let held = self.key_held.get() || self.pad_held.get();
        if !(held && game_here && self.overlay_wanted.get()) {
            overlay.hide_panel();
            return;
        }
        if !self.usual_loaded.replace(true) {
            let store = Store::open(&self.db_path).ok();
            *self.usual.borrow_mut() = store
                .as_ref()
                .and_then(|store| store.my_movement_averages(30).ok().flatten());
            *self.recent.borrow_mut() = store.as_ref().and_then(|store| {
                let form = store.recent_form(20).ok().flatten()?;
                let matches = store.recent_matches(20).ok()?;
                Some(panel::Recent { matches, form })
            });
        }
        overlay.show_panel(
            self.latest_live.borrow().as_ref(),
            self.usual.borrow().as_ref(),
            self.recent.borrow().as_ref(),
        );
    }

    fn close_overlay(&self) {
        self.overlay_tried.set(false);
        let overlay = self.overlay.borrow_mut().take();
        if let Some(overlay) = overlay {
            overlay.close();
        }
    }

    fn close_live(&self) {
        let window = self.live.borrow_mut().take();
        if let Some(window) = window {
            window.close();
        }
    }

    /// The calendar page; clicking a played day opens it in your matches.
    fn open_calendar(self: &Rc<Self>) {
        let (offset, today) = mainview::local_now();
        let weak = Rc::downgrade(self);
        let hooks = calendar::Hooks {
            on_back: {
                let weak = weak.clone();
                Rc::new(move || {
                    if let Some(app) = weak.upgrade() {
                        app.pages.set_visible_child_name("main");
                    }
                })
            },
            on_open: Rc::new(move |key: &str| {
                if let Some(app) = weak.upgrade() {
                    app.pages.set_visible_child_name("main");
                    if let Some(view) = app.main_view.borrow().as_ref() {
                        view.open_period(key);
                    }
                }
            }),
        };
        let page = calendar::build(self.db_path.clone(), offset, today, hooks);
        self.set_page("calendar", &page);
        self.pages.set_visible_child_name("calendar");
    }

    /// Shows the settings as a page inside the window (the back arrow returns to your matches).
    fn open_settings(self: &Rc<Self>) {
        self.open_settings_at(None);
    }

    /// Same, with one section selected (`None`: the first).
    fn open_settings_at(self: &Rc<Self>, section: Option<&str>) {
        let weak = Rc::downgrade(self);
        let hooks = settings_page::Hooks {
            on_back: {
                let weak = weak.clone();
                Rc::new(move || {
                    if let Some(app) = weak.upgrade() {
                        app.cancel_pad_capture();
                        app.pages.set_visible_child_name("main");
                    }
                })
            },
            on_setup: {
                let weak = weak.clone();
                Rc::new(move || {
                    if let Some(app) = weak.upgrade() {
                        app.show_setup(None);
                    }
                })
            },
            toast: {
                let weak = weak.clone();
                Rc::new(move |message: &str| {
                    if let Some(app) = weak.upgrade() {
                        app.toast(message);
                    }
                })
            },
            capture_button: {
                let weak = weak.clone();
                Rc::new(move |done| {
                    if let Some(app) = weak.upgrade() {
                        app.capture_pad(done);
                    }
                })
            },
            cancel_capture: {
                let weak = weak.clone();
                Rc::new(move || {
                    if let Some(app) = weak.upgrade() {
                        app.cancel_pad_capture();
                    }
                })
            },
            devices: {
                let weak = weak.clone();
                Rc::new(move || weak.upgrade().and_then(|app| app.key_devices()))
            },
            capture: settings_page::CaptureHooks {
                status: {
                    let weak = weak.clone();
                    Rc::new(move || {
                        weak.upgrade()
                            .map_or(capture::Status::Off, |app| app.screen.status())
                    })
                },
                turn: {
                    let weak = weak.clone();
                    Rc::new(move |on| {
                        if let Some(app) = weak.upgrade() {
                            app.turn_capture(on);
                        }
                    })
                },
                choose_again: {
                    let weak = weak.clone();
                    Rc::new(move || {
                        if let Some(app) = weak.upgrade() {
                            app.screen.forget_choice();
                            app.turn_capture(true);
                        }
                    })
                },
                screenshot: {
                    let weak = weak.clone();
                    Rc::new(move || {
                        if let Some(app) = weak.upgrade() {
                            app.take_screenshot();
                        }
                    })
                },
                recording: {
                    let weak = weak.clone();
                    Rc::new(move || {
                        weak.upgrade()
                            .map_or_else(String::new, |app| app.recording_text())
                    })
                },
                preview: Rc::new(move || {
                    if let Some(app) = weak.upgrade() {
                        app.announce(&notify::Popup::saved_match(
                            "Casual Duel",
                            "4 \u{2013} 2",
                            notify::Outcome::Win,
                        ));
                    }
                }),
            },
        };
        let page = settings_page::build(
            &self.window,
            self.settings_path.clone(),
            self.db_path.clone(),
            hooks,
            section,
        );
        self.set_page("settings", &page);
        self.pages.set_visible_child_name("settings");
    }

    /// Starts the setup from the settings that are saved now. `jump` opens a later step directly.
    fn show_setup(self: &Rc<Self>, jump: Option<Step>) {
        let current = Settings::load(&self.settings_path).unwrap_or_default();
        let app = Rc::downgrade(self);
        let wizard = setup::Wizard::new(current, move |settings| {
            let Some(app) = app.upgrade() else { return };
            match settings.save(&app.settings_path) {
                Ok(()) => {
                    app.show_main();
                    app.wizard.borrow_mut().take();
                }
                Err(err) => app.toast(&format!("Could not save your settings: {err}")),
            }
        });
        self.set_page("setup", &with_chrome(&wizard.widget()));
        self.pages.set_visible_child_name("setup");
        if let Some(step) = jump {
            wizard.jump(step);
        } else {
            wizard.show(Step::Username, true);
        }
        *self.wizard.borrow_mut() = Some(wizard);
    }
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("theme.css"));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            // Same level as the user's own ~/.config/gtk-4.0/gtk.css (and added after it), so a themed
            // gtk.css with see-through window colours cannot make this app see-through.
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }
}

fn page_step(name: &str) -> Option<Step> {
    Some(match name {
        "username" => Step::Username,
        "mode" => Step::Mode,
        "kind" => Step::Kind,
        "regular" => Step::Regular,
        "adv1" => Step::AdvMatches,
        "adv2" => Step::AdvEvents,
        "adv3" => Step::AdvHabits,
        "finish" => Step::Finish,
        _ => return None,
    })
}

fn activate(application: &adw::Application, args: &Args) {
    adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    load_css();

    let settings_path = args
        .settings
        .clone()
        .or_else(Settings::default_path)
        .unwrap_or_else(|| PathBuf::from("settings.json"));
    // A settings file we cannot read counts as "not set up yet", and we say so after the window opens.
    let (settings, settings_problem) = match Settings::load(&settings_path) {
        Ok(settings) => (settings, None),
        Err(err) => (Settings::default(), Some(err.to_string())),
    };
    let first_launch = !settings.setup_complete || args.reset_setup;

    let pages = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(450)
        .build();
    pages.add_named(&with_chrome(&splash::build()), Some("splash"));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&pages));

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Reclutched")
        .css_classes(["rc-window"])
        .default_width(args.size.map_or(1100, |s| s.0))
        .default_height(args.size.map_or(700, |s| s.1))
        .width_request(700)
        .height_request(480)
        .content(&toasts)
        .build();

    let db_path = args
        .db
        .clone()
        .or_else(default_db_path)
        .unwrap_or_else(|| PathBuf::from("matches.db"));
    let settings_path_for_capture = settings_path.clone();
    let app = Rc::new(App {
        window: window.clone(),
        pages,
        toasts,
        settings_path,
        db_path,
        addr: args.addr.clone(),
        tracking: RefCell::new(None),
        wizard: RefCell::new(None),
        main_view: RefCell::new(None),
        watching: Cell::new(false),
        live: RefCell::new(None),
        live_dismissed: Cell::new(0),
        focus: focus::GameFocus::start(settings.game_window_hints.clone()),
        overlay: RefCell::new(None),
        overlay_tried: Cell::new(false),
        overlay_wanted: Cell::new(false),
        status: Cell::new(tracking::Status::Off),
        ticks: Cell::new(0),
        keys: RefCell::new(None),
        key_held: Cell::new(args.hold_key),
        pad_held: Cell::new(false),
        key_hint_shown: Cell::new(false),
        capture_cb: RefCell::new(None),
        capture_gen: Cell::new(0),
        keys_timer_started: Cell::new(false),
        latest_live: RefCell::new(None),
        usual: RefCell::new(None),
        recent: RefCell::new(None),
        usual_loaded: Cell::new(false),
        screen: capture::Capture::new(
            settings_path_for_capture
                .parent()
                .map_or_else(|| PathBuf::from("."), PathBuf::from)
                .join("capture_token"),
            args.capture_test,
        ),
        rec: recording::RecState::default(),
        shot_at_start: Cell::new(args.capture_shot_at_start),
        clip_after: Cell::new(args.capture_clip_after),
        popup_demo: RefCell::new(args.popup_demo.clone()),
        notifier: RefCell::new(None),
    });
    // The window must keep `app` (and with it the wizard) alive. Without this, `app` is dropped
    // when this function returns and every button handler would find a wizard that is gone.
    let keep_alive = app.clone();
    window.connect_destroy(move |_| {
        let _ = &keep_alive;
    });
    {
        let closing = Rc::downgrade(&app);
        window.connect_close_request(move |_| {
            if let Some(app) = closing.upgrade() {
                app.stop_tracking();
                app.close_overlay();
            }
            gtk::glib::Propagation::Proceed
        });
    }
    window.present();
    if args.minimized {
        window.minimize();
    }
    if let Some(problem) = settings_problem {
        app.toast(&format!(
            "Your settings file could not be read, so we start fresh ({problem})"
        ));
    }

    // Developer shortcut: open one page right away.
    if let Some(name) = args.page.as_deref() {
        match (name, page_step(name)) {
            ("splash", _) => {}
            ("main", _) => app.show_main(),
            ("tracking", _) => {
                app.show_main();
                if let Some(view) = app.main_view.borrow().as_ref() {
                    view.show(shell::Tab::Tracking);
                }
            }
            ("capturing", _) => {
                app.show_main();
                if let Some(view) = app.main_view.borrow().as_ref() {
                    view.show(shell::Tab::Capturing);
                }
            }
            // The newest match with one of its tabs ("main-overview", "main-advanced").
            (tab, _) if tab.starts_with("main-") => {
                app.show_main();
                if let Some(view) = app.main_view.borrow().as_ref() {
                    view.show_match_tab(&tab["main-".len()..]);
                }
            }
            ("calendar", _) => {
                app.show_main();
                app.open_calendar();
            }
            ("settings", _) => {
                app.show_main();
                app.open_settings();
            }
            (section, _) if section.starts_with("settings-") => {
                app.show_main();
                app.open_settings_at(Some(&section["settings-".len()..]));
            }
            (_, Some(step)) => app.show_setup(Some(step)),
            _ => eprintln!("unknown page '{name}'"),
        }
        return;
    }

    // Normal start: loading screen, then setup on the first launch or the ready page after.
    let wait = args.splash_ms.unwrap_or(if first_launch {
        FIRST_LAUNCH_SPLASH_MS
    } else {
        LATER_LAUNCH_SPLASH_MS
    });
    gtk::glib::timeout_add_local_once(Duration::from_millis(wait), move || {
        if first_launch {
            app.show_setup(None);
        } else {
            app.show_main();
        }
    });
}

const APP_ID: &str = "dev.reclutched.Reclutched";

fn main() -> gtk::glib::ExitCode {
    let args = Args::parse();
    let application = adw::Application::builder().application_id(APP_ID).build();
    // Found once the desktop entry and icon are installed (packaging/install-desktop.sh or the package).
    application.connect_startup(|_| gtk::Window::set_default_icon_name(APP_ID));
    application.connect_activate(move |app| activate(app, &args));
    // clap already read the command line, so GTK gets none.
    application.run_with_args::<&str>(&[])
}
