//! `reclutched map`: name playlist ids that aren't in the built-in table.
//!
//! The logic lives in [`Mapper`], which only turns events/typed lines into text to print,
//! so it can be unit tested without a game or a terminal.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Context, Result};
use reclutched_core::rocketleague::playlist::PlaylistMap;
use reclutched_core::rocketleague::{parse_message, StatsEvent};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::source;

/// What the main loop should do after handling something.
#[derive(Debug, Default, PartialEq)]
pub struct Step {
    /// Lines to print.
    pub lines: Vec<String>,
    /// True when we should stop (the user quit).
    pub finished: bool,
}

const HELP: &str = "Type a name for it (e.g. Ranked Solo Standard), or 'skip' / 'list' / 'quit'.";

pub struct Mapper {
    map: PlaylistMap,
    /// Where to save after each answer; `None` in tests.
    path: Option<PathBuf>,
    /// Playlist id and highest player count seen in the current match.
    current: Option<(i64, usize)>,
    /// The id we are asking about right now.
    pending: Option<i64>,
    /// Ids the user chose to skip, so we don't ask again this session.
    skipped: HashSet<i64>,
}

impl Mapper {
    pub fn new(map: PlaylistMap, path: Option<PathBuf>) -> Self {
        Self {
            map,
            path,
            current: None,
            pending: None,
            skipped: HashSet::new(),
        }
    }

    /// Text shown when starting.
    pub fn intro(&self) -> Step {
        Step {
            lines: vec![
                "Play as normal. If a match uses a playlist id I don't know, I'll ask you to name it when it ends."
                    .into(),
                "Commands any time: list (your saved names), skip, quit.".into(),
            ],
            finished: false,
        }
    }

    pub fn on_event(&mut self, event: &StatsEvent) -> Step {
        match event {
            StatsEvent::UpdateState(u) => {
                let id = u.game.playlist_id;
                let players = u.players.len();
                self.current = match self.current {
                    Some((cur, peak)) if cur == id => Some((id, peak.max(players))),
                    _ => Some((id, players)),
                };
                Step::default()
            }
            // The match is over (or we left it): time to ask, once, if we don't know this id.
            StatsEvent::MatchEnded(_) | StatsEvent::MatchDestroyed(_) => self.maybe_ask(),
            _ => Step::default(),
        }
    }

    fn maybe_ask(&mut self) -> Step {
        let Some((id, players)) = self.current else {
            return Step::default();
        };
        if self.pending.is_some() || self.map.name(id).is_some() || self.skipped.contains(&id) {
            return Step::default();
        }
        self.pending = Some(id);
        Step {
            lines: vec![
                String::new(),
                format!("Match finished with an unknown playlist id {id} (up to {players} players). What mode was it?"),
                HELP.into(),
            ],
            finished: false,
        }
    }

    /// Handles one line the user typed.
    pub fn on_input(&mut self, line: &str) -> Step {
        let line = line.trim();
        match line.to_lowercase().as_str() {
            "" => Step::default(),
            "quit" | "q" | "exit" => Step {
                lines: vec!["Bye. Your answers are saved.".into()],
                finished: true,
            },
            "list" => {
                let entries = self.map.user_entries();
                let mut lines = vec![format!("{} names saved by you:", entries.len())];
                lines.extend(entries.iter().map(|(id, name)| format!("  {id}: {name}")));
                Step {
                    lines,
                    finished: false,
                }
            }
            "skip" => match self.pending.take() {
                Some(id) => {
                    self.skipped.insert(id);
                    Step {
                        lines: vec![format!("Skipped playlist id {id}.")],
                        finished: false,
                    }
                }
                None => Step {
                    lines: vec!["Nothing to skip right now.".into()],
                    finished: false,
                },
            },
            _ => self.answer(line),
        }
    }

    /// Anything that isn't a command is taken as the mode name for the open question.
    fn answer(&mut self, name: &str) -> Step {
        let Some(id) = self.pending else {
            return Step {
                lines: vec!["No question open. Finish a match first, or type 'list'.".into()],
                finished: false,
            };
        };
        if name.len() > 60 {
            return Step {
                lines: vec!["That name is too long (60 characters max).".into()],
                finished: false,
            };
        }
        self.map.insert(id, name);
        self.pending = None;

        let mut lines = vec![format!("Saved: playlist id {id} = {name}")];
        if let Some(path) = &self.path {
            if let Err(err) = self.map.save(path) {
                lines.push(format!("WARNING: could not save {}: {err}", path.display()));
            }
        }
        Step {
            lines,
            finished: false,
        }
    }
}

fn print_step(step: &Step) {
    for line in &step.lines {
        println!("{line}");
    }
}

/// Runs the interactive helper until the user quits.
pub async fn run(addr: &str, path: PathBuf) -> Result<()> {
    let map = PlaylistMap::load(&path).with_context(|| format!("loading {}", path.display()))?;
    let mut mapper = Mapper::new(map, Some(path.clone()));

    print_step(&mapper.intro());
    println!("Saving names to {}", path.display());

    let mut events = source::spawn(addr);
    let mut input = BufReader::new(tokio::io::stdin()).lines();

    // `select!` waits for whichever happens first: a game message or a typed line.
    loop {
        let step = tokio::select! {
            msg = events.recv() => match msg {
                Some(raw) => match parse_message(&raw) {
                    Ok(event) => mapper.on_event(&event),
                    Err(err) => {
                        tracing::warn!("could not parse a message: {err}");
                        Step::default()
                    }
                },
                None => return Ok(()),
            },
            line = input.next_line() => match line? {
                Some(line) => mapper.on_input(&line),
                None => return Ok(()), // stdin closed
            },
        };
        print_step(&step);
        if step.finished {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reclutched_core::rocketleague::events::{
        BallState, Game, MatchEnded, MatchRef, Player, Team, UpdateState,
    };

    fn state(playlist_id: i64, players: usize) -> StatsEvent {
        let player = Player {
            name: "P".into(),
            primary_id: "Unknown|0|0".into(),
            shortcut: 1,
            team_num: 0,
            score: 0,
            goals: 0,
            shots: 0,
            assists: 0,
            saves: 0,
            touches: 0,
            car_touches: 0,
            demos: 0,
            loadout: vec![],
            has_car: None,
            speed: None,
            boost: None,
            boosting: None,
            on_ground: None,
            on_wall: None,
            powersliding: None,
            demolished: None,
            supersonic: None,
            pickup_class: None,
            attacker: None,
        };
        StatsEvent::UpdateState(UpdateState {
            match_guid: None,
            players: vec![player; players],
            game: Game {
                teams: Vec::<Team>::new(),
                playlist_id,
                time_seconds: 0,
                overtime: false,
                ball: BallState {
                    speed: 0.0,
                    team_num: 255,
                },
                replay: false,
                has_winner: false,
                winner: String::new(),
                arena: String::new(),
                has_target: false,
                target: None,
                frame: None,
                elapsed: None,
            },
        })
    }

    fn ended() -> StatsEvent {
        StatsEvent::MatchEnded(MatchEnded {
            match_guid: None,
            winner_team_num: 0,
        })
    }

    fn mapper() -> Mapper {
        Mapper::new(PlaylistMap::with_builtin(), None)
    }

    #[test]
    fn known_ids_never_ask() {
        let mut m = mapper();
        for id in [3, 8, 9] {
            m.on_event(&state(id, 4));
            assert_eq!(m.on_event(&ended()), Step::default());
        }
    }

    #[test]
    fn asks_about_unknown_id_and_saves_the_typed_name() {
        let mut m = mapper();
        m.on_event(&state(12, 1));
        m.on_event(&state(12, 2));
        let ask = m.on_event(&ended());
        assert!(ask
            .lines
            .iter()
            .any(|l| l.contains("playlist id 12 (up to 2 players)")));

        let reply = m.on_input("Ranked Solo Standard");
        assert_eq!(
            reply.lines[0],
            "Saved: playlist id 12 = Ranked Solo Standard"
        );
        assert_eq!(m.map.name(12), Some("Ranked Solo Standard"));

        // Known now: MatchDestroyed after MatchEnded does not ask twice.
        let destroyed = StatsEvent::MatchDestroyed(MatchRef { match_guid: None });
        assert_eq!(m.on_event(&destroyed), Step::default());
    }

    #[test]
    fn skip_means_dont_ask_again() {
        let mut m = mapper();
        m.on_event(&state(99, 5));
        m.on_event(&ended());
        m.on_input("skip");
        m.on_event(&state(99, 5));
        assert_eq!(m.on_event(&ended()), Step::default());
    }

    #[test]
    fn list_quit_and_stray_text() {
        let mut m = mapper();
        assert!(m.on_input("list").lines[0].contains("0 names saved"));
        assert!(m.on_input("some text").lines[0].contains("No question open"));
        assert!(m.on_input("quit").finished);
    }

    #[test]
    fn overlong_name_is_rejected_and_keeps_question_open() {
        let mut m = mapper();
        m.on_event(&state(12, 4));
        m.on_event(&ended());
        assert!(m.on_input(&"x".repeat(61)).lines[0].contains("too long"));
        assert!(m.pending.is_some());
    }
}
