//! Runs the recording pipeline and cuts clips from what it wrote.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::Duration;

use super::gst::{self, Source};
use super::record::{self, Config, Encoder};
use super::ring;

/// A running recording. The pieces are in `ring`, the little pictures in `thumbs`.
pub struct Recorder {
    child: Child,
    pub ring: PathBuf,
    pub thumbs: PathBuf,
    /// The sound was recorded too (the clip must be told, to keep it).
    pub audio: bool,
    pub encoder: Encoder,
}

/// Where the rolling recording lives: `$XDG_CACHE_HOME/reclutched` (or `~/.cache/reclutched`).
pub fn cache_dir() -> PathBuf {
    let base = match std::env::var_os("XDG_CACHE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => std::env::var_os("HOME")
            .map_or_else(std::env::temp_dir, |h| PathBuf::from(h).join(".cache")),
    };
    base.join("reclutched")
}

fn clear_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for entry in std::fs::read_dir(dir)?.flatten() {
        if entry.path().is_file() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

impl Recorder {
    /// Starts recording. `fd` is the screen-share connection (inherited by the recorder), or
    /// `None` for the test picture. Old pieces are removed first.
    pub fn start(
        base: &Path,
        node: Option<u32>,
        fd: Option<std::os::fd::OwnedFd>,
        config: &Config,
        encoder: Encoder,
        aac: Option<&str>,
    ) -> Result<Self, String> {
        let ring = base.join("ring");
        let thumbs = base.join("thumbs");
        clear_dir(&ring).map_err(|e| format!("could not prepare {}: {e}", ring.display()))?;
        clear_dir(&thumbs).map_err(|e| format!("could not prepare {}: {e}", thumbs.display()))?;
        let args = match (&fd, node) {
            (Some(fd), Some(node)) => record::record_args(
                &Source::Portal { fd, node },
                &ring,
                &thumbs,
                config,
                encoder,
                aac,
            ),
            _ => record::record_args(&Source::Test, &ring, &thumbs, config, encoder, aac),
        };
        let child = gst::spawn(&args, fd)?;
        Ok(Self {
            child,
            ring,
            thumbs,
            audio: config.wants_audio() && aac.is_some(),
            encoder,
        })
    }

    /// "x264enc, with sound" for the settings page.
    pub fn describe(&self) -> String {
        format!(
            "Recording with {}{}",
            self.encoder.element(),
            if self.audio { ", with sound" } else { "" }
        )
    }

    /// Still running?
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// What it said when it stopped by itself (empty while it runs).
    pub fn complaint(&mut self) -> String {
        match self.child.try_wait() {
            Ok(Some(_)) => self
                .child
                .stderr
                .take()
                .map(|mut s| {
                    let mut text = String::new();
                    let _ = std::io::Read::read_to_string(&mut s, &mut text);
                    gst::last_line(&text)
                })
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// The newest piece's number.
    pub fn latest(&self) -> Option<u64> {
        ring::latest(&self.ring)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A clip or recording that was written.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub path: PathBuf,
    pub thumb: Option<PathBuf>,
    pub seconds: i64,
}

/// Cuts pieces `from..=to` out of `ring` into `out` (an MP4 without re-encoding; if that fails,
/// the plain `.ts` is kept) and copies the picture of the first piece to `thumb_out`.
pub fn save_range(
    ring_dir: &Path,
    thumbs: &Path,
    from: u64,
    to: u64,
    audio: bool,
    out: &Path,
    thumb_out: Option<&Path>,
) -> Result<Saved, String> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let joined = out.with_extension("joining.ts");
    ring::join(ring_dir, from, to, &joined).map_err(|e| e.to_string())?;
    let limit = Duration::from_secs(120);
    let mut result = gst::run(&record::remux_args(&joined, out, audio), None, limit);
    if result.is_err() && audio {
        // Sound that did not fit (or was never there): keep the picture at least.
        result = gst::run(&record::remux_args(&joined, out, false), None, limit);
    }
    let path = match result {
        Ok(()) if out.metadata().is_ok_and(|m| m.len() > 0) => {
            let _ = std::fs::remove_file(&joined);
            out.to_path_buf()
        }
        _ => {
            // Could not make an MP4: the glued pieces play fine on their own.
            let ts = out.with_extension("ts");
            std::fs::rename(&joined, &ts).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(out);
            ts
        }
    };
    let thumb = thumb_out.and_then(|dest| {
        let source = ring::thumb_for(thumbs, from, to)?;
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).ok()?;
        }
        std::fs::copy(source, dest).ok()?;
        Some(dest.to_path_buf())
    });
    Ok(Saved {
        path,
        thumb,
        seconds: ((to - from + 1) * u64::from(ring::SEGMENT_SECONDS)) as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Option<(Encoder, Option<&'static str>)> {
        gst::check_elements(&[
            "videotestsrc",
            "videoconvert",
            "videoscale",
            "videorate",
            "tee",
            "jpegenc",
            "multifilesink",
            "splitmuxsink",
            "mpegtsmux",
            "h264parse",
            "tsdemux",
            "mp4mux",
            "filesink",
        ])
        .ok()?;
        let encoder = record::pick_encoder()?;
        Some((encoder, record::choose_aac(&gst::has_element)))
    }

    /// Needs GStreamer with an H.264 encoder: records a test picture for a few seconds, then
    /// cuts a clip and checks that it is a real MP4 with a first-frame picture.
    #[test]
    fn a_recording_makes_a_clip_with_a_picture() {
        record_and_cut("clip-video-only", false, false);
        record_and_cut("clip-game-sound", true, false);
        // The sound of the game and the microphone mixed together.
        record_and_cut("clip-both-sounds", true, true);
    }

    fn record_and_cut(name: &str, game: bool, mic: bool) {
        let Some((encoder, aac)) = ready() else {
            eprintln!("skipped: GStreamer or an encoder is missing");
            return;
        };
        if (game || mic) && aac.is_none() {
            eprintln!("skipped {name}: no AAC encoder");
            return;
        }
        let base =
            std::env::temp_dir().join(format!("reclutched-rec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let config = Config {
            fps: 30,
            preset: "low".into(),
            audio_game: game,
            audio_mic: mic,
        };
        let mut recorder = Recorder::start(&base, None, None, &config, encoder, aac).unwrap();
        // Wait for a few pieces.
        let mut waited = 0;
        while recorder.latest().unwrap_or(0) < 4 && waited < 100 {
            assert!(
                recorder.alive(),
                "the recorder stopped: {}",
                recorder.complaint()
            );
            std::thread::sleep(Duration::from_millis(200));
            waited += 1;
        }
        let latest = recorder.latest().expect("pieces were written");
        assert!(latest >= 4, "only {latest} pieces");
        let from = ring::clip_start(latest, 4);
        let out = base.join("clips").join("c.mp4");
        let thumb = base.join("clips").join("c.jpg");
        let saved = save_range(
            &recorder.ring,
            &recorder.thumbs,
            from,
            latest,
            recorder.audio,
            &out,
            Some(&thumb),
        )
        .unwrap();
        recorder.stop();
        assert_eq!(saved.path, out, "{name}: it should be an MP4");
        let size = std::fs::metadata(&out).unwrap().len();
        assert!(size > 10_000, "clip is {size} bytes");
        let picture = std::fs::read(saved.thumb.as_ref().expect("a first-frame picture")).unwrap();
        assert!(picture.starts_with(&[0xFF, 0xD8]), "not a JPEG");
        // The picture keeps its shape (16:9 in, 16:9 out).
        if let Ok(probe) = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height",
                "-of",
                "csv=p=0",
            ])
            .arg(&out)
            .output()
        {
            if probe.status.success() {
                let text = String::from_utf8_lossy(&probe.stdout).trim().to_string();
                let sizes: Vec<u32> = text.split(',').filter_map(|n| n.parse().ok()).collect();
                assert_eq!(sizes.len(), 2, "{text}");
                assert_eq!(
                    sizes[0] * 9,
                    sizes[1] * 16,
                    "{name}: the picture was squashed: {text}"
                );
            }
        }
        if let Ok(probe) = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(&out)
            .output()
        {
            if probe.status.success() {
                let seconds: f64 = String::from_utf8_lossy(&probe.stdout)
                    .trim()
                    .parse()
                    .unwrap();
                assert!((2.0..=14.0).contains(&seconds), "clip is {seconds} s");
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
