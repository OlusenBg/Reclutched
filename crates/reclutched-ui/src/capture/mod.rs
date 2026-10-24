//! Capturing: screenshots, a rolling recording for clips, and whole-match recordings. The
//! picture comes from the desktop's screen-share portal (PipeWire) and is processed by
//! GStreamer.

pub mod gst;
mod portal;
pub mod record;
pub mod recorder;
pub mod ring;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use portal::Portal;
pub use portal::State;
pub use record::Config as RecordConfig;
pub use recorder::{cache_dir, save_range, Recorder, Saved};

/// What the settings page and the app show about capturing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Off,
    /// The desktop is asking what to capture.
    Asking,
    /// Ready; says what is captured ("1920 x 1080").
    Ready(String),
    Failed(String),
}

/// The elements a screenshot needs.
pub const SCREENSHOT_ELEMENTS: [&str; 4] = ["pipewiresrc", "videoconvert", "pngenc", "filesink"];

pub struct Capture {
    /// `None` in test mode, where a test picture replaces the screen.
    portal: Option<Portal>,
    test_on: std::cell::Cell<bool>,
}

impl Capture {
    /// `token_path` keeps what you chose to capture. `test` swaps the screen for a test picture.
    pub fn new(token_path: PathBuf, test: bool) -> Self {
        Self {
            portal: (!test).then(|| Portal::start(token_path)),
            test_on: std::cell::Cell::new(false),
        }
    }

    /// A test picture stands in for the screen (`--capture-test`).
    pub fn is_test(&self) -> bool {
        self.portal.is_none()
    }

    pub fn turn_on(&self) {
        match &self.portal {
            Some(portal) => portal.begin(),
            None => self.test_on.set(true),
        }
    }

    pub fn turn_off(&self) {
        match &self.portal {
            Some(portal) => portal.end(),
            None => self.test_on.set(false),
        }
    }

    /// Shows the desktop's picker again next time.
    pub fn forget_choice(&self) {
        if let Some(portal) = &self.portal {
            portal.forget_choice();
        }
    }

    pub fn status(&self) -> Status {
        match &self.portal {
            None if self.test_on.get() => Status::Ready("test picture".to_string()),
            None => Status::Off,
            Some(portal) => match portal.state() {
                State::Off => Status::Off,
                State::Asking => Status::Asking,
                State::Ready { size, .. } => Status::Ready(match size {
                    Some((w, h)) => format!("{w} x {h}"),
                    None => "your screen".to_string(),
                }),
                State::Failed(e) => Status::Failed(e),
            },
        }
    }

    /// Starts the rolling recording on a worker thread (picking an encoder takes a moment). The
    /// answer is the running recorder, or what is missing.
    pub fn start_recording(
        &self,
        config: RecordConfig,
    ) -> mpsc::Receiver<Result<Recorder, String>> {
        let (tx, rx) = mpsc::channel();
        let portal = self.portal.clone();
        std::thread::spawn(move || {
            let result = (|| {
                record::requirements()?;
                let encoder = record::encoder().ok_or("no video encoder")?;
                let aac = record::choose_aac(&gst::has_element);
                let (node, fd) = match &portal {
                    None => (None, None),
                    Some(portal) => match portal.state() {
                        State::Ready { node, .. } => (Some(node), Some(portal.remote()?)),
                        _ => return Err("capturing is not ready yet".to_string()),
                    },
                };
                Recorder::start(&cache_dir(), node, fd, &config, encoder, aac)
            })();
            let _ = tx.send(result);
        });
        rx
    }

    /// Saves a screenshot as `<folder>/Screenshots/Reclutched <stamp>.png` on a worker thread.
    /// The answer (the file, or what went wrong) arrives on the returned channel.
    pub fn screenshot(
        &self,
        folder: PathBuf,
        stamp: &str,
    ) -> mpsc::Receiver<Result<PathBuf, String>> {
        let (tx, rx) = mpsc::channel();
        let ready = match (&self.portal, self.test_on.get()) {
            (None, false) => None,
            (None, true) => Some(None),
            (Some(portal), _) => match portal.state() {
                State::Ready { node, .. } => Some(Some(node)),
                _ => None,
            },
        };
        let Some(node) = ready else {
            let _ = tx.send(Err("capturing is not on yet".to_string()));
            return rx;
        };
        let remote = match (&self.portal, node) {
            (Some(portal), Some(_)) => Some(portal.remote()),
            _ => None,
        };
        let path = folder
            .join("Screenshots")
            .join(format!("Reclutched {stamp}.png"));
        std::thread::spawn(move || {
            let result = take(&path, node, remote);
            let _ = tx.send(result.map(|()| path));
        });
        rx
    }
}

fn take(
    path: &std::path::Path,
    node: Option<u32>,
    remote: Option<Result<std::os::fd::OwnedFd, String>>,
) -> Result<(), String> {
    gst::check_elements(&if node.is_some() {
        SCREENSHOT_ELEMENTS.to_vec()
    } else {
        vec!["videotestsrc", "videoconvert", "pngenc", "filesink"]
    })?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let fd = match remote {
        Some(Ok(fd)) => Some(fd),
        Some(Err(e)) => return Err(e),
        None => None,
    };
    let args = match (&fd, node) {
        (Some(fd), Some(node)) => gst::screenshot_args(&gst::Source::Portal { fd, node }, path),
        _ => gst::screenshot_args(&gst::Source::Test, path),
    };
    gst::run(&args, fd, Duration::from_secs(15))?;
    match std::fs::metadata(path) {
        Ok(m) if m.len() > 0 => Ok(()),
        _ => Err("the screenshot came out empty".to_string()),
    }
}
