//! Rocket League: the official local Stats API.

pub mod dates;
pub mod events;
pub mod parse;
pub mod periods;
pub mod playlist;
pub mod settings;
pub mod statsapi;
pub mod storage;
pub mod tracker;

pub use events::StatsEvent;
pub use parse::{parse_message, ParseError};
