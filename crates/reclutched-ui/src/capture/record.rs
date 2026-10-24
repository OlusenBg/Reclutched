//! The recording pipeline: what `gst-launch-1.0` is asked to do to keep the rolling recording,
//! how clips are cut from it, and which video and audio encoders the computer has. The
//! argument lists come from pure functions that are tested without any video.

use std::path::Path;
use std::process::{Command, Stdio};

use super::gst::{quote, source_args, Source};
use super::ring;

/// The H.264 encoders we know, best first: hardware ones (any GPU), then software.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    /// `vah264enc`: Intel and AMD through the newer VA plugin.
    Va,
    /// `vaapih264enc`: Intel and AMD through the older VA-API plugin.
    Vaapi,
    /// `nvh264enc`: NVIDIA.
    Nvenc,
    /// `x264enc`: software, works everywhere.
    X264,
    /// `openh264enc`: software, when x264 is not installed.
    OpenH264,
}

pub const ENCODERS: [Encoder; 5] = [
    Encoder::Va,
    Encoder::Vaapi,
    Encoder::Nvenc,
    Encoder::X264,
    Encoder::OpenH264,
];

impl Encoder {
    pub fn element(self) -> &'static str {
        match self {
            Encoder::Va => "vah264enc",
            Encoder::Vaapi => "vaapih264enc",
            Encoder::Nvenc => "nvh264enc",
            Encoder::X264 => "x264enc",
            Encoder::OpenH264 => "openh264enc",
        }
    }

    pub fn is_hardware(self) -> bool {
        !matches!(self, Encoder::X264 | Encoder::OpenH264)
    }

    /// The element and its settings, for a bitrate in kbit/s and a keyframe every `gop` frames.
    pub fn args(self, kbps: u32, gop: u32) -> Vec<String> {
        let mut args = vec![self.element().to_string()];
        let extra: Vec<String> = match self {
            Encoder::Va => vec![format!("bitrate={kbps}"), format!("key-int-max={gop}")],
            Encoder::Vaapi => vec![format!("bitrate={kbps}"), format!("keyframe-period={gop}")],
            Encoder::Nvenc => vec![
                format!("bitrate={kbps}"),
                format!("gop-size={gop}"),
                "preset=low-latency-hq".to_string(),
            ],
            Encoder::X264 => vec![
                format!("bitrate={kbps}"),
                format!("key-int-max={gop}"),
                "tune=zerolatency".to_string(),
                "speed-preset=veryfast".to_string(),
            ],
            // openh264 wants bits per second.
            Encoder::OpenH264 => vec![
                format!("bitrate={}", u64::from(kbps) * 1000),
                format!("gop-size={gop}"),
            ],
        };
        args.extend(extra);
        args
    }
}

/// The first encoder in `ENCODERS` order that exists and passes `works`.
pub fn choose_encoder(
    exists: &dyn Fn(&str) -> bool,
    works: &dyn Fn(Encoder) -> bool,
) -> Option<Encoder> {
    ENCODERS
        .into_iter()
        .filter(|e| exists(e.element()))
        .find(|e| works(*e))
}

/// Whether an encoder really runs here (a GPU encoder can be installed but unusable).
pub fn probe_encoder(encoder: Encoder) -> bool {
    let mut args = vec![
        "-q",
        "videotestsrc",
        "num-buffers=15",
        "!",
        "video/x-raw,width=640,height=360,framerate=30/1",
        "!",
        "videoconvert",
        "!",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    args.extend(encoder.args(2000, 30));
    args.extend(["!", "fakesink"].map(String::from));
    Command::new("gst-launch-1.0")
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The best encoder this computer can use.
pub fn pick_encoder() -> Option<Encoder> {
    choose_encoder(&super::gst::has_element, &probe_encoder)
}

/// Same, but asked only once per run (testing an encoder takes a moment).
pub fn encoder() -> Option<Encoder> {
    static FOUND: std::sync::OnceLock<Option<Encoder>> = std::sync::OnceLock::new();
    *FOUND.get_or_init(pick_encoder)
}

/// What the recording needs besides an encoder.
pub const RECORD_ELEMENTS: [&str; 10] = [
    "videoconvert",
    "videoscale",
    "videorate",
    "tee",
    "jpegenc",
    "multifilesink",
    "splitmuxsink",
    "mpegtsmux",
    "h264parse",
    "mp4mux",
];

/// Whether recording can work here: the encoder it would use, or a sentence about what is missing.
pub fn requirements() -> Result<String, String> {
    super::gst::check_elements(&RECORD_ELEMENTS)?;
    super::gst::check_elements(&["tsdemux"])?;
    let Some(encoder) = encoder() else {
        return Err("No H.264 encoder found (on Arch: gst-plugins-ugly for x264, or gst-plugin-va for AMD and Intel GPUs)".to_string());
    };
    let kind = if encoder.is_hardware() {
        "GPU"
    } else {
        "software"
    };
    let sound = match choose_aac(&super::gst::has_element) {
        Some(_) => "with sound",
        None => "no sound (no AAC encoder: gst-libav)",
    };
    Ok(format!("{} ({kind}), {sound}", encoder.element()))
}

/// AAC encoders, in the order we like them.
pub const AAC_ENCODERS: [&str; 4] = ["avenc_aac", "fdkaacenc", "voaacenc", "faac"];

pub fn choose_aac(exists: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    AAC_ENCODERS.into_iter().find(|e| exists(e))
}

/// How good the picture is: its height (`None` keeps the screen's own) and its bitrate for one
/// second of 60 frames at that height.
pub fn quality(preset: &str) -> (Option<u32>, u32) {
    match preset {
        "low" => (Some(720), 4_000),
        "high" => (None, 14_000),
        _ => (Some(1080), 8_000),
    }
}

/// The bitrate (kbit/s) for a preset and a frame rate.
pub fn bitrate_kbps(preset: &str, fps: u32) -> u32 {
    let (_, base) = quality(preset);
    (base * fps.clamp(15, 120) / 60).max(1_500)
}

/// What to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub fps: u32,
    pub preset: String,
    pub audio_game: bool,
    pub audio_mic: bool,
}

impl Config {
    pub fn wants_audio(&self) -> bool {
        self.audio_game || self.audio_mic
    }
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(String::from).collect()
}

/// The audio part of the pipeline: game sound and/or the microphone into one AAC track.
fn audio_args(config: &Config, aac: &str, test: bool) -> Vec<String> {
    let source = |device: &str| -> Vec<String> {
        if test {
            words("audiotestsrc is-live=true wave=sine freq=440 volume=0.03")
        } else {
            let mut args = vec!["pulsesrc".to_string()];
            if !device.is_empty() {
                args.push(format!("device={device}"));
            }
            args
        }
    };
    let mut out = Vec::new();
    let both = config.audio_game && config.audio_mic;
    let tail = |out: &mut Vec<String>| {
        out.extend(words(&format!(
            "! audioconvert ! audioresample ! {aac} bitrate=160000 ! aacparse ! queue ! sink.audio_0"
        )));
    };
    if both {
        out.extend(words("audiomixer name=mix"));
        tail(&mut out);
        for device in ["@DEFAULT_MONITOR@", ""] {
            out.extend(source(device));
            out.extend(words("! audioconvert ! audioresample ! queue ! mix."));
        }
    } else {
        out.extend(source(if config.audio_game {
            "@DEFAULT_MONITOR@"
        } else {
            ""
        }));
        tail(&mut out);
    }
    out
}

/// The whole recording pipeline: the picture is scaled, given a steady frame rate, encoded and
/// written as pieces in `dir`; a small picture every piece goes to `thumbs`; the sound (when
/// wanted and an AAC encoder exists) is mixed and muxed in.
pub fn record_args(
    source: &Source,
    dir: &Path,
    thumbs: &Path,
    config: &Config,
    encoder: Encoder,
    aac: Option<&str>,
) -> Vec<String> {
    let test = matches!(source, Source::Test);
    let (height, _) = quality(&config.preset);
    let fps = config.fps.clamp(15, 120);
    // `pixel-aspect-ratio=1/1` makes the scaler work out the other side, keeping the picture undistorted.
    let mut caps = format!("video/x-raw,framerate={fps}/1,pixel-aspect-ratio=1/1");
    if let Some(h) = height {
        caps.push_str(&format!(",height={h}"));
    }
    let mut args = words("-q");
    args.extend(source_args(source));
    // The raw picture is shared by the two branches below, each with its own scaler (a scaler
    // before the split would be told its size by both and squash the picture).
    args.extend(words("! videoconvert ! tee name=t"));
    // The picture: scale, steady frame rate, encode, write.
    args.extend(words(
        "t. ! queue max-size-time=4000000000 ! videoscale ! videorate !",
    ));
    args.push(caps);
    args.extend(words("! videoconvert !"));
    args.extend(encoder.args(bitrate_kbps(&config.preset, fps), fps * 2));
    args.extend(words(
        "! h264parse config-interval=-1 ! queue max-size-time=4000000000 ! sink.video",
    ));
    // A small picture for every piece, for the clip's first frame.
    args.extend(words(
        "t. ! queue leaky=downstream max-size-buffers=2 ! videorate ! video/x-raw,framerate=1/2 ! videoscale ! video/x-raw,width=320,pixel-aspect-ratio=1/1 ! videoconvert ! jpegenc quality=70 ! multifilesink max-files=60",
    ));
    args.push(format!("location={}", quote(&ring::thumb_pattern(thumbs))));
    // The sound.
    if let (true, Some(aac)) = (config.wants_audio(), aac) {
        args.extend(audio_args(config, aac, test));
    }
    // The muxer, writing a new piece every few seconds.
    args.extend(words("splitmuxsink name=sink muxer-factory=mpegtsmux"));
    args.push(format!(
        "max-size-time={}",
        u64::from(ring::SEGMENT_SECONDS) * 1_000_000_000
    ));
    args.push(format!("location={}", quote(&ring::segment_pattern(dir))));
    args
}

/// Turns glued-together pieces into an MP4 without re-encoding.
pub fn remux_args(input: &Path, output: &Path, with_audio: bool) -> Vec<String> {
    let mut args = words("-q filesrc");
    args.push(format!("location={}", quote(&input.to_string_lossy())));
    args.extend(words(
        "! tsdemux name=d mp4mux name=m faststart=true ! filesink",
    ));
    args.push(format!("location={}", quote(&output.to_string_lossy())));
    args.extend(words("d. ! video/x-h264 ! queue ! h264parse ! m."));
    if with_audio {
        args.extend(words("d. ! audio/mpeg ! queue ! aacparse ! m."));
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn config(game: bool, mic: bool) -> Config {
        Config {
            fps: 60,
            preset: "medium".into(),
            audio_game: game,
            audio_mic: mic,
        }
    }

    fn line(args: &[String]) -> String {
        args.join(" ")
    }

    #[test]
    fn the_first_working_encoder_wins_and_hardware_comes_first() {
        let all = |_: &str| true;
        let none = |_: &str| false;
        assert_eq!(choose_encoder(&all, &|_| true), Some(Encoder::Va));
        // An installed GPU encoder that does not actually run is skipped.
        assert_eq!(
            choose_encoder(&all, &|e| !e.is_hardware()),
            Some(Encoder::X264)
        );
        let only_openh264 = |name: &str| name == "openh264enc";
        assert_eq!(
            choose_encoder(&only_openh264, &|_| true),
            Some(Encoder::OpenH264)
        );
        assert_eq!(choose_encoder(&none, &|_| true), None);
    }

    #[test]
    fn each_encoder_gets_its_own_settings() {
        assert_eq!(
            line(&Encoder::X264.args(8000, 120)),
            "x264enc bitrate=8000 key-int-max=120 tune=zerolatency speed-preset=veryfast"
        );
        assert_eq!(
            line(&Encoder::Vaapi.args(8000, 120)),
            "vaapih264enc bitrate=8000 keyframe-period=120"
        );
        assert_eq!(
            line(&Encoder::OpenH264.args(2000, 60)),
            "openh264enc bitrate=2000000 gop-size=60"
        );
        assert!(line(&Encoder::Nvenc.args(1, 1)).contains("gop-size=1"));
        assert!(line(&Encoder::Va.args(1, 1)).contains("key-int-max=1"));
    }

    #[test]
    fn quality_sets_height_and_bitrate_with_the_frame_rate() {
        assert_eq!(quality("low").0, Some(720));
        assert_eq!(quality("medium").0, Some(1080));
        assert_eq!(quality("high").0, None);
        assert_eq!(quality("anything").0, Some(1080));
        assert_eq!(bitrate_kbps("medium", 60), 8_000);
        assert_eq!(bitrate_kbps("medium", 30), 4_000);
        assert_eq!(bitrate_kbps("low", 15), 1_500);
    }

    #[test]
    fn an_aac_encoder_is_found_or_not() {
        assert_eq!(choose_aac(&|e| e == "voaacenc"), Some("voaacenc"));
        assert_eq!(choose_aac(&|_| true), Some("avenc_aac"));
        assert_eq!(choose_aac(&|_| false), None);
    }

    #[test]
    fn the_recording_pipeline_writes_pieces_and_pictures() {
        let args = record_args(
            &Source::Test,
            &PathBuf::from("/tmp/r a/ring"),
            &PathBuf::from("/tmp/r a/thumbs"),
            &config(false, false),
            Encoder::X264,
            None,
        );
        let text = line(&args);
        assert!(text.starts_with("-q videotestsrc"), "{text}");
        assert!(text.contains("videoconvert ! tee name=t"));
        assert!(text.contains(
            "videoscale ! videorate ! video/x-raw,framerate=60/1,pixel-aspect-ratio=1/1,height=1080 ! videoconvert ! x264enc"
        ));
        assert!(
            text.contains("video/x-raw,width=320,pixel-aspect-ratio=1/1 ! videoconvert ! jpegenc")
        );
        assert!(text.contains("x264enc bitrate=8000 key-int-max=120"));
        assert!(text.contains("h264parse config-interval=-1"));
        assert!(text.contains("jpegenc quality=70 ! multifilesink max-files=60 location=\"/tmp/r a/thumbs/th%08d.jpg\""));
        assert!(text.contains("splitmuxsink name=sink muxer-factory=mpegtsmux max-size-time=2000000000 location=\"/tmp/r a/ring/seg%08d.ts\""));
        assert!(!text.contains("audio"), "no sound was asked for: {text}");
    }

    #[test]
    fn sound_is_the_game_the_microphone_or_both_mixed() {
        let build = |game, mic, aac| {
            line(&record_args(
                &Source::Test,
                &PathBuf::from("/r"),
                &PathBuf::from("/t"),
                &config(game, mic),
                Encoder::X264,
                aac,
            ))
        };
        let game = build(true, false, Some("voaacenc"));
        assert!(
            game.contains("audiotestsrc")
                && game.contains("voaacenc bitrate=160000 ! aacparse ! queue ! sink.audio_0")
        );
        assert!(!game.contains("audiomixer"));
        let both = build(true, true, Some("voaacenc"));
        assert!(both.contains("audiomixer name=mix ! audioconvert"));
        assert_eq!(both.matches("! queue ! mix.").count(), 2);
        // Sound wanted but no AAC encoder: video only.
        assert!(!build(true, true, None).contains("audio"));
    }

    #[test]
    fn real_sound_comes_from_pulse_devices() {
        let node = std::fs::File::open("/dev/null").unwrap();
        let fd: std::os::fd::OwnedFd = node.into();
        let text = line(&record_args(
            &Source::Portal { fd: &fd, node: 42 },
            &PathBuf::from("/r"),
            &PathBuf::from("/t"),
            &config(true, true),
            Encoder::X264,
            Some("voaacenc"),
        ));
        assert!(text.contains("pipewiresrc"));
        assert!(text.contains("path=42"));
        assert!(text.contains("pulsesrc device=@DEFAULT_MONITOR@"));
        // The microphone is the default source, no device given.
        assert!(text.contains("pulsesrc ! audioconvert"));
    }

    #[test]
    fn remuxing_copies_video_and_optionally_sound() {
        let with = line(&remux_args(
            &PathBuf::from("/a b/in.ts"),
            &PathBuf::from("/o.mp4"),
            true,
        ));
        assert!(with.contains("filesrc location=\"/a b/in.ts\""));
        assert!(with.contains("d. ! audio/mpeg ! queue ! aacparse ! m."));
        let without = line(&remux_args(
            &PathBuf::from("/in.ts"),
            &PathBuf::from("/o.mp4"),
            false,
        ));
        assert!(!without.contains("audio/mpeg"));
    }
}
