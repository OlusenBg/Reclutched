//! Playlist ids -> readable game mode names.
//!
//! The official docs don't list the `PlaylistId` values, so we learn them from real matches
//! (`reclutched map`). Confirmed ones are in [`BUILTIN`]; the user's own file adds to them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Ids confirmed from real matches. Empty until we have confirmed ones (see `reclutched map`).
pub const BUILTIN: &[(i64, &str)] = &[];

/// Every mode we want a mapping for. `reclutched map` finishes when all are mapped.
pub const MODES: &[&str] = &[
    "Casual 1v1",
    "Casual 2v2",
    "Casual 3v3",
    "Casual 4v4",
    "Casual Chaos",
    "Casual Free for all",
    "Casual Non-standard 2v2",
    "Ranked 1v1",
    "Ranked 2v2",
    "Ranked 3v3",
    "Ranked Hoops",
    "Ranked Rumble",
    "Ranked Dropshot",
    "Ranked Snow Day",
    "Extra Hoops",
    "Extra Rumble",
    "Extra Dropshot",
    "Extra Snow Day",
    "Extra Heatseeker",
    "Extra Gridiron",
    "Extra Spike Rush",
    "Extra Knockout",
    "Private match",
    "Tournament",
    "Offline / Exhibition",
    "Season",
    "Free play / Training",
];

#[derive(Debug, thiserror::Error)]
pub enum MapFileError {
    #[error("cannot read or write the playlist map file: {0}")]
    Io(#[from] std::io::Error),
    #[error("the playlist map file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Where the user's map lives: `$XDG_CONFIG_HOME/reclutched/playlist_map.json`
/// (or `~/.config/...` when that variable isn't set).
pub fn default_map_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("reclutched").join("playlist_map.json"))
}

#[derive(Debug, Clone, Default)]
pub struct PlaylistMap {
    ids: BTreeMap<i64, String>,
}

impl PlaylistMap {
    /// Just the built-in ids.
    pub fn with_builtin() -> Self {
        let ids = BUILTIN.iter().map(|&(id, n)| (id, n.to_string())).collect();
        Self { ids }
    }

    /// Built-in ids plus the user's file. A missing file is fine (nothing mapped yet).
    pub fn load(path: &Path) -> Result<Self, MapFileError> {
        let mut map = Self::with_builtin();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                // JSON object keys are always strings, serde turns "9" back into 9 for us.
                let user: BTreeMap<i64, String> = serde_json::from_str(&text)?;
                map.ids.extend(user);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(map)
    }

    /// Writes only the user's own entries (built-ins live in the binary).
    pub fn save(&self, path: &Path) -> Result<(), MapFileError> {
        let user: BTreeMap<&i64, &String> = self
            .ids
            .iter()
            .filter(|(id, name)| !BUILTIN.contains(&(**id, name.as_str())))
            .collect();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&user)?)?;
        Ok(())
    }

    pub fn name(&self, id: i64) -> Option<&str> {
        self.ids.get(&id).map(String::as_str)
    }

    /// The name, or "Playlist 17" when we don't know it yet.
    pub fn display(&self, id: i64) -> String {
        self.name(id)
            .map_or_else(|| format!("Playlist {id}"), str::to_string)
    }

    pub fn insert(&mut self, id: i64, name: &str) {
        self.ids.insert(id, name.to_string());
    }

    /// Modes from [`MODES`] that no id points at yet.
    pub fn remaining(&self) -> Vec<&'static str> {
        MODES
            .iter()
            .copied()
            .filter(|m| !self.ids.values().any(|v| v == m))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_names_are_valid_modes() {
        for (_, name) in BUILTIN {
            assert!(MODES.contains(name), "{name} missing from MODES");
        }
    }

    #[test]
    fn remaining_shrinks_when_mapped() {
        let mut map = PlaylistMap::with_builtin();
        let before = map.remaining().len();
        assert!(map.remaining().contains(&"Ranked 1v1"));
        map.insert(10, "Ranked 1v1");
        assert_eq!(map.remaining().len(), before - 1);
        assert_eq!(map.display(10), "Ranked 1v1");
        assert_eq!(map.display(99), "Playlist 99");
    }

    #[test]
    fn save_and_load_roundtrip_only_stores_user_entries() {
        let dir = std::env::temp_dir().join(format!("reclutched-test-{}", std::process::id()));
        let path = dir.join("playlist_map.json");
        let mut map = PlaylistMap::with_builtin();
        map.insert(8, "Offline / Exhibition");
        map.save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Offline / Exhibition"));

        let loaded = PlaylistMap::load(&path).unwrap();
        assert_eq!(loaded.name(8), Some("Offline / Exhibition"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let map = PlaylistMap::load(Path::new("/nonexistent/dir/map.json")).unwrap();
        assert_eq!(map.name(9), None);
    }
}
