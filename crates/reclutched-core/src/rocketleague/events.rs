//! Typed versions of every message in the official Stats API docs.
//! Field names follow the docs; the Rust names are snake_case versions of them.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// `MatchGuid` is only set for online/LAN matches; offline matches send "".
/// We turn "" (or a missing field) into `None`.
///
/// The `'de` is a lifetime: it just says "the data being deserialized lives at least this long".
fn empty_as_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let s = Option::<String>::deserialize(d)?;
    Ok(s.filter(|s| !s.is_empty()))
}

/// One decoded Stats API message.
#[derive(Debug, Clone, PartialEq)]
pub enum StatsEvent {
    /// The periodic tick with every player's stats.
    UpdateState(UpdateState),
    BallHit(BallHit),
    BoostPickup(BoostPickup),
    ClockUpdatedSeconds(ClockUpdated),
    CrossbarHit(CrossbarHit),
    GoalScored(GoalScored),
    MatchEnded(MatchEnded),
    PlayerJoined(PlayerChange),
    PlayerLeft(PlayerChange),
    ReplayCreated(ReplayCreated),
    StatfeedEvent(StatfeedEvent),
    // These events carry nothing except the match id.
    CountdownBegin(MatchRef),
    GoalReplayEnd(MatchRef),
    GoalReplayStart(MatchRef),
    GoalReplayWillEnd(MatchRef),
    MatchCreated(MatchRef),
    MatchDestroyed(MatchRef),
    MatchInitialized(MatchRef),
    MatchPaused(MatchRef),
    MatchUnpaused(MatchRef),
    PodiumStart(MatchRef),
    RoundStarted(MatchRef),
    /// An event name we don't know (e.g. added by a game update). Kept, not dropped.
    Unknown {
        name: String,
        data: Value,
    },
}

/// A point in the arena, in Unreal units.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// The short "who" reference used by most events.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayerRef {
    pub name: String,
    /// Spectator shortcut number.
    pub shortcut: i64,
    /// 0 = Blue, 1 = Orange.
    pub team_num: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MatchRef {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateState {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    pub players: Vec<Player>,
    pub game: Game,
}

/// A player's stats.
///
/// The `Option` fields are the docs' SPECTATOR/TEAM fields: the game sends them for you and
/// your teammates only. Real captures show the game then OMITS values that are 0 or false:
/// `Speed` and `bHasCar` are always there for those players, but `Boost` is missing at 0 boost
/// and `bBoosting`, `bOnGround`, `bOnWall`, `bPowersliding`, `bSupersonic` are missing when false.
/// So a player with `speed` set and no `boost` has 0 boost, while an opponent (no such fields at
/// all) is simply not visible. Speeds are in km/h (the docs say Unreal units/s; the top car speed
/// shows up as 82.8, which is 2300 uu/s in km/h).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Player {
    pub name: String,
    /// "Platform|Uid|Splitscreen", e.g. "Epic|456|0".
    pub primary_id: String,
    pub shortcut: i64,
    pub team_num: i64,
    pub score: i64,
    pub goals: i64,
    pub shots: i64,
    pub assists: i64,
    pub saves: i64,
    pub touches: i64,
    pub car_touches: i64,
    pub demos: i64,
    #[serde(default)]
    pub loadout: Vec<String>,
    #[serde(rename = "bHasCar")]
    pub has_car: Option<bool>,
    pub speed: Option<f64>,
    pub boost: Option<i64>,
    #[serde(rename = "bBoosting")]
    pub boosting: Option<bool>,
    #[serde(rename = "bOnGround")]
    pub on_ground: Option<bool>,
    #[serde(rename = "bOnWall")]
    pub on_wall: Option<bool>,
    #[serde(rename = "bPowersliding")]
    pub powersliding: Option<bool>,
    #[serde(rename = "bDemolished")]
    pub demolished: Option<bool>,
    #[serde(rename = "bSupersonic")]
    pub supersonic: Option<bool>,
    pub pickup_class: Option<String>,
    /// Who demolished this player; only present while demolished.
    pub attacker: Option<PlayerRef>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Game {
    pub teams: Vec<Team>,
    /// Which game mode this is. The docs don't list the values; see `playlist`.
    pub playlist_id: i64,
    /// Seconds REMAINING in the match (it counts down).
    pub time_seconds: i64,
    #[serde(rename = "bOvertime")]
    pub overtime: bool,
    pub ball: BallState,
    #[serde(rename = "bReplay")]
    pub replay: bool,
    #[serde(rename = "bHasWinner")]
    pub has_winner: bool,
    /// Name of the winning team, "" while there is no winner.
    pub winner: String,
    pub arena: String,
    #[serde(rename = "bHasTarget")]
    pub has_target: bool,
    pub target: Option<PlayerRef>,
    /// Only while a replay is active.
    pub frame: Option<i64>,
    pub elapsed: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Team {
    pub name: String,
    pub team_num: i64,
    /// Goals scored by the team.
    pub score: i64,
    pub color_primary: String,
    pub color_secondary: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BallState {
    pub speed: f64,
    /// Team that touched the ball last; 255 if nobody has.
    pub team_num: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BallHit {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    pub players: Vec<PlayerRef>,
    pub ball: BallHitState,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BallHitState {
    pub pre_hit_speed: f64,
    pub post_hit_speed: f64,
    pub location: Vec3,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BoostPickup {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    pub player: PlayerRef,
    pub location: Vec3,
    pub boost_amount: f64,
    /// "BoostType_Pad" (small) or "BoostType_Pill" (big).
    pub boost_type: String,
    #[serde(rename = "bReplay")]
    pub replay: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ClockUpdated {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    pub time_seconds: i64,
    #[serde(rename = "bOvertime")]
    pub overtime: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LastTouch {
    pub player: PlayerRef,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CrossbarHit {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    pub ball_location: Vec3,
    pub ball_speed: f64,
    pub impact_force: f64,
    pub ball_last_touch: LastTouch,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GoalScored {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    pub goal_speed: f64,
    /// Despite the name, the docs say this is the length of the previous round in seconds.
    #[serde(rename = "GoalTime")]
    pub round_length: f64,
    pub impact_location: Vec3,
    pub scorer: PlayerRef,
    /// Only present when an assist was recorded.
    pub assister: Option<PlayerRef>,
    pub ball_last_touch: LastTouch,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MatchEnded {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    pub winner_team_num: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayerChange {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    pub player_name: String,
    pub primary_id: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReplayCreated {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    pub file_name: String,
    pub date: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StatfeedEvent {
    #[serde(default, deserialize_with = "empty_as_none")]
    pub match_guid: Option<String>,
    /// Asset name of the stat, e.g. "Goal", "Save", "Demolish", "MVP".
    pub event_name: String,
    /// Localized display label, e.g. "Shot on Goal".
    #[serde(rename = "Type")]
    pub label: String,
    pub main_target: PlayerRef,
    pub secondary_target: Option<PlayerRef>,
}
