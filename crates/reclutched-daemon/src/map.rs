//! `reclutched map`: learn which `PlaylistId` is which game mode by asking the player.
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
    /// True when we should stop (everything mapped, or the user quit).
    pub finished: bool,
}

pub struct Mapper {
    map: PlaylistMap,
    /// Where to save after each answer; `None` in tests.
    path: Option<PathBuf>,
    /// Playlist id and highest player count seen in the current match.
    current: Option<(i64, usize)>,
    /// Ids we are asking about right now.
    pending: Option<(i64, usize)>,
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
        let left = self.map.remaining().len();
        if left == 0 {
            return Step {
                lines: vec!["Done! Every game mode is already mapped.".into()],
                finished: true,
            };
        }
        Step {
            lines: vec![
                format!("{left} game modes left to map. Play a match; when it ends I'll ask what it was."),
                "Commands any time: list, skip, quit.".into(),
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
        self.pending = Some((id, players));
        let mut lines = vec![
            String::new(),
            format!(
                "Match finished (playlist id {id}, up to {players} players). What did you play?"
            ),
        ];
        lines.extend(self.numbered_list());
        lines.push("Type a number, or 'skip' / 'list' / 'quit'.".into());
        Step {
            lines,
            finished: false,
        }
    }

    fn numbered_list(&self) -> Vec<String> {
        self.map
            .remaining()
            .iter()
            .enumerate()
            .map(|(i, mode)| format!("  {}) {mode}", i + 1))
            .collect()
    }

    /// Handles one line the user typed.
    pub fn on_input(&mut self, line: &str) -> Step {
        let line = line.trim().to_lowercase();
        match line.as_str() {
            "" => Step::default(),
            "quit" | "q" | "exit" => Step {
                lines: vec!["Bye. Your answers are saved.".into()],
                finished: true,
            },
            "list" => {
                let mut lines = vec![format!("{} modes left to map:", self.map.remaining().len())];
                lines.extend(self.numbered_list());
                Step {
                    lines,
                    finished: false,
                }
            }
            "skip" => match self.pending.take() {
                Some((id, _)) => {
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
            other => match other.parse::<usize>() {
                Ok(n) => self.answer(n),
                Err(_) => Step {
                    lines: vec!["Type a number, or 'skip' / 'list' / 'quit'.".into()],
                    finished: false,
                },
            },
        }
    }

    fn answer(&mut self, n: usize) -> Step {
        let Some((id, _)) = self.pending else {
            return Step {
                lines: vec!["No question open. Finish a match first, or type 'list'.".into()],
                finished: false,
            };
        };
        let remaining = self.map.remaining();
        let Some(mode) = n.checked_sub(1).and_then(|i| remaining.get(i)) else {
            return Step {
                lines: vec![format!("Pick a number from 1 to {}.", remaining.len())],
                finished: false,
            };
        };
        self.map.insert(id, mode);
        self.pending = None;

        let mut lines = vec![format!("Saved: playlist id {id} = {mode}")];
        if let Some(path) = &self.path {
            if let Err(err) = self.map.save(path) {
                lines.push(format!("WARNING: could not save {}: {err}", path.display()));
            }
        }
        let left = self.map.remaining().len();
        if left == 0 {
            lines.push("Done!".into());
        } else {
            lines.push(format!("{left} left."));
        }
        Step {
            lines,
            finished: left == 0,
        }
    }
}

fn print_step(step: &Step) {
    for line in &step.lines {
        println!("{line}");
    }
}

/// Runs the interactive mapper until everything is mapped or the user quits.
pub async fn run(addr: &str, path: PathBuf) -> Result<()> {
    let map = PlaylistMap::load(&path).with_context(|| format!("loading {}", path.display()))?;
    let mut mapper = Mapper::new(map, Some(path.clone()));

    let first = mapper.intro();
    print_step(&first);
    if first.finished {
        return Ok(());
    }
    println!("Saving answers to {}", path.display());

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
    fn asks_after_match_end_and_saves_the_answer() {
        let mut m = mapper();
        m.on_event(&state(3, 4));
        m.on_event(&state(3, 6));
        let ask = m.on_event(&ended());
        assert!(ask
            .lines
            .iter()
            .any(|l| l.contains("playlist id 3, up to 6 players")));

        let reply = m.on_input("3"); // 3rd item = "Casual 3v3"
        assert!(reply.lines[0].contains("playlist id 3 = Casual 3v3"));
        assert_eq!(m.map.name(3), Some("Casual 3v3"));

        // Known now: the same match ending again (MatchDestroyed) does not ask twice.
        assert_eq!(
            m.on_event(&StatsEvent::MatchDestroyed(MatchRef { match_guid: None })),
            Step::default()
        );
    }

    #[test]
    fn skip_means_dont_ask_again() {
        let mut m = mapper();
        m.on_event(&state(8, 5));
        m.on_event(&ended());
        m.on_input("skip");
        m.on_event(&state(8, 5));
        assert_eq!(m.on_event(&ended()), Step::default());
    }

    #[test]
    fn list_quit_and_bad_input() {
        let mut m = mapper();
        assert!(m.on_input("list").lines[0].contains("modes left"));
        assert!(m.on_input("5").lines[0].contains("No question open"));
        assert!(m.on_input("banana").lines[0].contains("Type a number"));
        assert!(m.on_input("quit").finished);
    }

    #[test]
    fn out_of_range_number_is_rejected_and_keeps_question_open() {
        let mut m = mapper();
        m.on_event(&state(3, 4));
        m.on_event(&ended());
        assert!(m.on_input("999").lines[0].contains("Pick a number"));
        assert!(m.on_input("0").lines[0].contains("Pick a number"));
        assert!(m.pending.is_some());
    }

    #[test]
    fn finishes_with_done_when_everything_is_mapped() {
        let mut m = mapper();
        let total = m.map.remaining().len();
        let mut last = Step::default();
        for id in 0..total as i64 {
            m.on_event(&state(100 + id, 2));
            m.on_event(&ended());
            last = m.on_input("1");
        }
        assert!(last.finished);
        assert!(last.lines.iter().any(|l| l == "Done!"));
    }
}
