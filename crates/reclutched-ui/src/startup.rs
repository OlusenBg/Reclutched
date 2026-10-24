//! Starting with the computer: an autostart entry in `~/.config/autostart`.

use std::path::{Path, PathBuf};

const FILE_NAME: &str = "dev.reclutched.Reclutched.desktop";

/// Where the entry lives (`$XDG_CONFIG_HOME/autostart`, or `~/.config/autostart`).
fn entry_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("autostart").join(FILE_NAME))
}

/// Quotes an `Exec=` argument the way the desktop-entry spec wants it.
fn quote_exec(arg: &str) -> String {
    if arg.chars().any(|c| " \t\n\"'\\><~|&;$*?#()`".contains(c)) {
        let escaped: String = arg
            .chars()
            .flat_map(|c| match c {
                '"' | '`' | '$' | '\\' => vec!['\\', c],
                _ => vec![c],
            })
            .collect();
        format!("\"{escaped}\"")
    } else {
        arg.to_string()
    }
}

/// The text of the autostart entry.
pub fn entry_text(exe: &Path, minimized: bool) -> String {
    let mut exec = quote_exec(&exe.to_string_lossy());
    if minimized {
        exec.push_str(" --minimized");
    }
    format!(
        "[Desktop Entry]\nType=Application\nName=Reclutched\nComment=Rocket League stats tracker\nExec={exec}\nIcon=dev.reclutched.Reclutched\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// Turns starting with the computer on or off. `minimized` starts it without showing the window
/// (tracking and the overlay still run).
pub fn set_autostart(on: bool, minimized: bool) -> Result<(), String> {
    let path = entry_path().ok_or("no home folder")?;
    if !on {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("reclutched-ui"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, entry_text(&exe, minimized)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_runs_the_program_and_can_start_minimized() {
        let plain = entry_text(Path::new("/home/me/.cargo/bin/reclutched-ui"), false);
        assert!(
            plain.contains("\nExec=/home/me/.cargo/bin/reclutched-ui\n"),
            "{plain}"
        );
        let mini = entry_text(Path::new("/usr/bin/reclutched-ui"), true);
        assert!(mini.contains("\nExec=/usr/bin/reclutched-ui --minimized\n"));
        assert!(mini.starts_with("[Desktop Entry]\nType=Application\n"));
    }

    #[test]
    fn a_path_with_spaces_is_quoted() {
        let text = entry_text(Path::new("/opt/my apps/reclutched-ui"), false);
        assert!(
            text.contains("Exec=\"/opt/my apps/reclutched-ui\"\n"),
            "{text}"
        );
    }
}
