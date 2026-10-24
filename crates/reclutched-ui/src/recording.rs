//! The app side of capturing: the capture keys, the rolling recording that follows the game,
//! clips (by key or automatic), screenshots and full-match recordings, and where they are
//! remembered. The pure parts live in `triggers`, `hotkeys` and `capture::{record, ring}`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use reclutched_core::rocketleague::settings::Settings;
use reclutched_core::rocketleague::storage::{NewClip, Store};
use reclutched_core::rocketleague::tracker::LiveSnapshot;

use crate::capture::{self, ring, RecordConfig, Recorder, Saved};
use crate::hotkeys::{Action, Hotkeys};
use crate::notify::Popup;
use crate::triggers::{Trigger, Watcher};
use crate::{tracking, App};

/// What a finished clip job is.
enum Kind {
    Clip(Trigger),
    /// The recording of a whole match, with the pop-up that goes with it.
    Match {
        match_id: i64,
        popup: Option<Popup>,
    },
}

struct ClipJob {
    rx: Receiver<Result<Saved, String>>,
    kind: Kind,
    taken_at: i64,
    /// The match it was taken in: it is tied to it once that is saved.
    match_seq: Option<u64>,
}

/// What we remember about one match until it is saved.
struct MatchRec {
    seq: u64,
    /// The piece the match began with, so a full recording can start there.
    start: Option<u64>,
    /// The piece where the match is over (the podium included).
    end: Option<u64>,
    /// Clips and screenshots made during it, waiting for its id.
    pending: Vec<i64>,
}

/// After the winner is known the recording goes on for this many pieces (the podium).
const AFTER_WINNER_PIECES: u64 = 4;

#[derive(Default)]
pub struct RecState {
    recorder: RefCell<Option<Recorder>>,
    starting: RefCell<Option<Receiver<Result<Recorder, String>>>>,
    /// We already said why recording could not start (once until it works or capturing stops).
    complained: Cell<bool>,
    watcher: RefCell<Watcher>,
    /// The matches that are on or were not saved yet, oldest first. A match is saved a little
    /// after the next one's first snapshot can arrive, so we keep a few.
    matches: RefCell<VecDeque<MatchRec>>,
    /// The match we are in right now (cleared when the game is back in the menus).
    current: Cell<Option<u64>>,
    jobs: RefCell<Vec<ClipJob>>,
    shots: RefCell<Vec<Receiver<Result<PathBuf, String>>>>,
    hotkeys: RefCell<Hotkeys>,
    timer: Cell<bool>,
    ticks: Cell<u32>,
    /// Settings read every couple of seconds (the live feed asks many times a second).
    settings: RefCell<Settings>,
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn stamp() -> String {
    gtk::glib::DateTime::now_local()
        .ok()
        .and_then(|now| now.format("%Y-%m-%d %H-%M-%S").ok())
        .map_or_else(|| "capture".to_string(), |s| s.to_string())
}

impl RecState {
    /// The running recorder (dropping it stops the recording).
    pub(crate) fn recorder_slot(&self) -> std::cell::RefMut<'_, Option<Recorder>> {
        self.recorder.borrow_mut()
    }
}

impl App {
    /// Starts the 100 ms timer that serves the capture keys, the recording and finished jobs.
    pub(crate) fn watch_capture(self: &Rc<Self>) {
        if self.rec.timer.replace(true) {
            return;
        }
        let app = Rc::downgrade(self);
        gtk::glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(app) = app.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            app.capture_tick();
            gtk::glib::ControlFlow::Continue
        });
    }

    fn capture_tick(self: &Rc<Self>) {
        let ticks = self.rec.ticks.get();
        self.rec.ticks.set(ticks.wrapping_add(1));
        if ticks.is_multiple_of(20) {
            let settings = Settings::load(&self.settings_path).unwrap_or_default();
            self.rec.hotkeys.borrow_mut().sync(&settings);
            self.sync_recorder(&settings);
            if ticks.is_multiple_of(100) {
                self.prune_recording(&settings);
            }
            *self.rec.settings.borrow_mut() = settings;
        }
        let pressed = self.rec.hotkeys.borrow().pressed();
        for action in pressed {
            match action {
                Action::Screenshot => self.take_screenshot(),
                Action::Clip => self.make_clip(Trigger::Key),
            }
        }
        self.poll_screenshots();
        self.poll_jobs();
    }

    // ---------- the rolling recording ----------

    /// Starts or stops the recording: it runs while capturing is on, the screen is shared and
    /// the game is connected.
    fn sync_recorder(self: &Rc<Self>, settings: &Settings) {
        let connected = self.status.get() == tracking::Status::Active || self.screen.is_test();
        let ready = matches!(self.screen.status(), capture::Status::Ready(_));
        let want = settings.capture_enabled && ready && connected;

        // Did a start finish?
        let finished = self.rec.starting.borrow().as_ref().map(|rx| rx.try_recv());
        match finished {
            Some(Ok(Ok(recorder))) => {
                *self.rec.recorder.borrow_mut() = Some(recorder);
                *self.rec.starting.borrow_mut() = None;
                self.rec.complained.set(false);
            }
            Some(Ok(Err(problem))) => {
                *self.rec.starting.borrow_mut() = None;
                if !self.rec.complained.replace(true) {
                    self.toast(&format!("Recording could not start: {problem}"));
                }
            }
            Some(Err(TryRecvError::Disconnected)) => *self.rec.starting.borrow_mut() = None,
            _ => {}
        }

        let mut recorder = self.rec.recorder.borrow_mut();
        if let Some(running) = recorder.as_mut() {
            if !want {
                *recorder = None;
                self.rec
                    .matches
                    .borrow_mut()
                    .iter_mut()
                    .for_each(|m| m.start = None);
            } else if !running.alive() {
                let why = running.complaint();
                *recorder = None;
                self.rec
                    .matches
                    .borrow_mut()
                    .iter_mut()
                    .for_each(|m| m.start = None);
                if !self.rec.complained.replace(true) {
                    self.toast(&format!("Recording stopped: {why}"));
                }
            }
        } else if want && self.rec.starting.borrow().is_none() {
            *self.rec.starting.borrow_mut() = Some(self.screen.start_recording(RecordConfig {
                fps: settings.capture_fps,
                preset: settings.capture_quality.clone(),
                audio_game: settings.audio_game,
                audio_mic: settings.audio_mic,
            }));
        } else if !want {
            self.rec.complained.set(false);
        }
    }

    /// What the settings page says about the recording.
    pub(crate) fn recording_text(&self) -> String {
        if let Some(recorder) = self.rec.recorder.borrow().as_ref() {
            return recorder.describe();
        }
        if self.rec.starting.borrow().is_some() {
            return "Starting the recording\u{2026}".to_string();
        }
        match (self.screen.status(), self.status.get()) {
            (capture::Status::Ready(_), s)
                if s != tracking::Status::Active && !self.screen.is_test() =>
            {
                "Waiting for the game (recording starts when it is connected)".to_string()
            }
            (capture::Status::Ready(_), _) => "Getting ready".to_string(),
            _ => "Not recording".to_string(),
        }
    }

    fn prune_recording(&self, settings: &Settings) {
        let Some(recorder) = self
            .rec
            .recorder
            .borrow()
            .as_ref()
            .map(|r| (r.ring.clone(), r.thumbs.clone()))
        else {
            return;
        };
        let held = if settings.record_mode == "full" {
            self.rec
                .matches
                .borrow()
                .iter()
                .filter_map(|m| m.start)
                .min()
        } else {
            None
        };
        if let Some(latest) = ring::latest(&recorder.0) {
            let keep = ring::keep_from(latest, held);
            ring::prune_before(&recorder.0, keep);
            ring::prune_thumbs(&recorder.1, keep);
        }
    }

    // ---------- the match ----------

    /// The match in progress changed: remember where it began and ended, and make automatic clips.
    pub(crate) fn on_live(self: &Rc<Self>, snapshot: Option<&LiveSnapshot>) {
        let latest = self
            .rec
            .recorder
            .borrow()
            .as_ref()
            .and_then(Recorder::latest);
        let Some(snapshot) = snapshot else {
            // Back in the menus: what we do now belongs to no match.
            self.rec.current.set(None);
            if let Some(last) = self.rec.matches.borrow_mut().back_mut() {
                last.end = last.end.or(latest);
            }
            return;
        };
        let settings = self.rec.settings.borrow().clone();
        if self.rec.watcher.borrow().is_new_match(snapshot) {
            let mut matches = self.rec.matches.borrow_mut();
            if let Some(last) = matches.back_mut() {
                last.end = last.end.or(latest);
            }
            matches.push_back(MatchRec {
                seq: snapshot.seq,
                start: latest,
                end: None,
                pending: Vec::new(),
            });
            while matches.len() > 4 {
                matches.pop_front();
            }
            self.rec.current.set(Some(snapshot.seq));
        }
        if snapshot.winner.is_some() {
            let mut matches = self.rec.matches.borrow_mut();
            if let Some(rec) = matches.iter_mut().find(|m| m.seq == snapshot.seq) {
                if rec.end.is_none() {
                    rec.end = latest.map(|l| l + AFTER_WINNER_PIECES);
                }
            }
        }
        let triggers = self.rec.watcher.borrow_mut().update(snapshot, &settings);
        for trigger in triggers {
            // Wait a few seconds so the moment itself is in the clip.
            let app = Rc::downgrade(self);
            gtk::glib::timeout_add_local_once(
                Duration::from_secs(u64::from(trigger.tail_seconds())),
                move || {
                    if let Some(app) = app.upgrade() {
                        app.make_clip(trigger);
                    }
                },
            );
        }
    }

    /// The match was saved: tie its clips to it, and cut its full recording if wanted.
    pub(crate) fn on_match_saved(self: &Rc<Self>, id: i64, summary: &str) {
        let settings = Settings::load(&self.settings_path).unwrap_or_default();
        // The oldest match we still remember is the one that was just saved.
        let rec = self.rec.matches.borrow_mut().pop_front();
        let (pending, start, end) =
            rec.map_or((Vec::new(), None, None), |m| (m.pending, m.start, m.end));
        if !pending.is_empty() {
            if let Ok(store) = Store::open(&self.db_path) {
                let _ = store.attach_clips(&pending, id);
            }
        }
        let popup = self.match_popup();
        let recording = self
            .rec
            .recorder
            .borrow()
            .as_ref()
            .map(|r| (r.ring.clone(), r.thumbs.clone(), r.audio, r.latest()));
        // The recording ends where the match did (the podium included), or now.
        let (Some((ring_dir, thumbs, audio, Some(newest))), Some(from), "full", Some(folder)) = (
            recording,
            start,
            settings.record_mode.as_str(),
            settings.capture_folder(),
        ) else {
            match popup {
                Some(popup) => self.announce(&popup),
                None => self.toast(&format!("Saved: {summary}")),
            }
            return;
        };
        let latest = end.map_or(newest, |e| e.min(newest));
        let out = folder
            .join("Matches")
            .join(format!("Reclutched {} match.mp4", stamp()));
        let thumb = crate::gallery::thumb_path(&folder, &out);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(capture::save_range(
                &ring_dir,
                &thumbs,
                from,
                latest,
                audio,
                &out,
                Some(&thumb),
            ));
        });
        self.rec.jobs.borrow_mut().push(ClipJob {
            rx,
            kind: Kind::Match {
                match_id: id,
                popup,
            },
            taken_at: now_unix(),
            match_seq: None,
        });
        // Old recordings go when the saved number of days has passed.
        ring::delete_older_than(&folder.join("Matches"), settings.full_delete_days);
    }

    // ---------- clips ----------

    /// Saves the last seconds as a clip (a key press, or an automatic trigger).
    pub(crate) fn make_clip(self: &Rc<Self>, trigger: Trigger) {
        let settings = self.rec.settings.borrow().clone();
        let Some(folder) = settings.capture_folder() else {
            self.toast("No folder to save clips in (set one in Settings, Capture)");
            return;
        };
        let source = self
            .rec
            .recorder
            .borrow()
            .as_ref()
            .map(|r| (r.ring.clone(), r.thumbs.clone(), r.audio, r.latest()));
        let Some((ring_dir, thumbs, audio, Some(latest))) = source else {
            if trigger == Trigger::Key {
                self.toast("Not recording right now: turn capturing on and start the game first");
            }
            return;
        };
        let seconds = settings.clip_seconds.clamp(10, 50) + trigger.tail_seconds();
        let from = ring::clip_start(latest, seconds);
        let out = folder.join("Clips").join(format!(
            "Reclutched {} {}.mp4",
            stamp(),
            trigger.label().replace(' ', "-")
        ));
        let thumb = crate::gallery::thumb_path(&folder, &out);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(capture::save_range(
                &ring_dir,
                &thumbs,
                from,
                latest,
                audio,
                &out,
                Some(&thumb),
            ));
        });
        self.rec.jobs.borrow_mut().push(ClipJob {
            rx,
            kind: Kind::Clip(trigger),
            taken_at: now_unix(),
            match_seq: self.rec.current.get(),
        });
    }

    /// Reports the clips and recordings that finished.
    fn poll_jobs(self: &Rc<Self>) {
        let mut done = Vec::new();
        self.rec
            .jobs
            .borrow_mut()
            .retain(|job| match job.rx.try_recv() {
                Ok(result) => {
                    done.push((job.kind_label(), job.taken_at, job.match_seq, result));
                    false
                }
                Err(TryRecvError::Empty) => true,
                Err(TryRecvError::Disconnected) => false,
            });
        for (kind, taken_at, match_seq, result) in done {
            match (kind, result) {
                (JobKind::Clip(trigger), Ok(saved)) => {
                    self.remember(&saved, "clip", trigger.label(), taken_at, None, match_seq);
                    self.announce(&Popup::clip(saved.path, saved.thumb));
                }
                (JobKind::Clip(_), Err(e)) => self.toast(&format!("Clip failed: {e}")),
                (JobKind::Match(match_id, popup), Ok(saved)) => {
                    self.remember(&saved, "match", "recording", taken_at, Some(match_id), None);
                    match popup {
                        Some(mut popup) => {
                            popup.image = saved.thumb;
                            self.announce(&popup);
                        }
                        None => self.toast("Match recording saved"),
                    }
                }
                (JobKind::Match(_, popup), Err(e)) => {
                    self.toast(&format!("The match recording failed: {e}"));
                    if let Some(popup) = popup {
                        self.announce(&popup);
                    }
                }
            }
        }
    }

    /// Writes a clip, screenshot or recording to the database. During a match it waits for the
    /// match id.
    fn remember(
        &self,
        saved: &Saved,
        kind: &str,
        trigger: &str,
        taken_at: i64,
        match_id: Option<i64>,
        match_seq: Option<u64>,
    ) {
        let Ok(store) = Store::open(&self.db_path) else {
            return;
        };
        let added = store.add_clip(&NewClip {
            path: saved.path.to_string_lossy().to_string(),
            kind: kind.to_string(),
            trigger: trigger.to_string(),
            taken_at,
            seconds: saved.seconds,
            thumb: saved
                .thumb
                .as_ref()
                .map(|t| t.to_string_lossy().to_string()),
        });
        let Ok(id) = added else { return };
        if let Some(match_id) = match_id {
            let _ = store.attach_clips(&[id], match_id);
        } else if let Some(seq) = match_seq {
            if let Some(rec) = self
                .rec
                .matches
                .borrow_mut()
                .iter_mut()
                .find(|m| m.seq == seq)
            {
                rec.pending.push(id);
            }
        }
    }

    // ---------- screenshots ----------

    /// Saves a screenshot and says where it went (or why not).
    pub(crate) fn take_screenshot(self: &Rc<Self>) {
        let settings = Settings::load(&self.settings_path).unwrap_or_default();
        let Some(folder) = settings.capture_folder() else {
            self.toast("No folder to save screenshots in (set one in Settings, Capture)");
            return;
        };
        let answer = self.screen.screenshot(folder, &stamp());
        self.rec.shots.borrow_mut().push(answer);
    }

    fn poll_screenshots(self: &Rc<Self>) {
        let mut finished = Vec::new();
        self.rec
            .shots
            .borrow_mut()
            .retain(|rx| match rx.try_recv() {
                Ok(result) => {
                    finished.push(result);
                    false
                }
                Err(TryRecvError::Empty) => true,
                Err(_) => false,
            });
        for result in finished {
            match result {
                Ok(path) => {
                    let saved = Saved {
                        path: path.clone(),
                        thumb: Some(path.clone()),
                        seconds: 0,
                    };
                    self.remember(
                        &saved,
                        "screenshot",
                        "key",
                        now_unix(),
                        None,
                        self.rec.current.get(),
                    );
                    self.announce(&Popup::screenshot(path));
                }
                Err(e) => self.toast(&format!("Screenshot failed: {e}")),
            }
        }
    }
}

enum JobKind {
    Clip(Trigger),
    Match(i64, Option<Popup>),
}

impl ClipJob {
    fn kind_label(&self) -> JobKind {
        match &self.kind {
            Kind::Clip(t) => JobKind::Clip(*t),
            Kind::Match { match_id, popup } => JobKind::Match(*match_id, popup.clone()),
        }
    }
}
