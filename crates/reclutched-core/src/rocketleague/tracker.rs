//! Turns the stream of Stats API events into one summary per finished match.
//!
//! This is a plain state machine: no network, no database, no clock of its own. That keeps it
//! easy to test by feeding it saved events. Raw `UpdateState` snapshots are never kept; we only
//! remember the last stats per player and a few running totals.

use std::collections::{BTreeMap, HashMap};

use super::events::{Player, PlayerRef, StatsEvent, UpdateState};
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
    /// The scorer's boost and car speed just before the goal (when the game sends them).
    pub scorer_boost: Option<i64>,
    pub scorer_speed: Option<f64>,
    /// Team 0 and team 1 goals right after this one (filled by the next state update).
    pub score_after: Option<[i64; 2]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatEventRow {
    pub clock: Clock,
    pub name: String,
    pub main: usize,
    pub secondary: Option<usize>,
    /// The main player's boost and car speed when it happened (when the game sends them).
    pub boost: Option<i64>,
    pub speed: Option<f64>,
}

/// One kind of stat event for one player, as counts and averages (see [`StatEventRow`] for
/// the individual events).
#[derive(Debug, Clone, PartialEq)]
pub struct EventStat {
    pub slot: usize,
    pub event: String,
    pub count: i64,
    pub avg_boost: Option<f64>,
    pub avg_speed: Option<f64>,
}

/// Ball touches of one player. The field is split in thirds from the player's own point of view.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchSummary {
    pub slot: usize,
    pub touches: i64,
    pub def_third: i64,
    pub mid_third: i64,
    pub att_third: i64,
    pub avg_post_speed: f64,
    pub max_post_speed: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CrossbarSummary {
    pub slot: usize,
    pub count: i64,
    pub max_impact: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickupSummary {
    pub slot: usize,
    pub pads: i64,
    pub pills: i64,
    pub amount: f64,
}

/// Boost and speed habits of one player over the live part of the match. Only players the game
/// gives boost data for (you and your teammates) have one. Percentages are 0..=100 and do not
/// depend on how often the game sends updates.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementSummary {
    pub slot: usize,
    /// Live ticks with a working car.
    pub samples: i64,
    pub avg_boost: f64,
    /// Share of time with no boost, a little (1-33), some (34-66) and a lot (67-100).
    pub pct_boost_empty: f64,
    pub pct_boost_low: f64,
    pub pct_boost_mid: f64,
    pub pct_boost_high: f64,
    pub pct_boosting: f64,
    /// Total boost spent and picked up (in boost units, 0-100 per tank).
    pub boost_used: i64,
    pub boost_gained: i64,
    /// Car speed in km/h (what the game sends).
    pub avg_speed: f64,
    pub max_speed: f64,
    pub pct_supersonic: f64,
    pub pct_powersliding: f64,
    pub pct_on_ground: f64,
    pub pct_on_wall: f64,
    pub pct_airborne: f64,
    /// Share of live time spent demolished, and how many times it happened.
    pub pct_demolished: f64,
    pub times_demolished: i64,
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
    /// Share of the time your team last touched the ball (time nobody touched it is ignored).
    pub my_possession_pct: Option<f64>,
    pub ball_avg_speed: Option<f64>,
    pub ball_max_speed: Option<f64>,
    /// Share of your live time when you AND a teammate both had no boost.
    pub team_empty_overlap: Option<f64>,
    pub players: Vec<PlayerSummary>,
    pub goals: Vec<GoalRow>,
    pub stat_events: Vec<StatEventRow>,
    pub event_stats: Vec<EventStat>,
    pub touches: Vec<TouchSummary>,
    pub crossbar: Vec<CrossbarSummary>,
    pub pickups: Vec<PickupSummary>,
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
                    // Remember it right away so later matches can tell when we are spectating.
                    self.settings.my_primary_id = Some(id.clone());
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
    /// Live ticks where the game sent boost data for this player.
    ticks: i64,
    /// Of those, ticks with a working car.
    alive: i64,
    boost_sum: i64,
    /// Ticks with boost 0, 1-33, 34-66, 67-100.
    bands: [i64; 4],
    boosting: i64,
    speed_sum: f64,
    speed_max: f64,
    supersonic: i64,
    powersliding: i64,
    on_ground: i64,
    on_wall: i64,
    airborne: i64,
    boost_used: i64,
    boost_gained: i64,
    /// Boost on the previous live tick; `None` after a gap (countdown, replay, demolished) so
    /// the jump back to the starting 33 boost isn't counted as a pickup.
    last_boost: Option<i64>,
    was_dead: bool,
    times_demolished: i64,
}

#[derive(Default)]
struct TouchAcc {
    touches: i64,
    thirds: [i64; 3],
    speed_sum: f64,
    speed_max: f64,
}

#[derive(Default)]
struct EventAcc {
    count: i64,
    boost_sum: i64,
    boost_n: i64,
    speed_sum: f64,
    speed_n: i64,
}

#[derive(Default)]
struct PickupAcc {
    pads: i64,
    pills: i64,
    amount: f64,
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
    /// Latest boost and speed outside replays, used as context for goals and stat events.
    last_boost: Option<i64>,
    last_speed: Option<f64>,
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
    /// True while the ball is actually in play (after RoundStarted until a goal or the end).
    /// Countdowns and replays are left out of every average.
    live: bool,
    /// Once we have seen a countdown/round/goal we trust those events; before that (we joined
    /// the match half way) we assume play is live as soon as the clock moves.
    phase_known: bool,
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
    event_acc: BTreeMap<(usize, String), EventAcc>,
    touches: BTreeMap<usize, TouchAcc>,
    crossbar: BTreeMap<usize, (i64, f64)>,
    pickups: BTreeMap<usize, PickupAcc>,
    /// Live ticks in which team 0 / team 1 touched the ball last.
    possession_ticks: [i64; 2],
    ball_ticks: i64,
    ball_speed_sum: f64,
    ball_speed_max: f64,
    my_ticks: i64,
    overlap_ticks: i64,
    teammate_seen: bool,
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
            live: false,
            phase_known: false,
            dropped: false,
            winner: None,
            team_goals: [0, 0],
            players: Vec::new(),
            pending_joins: HashMap::new(),
            my_slot: None,
            goals: Vec::new(),
            awaiting_score: None,
            stat_events: Vec::new(),
            event_acc: BTreeMap::new(),
            touches: BTreeMap::new(),
            crossbar: BTreeMap::new(),
            pickups: BTreeMap::new(),
            possession_ticks: [0, 0],
            ball_ticks: 0,
            ball_speed_sum: 0.0,
            ball_speed_max: 0.0,
            my_ticks: 0,
            overlap_ticks: 0,
            teammate_seen: false,
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

    /// Forget the previous boost of every player. Called whenever play starts or stops, so the
    /// jump to the kickoff boost (or back from a demolition) is never counted as a pickup.
    fn reset_boost_history(&mut self) {
        for p in &mut self.players {
            p.movement.last_boost = None;
            p.movement.was_dead = false;
        }
    }

    /// Whether to collect rows right now: a real match, not a replay, not dropped.
    fn collecting(&self) -> bool {
        !self.dropped && !self.in_replay() && self.playlist_id.is_none_or(is_real_match)
    }

    /// Whether to add to the per-tick averages: collecting, and the ball is in play.
    fn sampling(&self) -> bool {
        self.collecting() && self.live
    }

    fn set_clock(&mut self, remaining: i64, overtime: bool) {
        self.clock = Clock {
            remaining,
            overtime,
        };
        self.overtime_seen |= overtime;
        match self.first_remaining {
            None => self.first_remaining = Some(remaining),
            Some(first) if first != remaining => {
                self.play_seen = true;
                if !self.phase_known && !self.in_replay() {
                    self.live = true;
                }
            }
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
            last_boost: None,
            last_speed: None,
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

        let mut slots = Vec::with_capacity(u.players.len());
        for p in &u.players {
            let slot = self.slot(p.shortcut, &p.name, p.team_num);
            slots.push(slot);
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
            // Replays show other cars' values, so only remember live ones as "context".
            if !game.replay {
                acc.last_boost = boost_of(p);
                acc.last_speed = p.speed;
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

        if self.sampling() {
            if settings.movement {
                self.sample_movement(u, &slots);
            }
            if settings.possession {
                self.sample_ball(game.ball.team_num, game.ball.speed);
            }
        } else {
            self.reset_boost_history();
        }
        learned
    }

    /// Adds one live tick to every player the game sends boost data for.
    fn sample_movement(&mut self, u: &UpdateState, slots: &[usize]) {
        let my = self.my_slot;
        let my_team = my.map(|s| self.players[s].team);
        let (mut me_empty, mut me_alive, mut mate_empty) = (false, false, false);

        for (p, &slot) in u.players.iter().zip(slots) {
            let Some(boost) = boost_of(p) else { continue };
            let dead =
                p.has_car == Some(false) || p.demolished == Some(true) || p.attacker.is_some();
            let m = &mut self.players[slot].movement;
            m.ticks += 1;
            if dead {
                if !m.was_dead {
                    m.times_demolished += 1;
                }
                m.was_dead = true;
                m.last_boost = None;
                continue;
            }
            m.was_dead = false;
            m.alive += 1;
            m.boost_sum += boost;
            let band = match boost {
                0 => 0,
                1..=33 => 1,
                34..=66 => 2,
                _ => 3,
            };
            m.bands[band] += 1;
            m.boosting += i64::from(p.boosting == Some(true));
            if let Some(speed) = p.speed {
                m.speed_sum += speed;
                m.speed_max = m.speed_max.max(speed);
            }
            m.supersonic += i64::from(p.supersonic == Some(true));
            m.powersliding += i64::from(p.powersliding == Some(true));
            m.on_ground += i64::from(p.on_ground == Some(true));
            m.on_wall += i64::from(p.on_wall == Some(true));
            // The game omits false flags, so "not on the ground and not on a wall" is the air.
            m.airborne += i64::from(p.on_ground != Some(true) && p.on_wall != Some(true));
            if let Some(prev) = m.last_boost {
                if boost > prev {
                    m.boost_gained += boost - prev;
                } else {
                    m.boost_used += prev - boost;
                }
            }
            m.last_boost = Some(boost);

            if Some(slot) == my {
                me_alive = true;
                me_empty = boost == 0;
            } else if Some(p.team_num) == my_team {
                self.teammate_seen = true;
                mate_empty |= boost == 0;
            }
        }
        if me_alive {
            self.my_ticks += 1;
            self.overlap_ticks += i64::from(me_empty && mate_empty);
        }
    }

    fn sample_ball(&mut self, last_touch_team: i64, speed: f64) {
        self.ball_ticks += 1;
        self.ball_speed_sum += speed;
        self.ball_speed_max = self.ball_speed_max.max(speed);
        // 255 means nobody has touched the ball yet.
        if let Some(t) = usize::try_from(last_touch_team)
            .ok()
            .and_then(|t| self.possession_ticks.get_mut(t))
        {
            *t += 1;
        }
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
                self.live = true;
                self.phase_known = true;
                self.reset_boost_history();
            }
            StatsEvent::CountdownBegin(_) => {
                self.replay_events = false;
                self.live = false;
                self.phase_known = true;
                self.reset_boost_history();
            }
            StatsEvent::GoalReplayStart(_) => {
                self.replay_events = true;
                self.live = false;
                self.phase_known = true;
                self.reset_boost_history();
            }
            StatsEvent::GoalReplayEnd(_) => self.replay_events = false,
            StatsEvent::ReplayCreated(_) => self.dropped = true,
            StatsEvent::MatchEnded(m) => {
                self.note_guid(m.match_guid.as_deref());
                self.winner = Some(m.winner_team_num);
                self.play_seen = true;
                self.live = false;
                self.phase_known = true;
                self.reset_boost_history();
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
                self.live = false;
                self.phase_known = true;
                self.reset_boost_history();
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
                        scorer_boost: self.players[scorer].last_boost,
                        scorer_speed: self.players[scorer].last_speed,
                        score_after: None,
                    });
                    self.awaiting_score = Some(self.goals.len() - 1);
                }
            }
            StatsEvent::StatfeedEvent(s) if self.collecting() && settings.stat_events => {
                let main = self.slot_of(&s.main_target);
                let secondary = s.secondary_target.as_ref().map(|t| self.slot_of(t));
                let (boost, speed) = (self.players[main].last_boost, self.players[main].last_speed);
                self.stat_events.push(StatEventRow {
                    clock: self.clock,
                    name: s.event_name.clone(),
                    main,
                    secondary,
                    boost,
                    speed,
                });
                let acc = self
                    .event_acc
                    .entry((main, s.event_name.clone()))
                    .or_default();
                acc.count += 1;
                if let Some(b) = boost {
                    acc.boost_sum += b;
                    acc.boost_n += 1;
                }
                if let Some(v) = speed {
                    acc.speed_sum += v;
                    acc.speed_n += 1;
                }
            }
            StatsEvent::BallHit(b) if self.collecting() && settings.ball_hits => {
                for hitter in &b.players {
                    let slot = self.slot_of(hitter);
                    // Team 0 attacks toward +Y and team 1 toward -Y (checked against real goals).
                    let toward_enemy = if hitter.team_num == 0 {
                        b.ball.location.y
                    } else {
                        -b.ball.location.y
                    };
                    let third = if toward_enemy < -THIRD {
                        0
                    } else if toward_enemy > THIRD {
                        2
                    } else {
                        1
                    };
                    let acc = self.touches.entry(slot).or_default();
                    acc.touches += 1;
                    acc.thirds[third] += 1;
                    acc.speed_sum += b.ball.post_hit_speed;
                    acc.speed_max = acc.speed_max.max(b.ball.post_hit_speed);
                }
            }
            StatsEvent::CrossbarHit(c) if self.collecting() && settings.crossbar_hits => {
                let slot = self.slot_of(&c.ball_last_touch.player);
                let acc = self.crossbar.entry(slot).or_insert((0, 0.0));
                acc.0 += 1;
                acc.1 = acc.1.max(c.impact_force);
            }
            StatsEvent::BoostPickup(b)
                if self.collecting() && !b.replay && settings.boost_pickups =>
            {
                let slot = self.slot_of(&b.player);
                let acc = self.pickups.entry(slot).or_default();
                if b.boost_type == "BoostType_Pill" {
                    acc.pills += 1;
                } else {
                    acc.pads += 1;
                }
                acc.amount += b.boost_amount;
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

        let touched = self.possession_ticks[0] + self.possession_ticks[1];
        let my_possession_pct = usize::try_from(my_team)
            .ok()
            .and_then(|t| self.possession_ticks.get(t))
            .filter(|_| touched > 0)
            .map(|&mine| mine as f64 * 100.0 / touched as f64);
        let has_ball = settings.possession && self.ball_ticks > 0;
        let team_empty_overlap = (settings.movement && self.teammate_seen && self.my_ticks > 0)
            .then(|| self.overlap_ticks as f64 * 100.0 / self.my_ticks as f64);

        let event_stats = self
            .event_acc
            .iter()
            .map(|((slot, event), a)| EventStat {
                slot: *slot,
                event: event.clone(),
                count: a.count,
                avg_boost: (a.boost_n > 0).then(|| a.boost_sum as f64 / a.boost_n as f64),
                avg_speed: (a.speed_n > 0).then(|| a.speed_sum / a.speed_n as f64),
            })
            .collect();
        let touches = self
            .touches
            .iter()
            .map(|(slot, a)| TouchSummary {
                slot: *slot,
                touches: a.touches,
                def_third: a.thirds[0],
                mid_third: a.thirds[1],
                att_third: a.thirds[2],
                avg_post_speed: a.speed_sum / a.touches as f64,
                max_post_speed: a.speed_max,
            })
            .collect();
        let crossbar = self
            .crossbar
            .iter()
            .map(|(slot, &(count, max_impact))| CrossbarSummary {
                slot: *slot,
                count,
                max_impact,
            })
            .collect();
        let pickups = self
            .pickups
            .iter()
            .map(|(slot, a)| PickupSummary {
                slot: *slot,
                pads: a.pads,
                pills: a.pills,
                amount: a.amount,
            })
            .collect();

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
            my_possession_pct: my_possession_pct.filter(|_| settings.possession),
            ball_avg_speed: has_ball.then(|| self.ball_speed_sum / self.ball_ticks as f64),
            ball_max_speed: has_ball.then_some(self.ball_speed_max),
            team_empty_overlap,
            players,
            goals: self.goals,
            stat_events: self.stat_events,
            event_stats,
            touches,
            crossbar,
            pickups,
            movement,
        })
    }
}

/// Whether the game sends movement data for this player (you and your teammates). Opponents
/// have none of these fields. See the note on [`Player`] about omitted zero/false values.
fn visible(p: &Player) -> bool {
    p.speed.is_some()
        || p.has_car.is_some()
        || p.boost.is_some()
        || p.boosting.is_some()
        || p.on_ground.is_some()
        || p.supersonic.is_some()
}

/// Boost 0-100 for visible players. A missing `Boost` means 0.
fn boost_of(p: &Player) -> Option<i64> {
    visible(p).then(|| p.boost.unwrap_or(0))
}

/// A third of the field length (the field runs from about -5120 to +5120).
const THIRD: f64 = 1707.0;

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
    if m.alive == 0 {
        return None;
    }
    let n = m.alive as f64;
    let pct = |count: i64| count as f64 * 100.0 / n;
    Some(MovementSummary {
        slot,
        samples: m.alive,
        avg_boost: m.boost_sum as f64 / n,
        pct_boost_empty: pct(m.bands[0]),
        pct_boost_low: pct(m.bands[1]),
        pct_boost_mid: pct(m.bands[2]),
        pct_boost_high: pct(m.bands[3]),
        pct_boosting: pct(m.boosting),
        boost_used: m.boost_used,
        boost_gained: m.boost_gained,
        avg_speed: m.speed_sum / n,
        max_speed: m.speed_max,
        pct_supersonic: pct(m.supersonic),
        pct_powersliding: pct(m.powersliding),
        pct_on_ground: pct(m.on_ground),
        pct_on_wall: pct(m.on_wall),
        pct_airborne: pct(m.airborne),
        pct_demolished: (m.ticks - m.alive) as f64 * 100.0 / m.ticks as f64,
        times_demolished: m.times_demolished,
    })
}
