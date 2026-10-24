//! Decides when an automatic clip is made: your goals, saves and demolitions, and the end of
//! the match, from the live feed of the game. Plain code, tested without a game.

use std::collections::HashSet;

use reclutched_core::rocketleague::settings::Settings;
use reclutched_core::rocketleague::tracker::{LiveKind, LiveSnapshot};

/// Why a clip was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// You pressed the clip key.
    Key,
    Goal,
    Save,
    Demo,
    MatchEnd,
}

impl Trigger {
    /// The word stored with the clip.
    pub fn label(self) -> &'static str {
        match self {
            Trigger::Key => "key",
            Trigger::Goal => "goal",
            Trigger::Save => "save",
            Trigger::Demo => "demo",
            Trigger::MatchEnd => "match end",
        }
    }

    /// A few seconds after the moment are kept too, so a goal's explosion is in the clip.
    pub fn tail_seconds(self) -> u32 {
        match self {
            Trigger::Key => 0,
            _ => TAIL_SECONDS,
        }
    }
}

/// Seconds recorded after an automatic trigger before the clip is cut.
pub const TAIL_SECONDS: u32 = 3;

/// Whether automatic clips are on at all.
pub fn auto_on(settings: &Settings) -> bool {
    matches!(settings.clip_mode.as_str(), "auto" | "both")
}

/// Whether the clip key does anything.
pub fn key_on(settings: &Settings) -> bool {
    matches!(settings.clip_mode.as_str(), "manual" | "both")
}

type EventKey = (i64, bool, u8, String, i64);

/// Follows the live feed of one match at a time.
#[derive(Default)]
pub struct Watcher {
    seq: Option<u64>,
    /// Events we have already looked at.
    seen: HashSet<EventKey>,
    /// The first snapshot of a match only learns what is already in the feed.
    primed: bool,
    ended: bool,
}

fn kind_number(kind: LiveKind) -> u8 {
    match kind {
        LiveKind::Goal => 0,
        LiveKind::Save => 1,
        LiveKind::EpicSave => 2,
        LiveKind::Demolish => 3,
    }
}

impl Watcher {
    /// Whether this snapshot is the first of a new match (the caller marks where it starts).
    pub fn is_new_match(&self, snapshot: &LiveSnapshot) -> bool {
        self.seq != Some(snapshot.seq)
    }

    /// The triggers that happened since the last snapshot.
    pub fn update(&mut self, snapshot: &LiveSnapshot, settings: &Settings) -> Vec<Trigger> {
        if self.is_new_match(snapshot) {
            self.seq = Some(snapshot.seq);
            self.seen.clear();
            self.primed = false;
            self.ended = false;
        }
        let me = snapshot
            .players
            .iter()
            .find(|p| p.is_me)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let mut found = Vec::new();
        for event in &snapshot.feed {
            let key = (
                event.clock.remaining,
                event.clock.overtime,
                kind_number(event.kind),
                event.name.clone(),
                event.team,
            );
            if !self.seen.insert(key) || !self.primed || me.is_empty() || event.name != me {
                continue;
            }
            let trigger = match event.kind {
                LiveKind::Goal if settings.clip_on_goal => Trigger::Goal,
                LiveKind::Save | LiveKind::EpicSave if settings.clip_on_save => Trigger::Save,
                LiveKind::Demolish if settings.clip_on_demo => Trigger::Demo,
                _ => continue,
            };
            found.push(trigger);
        }
        self.primed = true;
        if snapshot.winner.is_some() && !self.ended {
            self.ended = true;
            if settings.clip_on_match_end {
                found.push(Trigger::MatchEnd);
            }
        }
        if auto_on(settings) {
            found
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reclutched_core::rocketleague::tracker::{Clock, LiveEvent, LivePlayer};

    fn player(name: &str, me: bool) -> LivePlayer {
        LivePlayer {
            name: name.to_string(),
            team: 0,
            is_me: me,
            score: 0,
            goals: 0,
            saves: 0,
            shots: 0,
            demos: 0,
            boost: None,
            speed: None,
            demolished: false,
            left: false,
        }
    }

    fn event(kind: LiveKind, name: &str, remaining: i64) -> LiveEvent {
        LiveEvent {
            clock: Clock {
                remaining,
                overtime: false,
            },
            kind,
            team: 0,
            name: name.to_string(),
            other: None,
        }
    }

    fn snapshot(seq: u64, feed: Vec<LiveEvent>, winner: Option<i64>) -> LiveSnapshot {
        LiveSnapshot {
            seq,
            playlist_id: 3,
            clock: Clock::default(),
            team_goals: [0, 0],
            team_names: ["Blue".into(), "Orange".into()],
            my_team: Some(0),
            replay: false,
            winner,
            players: vec![player("Zed", true), player("Nova", false)],
            possession: None,
            ball_speed: 0.0,
            me_movement: None,
            feed,
        }
    }

    fn settings() -> Settings {
        Settings {
            clip_mode: "auto".into(),
            ..Settings::default()
        }
    }

    #[test]
    fn only_your_new_events_make_a_clip() {
        let mut w = Watcher::default();
        let s = settings();
        // The first snapshot of the match only learns what was already there.
        assert!(w
            .update(
                &snapshot(1, vec![event(LiveKind::Goal, "Zed", 250)], None),
                &s
            )
            .is_empty());
        // The same feed again: nothing new.
        assert!(w
            .update(
                &snapshot(1, vec![event(LiveKind::Goal, "Zed", 250)], None),
                &s
            )
            .is_empty());
        let feed = vec![
            event(LiveKind::Goal, "Zed", 250),
            event(LiveKind::Goal, "Nova", 200), // a teammate: not yours
            event(LiveKind::Save, "Zed", 180),
            event(LiveKind::Demolish, "Zed", 150),
        ];
        assert_eq!(
            w.update(&snapshot(1, feed, None), &s),
            [Trigger::Save, Trigger::Demo]
        );
    }

    #[test]
    fn the_switches_choose_what_counts() {
        let mut w = Watcher::default();
        let s = Settings {
            clip_on_goal: false,
            clip_on_save: true,
            clip_on_demo: false,
            ..settings()
        };
        w.update(&snapshot(1, vec![], None), &s);
        let feed = vec![
            event(LiveKind::Goal, "Zed", 250),
            event(LiveKind::EpicSave, "Zed", 200),
            event(LiveKind::Demolish, "Zed", 150),
        ];
        assert_eq!(w.update(&snapshot(1, feed, None), &s), [Trigger::Save]);
    }

    #[test]
    fn the_end_of_the_match_comes_once_and_a_new_match_starts_fresh() {
        let mut w = Watcher::default();
        let s = Settings {
            clip_on_match_end: true,
            ..settings()
        };
        w.update(&snapshot(1, vec![], None), &s);
        assert_eq!(
            w.update(&snapshot(1, vec![], Some(0)), &s),
            [Trigger::MatchEnd]
        );
        assert!(w.update(&snapshot(1, vec![], Some(0)), &s).is_empty());
        let next = snapshot(2, vec![], None);
        assert!(w.is_new_match(&next));
        w.update(&next, &s);
        assert_eq!(
            w.update(&snapshot(2, vec![], Some(1)), &s),
            [Trigger::MatchEnd]
        );
    }

    #[test]
    fn manual_mode_makes_no_automatic_clips() {
        let mut w = Watcher::default();
        let s = Settings {
            clip_mode: "manual".into(),
            ..settings()
        };
        w.update(&snapshot(1, vec![], None), &s);
        let feed = vec![event(LiveKind::Goal, "Zed", 250)];
        assert!(w.update(&snapshot(1, feed, None), &s).is_empty());
        assert!(key_on(&s) && !auto_on(&s));
        assert!(key_on(&Settings {
            clip_mode: "both".into(),
            ..s.clone()
        }));
        assert!(!key_on(&Settings {
            clip_mode: "auto".into(),
            ..s
        }));
    }

    #[test]
    fn automatic_clips_keep_a_few_seconds_after_the_moment() {
        assert_eq!(Trigger::Key.tail_seconds(), 0);
        assert_eq!(Trigger::Goal.tail_seconds(), TAIL_SECONDS);
        assert_eq!(Trigger::MatchEnd.label(), "match end");
    }
}
