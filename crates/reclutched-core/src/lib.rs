//! Game-agnostic events, game modules and storage live here.

use std::path::PathBuf;

pub mod rocketleague;

/// `$XDG_CONFIG_HOME/reclutched` (or `~/.config/reclutched` when that variable isn't set).
pub fn config_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("reclutched"))
}
