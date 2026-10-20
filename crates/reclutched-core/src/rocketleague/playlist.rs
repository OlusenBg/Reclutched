//! Playlist ids -> readable game mode names.
//!
//! The official Stats API docs don't list the `PlaylistId` values. These come from the
//! BakkesMod "Known Playlist IDs" list (copied from the wiki, October 2026), which uses the
//! game's own internal ids. Our real captures confirm 3 (Standard), 8 (Exhibition) and
//! 9 (Training). Names were lightly tidied (e.g. "Casual Standard" for 3).
//!
//! Ids missing here (the list skips some, and new modes appear) fall back to "Playlist N";
//! `reclutched map` lets the player name them, saved to a small JSON file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Known ids. Not in the list on purpose: 55 (name was cut off in the copy we had).
pub const BUILTIN: &[(i64, &str)] = &[
    (-2, "Intermission"),
    (0, "Casual (generic)"),
    (1, "Casual Duel"),
    (2, "Casual Doubles"),
    (3, "Casual Standard"),
    (4, "Casual Chaos"),
    (6, "Private Match"),
    (7, "Season"),
    (8, "Exhibition"),
    (9, "Training"),
    (10, "Ranked Duel"),
    (11, "Ranked Doubles"),
    (13, "Ranked Standard"),
    (15, "Snow Day"),
    (16, "Rocket Labs"),
    (17, "Hoops"),
    (18, "Rumble"),
    (19, "Workshop"),
    (20, "Custom Training Editor"),
    (21, "Custom Training"),
    (22, "Tournament Match (Custom)"),
    (23, "Dropshot"),
    (24, "Local Match"),
    (26, "External Match (Ranked)"),
    (27, "Ranked Hoops"),
    (28, "Ranked Rumble"),
    (29, "Ranked Dropshot"),
    (30, "Ranked Snow Day"),
    (31, "Ghost Hunt"),
    (32, "Beach Ball"),
    (33, "Spike Rush"),
    (34, "Tournament Match (Automatic)"),
    (35, "Rocket Labs"),
    (37, "Dropshot Rumble"),
    (38, "Heatseeker"),
    (41, "Boomer Ball"),
    (43, "Heatseeker Doubles"),
    (44, "Winter Breakaway"),
    (46, "Gridiron"),
    (47, "Super Cube"),
    (48, "Tactical Rumble"),
    (49, "Spring Loaded"),
    (50, "Speed Demon"),
    (52, "Gotham City Rumble"),
    (54, "Knockout"),
    (61, "Ranked 4v4 Quads"),
    (62, "Magnus Futball"),
    (64, "Godball Spooky"),
    (65, "Godball Haunted"),
    (66, "Godball Ricochet"),
    (67, "Cubic Spooky"),
    (68, "G-Force Frenzy"),
    (70, "RumShot Doubles"),
    (72, "Territory"),
    (73, "Online Freeplay"),
    (74, "Territory Doubles"),
    (75, "Godball Territory"),
    (76, "Godball Territory Doubles"),
    (77, "Non-standard Soccer"),
    (79, "Snowday Territory"),
    (80, "Run It Back"),
    (81, "Car Wars"),
    (82, "Pizza Party"),
    (83, "Push The Puck"),
    (84, "Possession"),
    (86, "FC Showdown"),
    (87, "Sacrifice"),
    (88, "Jump Jam"),
];

/// Ids that are never a real match: intermission, training, workshop, custom training and
/// online free play. The tracker ignores these sessions.
pub fn is_real_match(id: i64) -> bool {
    !matches!(id, -2 | 9 | 19 | 20 | 21 | 73)
}

#[derive(Debug, thiserror::Error)]
pub enum MapFileError {
    #[error("cannot read or write the playlist map file: {0}")]
    Io(#[from] std::io::Error),
    #[error("the playlist map file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Where the user's own entries live: `<config dir>/playlist_map.json`.
pub fn default_map_path() -> Option<PathBuf> {
    Some(crate::config_dir()?.join("playlist_map.json"))
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

    /// Built-in ids plus the user's file (which wins on conflicts).
    /// A missing file is fine: nothing added yet.
    pub fn load(path: &Path) -> Result<Self, MapFileError> {
        let mut map = Self::with_builtin();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                // JSON object keys are always strings; serde turns "12" back into 12 for us.
                let user: BTreeMap<i64, String> = serde_json::from_str(&text)?;
                map.ids.extend(user);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(map)
    }

    /// Entries the user added or changed (everything not identical to a built-in).
    pub fn user_entries(&self) -> Vec<(i64, &str)> {
        self.ids
            .iter()
            .filter(|(id, name)| !BUILTIN.contains(&(**id, name.as_str())))
            .map(|(id, name)| (*id, name.as_str()))
            .collect()
    }

    /// Writes only the user's own entries (the built-ins live in the binary).
    pub fn save(&self, path: &Path) -> Result<(), MapFileError> {
        let user: BTreeMap<i64, &str> = self.user_entries().into_iter().collect();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&user)?)?;
        Ok(())
    }

    pub fn name(&self, id: i64) -> Option<&str> {
        self.ids.get(&id).map(String::as_str)
    }

    /// The name, or "Playlist 17" when we don't know it.
    pub fn display(&self, id: i64) -> String {
        self.name(id)
            .map_or_else(|| format!("Playlist {id}"), str::to_string)
    }

    pub fn insert(&mut self, id: i64, name: &str) {
        self.ids.insert(id, name.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_ids_are_unique_and_confirmed_ones_match_captures() {
        let mut ids: Vec<i64> = BUILTIN.iter().map(|&(id, _)| id).collect();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate or unsorted ids in BUILTIN");
        let map = PlaylistMap::with_builtin();
        assert_eq!(map.name(3), Some("Casual Standard"));
        assert_eq!(map.name(8), Some("Exhibition"));
        assert_eq!(map.name(9), Some("Training"));
    }

    #[test]
    fn training_and_friends_are_not_real_matches() {
        for id in [-2, 9, 19, 20, 21, 73] {
            assert!(!is_real_match(id), "{id}");
        }
        for id in [3, 8, 11, 13, 54] {
            assert!(is_real_match(id), "{id}");
        }
    }

    #[test]
    fn unknown_ids_fall_back_and_user_entries_are_tracked() {
        let mut map = PlaylistMap::with_builtin();
        assert_eq!(map.display(12), "Playlist 12");
        assert!(map.user_entries().is_empty());
        map.insert(12, "Ranked Solo Standard");
        assert_eq!(map.display(12), "Ranked Solo Standard");
        assert_eq!(map.user_entries(), vec![(12, "Ranked Solo Standard")]);
    }

    #[test]
    fn save_and_load_roundtrip_only_stores_user_entries() {
        let dir = std::env::temp_dir().join(format!("reclutched-test-{}", std::process::id()));
        let path = dir.join("playlist_map.json");
        let mut map = PlaylistMap::with_builtin();
        map.insert(12, "Ranked Solo Standard");
        map.save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Ranked Solo Standard") && !text.contains("Exhibition"));

        let loaded = PlaylistMap::load(&path).unwrap();
        assert_eq!(loaded.name(12), Some("Ranked Solo Standard"));
        assert_eq!(loaded.name(8), Some("Exhibition"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let map = PlaylistMap::load(Path::new("/nonexistent/dir/map.json")).unwrap();
        assert_eq!(map.name(9), Some("Training"));
    }
}
