//! Finding the game's config folder and turning its Stats API on.
//!
//! The Stats API is switched on by a setting in `TAGame/Config/DefaultStatsAPI.ini` inside the
//! game's install folder (or `TAStatsAPI.ini` next to it, which wins when it exists):
//!
//! ```text
//! [TAGame.MatchStatsExporter_TA]
//! PacketSendRate=10
//! ```
//!
//! We only edit that file when the user asks for it, and keep a backup of the original.

use std::fs;
use std::path::{Path, PathBuf};

pub const SECTION: &str = "TAGame.MatchStatsExporter_TA";
/// Updates per second we ask the game for. The stats we keep need about this many.
pub const RECOMMENDED_RATE: u32 = 10;
/// The websocket port the game uses when `WebPort` is not set.
pub const DEFAULT_WEB_PORT: u32 = 49124;

/// The file that decides the Stats API settings in this folder: `TAStatsAPI.ini` when it
/// exists (the game prefers it), otherwise `DefaultStatsAPI.ini`.
pub fn config_file_in(dir: &Path) -> Option<PathBuf> {
    ["TAStatsAPI.ini", "DefaultStatsAPI.ini"]
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}

/// What the ini says about the Stats API.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Updates per second; 0 means the Stats API is off.
    pub packet_send_rate: f64,
    /// 0 means the websocket is off.
    pub web_port: u32,
}

impl Settings {
    /// True when the game will send us data over the websocket.
    pub fn is_on(&self) -> bool {
        self.packet_send_rate > 0.0 && self.web_port > 0
    }
}

/// Splits `key=value`, trimming spaces. Comment (`;`, `#`) and section lines give `None`.
fn key_value(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

fn is_section(line: &str, name: &str) -> bool {
    let line = line.trim();
    line.strip_prefix('[')
        .and_then(|l| l.strip_suffix(']'))
        .is_some_and(|n| n.trim().eq_ignore_ascii_case(name))
}

/// Reads the Stats API settings out of the ini text. Missing values use the game's defaults
/// (rate 0 = off, web port 49124).
pub fn read(text: &str) -> Settings {
    let mut in_section = false;
    let mut found = Settings {
        packet_send_rate: 0.0,
        web_port: DEFAULT_WEB_PORT,
    };
    for line in text.lines() {
        if line.trim().starts_with('[') {
            in_section = is_section(line, SECTION);
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = key_value(line) {
            if key.eq_ignore_ascii_case("PacketSendRate") {
                found.packet_send_rate = value.parse().unwrap_or(0.0);
            } else if key.eq_ignore_ascii_case("WebPort") {
                found.web_port = value.parse().unwrap_or(0);
            }
        }
    }
    found
}

/// Returns the ini text with the Stats API turned on: `PacketSendRate` set to `rate`, and
/// `WebPort` set to the default if it was switched off. Everything else (other settings,
/// comments, line endings) is left exactly as it was.
pub fn turn_on(text: &str, rate: u32) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    // Where does our section start and end?
    let start = lines.iter().position(|l| is_section(l, SECTION));
    let Some(start) = start else {
        // No section at all: add one at the end.
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("[{SECTION}]"));
        lines.push(format!("PacketSendRate={rate}"));
        return join(&lines, newline);
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim().starts_with('['))
        .map_or(lines.len(), |i| start + 1 + i);

    let mut rate_done = false;
    for line in &mut lines[start + 1..end] {
        match key_value(line) {
            Some((key, _)) if key.eq_ignore_ascii_case("PacketSendRate") => {
                *line = format!("PacketSendRate={rate}");
                rate_done = true;
            }
            Some((key, value))
                if key.eq_ignore_ascii_case("WebPort")
                    && value.parse::<u32>().unwrap_or(0) == 0 =>
            {
                *line = format!("WebPort={DEFAULT_WEB_PORT}");
            }
            _ => {}
        }
    }
    if !rate_done {
        // Put it right after the last setting of the section, before trailing blank lines.
        let mut at = end;
        while at > start + 1 && lines[at - 1].trim().is_empty() {
            at -= 1;
        }
        lines.insert(at, format!("PacketSendRate={rate}"));
    }
    join(&lines, newline)
}

fn join(lines: &[String], newline: &str) -> String {
    let mut out = lines.join(newline);
    out.push_str(newline);
    out
}

/// Turns the Stats API on in this config file. The first time, the original is copied to
/// `<name>.reclutched-backup` so nothing is lost.
pub fn turn_on_in_file(file: &Path, rate: u32) -> std::io::Result<()> {
    let text = fs::read_to_string(file)?;
    let backup = file.with_extension("ini.reclutched-backup");
    if !backup.exists() {
        fs::write(&backup, &text)?;
    }
    fs::write(file, turn_on(&text, rate))
}

// ---------------------------------------------------------------------------------------
// Finding the game
// ---------------------------------------------------------------------------------------

/// Folders we never look inside: Wine prefixes are huge and never hold the game's own config.
const SKIP_DIRS: &[&str] = &[
    "drive_c",
    "dosdevices",
    "compatdata",
    "pfx",
    "Prefixes",
    "shadercache",
    ".git",
    "node_modules",
];

/// Looks below `root` (at most `max_depth` folders deep) for `TAGame/Config` folders that
/// contain a Stats API ini. Returns the `Config` folders.
pub fn search(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        visited += 1;
        if visited > 100_000 {
            break; // a safety net for enormous folders
        }
        if dir.file_name().is_some_and(|n| n == "Config")
            && dir
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "TAGame")
            && config_file_in(&dir).is_some()
        {
            found.push(dir);
            continue;
        }
        if depth >= max_depth {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() && !SKIP_DIRS.contains(&name.as_ref()) && !path.is_symlink() {
                stack.push((path, depth + 1));
            }
        }
    }
    found.sort();
    found
}

/// Extra Steam library folders listed in Steam's `libraryfolders.vdf`.
pub fn steam_library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            let (key, value) = (quoted.next()?, quoted.next()?);
            key.eq_ignore_ascii_case("path")
                .then(|| PathBuf::from(value))
        })
        .collect()
}

/// Looks in the usual places on Linux (Heroic, Steam including extra libraries and Flatpak,
/// Lutris) for Rocket League's config folder.
pub fn find_game_config_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut roots = vec![
        home.join("Games"),
        home.join(".local/share/Steam/steamapps/common"),
        home.join(".steam/steam/steamapps/common"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common"),
        home.join(".local/share/lutris"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
    ];
    // More Steam libraries on other drives.
    for steam in [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ] {
        if let Ok(vdf) = fs::read_to_string(steam.join("steamapps/libraryfolders.vdf")) {
            roots.extend(
                steam_library_paths(&vdf)
                    .into_iter()
                    .map(|p| p.join("steamapps/common")),
            );
        }
    }
    let mut found = Vec::new();
    for root in roots {
        if root.is_dir() {
            for dir in search(&root, 6) {
                if !found.contains(&dir) {
                    found.push(dir);
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_INI: &str = "[TAGame.MatchStatsExporter_TA]\n\n; Port the client will listen for tcp connections on\nPort=49123\n\n; web port\nWebPort=49124\n\n; rate\nPacketSendRate=0\n";

    #[test]
    fn reads_the_settings_and_the_defaults() {
        let s = read(DEFAULT_INI);
        assert_eq!(s.packet_send_rate, 0.0);
        assert_eq!(s.web_port, 49124);
        assert!(!s.is_on());
        // Nothing about the Stats API at all: off, with the default web port.
        let none = read("[Other]\nPacketSendRate=99\n");
        assert_eq!(
            (none.packet_send_rate, none.web_port),
            (0.0, DEFAULT_WEB_PORT)
        );
        assert!(read("[TAGame.MatchStatsExporter_TA]\nPacketSendRate=30\n").is_on());
        assert!(!read("[TAGame.MatchStatsExporter_TA]\nPacketSendRate=30\nWebPort=0\n").is_on());
        // Keys and the section name are not case sensitive; spaces are fine.
        assert!(read("[tagame.matchstatsexporter_ta]\n packetsendrate = 10 \n").is_on());
    }

    #[test]
    fn turning_on_changes_only_the_rate_and_keeps_comments() {
        let out = turn_on(DEFAULT_INI, 10);
        assert!(out.contains("PacketSendRate=10"));
        assert!(!out.contains("PacketSendRate=0"));
        assert!(out.contains("; Port the client will listen for tcp connections on"));
        assert!(out.contains("Port=49123") && out.contains("WebPort=49124"));
        assert!(read(&out).is_on());
        // Doing it twice changes nothing more.
        assert_eq!(turn_on(&out, 10), out);
    }

    #[test]
    fn turning_on_adds_the_missing_pieces() {
        // Section without the rate: it is added inside the section, not at the very end.
        let out = turn_on(
            "[TAGame.MatchStatsExporter_TA]\nPort=49123\n\n[Other]\nA=1\n",
            10,
        );
        assert_eq!(
            out,
            "[TAGame.MatchStatsExporter_TA]\nPort=49123\nPacketSendRate=10\n\n[Other]\nA=1\n"
        );
        // No section: appended.
        let out = turn_on("[Other]\nA=1\n", 10);
        assert_eq!(
            out,
            "[Other]\nA=1\n\n[TAGame.MatchStatsExporter_TA]\nPacketSendRate=10\n"
        );
        // Empty file.
        assert_eq!(
            turn_on("", 10),
            "[TAGame.MatchStatsExporter_TA]\nPacketSendRate=10\n"
        );
        // A switched-off web port is switched back on.
        let out = turn_on(
            "[TAGame.MatchStatsExporter_TA]\nWebPort=0\nPacketSendRate=0\n",
            10,
        );
        assert!(read(&out).is_on());
        assert!(out.contains("WebPort=49124"));
    }

    #[test]
    fn the_real_game_file_is_handled() {
        // Exactly what Rocket League ships in TAGame/Config/DefaultStatsAPI.ini.
        let real = "[TAGame.MatchStatsExporter_TA]\n\n; Port the client will listen for tcp connections on (must be different than WebPort, set to 0 to disable)\nPort=49123\n\n; Port the client will listen for web connections on (must be different than Port, set to 0 to disable)\nWebPort=49124\n\n; How many times per second the game sends the update state (capped at 120, 0 disables this feature)\nPacketSendRate=0";
        assert!(!read(real).is_on());
        let out = turn_on(real, RECOMMENDED_RATE);
        assert!(read(&out).is_on());
        assert_eq!(read(&out).packet_send_rate, 10.0);
        // Only that one line differs.
        let changed: Vec<_> = real
            .lines()
            .zip(out.lines())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(changed, vec![("PacketSendRate=0", "PacketSendRate=10")]);
    }

    #[test]
    fn windows_line_endings_are_kept() {
        let ini = "[TAGame.MatchStatsExporter_TA]\r\nPort=49123\r\nPacketSendRate=0\r\n";
        let out = turn_on(ini, 10);
        assert_eq!(
            out,
            "[TAGame.MatchStatsExporter_TA]\r\nPort=49123\r\nPacketSendRate=10\r\n"
        );
    }

    #[test]
    fn steam_library_paths_are_read_from_the_vdf() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/me/.local/share/Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"/mnt/games/SteamLibrary\"\n\t}\n}\n";
        assert_eq!(
            steam_library_paths(vdf),
            vec![
                PathBuf::from("/home/me/.local/share/Steam"),
                PathBuf::from("/mnt/games/SteamLibrary")
            ]
        );
    }

    #[test]
    fn search_finds_the_config_folder_and_skips_wine_prefixes() {
        let root = std::env::temp_dir().join(format!("reclutched-find-{}", std::process::id()));
        let good = root.join("Heroic/RocketLeague/TAGame/Config");
        let decoy = root.join("Prefixes/Rocket League/drive_c/RL/TAGame/Config");
        let unrelated = root.join("Other/Config");
        for d in [&good, &decoy, &unrelated] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(good.join("DefaultStatsAPI.ini"), DEFAULT_INI).unwrap();
        fs::write(decoy.join("DefaultStatsAPI.ini"), DEFAULT_INI).unwrap();
        fs::write(unrelated.join("DefaultStatsAPI.ini"), DEFAULT_INI).unwrap();

        assert_eq!(search(&root, 6), vec![good.clone()]);
        // Too shallow to reach it.
        assert!(search(&root, 2).is_empty());
        // TAStatsAPI.ini wins over the default one when both exist.
        fs::write(good.join("TAStatsAPI.ini"), "").unwrap();
        assert_eq!(config_file_in(&good), Some(good.join("TAStatsAPI.ini")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn turning_on_a_file_keeps_a_backup_of_the_original() {
        let dir = std::env::temp_dir().join(format!("reclutched-ini-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("DefaultStatsAPI.ini");
        fs::write(&file, DEFAULT_INI).unwrap();

        turn_on_in_file(&file, 10).unwrap();
        assert!(read(&fs::read_to_string(&file).unwrap()).is_on());
        let backup = dir.join("DefaultStatsAPI.ini.reclutched-backup");
        assert_eq!(fs::read_to_string(&backup).unwrap(), DEFAULT_INI);

        // A second run must not overwrite the backup with the already edited file.
        turn_on_in_file(&file, 20).unwrap();
        assert_eq!(fs::read_to_string(&backup).unwrap(), DEFAULT_INI);
        fs::remove_dir_all(dir).unwrap();
    }
}
