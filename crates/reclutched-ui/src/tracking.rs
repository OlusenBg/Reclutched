//! Runs the tracker (the same one `reclutched track` uses) on a background thread, so the app
//! saves your matches by itself. The thread has its own small tokio runtime; the app polls it
//! for news every half second, because GTK widgets may only be touched on the main thread.

use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;

use reclutched_daemon::track::{self, Paths, TrackEvent};
use tokio::sync::oneshot;

/// What the status light in the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Tracking is switched off.
    Off,
    /// On, but Rocket League is not running (or its Stats API is off).
    Waiting,
    /// Connected to the game; matches will be saved.
    Active,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Off => "Tracking off",
            Status::Waiting => "Waiting for Rocket League",
            Status::Active => "Tracking",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Status::Off => "Matches are not being saved. Click to start tracking.",
            Status::Waiting => {
                "Tracking is on. Start Rocket League (with the Stats API enabled) and matches are saved. Click to stop."
            }
            Status::Active => "Connected to Rocket League. Finished matches are saved. Click to stop.",
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Status::Off => "off",
            Status::Waiting => "waiting",
            Status::Active => "active",
        }
    }
}

/// Something that happened on the tracker thread.
pub enum Update {
    Event(TrackEvent),
    /// The tracker stopped by itself, with the reason if it failed.
    Ended(Option<String>),
}

pub struct Tracking {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    updates: mpsc::Receiver<Update>,
}

impl Tracking {
    pub fn start(addr: String, paths: Paths) -> Self {
        let (tx, updates) = mpsc::channel();
        let (stop_tx, stop_rx) = oneshot::channel::<()>();

        let events = tx.clone();
        let report: track::Reporter = Arc::new(move |event| {
            let _ = events.send(Update::Event(event));
        });
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let outcome = match runtime {
                Ok(rt) => rt
                    .block_on(track::run(&addr, paths, report, async {
                        // Finishes when we send a stop, or when the `Tracking` is dropped.
                        let _ = stop_rx.await;
                    }))
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(Update::Ended(outcome.err()));
        });
        Self {
            stop: Some(stop_tx),
            thread: Some(thread),
            updates,
        }
    }

    /// Everything that happened since the last call.
    pub fn drain(&self) -> Vec<Update> {
        self.updates.try_iter().collect()
    }

    /// Stops tracking. The tracker first saves a match that is already decided, which takes
    /// a moment, so this waits for it.
    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Tracking {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_has_text_and_a_distinct_class() {
        let all = [Status::Off, Status::Waiting, Status::Active];
        for s in all {
            assert!(!s.label().is_empty() && !s.tooltip().is_empty());
        }
        let classes: std::collections::HashSet<_> = all.iter().map(|s| s.css_class()).collect();
        assert_eq!(classes.len(), 3);
    }

    #[test]
    fn tracking_reports_waiting_then_ends_cleanly_when_stopped() {
        // Nothing listens on this port, so the tracker can only wait for the game.
        let dir = std::env::temp_dir().join(format!("reclutched-ui-track-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths {
            db: dir.join("m.db"),
            settings: dir.join("s.json"),
            playlist_map: None,
        };
        let mut tracking = Tracking::start("127.0.0.1:1".into(), paths);

        let mut saw_waiting = false;
        for _ in 0..50 {
            for update in tracking.drain() {
                saw_waiting |= matches!(update, Update::Event(TrackEvent::Waiting));
            }
            if saw_waiting {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(
            saw_waiting,
            "the tracker should say it is waiting for the game"
        );

        tracking.stop();
        let ended = tracking
            .drain()
            .into_iter()
            .any(|u| matches!(u, Update::Ended(None)));
        assert!(ended, "stopping ends the tracker without an error");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
