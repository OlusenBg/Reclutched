//! Reclutched desktop app.
//!
//! Right now this is the first-launch flow: a loading screen, then the setup wizard, then a
//! simple "ready" page. The match history and details come next.

mod flow;
mod ready;
mod setup;
mod splash;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use clap::Parser;
use reclutched_core::rocketleague::settings::Settings;

use flow::Step;

const FIRST_LAUNCH_SPLASH_MS: u64 = 3000;
const LATER_LAUNCH_SPLASH_MS: u64 = 1000;

#[derive(Parser)]
#[command(name = "reclutched-ui", version, about = "Reclutched desktop app")]
struct Args {
    /// Settings file (default: ~/.config/reclutched/settings.json).
    #[arg(long)]
    settings: Option<PathBuf>,
    /// Run the first-launch setup again, even if it was finished.
    #[arg(long)]
    reset_setup: bool,
    /// Developer option: how long the loading screen lasts, in milliseconds.
    #[arg(long, hide = true)]
    splash_ms: Option<u64>,
    /// Developer option: open one page directly (for screenshots).
    #[arg(long, hide = true)]
    page: Option<String>,
}

/// Everything the window needs to move between its three screens.
struct App {
    pages: gtk::Stack,
    toasts: adw::ToastOverlay,
    settings_path: PathBuf,
    /// Keeps the current wizard alive while it is on screen.
    wizard: RefCell<Option<Rc<setup::Wizard>>>,
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

    fn show_ready(self: &Rc<Self>, settings: &Settings) {
        let app = Rc::downgrade(self);
        let page = ready::build(settings, move || {
            if let Some(app) = app.upgrade() {
                app.show_setup(None);
            }
        });
        self.set_page("ready", &page);
        self.pages.set_visible_child_name("ready");
    }

    /// Starts the setup from the settings that are saved now. `jump` opens a later step directly.
    fn show_setup(self: &Rc<Self>, jump: Option<Step>) {
        let current = Settings::load(&self.settings_path).unwrap_or_default();
        let app = Rc::downgrade(self);
        let wizard = setup::Wizard::new(current, move |settings| {
            let Some(app) = app.upgrade() else { return };
            match settings.save(&app.settings_path) {
                Ok(()) => {
                    app.show_ready(&settings);
                    app.wizard.borrow_mut().take();
                }
                Err(err) => app.toast(&format!("Could not save your settings: {err}")),
            }
        });
        self.set_page("setup", &wizard.widget());
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
    let settings = Settings::load(&settings_path).unwrap_or_default();
    let first_launch = !settings.setup_complete || args.reset_setup;

    let pages = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(450)
        .build();
    pages.add_named(&splash::build(), Some("splash"));
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&pages));

    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Reclutched")
        .default_width(960)
        .default_height(640)
        .width_request(560)
        .height_request(520)
        .content(&toasts)
        .build();

    let app = Rc::new(App {
        pages,
        toasts,
        settings_path,
        wizard: RefCell::new(None),
    });
    // The window must keep `app` (and with it the wizard) alive. Without this, `app` is dropped
    // when this function returns and every button handler would find a wizard that is gone.
    let keep_alive = app.clone();
    window.connect_destroy(move |_| {
        let _ = &keep_alive;
    });
    window.present();

    // Developer shortcut: open one page right away.
    if let Some(name) = args.page.as_deref() {
        match (name, page_step(name)) {
            ("splash", _) => {}
            ("ready", _) => app.show_ready(&settings),
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
            app.show_ready(&settings);
        }
    });
}

fn main() -> gtk::glib::ExitCode {
    let args = Args::parse();
    let application = adw::Application::builder()
        .application_id("dev.reclutched.Reclutched")
        .build();
    application.connect_activate(move |app| activate(app, &args));
    // clap already read the command line, so GTK gets none.
    application.run_with_args::<&str>(&[])
}
