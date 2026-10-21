//! Reclutched desktop app.
//!
//! First launch: a loading screen, then the setup wizard. After that the main window with your
//! match history, the details of each match and the settings.

mod detail;
mod flow;
mod format;
mod list;
mod mainview;
mod settings_dialog;
mod setup;
mod splash;
mod tracking;
mod widgets;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use clap::Parser;
use reclutched_core::rocketleague::playlist::default_map_path;
use reclutched_core::rocketleague::settings::Settings;
use reclutched_core::rocketleague::storage::{default_db_path, Store};
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
    main_view: RefCell<Option<Rc<mainview::MainView>>>,
    /// True once the timer that reads tracker news is running.
    watching: Cell<bool>,
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
        let toggle = Rc::downgrade(self);
        let on_toggle: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(app) = toggle.upgrade() {
                app.toggle_tracking();
            }
        });
        let view = mainview::MainView::new(
            &self.window,
            store,
            default_map_path(),
            on_settings,
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
    }

    fn set_status(&self, status: tracking::Status) {
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
        gtk::glib::timeout_add_local(Duration::from_millis(500), move || {
            let Some(app) = app.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            app.apply_updates();
            gtk::glib::ControlFlow::Continue
        });
    }

    fn apply_updates(&self) {
        let updates = match self.tracking.borrow().as_ref() {
            Some(t) => t.drain(),
            None => return,
        };
        let mut ended = None;
        for update in updates {
            match update {
                tracking::Update::Event(TrackEvent::Connected) => {
                    self.set_status(tracking::Status::Active);
                }
                tracking::Update::Event(TrackEvent::Waiting) => {
                    self.set_status(tracking::Status::Waiting);
                }
                tracking::Update::Event(TrackEvent::MatchSaved { summary, .. }) => {
                    self.toast(&format!("Saved: {summary}"));
                    if let Some(view) = self.main_view.borrow().as_ref() {
                        view.reload();
                    }
                }
                tracking::Update::Event(TrackEvent::PlayerIdLearned) => {}
                tracking::Update::Event(TrackEvent::Error(message)) => self.toast(&message),
                tracking::Update::Ended(reason) => ended = Some(reason),
            }
        }
        // The tracker stopped on its own (for example it could not open the database).
        if let Some(reason) = ended {
            self.tracking.borrow_mut().take();
            self.set_status(tracking::Status::Off);
            if let Some(reason) = reason {
                self.toast(&format!("Tracking stopped: {reason}"));
            }
        }
    }

    fn open_settings(self: &Rc<Self>) {
        let again = Rc::downgrade(self);
        settings_dialog::open(
            &self.window,
            self.settings_path.clone(),
            self.db_path.clone(),
            move || {
                if let Some(app) = again.upgrade() {
                    app.show_setup(None);
                }
            },
        );
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
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
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
            }
            gtk::glib::Propagation::Proceed
        });
    }
    window.present();
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
            (tab, _) if tab.starts_with("main-") => {
                app.show_main();
                if let Some(view) = app.main_view.borrow().as_ref() {
                    view.show_tab(&tab["main-".len()..]);
                }
            }
            ("settings", _) => {
                app.show_main();
                app.open_settings();
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
