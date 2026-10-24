//! The app side of updating (checking now and then, installing, restarting) and of the full
//! reset. The decisions are in `update`.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};

use adw::prelude::*;
use reclutched_core::rocketleague::settings::Settings;
use reclutched_core::rocketleague::storage::Store;

use crate::update::{self, Check, Progress};
use crate::{tracking, App};

/// Where the update stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(String),
    Installing,
    /// Installed; restart to use it.
    Ready,
    Failed(String),
    /// Could not tell (offline, ...).
    Unknown(String),
}

#[derive(Default)]
pub struct UpdateState {
    phase: RefCell<Phase>,
    checking: RefCell<Option<Receiver<Check>>>,
    installing: RefCell<Option<Receiver<Progress>>>,
    /// What the installation is doing and its latest line.
    doing: RefCell<String>,
    timer: Cell<bool>,
    ticks: Cell<u64>,
}

/// Seconds between two automatic looks (and the wait before the first).
const FIRST_CHECK_AFTER: u64 = 25;
const CHECK_EVERY: u64 = 6 * 3600;

fn cache_log() -> PathBuf {
    crate::capture::cache_dir().join("update.log")
}

impl App {
    /// Starts the one-second timer that looks for updates now and then and follows an install.
    pub(crate) fn watch_updates(self: &Rc<Self>) {
        if self.upd.timer.replace(true) {
            return;
        }
        let app = Rc::downgrade(self);
        gtk::glib::timeout_add_seconds_local(1, move || {
            let Some(app) = app.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            app.update_tick();
            gtk::glib::ControlFlow::Continue
        });
    }

    fn update_tick(self: &Rc<Self>) {
        let tick = self.upd.ticks.get();
        self.upd.ticks.set(tick + 1);
        let settings = Settings::load(&self.settings_path).unwrap_or_default();

        // A check finished?
        let found = self.upd.checking.borrow().as_ref().map(|rx| rx.try_recv());
        match found {
            Some(Ok(check)) => {
                *self.upd.checking.borrow_mut() = None;
                *self.upd.phase.borrow_mut() = match check {
                    Check::UpToDate => Phase::UpToDate,
                    Check::Available { commit } => Phase::Available(commit),
                    Check::Unknown(why) => Phase::Unknown(why),
                };
                self.show_update_pill();
            }
            Some(Err(TryRecvError::Disconnected)) => {
                *self.upd.checking.borrow_mut() = None;
                *self.upd.phase.borrow_mut() = Phase::Unknown("the check stopped".to_string());
            }
            _ => {}
        }

        // An install moved on?
        let mut messages = Vec::new();
        if let Some(rx) = self.upd.installing.borrow().as_ref() {
            while let Ok(message) = rx.try_recv() {
                messages.push(message);
            }
        }
        for message in messages {
            match message {
                Progress::Step(what) => *self.upd.doing.borrow_mut() = what.to_string(),
                Progress::Line(line) => {
                    let what = self
                        .upd
                        .doing
                        .borrow()
                        .split(" \u{00B7} ")
                        .next()
                        .unwrap_or("")
                        .to_string();
                    *self.upd.doing.borrow_mut() = format!("{what} \u{00B7} {line}");
                }
                Progress::Done => {
                    *self.upd.installing.borrow_mut() = None;
                    *self.upd.phase.borrow_mut() = Phase::Ready;
                    self.show_update_pill();
                    self.toast("Reclutched is updated. Restart it to use the new version.");
                }
                Progress::Failed(why) => {
                    *self.upd.installing.borrow_mut() = None;
                    *self.upd.phase.borrow_mut() = Phase::Failed(why.clone());
                    self.show_update_pill();
                    self.toast(&format!(
                        "The update did not work: {}. The full log is {}",
                        why.lines().last().unwrap_or("unknown error"),
                        cache_log().display()
                    ));
                }
            }
        }

        // Look now and then.
        let idle = matches!(
            *self.upd.phase.borrow(),
            Phase::Idle | Phase::UpToDate | Phase::Unknown(_)
        );
        if settings.update_check
            && idle
            && self.upd.checking.borrow().is_none()
            && (tick == FIRST_CHECK_AFTER
                || (tick > FIRST_CHECK_AFTER
                    && (tick - FIRST_CHECK_AFTER).is_multiple_of(CHECK_EVERY)))
        {
            self.check_updates_now();
        }

        // Install by itself, but never while the game is running.
        if settings.update_auto_install
            && self.status.get() != tracking::Status::Active
            && matches!(*self.upd.phase.borrow(), Phase::Available(_))
        {
            self.start_install();
        }
    }

    /// Looks for a new version in the background.
    pub(crate) fn check_updates_now(self: &Rc<Self>) {
        if self.upd.checking.borrow().is_some() || *self.upd.phase.borrow() == Phase::Installing {
            return;
        }
        let settings = Settings::load(&self.settings_path).unwrap_or_default();
        let branch = update::channel(&settings.update_channel, update::BRANCH);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(update::check(branch));
        });
        *self.upd.checking.borrow_mut() = Some(rx);
        *self.upd.phase.borrow_mut() = Phase::Checking;
    }

    /// Builds and installs the new version in the background.
    fn start_install(self: &Rc<Self>) {
        let source = match update::can_install() {
            Ok(source) => source,
            Err(why) => {
                *self.upd.phase.borrow_mut() = Phase::Failed(why.clone());
                self.toast(&why);
                self.show_update_pill();
                return;
            }
        };
        let settings = Settings::load(&self.settings_path).unwrap_or_default();
        let branch = update::channel(&settings.update_channel, update::BRANCH);
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let with_cli = home.join(".cargo/bin/reclutched").exists();
        let steps = update::plan(
            &source,
            branch,
            &update::checked_out_branch(&source),
            with_cli,
        );
        *self.upd.installing.borrow_mut() = Some(update::install(steps, cache_log()));
        *self.upd.doing.borrow_mut() = "Starting".to_string();
        *self.upd.phase.borrow_mut() = Phase::Installing;
        self.show_update_pill();
    }

    /// Developer helper: shows the update button as if a new version had been found.
    pub(crate) fn pretend_update_found(&self) {
        *self.upd.phase.borrow_mut() = Phase::Available("0123456789abcdef".to_string());
        self.show_update_pill();
    }

    /// What the pill in the top bar (and the About page) says.
    fn update_words(&self) -> String {
        match &*self.upd.phase.borrow() {
            Phase::Idle => "Not checked yet".to_string(),
            Phase::Checking => "Looking for a new version\u{2026}".to_string(),
            Phase::UpToDate => "You have the newest version".to_string(),
            Phase::Available(commit) => {
                format!("A new version is ready ({})", update::short(commit))
            }
            Phase::Installing => format!("Updating: {}", self.upd.doing.borrow()),
            Phase::Ready => "Updated. Restart Reclutched to use it.".to_string(),
            Phase::Failed(why) => format!(
                "The update did not work: {}",
                why.lines().last().unwrap_or("unknown error")
            ),
            Phase::Unknown(why) => format!("Could not check: {why}"),
        }
    }

    pub(crate) fn update_status_text(&self) -> String {
        self.update_words()
    }

    /// Shows or hides the button in the top bar.
    fn show_update_pill(&self) {
        let view = self.main_view.borrow();
        let Some(view) = view.as_ref() else { return };
        match &*self.upd.phase.borrow() {
            Phase::Available(_) => view.set_update(Some(("Update available", false))),
            Phase::Installing => view.set_update(Some(("Updating\u{2026}", true))),
            Phase::Ready => view.set_update(Some(("Restart to update", false))),
            Phase::Failed(_) => view.set_update(Some(("Update failed", false))),
            _ => view.set_update(None),
        }
    }

    /// The button in the top bar or the "Update now" row was used.
    pub(crate) fn update_clicked(self: &Rc<Self>) {
        let phase = self.upd.phase.borrow().clone();
        match phase {
            Phase::Available(_)
            | Phase::Failed(_)
            | Phase::UpToDate
            | Phase::Unknown(_)
            | Phase::Idle => {
                if matches!(phase, Phase::UpToDate | Phase::Idle | Phase::Unknown(_)) {
                    self.check_updates_now();
                    return;
                }
                self.confirm_update();
            }
            Phase::Checking => {}
            Phase::Installing => self.toast(&self.update_words()),
            Phase::Ready => self.restart_app(),
        }
    }

    fn confirm_update(self: &Rc<Self>) {
        let dialog = adw::AlertDialog::builder()
            .heading("Update Reclutched?")
            .body("Reclutched fetches the new version and builds it in the background. That takes a few minutes and you can keep using the app. When it is done it asks you to restart.")
            .build();
        dialog.add_responses(&[("cancel", "Not now"), ("update", "Update")]);
        dialog.set_response_appearance("update", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("update"));
        dialog.set_close_response("cancel");
        let app = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response == "update" {
                if let Some(app) = app.upgrade() {
                    app.start_install();
                }
            }
        });
        dialog.present(Some(&self.window));
    }

    /// Starts the new program and closes this one.
    pub(crate) fn restart_app(self: &Rc<Self>) {
        let exe = update::clean_exe(std::env::current_exe().unwrap_or_default());
        let args: Vec<String> = std::env::args().skip(1).collect();
        match std::process::Command::new(&exe).args(&args).spawn() {
            Ok(_) => {
                self.stop_tracking();
                if let Some(application) = self.window.application() {
                    application.quit();
                }
            }
            Err(e) => self.toast(&format!(
                "Could not restart: {e}. Start Reclutched again by hand."
            )),
        }
    }

    // ---------- reset ----------

    /// Erases settings, matches, clips and everything else Reclutched keeps, and starts like a
    /// new user. Your screenshots and videos are only deleted when `delete_media` is set.
    pub(crate) fn reset_everything(self: &Rc<Self>, delete_media: bool) {
        let settings = Settings::load(&self.settings_path).unwrap_or_default();
        // Stop everything that uses the files first.
        self.stop_tracking();
        self.close_overlay();
        self.close_live();
        self.screen.turn_off();
        *self.rec.recorder_slot() = None;

        let mut problems = Vec::new();
        match Store::open(&self.db_path).and_then(|mut s| s.delete_everything()) {
            Ok(_) => {}
            Err(e) => problems.push(format!("matches: {e}")),
        }
        if delete_media {
            if let Some(folder) = settings.capture_folder() {
                for sub in ["Screenshots", "Clips", "Matches", ".thumbs"] {
                    let _ = std::fs::remove_dir_all(folder.join(sub));
                }
            }
        }
        let cache = crate::capture::cache_dir();
        let _ = std::fs::remove_dir_all(&cache);
        let _ = crate::startup::set_autostart(false, false);
        if let Some(parent) = self.settings_path.parent() {
            let _ = std::fs::remove_file(parent.join("capture_token"));
        }
        // Fresh settings: the setup runs again.
        if let Err(e) = Settings::default().save(&self.settings_path) {
            problems.push(format!("settings: {e}"));
        }
        *self.main_view.borrow_mut() = None;
        self.latest_live.borrow_mut().take();
        self.usual_loaded.set(false);
        *self.upd.phase.borrow_mut() = Phase::Idle;
        if !problems.is_empty() {
            self.toast(&format!(
                "Some things could not be erased: {}",
                problems.join("; ")
            ));
        }
        self.show_setup(None);
    }
}
