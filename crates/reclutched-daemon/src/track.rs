//! `reclutched track`: follows the game and saves every finished match to the database.
//!
//! Events flow like this:
//!
//!   websocket -> parse -> broadcast bus -> writer (tracker + SQLite)
//!
//! The bus is a tokio `broadcast` channel: every subscriber gets its own copy of each event.
//! Right now only the writer listens; the clipper and the overlay will subscribe here later.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::settings::Settings;
use reclutched_core::rocketleague::storage::Store;
use reclutched_core::rocketleague::tracker::{FinishedMatch, LiveSnapshot, MatchResult, Tracker};
use reclutched_core::rocketleague::{parse_message, StatsEvent};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::source;

/// The live view is refreshed at most this often by the game's frequent updates.
const LIVE_EVERY: Duration = Duration::from_millis(250);

/// How many events the bus holds for a slow subscriber before it starts losing the oldest.
const BUS_CAPACITY: usize = 4096;

pub struct Paths {
    pub db: PathBuf,
    pub settings: PathBuf,
    /// Optional user playlist names (for printing the mode name).
    pub playlist_map: Option<PathBuf>,
}

/// What the tracker tells whoever started it (the terminal, or the app).
#[derive(Debug, Clone, PartialEq)]
pub enum TrackEvent {
    /// The game is not reachable (yet, or any more).
    Waiting,
    /// Connected to the game's Stats API.
    Connected,
    /// A finished match was written to the database.
    MatchSaved { id: i64, summary: String },
    /// We found out who you are and saved your player id.
    PlayerIdLearned,
    /// The match in progress (a few times a second), or `None` once there is none.
    Live(Option<Box<LiveSnapshot>>),
    /// Something went wrong that the user should know about.
    Error(String),
}

/// Called for every [`TrackEvent`]. It can run on any thread, so it must be `Send + Sync`.
pub type Reporter = Arc<dyn Fn(TrackEvent) + Send + Sync>;

/// Runs until `stop` finishes, then saves the match in progress (if it was already decided)
/// and returns. The terminal passes Ctrl-C as `stop`; the app passes a "stop tracking" signal.
pub async fn run(
    addr: &str,
    paths: Paths,
    report: Reporter,
    stop: impl Future<Output = ()>,
) -> Result<()> {
    let settings = Settings::load(&paths.settings)
        .with_context(|| format!("loading {}", paths.settings.display()))?;
    let store = Store::open(&paths.db)
        .with_context(|| format!("opening the database {}", paths.db.display()))?;
    let names = match &paths.playlist_map {
        Some(p) => PlaylistMap::load(p).unwrap_or_else(|_| PlaylistMap::with_builtin()),
        None => PlaylistMap::with_builtin(),
    };

    let (bus, writer_rx) = broadcast::channel::<StatsEvent>(BUS_CAPACITY);
    let writer = tokio::spawn(write_loop(
        writer_rx,
        settings,
        paths.settings,
        store,
        names,
        report.clone(),
    ));

    let status_report = report.clone();
    let mut raw = source::spawn_with_status(addr, move |connected| {
        status_report(if connected {
            TrackEvent::Connected
        } else {
            TrackEvent::Waiting
        });
    });
    // `stop` is a future we poll again and again in the loop, so it has to be pinned first.
    tokio::pin!(stop);
    loop {
        tokio::select! {
            msg = raw.recv() => {
                let Some(msg) = msg else { break };
                match parse_message(&msg) {
                    // `send` only fails when nobody is subscribed, which cannot happen
                    // while the writer runs.
                    Ok(event) => { let _ = bus.send(event); }
                    Err(err) => tracing::warn!("could not parse a message: {err}"),
                }
            }
            () = &mut stop => break,
        }
    }

    // Dropping the sender closes the bus; the writer then flushes and ends.
    drop(bus);
    writer.await.context("the writer task crashed")?;
    Ok(())
}

async fn write_loop(
    mut rx: broadcast::Receiver<StatsEvent>,
    settings: Settings,
    settings_path: PathBuf,
    mut store: Store,
    names: PlaylistMap,
    report: Reporter,
) {
    let mut tracker = Tracker::new(settings);
    let mut last_live: Option<Instant> = None;
    let mut live_shown = false;
    loop {
        let event = match rx.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!("the writer fell behind and missed {missed} events");
                continue;
            }
            Err(RecvError::Closed) => break,
        };

        let finished = tracker.on_event(&event);

        // A new match starts: pick up settings changed in the meantime (for example in the app).
        // The previous match was already closed above, with the settings it was played under.
        if matches!(event, StatsEvent::MatchCreated(_)) {
            match Settings::load(&settings_path) {
                Ok(fresh) => tracker.set_settings(fresh),
                Err(err) => tracing::warn!("could not re-read the settings: {err}"),
            }
        }

        if let Some(id) = tracker.take_learned_id() {
            // Load the file again before saving, so we only add the id and never overwrite
            // settings that changed while we were running.
            let mut saved = Settings::load(&settings_path).unwrap_or_default();
            saved.my_primary_id = Some(id);
            match saved.save(&settings_path) {
                Ok(()) => report(TrackEvent::PlayerIdLearned),
                Err(err) => report(TrackEvent::Error(format!(
                    "could not save your player id: {err}"
                ))),
            }
        }
        if let Some(m) = finished {
            save(&mut store, &m, &names, &report);
        }

        // The frequent events are throttled; everything else (goals, the end of the match, ...)
        // is passed on right away.
        let frequent = matches!(
            event,
            StatsEvent::UpdateState(_)
                | StatsEvent::BallHit(_)
                | StatsEvent::BoostPickup(_)
                | StatsEvent::ClockUpdatedSeconds(_)
        );
        if !frequent || last_live.is_none_or(|t| t.elapsed() >= LIVE_EVERY) {
            match tracker.live() {
                Some(snapshot) => {
                    report(TrackEvent::Live(Some(Box::new(snapshot))));
                    live_shown = true;
                    last_live = Some(Instant::now());
                }
                None if live_shown => {
                    report(TrackEvent::Live(None));
                    live_shown = false;
                }
                None => {}
            }
        }
    }

    if live_shown {
        report(TrackEvent::Live(None));
    }
    // Shutting down: save a match that already has a result.
    if let Some(m) = tracker.finish_pending(now_unix()) {
        save(&mut store, &m, &names, &report);
    }
}

/// Saves one match and reports it. (SQLite is quick here, one small transaction per match,
/// so blocking the async task for a moment is fine.)
fn save(store: &mut Store, m: &FinishedMatch, names: &PlaylistMap, report: &Reporter) {
    match store.save_match(m) {
        Ok(Some(id)) => report(TrackEvent::MatchSaved {
            id,
            summary: describe(m, names),
        }),
        Ok(None) => tracing::debug!("match was already saved"),
        Err(err) => report(TrackEvent::Error(format!(
            "could not save the match: {err}"
        ))),
    }
}

/// "Win 3-1 (Casual Standard)", with our own team's score first.
fn describe(m: &FinishedMatch, names: &PlaylistMap) -> String {
    let (mine, theirs) = if m.my_team == 0 {
        (m.team_goals[0], m.team_goals[1])
    } else {
        (m.team_goals[1], m.team_goals[0])
    };
    let result = match m.result {
        MatchResult::Win => "Win",
        MatchResult::Loss => "Loss",
        MatchResult::Abandoned => "Abandoned",
    };
    format!(
        "{result} {mine}-{theirs} ({})",
        names.display(m.playlist_id)
    )
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
