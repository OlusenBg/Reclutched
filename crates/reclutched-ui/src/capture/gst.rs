//! Talks to GStreamer by running `gst-launch-1.0`, so Reclutched needs no GStreamer libraries to
//! build and works with whatever GStreamer your distribution has. The pipelines are plain
//! argument lists, built by pure functions that are tested without any video.

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where the picture comes from.
pub enum Source<'a> {
    /// A PipeWire stream from the screen-share portal: our own connection `fd` and the stream's
    /// node id.
    Portal { fd: &'a OwnedFd, node: u32 },
    /// A moving test picture, for trying the rest without a screen (`--capture-test`).
    Test,
}

/// A value inside a pipeline description. `gst-launch` glues its arguments together and parses
/// the result, so anything with a space or a quote has to be quoted by us.
pub fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn source_args(source: &Source) -> Vec<String> {
    match source {
        Source::Portal { fd, node } => vec![
            "pipewiresrc".into(),
            format!("fd={}", fd.as_raw_fd()),
            format!("path={node}"),
            "do-timestamp=true".into(),
        ],
        Source::Test => vec![
            "videotestsrc".into(),
            "is-live=true".into(),
            "pattern=ball".into(),
            "!".into(),
            "video/x-raw,width=1280,height=720,framerate=30/1".into(),
        ],
    }
}

/// One frame of the source saved as a PNG.
pub fn screenshot_args(source: &Source, path: &Path) -> Vec<String> {
    let mut args = vec!["-q".to_string(), "-e".to_string()];
    args.extend(source_args(source));
    args.extend(
        [
            "!",
            "videoconvert",
            "!",
            "pngenc",
            "snapshot=true",
            "!",
            "filesink",
        ]
        .map(String::from),
    );
    args.push(format!("location={}", quote(&path.to_string_lossy())));
    args
}

/// The elements a step needs, and the first one that is missing. `Err` carries a sentence for
/// the user: GStreamer itself is not installed, or which part is missing.
pub fn check_elements(elements: &[&str]) -> Result<(), String> {
    if which("gst-launch-1.0").is_none() || which("gst-inspect-1.0").is_none() {
        return Err("GStreamer is not installed (gst-launch-1.0 was not found)".to_string());
    }
    let missing: Vec<&str> = elements
        .iter()
        .copied()
        .filter(|e| {
            !Command::new("gst-inspect-1.0")
                .args(["--exists", e])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "GStreamer is missing: {} (on Arch: gst-plugins-base gst-plugins-good gst-plugin-pipewire)",
            missing.join(", ")
        ))
    }
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(|dir| Path::new(dir).join(program))
        .find(|p| p.is_file())
}

/// Runs `gst-launch-1.0` and waits for it (killing it after `limit`). `keep` is a file
/// descriptor the child has to inherit; it is closed here once the child has started.
pub fn run(args: &[String], keep: Option<OwnedFd>, limit: Duration) -> Result<(), String> {
    if let Some(fd) = &keep {
        // Descriptors are closed on exec by default; this one has to survive into gst-launch.
        rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::empty())
            .map_err(|e| format!("could not share the screen stream: {e}"))?;
    }
    let mut child = Command::new("gst-launch-1.0")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start gst-launch-1.0: {e}"))?;
    drop(keep);
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = child
                    .stderr
                    .take()
                    .map(|mut s| {
                        let mut text = String::new();
                        let _ = std::io::Read::read_to_string(&mut s, &mut text);
                        text
                    })
                    .unwrap_or_default();
                return if status.success() {
                    Ok(())
                } else {
                    Err(last_line(&stderr))
                };
            }
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the capture did not finish in time".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("GStreamer stopped without saying why")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_with_spaces_and_quotes_are_quoted() {
        assert_eq!(quote("/home/a b/c.png"), "\"/home/a b/c.png\"");
        assert_eq!(quote(r#"x"y\z"#), r#""x\"y\\z""#);
    }

    #[test]
    fn the_screenshot_pipeline_is_a_one_frame_png() {
        let args = screenshot_args(&Source::Test, Path::new("/tmp/My Shots/a.png"));
        let line = args.join(" ");
        assert!(line.starts_with("-q -e videotestsrc"), "{line}");
        assert!(line.contains("! videoconvert ! pngenc snapshot=true ! filesink"));
        assert!(line.ends_with("location=\"/tmp/My Shots/a.png\""));
    }

    #[test]
    fn the_last_line_of_gstreamers_complaint_is_used() {
        assert_eq!(
            last_line("a\n\nERROR: no element \"x\"\n\n"),
            "ERROR: no element \"x\""
        );
        assert_eq!(last_line(""), "GStreamer stopped without saying why");
    }

    /// Needs GStreamer (`gst-launch-1.0` with videotestsrc, videoconvert and pngenc); skipped
    /// without it.
    #[test]
    fn a_test_screenshot_is_really_written() {
        if check_elements(&["videotestsrc", "videoconvert", "pngenc", "filesink"]).is_err() {
            eprintln!("skipped: GStreamer is not installed");
            return;
        }
        let dir = std::env::temp_dir().join(format!("reclutched-shot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a shot.png");
        run(
            &screenshot_args(&Source::Test, &path),
            None,
            Duration::from_secs(20),
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"), "not a PNG");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
