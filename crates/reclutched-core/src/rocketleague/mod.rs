//! Rocket League: the official local Stats API.

pub mod events;
pub mod parse;
pub mod playlist;

pub use events::StatsEvent;
pub use parse::{parse_message, ParseError};
