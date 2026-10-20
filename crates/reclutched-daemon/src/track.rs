//! `reclutched track`: follows the game and saves every finished match to the database.
//!
//! Events flow like this:
//!
//!   websocket -> parse -> broadcast bus -> writer (tracker + SQLite)
//!
//! The bus is a tokio `broadcast` channel: every subscriber gets its own copy of each event.
//! Right now only the writer listens; the clipper and the overlay will subscribe here later.

use std::path::PathBuf;

use anyhow::{Context, Result};
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::settings::Settings;
use reclutched_core::rocketleague::storage::Store;
use reclutched_core::rocketleague::tracker::{FinishedMatch, MatchResult, Tracker};
use reclutched_core::rocketleague::{parse_message, StatsEvent};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::source;

/// How many events the bus holds for a slow subscriber before it starts losing the oldest.
const BUS_CAPACITY: usize = 4096;

pub struct Paths {
    pub db: PathBuf,
    pub settings: PathBuf,
    /// Optional user playlist names (for printing the mode name).
    pub playlist_map: Option<PathBuf>,
}

/// Runs until Ctrl-C, then saves the match in progress (if it was already decided) and returns.
pub async fn run(addr: &str, paths: Paths) -> Result<()> {
    let settings = Settings::load(&paths.settings)
        .with_context(|| format!("loading {}", paths.settings.display()))?;
    let store = Store::open(&paths.db)
        .with_context(|| format!("opening the database {}", paths.db.display()))?;
    let names = match &paths.playlist_map {
        Some(p) => PlaylistMap::load(p).unwrap_or_else(|_| PlaylistMap::with_builtin()),
        None => PlaylistMap::with_builtin(),
    };
    println!("Saving matches to {}", paths.db.display());
    println!("Waiting for Rocket League (Ctrl-C to stop)...");

    let (bus, writer_rx) = broadcast::channel::<StatsEvent>(BUS_CAPACITY);
    let writer = tokio::spawn(write_loop(
        writer_rx,
        settings,
        paths.settings,
        store,
        names,
    ));

    let mut raw = source::spawn(addr);
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
            _ = tokio::signal::ctrl_c() => break,
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
) {
    let mut tracker = Tracker::new(settings.clone());
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
        if let Some(id) = tracker.take_learned_id() {
            let mut saved = settings.clone();
            saved.my_primary_id = Some(id);
            match saved.save(&settings_path) {
                Ok(()) => println!("Learned your player id and saved it in the settings."),
                Err(err) => tracing::error!("could not save your player id: {err}"),
            }
        }
        if let Some(m) = finished {
            save(&mut store, &m, &names);
        }
    }

    // Shutting down: save a match that already has a result.
    if let Some(m) = tracker.finish_pending(now_unix()) {
        save(&mut store, &m, &names);
    }
}

/// Saves one match and tells the player. (SQLite is quick here, one small transaction per
/// match, so blocking the async task for a moment is fine.)
fn save(store: &mut Store, m: &FinishedMatch, names: &PlaylistMap) {
    match store.save_match(m) {
        Ok(Some(id)) => println!("Saved match #{id}: {}", describe(m, names)),
        Ok(None) => tracing::debug!("match was already saved"),
        Err(err) => tracing::error!("could not save the match: {err}"),
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
