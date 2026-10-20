//! Turns the stream of Stats API events into one summary per finished match.
//!
//! This is a plain state machine: no network, no database, no clock of its own. That keeps it
//! easy to test by feeding it saved events. Raw `UpdateState` snapshots are never kept; we only
//! remember the last stats per player and a few running totals.

use std::collections::HashMap;

use super::events::{PlayerRef, StatsEvent, UpdateState};
use super::playlist::is_real_match;
use super::settings::Settings;

/// The in-game clock when something happened.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Clock {
    /// Seconds remaining (it counts down).
    pub remaining: i64,
    pub overtime: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchResult {
    Win,
    Loss,
    /// We saw the match start but it never reached `MatchEnded` (left early, crash, disconnect).
    Abandoned,
}

impl MatchResult {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchResult::Win => "win",
            MatchResult::Loss => "loss",
            MatchResult::Abandoned => "abandoned",
        }
    }
}

/// Final numbers for one player. `slot` numbers the players of one match in the order we first
/// saw them; other rows refer to players by slot (names and ids are not unique: bots all share
/// `Unknown|0|0`, and the game reuses shortcut numbers for replacement players).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSummary {
    pub slot: usize,
    pub name: String,
    pub primary_id: Option<String>,
    pub team: i64,
    pub is_me: bool,
    pub score: i64,
    pub goals: i64,
    pub shots: i64,
    pub assists: i64,
    pub saves: i64,
    pub touches: i64,
    pub car_touches: i64,
    pub demos: i64,
    pub joined: Option<Clock>,
    pub left: Option<Clock>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GoalRow {
    pub clock: Clock,
    pub scorer: usize,
    pub assister: Option<usize>,
    pub goal_speed: f64,
    pub last_touch_speed: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Team 0 and team 1 goals right after this one (filled by the next state update).
    pub score_after: Option<[i64; 2]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatEventRow {
    pub clock: Clock,
    pub name: String,
    pub main: usize,
    pub secondary: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BallHitRow {
    pub clock: Clock,
    pub player: usize,
    pub pre_speed: f64,
    pub post_speed: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CrossbarRow {
    pub clock: Clock,
    pub player: usize,
    pub ball_speed: f64,
    pub impact_force: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoostPickupRow {
    pub clock: Clock,
    pub player: usize,
    /// "BoostType_Pad" (small) or "BoostType_Pill" (big).
    pub kind: String,
    pub amount: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Boost and speed habits of one player over the whole match. Only players the game gives
/// boost data for (you and your teammates) have one.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementSummary {
    pub slot: usize,
    pub samples: i64,
    pub avg_boost: f64,
    /// Percentages of samples, 0..=100.
    pub pct_boost_empty: f64,
    pub pct_boosting: f64,
    pub pct_supersonic: f64,
    pub pct_on_ground: f64,
    pub pct_on_wall: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinishedMatch {
    pub guid: Option<String>,
    pub playlist_id: i64,
    pub arena: String,
    /// Unix seconds.
    pub started_at: i64,
    pub ended_at: i64,
    pub my_slot: usize,
    pub my_team: i64,
    pub team_goals: [i64; 2],
    pub overtime: bool,
    pub result: MatchResult,
    /// Where the clock stood when an abandoned match was lost.
    pub abandoned_clock: Option<Clock>,
    pub players: Vec<PlayerSummary>,
    pub goals: Vec<GoalRow>,
    pub stat_events: Vec<StatEventRow>,
    pub ball_hits: Vec<BallHitRow>,
    pub crossbar_hits: Vec<CrossbarRow>,
    pub boost_pickups: Vec<BoostPickupRow>,
    pub movement: Vec<MovementSummary>,
}

pub struct Tracker {
    settings: Settings,
    active: Option<Active>,
    /// Our own player id, found when the first match shows who we are watching.
    learned_id: Option<String>,
}

impl Tracker {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            active: None,
            learned_id: None,
        }
    }

    /// Feeds one event; returns a finished match when this event completes one.
    pub fn on_event(&mut self, event: &StatsEvent) -> Option<FinishedMatch> {
        self.on_event_at(event, now_unix())
    }

    /// Same as [`Tracker::on_event`] with an explicit time (unix seconds), for tests.
    pub fn on_event_at(&mut self, event: &StatsEvent, now: i64) -> Option<FinishedMatch> {
        if !self.settings.matches {
            return None;
        }
        match event {
            StatsEvent::MatchCreated(m) => {
                // A new match means the previous one (if still open on its podium) is over.
                let done = self.finish_pending(now);
                let mut active = Active::new(now);
                active.note_guid(m.match_guid.as_deref());
                self.active = Some(active);
                done
            }
            StatsEvent::MatchDestroyed(_) => self.finish_pending(now),
            StatsEvent::UpdateState(u) => {
                // We may connect in the middle of a match, so a state update can start tracking.
                let active = self.active.get_or_insert_with(|| Active::new(now));
                if let Some(id) = active.on_update_state(u, &self.settings) {
                    self.learned_id = Some(id);
                }
                None
            }
            other => {
                if let Some(active) = self.active.as_mut() {
                    active.on_other(other, &self.settings);
                }
                None
            }
        }
    }

    /// Closes the match in progress (used when the program shuts down).
    pub fn finish_pending(&mut self, now: i64) -> Option<FinishedMatch> {
        let active = self.active.take()?;
        active.finish(&self.settings, now)
    }

    /// Your player id, the first time we learn it. The caller saves it into the settings.
    pub fn take_learned_id(&mut self) -> Option<String> {
        self.learned_id.take()
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Running boost/speed totals for one player.
#[derive(Default)]
struct MoveAcc {
    samples: i64,
    boost_sum: i64,
    empty: i64,
    boosting: i64,
    supersonic: i64,
    on_ground: i64,
    on_wall: i64,
}

struct PlayerAcc {
    shortcut: i64,
    name: String,
    primary_id: Option<String>,
    team: i64,
    /// (score, goals, shots, assists, saves, touches, car_touches, demos) from the latest update.
    stats: [i64; 8],
    joined: Option<Clock>,
    left: Option<Clock>,
    movement: MoveAcc,
}

/// The match we are currently following.
struct Active {
    started_at: i64,
    guid: Option<String>,
    playlist_id: Option<i64>,
    arena: String,
    clock: Clock,
    first_remaining: Option<i64>,
    /// True once the match really got going (clock moved, round started or a goal).
    play_seen: bool,
    overtime_seen: bool,
    /// Between GoalReplayStart and GoalReplayEnd.
    replay_events: bool,
    /// The game's own "a replay is showing" flag from the latest state update.
    replay_state: bool,
    /// We loaded a replay from the match history, or are watching someone else.
    dropped: bool,
    winner: Option<i64>,
    team_goals: [i64; 2],
    players: Vec<PlayerAcc>,
    pending_joins: HashMap<String, Clock>,
    my_slot: Option<usize>,
    goals: Vec<GoalRow>,
    awaiting_score: Option<usize>,
    stat_events: Vec<StatEventRow>,
    ball_hits: Vec<BallHitRow>,
    crossbar_hits: Vec<CrossbarRow>,
    boost_pickups: Vec<BoostPickupRow>,
}

impl Active {
    fn new(now: i64) -> Self {
        Self {
            started_at: now,
            guid: None,
            playlist_id: None,
            arena: String::new(),
            clock: Clock::default(),
            first_remaining: None,
            play_seen: false,
            overtime_seen: false,
            replay_events: false,
            replay_state: false,
            dropped: false,
            winner: None,
            team_goals: [0, 0],
            players: Vec::new(),
            pending_joins: HashMap::new(),
            my_slot: None,
            goals: Vec::new(),
            awaiting_score: None,
            stat_events: Vec::new(),
            ball_hits: Vec::new(),
            crossbar_hits: Vec::new(),
            boost_pickups: Vec::new(),
        }
    }

    fn note_guid(&mut self, guid: Option<&str>) {
        if self.guid.is_none() {
            self.guid = guid.map(str::to_string);
        }
    }

    fn in_replay(&self) -> bool {
        self.replay_events || self.replay_state
    }

    /// Whether to collect rows right now: a real match, not a replay, not dropped.
    fn collecting(&self) -> bool {
        !self.dropped && !self.in_replay() && self.playlist_id.is_none_or(is_real_match)
    }

    fn set_clock(&mut self, remaining: i64, overtime: bool) {
        self.clock = Clock {
            remaining,
            overtime,
        };
        self.overtime_seen |= overtime;
        match self.first_remaining {
            None => self.first_remaining = Some(remaining),
            Some(first) if first != remaining => self.play_seen = true,
            Some(_) => {}
        }
    }

    /// Finds the player with this shortcut and name, adding them if we haven't seen them yet.
    fn slot(&mut self, shortcut: i64, name: &str, team: i64) -> usize {
        if let Some(i) = self
            .players
            .iter()
            .position(|p| p.shortcut == shortcut && p.name == name)
        {
            return i;
        }
        let joined = self.pending_joins.remove(name);
        self.players.push(PlayerAcc {
            shortcut,
            name: name.to_string(),
            primary_id: None,
            team,
            stats: [0; 8],
            joined,
            left: None,
            movement: MoveAcc::default(),
        });
        self.players.len() - 1
    }

    fn slot_of(&mut self, r: &PlayerRef) -> usize {
        self.slot(r.shortcut, &r.name, r.team_num)
    }

    /// The most recent player with this name (join/leave events only carry a name).
    fn slot_by_name(&self, name: &str) -> Option<usize> {
        self.players.iter().rposition(|p| p.name == name)
    }

    /// Returns your player id the first time it is learned.
    fn on_update_state(&mut self, u: &UpdateState, settings: &Settings) -> Option<String> {
        let game = &u.game;
        self.note_guid(u.match_guid.as_deref());
        self.playlist_id = Some(game.playlist_id);
        self.arena.clone_from(&game.arena);
        self.replay_state = game.replay;
        self.set_clock(game.time_seconds, game.overtime);
        for t in &game.teams {
            if let Some(g) = usize::try_from(t.team_num)
                .ok()
                .and_then(|i| self.team_goals.get_mut(i))
            {
                *g = t.score;
            }
        }
        if let Some(idx) = self.awaiting_score.take() {
            self.goals[idx].score_after = Some(self.team_goals);
        }

        let collect_movement = settings.movement && self.collecting();
        for p in &u.players {
            let slot = self.slot(p.shortcut, &p.name, p.team_num);
            let acc = &mut self.players[slot];
            acc.primary_id = Some(p.primary_id.clone());
            acc.team = p.team_num;
            acc.stats = [
                p.score,
                p.goals,
                p.shots,
                p.assists,
                p.saves,
                p.touches,
                p.car_touches,
                p.demos,
            ];
            // Boost data is only sent for you and your teammates. Skip dead cars and replays.
            if let (true, Some(boost)) = (collect_movement, p.boost) {
                if p.has_car != Some(false) && p.demolished != Some(true) {
                    let m = &mut acc.movement;
                    m.samples += 1;
                    m.boost_sum += boost;
                    m.empty += i64::from(boost == 0);
                    m.boosting += i64::from(p.boosting == Some(true));
                    m.supersonic += i64::from(p.supersonic == Some(true));
                    m.on_ground += i64::from(p.on_ground == Some(true));
                    m.on_wall += i64::from(p.on_wall == Some(true));
                }
            }
        }

        // The player the game is "watching" is us (or, if we are spectating, someone else).
        let mut learned = None;
        if self.my_slot.is_none() && game.has_target {
            if let Some(t) = game.target.as_ref().filter(|t| !t.name.is_empty()) {
                let slot = self.slot(t.shortcut, &t.name, t.team_num);
                self.my_slot = Some(slot);
                let id = self.players[slot].primary_id.clone();
                match (&settings.my_primary_id, id) {
                    (Some(mine), Some(id)) if *mine != id => self.dropped = true,
                    (None, Some(id)) if !id.starts_with("Unknown") => learned = Some(id),
                    _ => {}
                }
            }
        }
        learned
    }

    fn on_other(&mut self, event: &StatsEvent, settings: &Settings) {
        match event {
            StatsEvent::ClockUpdatedSeconds(c) => {
                self.note_guid(c.match_guid.as_deref());
                self.set_clock(c.time_seconds, c.overtime);
            }
            StatsEvent::RoundStarted(_) => {
                self.play_seen = true;
                self.replay_events = false;
            }
            StatsEvent::CountdownBegin(_) => self.replay_events = false,
            StatsEvent::GoalReplayStart(_) => self.replay_events = true,
            StatsEvent::GoalReplayEnd(_) => self.replay_events = false,
            StatsEvent::ReplayCreated(_) => self.dropped = true,
            StatsEvent::MatchEnded(m) => {
                self.note_guid(m.match_guid.as_deref());
                self.winner = Some(m.winner_team_num);
                self.play_seen = true;
            }
            StatsEvent::PlayerJoined(p) => match self.slot_by_name(&p.player_name) {
                Some(i) => {
                    let clock = self.clock;
                    self.players[i].joined.get_or_insert(clock);
                }
                None => {
                    self.pending_joins.insert(p.player_name.clone(), self.clock);
                }
            },
            StatsEvent::PlayerLeft(p) => {
                if let Some(i) = self.slot_by_name(&p.player_name) {
                    let clock = self.clock;
                    self.players[i].left.get_or_insert(clock);
                }
            }
            StatsEvent::GoalScored(g) if self.collecting() && !g.scorer.name.is_empty() => {
                self.play_seen = true;
                if settings.goals {
                    let scorer = self.slot_of(&g.scorer);
                    let assister = g.assister.as_ref().map(|a| self.slot_of(a));
                    self.goals.push(GoalRow {
                        clock: self.clock,
                        scorer,
                        assister,
                        goal_speed: g.goal_speed,
                        last_touch_speed: g.ball_last_touch.speed,
                        x: g.impact_location.x,
                        y: g.impact_location.y,
                        z: g.impact_location.z,
                        score_after: None,
                    });
                    self.awaiting_score = Some(self.goals.len() - 1);
                }
            }
            StatsEvent::StatfeedEvent(s) if self.collecting() && settings.stat_events => {
                let main = self.slot_of(&s.main_target);
                let secondary = s.secondary_target.as_ref().map(|t| self.slot_of(t));
                self.stat_events.push(StatEventRow {
                    clock: self.clock,
                    name: s.event_name.clone(),
                    main,
                    secondary,
                });
            }
            StatsEvent::BallHit(b) if self.collecting() && settings.ball_hits => {
                for hitter in &b.players {
                    let player = self.slot_of(hitter);
                    self.ball_hits.push(BallHitRow {
                        clock: self.clock,
                        player,
                        pre_speed: b.ball.pre_hit_speed,
                        post_speed: b.ball.post_hit_speed,
                        x: b.ball.location.x,
                        y: b.ball.location.y,
                        z: b.ball.location.z,
                    });
                }
            }
            StatsEvent::CrossbarHit(c) if self.collecting() && settings.crossbar_hits => {
                let player = self.slot_of(&c.ball_last_touch.player);
                self.crossbar_hits.push(CrossbarRow {
                    clock: self.clock,
                    player,
                    ball_speed: c.ball_speed,
                    impact_force: c.impact_force,
                    x: c.ball_location.x,
                    y: c.ball_location.y,
                    z: c.ball_location.z,
                });
            }
            StatsEvent::BoostPickup(b)
                if self.collecting() && !b.replay && settings.boost_pickups =>
            {
                let player = self.slot_of(&b.player);
                self.boost_pickups.push(BoostPickupRow {
                    clock: self.clock,
                    player,
                    kind: b.boost_type.clone(),
                    amount: b.boost_amount,
                    x: b.location.x,
                    y: b.location.y,
                    z: b.location.z,
                });
            }
            _ => {}
        }
    }

    /// Builds the summary, or `None` when this wasn't a match we should record.
    fn finish(self, settings: &Settings, now: i64) -> Option<FinishedMatch> {
        let playlist_id = self.playlist_id?;
        if self.dropped || !is_real_match(playlist_id) || self.players.len() < 2 {
            return None;
        }
        if self.guid.is_none() && !settings.offline_matches {
            return None;
        }
        // Without knowing who we are we can't say win or loss.
        let my_slot = self.my_slot?;
        let my_team = self.players[my_slot].team;
        let (result, abandoned_clock) = match self.winner {
            Some(w) if w == my_team => (MatchResult::Win, None),
            Some(_) => (MatchResult::Loss, None),
            None if self.play_seen => (MatchResult::Abandoned, Some(self.clock)),
            None => return None,
        };

        let players = self
            .players
            .iter()
            .enumerate()
            .map(|(slot, p)| player_summary(slot, p, slot == my_slot, settings.roster))
            .collect();
        let movement = if settings.movement {
            self.players
                .iter()
                .enumerate()
                .filter_map(|(slot, p)| movement_summary(slot, &p.movement))
                .collect()
        } else {
            Vec::new()
        };

        Some(FinishedMatch {
            guid: self.guid,
            playlist_id,
            arena: self.arena,
            started_at: self.started_at,
            ended_at: now,
            my_slot,
            my_team,
            team_goals: self.team_goals,
            overtime: self.overtime_seen,
            result,
            abandoned_clock,
            players,
            goals: self.goals,
            stat_events: self.stat_events,
            ball_hits: self.ball_hits,
            crossbar_hits: self.crossbar_hits,
            boost_pickups: self.boost_pickups,
            movement,
        })
    }
}

/// With the roster switch off we keep only slot, team and "is me": enough for other rows to
/// refer to, but no names, ids, stats or join/leave times.
fn player_summary(slot: usize, p: &PlayerAcc, is_me: bool, roster: bool) -> PlayerSummary {
    let [score, goals, shots, assists, saves, touches, car_touches, demos] =
        if roster { p.stats } else { [0; 8] };
    PlayerSummary {
        slot,
        name: if roster {
            p.name.clone()
        } else {
            String::new()
        },
        primary_id: if roster { p.primary_id.clone() } else { None },
        team: p.team,
        is_me,
        score,
        goals,
        shots,
        assists,
        saves,
        touches,
        car_touches,
        demos,
        joined: if roster { p.joined } else { None },
        left: if roster { p.left } else { None },
    }
}

fn movement_summary(slot: usize, m: &MoveAcc) -> Option<MovementSummary> {
    if m.samples == 0 {
        return None;
    }
    let n = m.samples as f64;
    let pct = |count: i64| count as f64 * 100.0 / n;
    Some(MovementSummary {
        slot,
        samples: m.samples,
        avg_boost: m.boost_sum as f64 / n,
        pct_boost_empty: pct(m.empty),
        pct_boosting: pct(m.boosting),
        pct_supersonic: pct(m.supersonic),
        pct_on_ground: pct(m.on_ground),
        pct_on_wall: pct(m.on_wall),
    })
}
