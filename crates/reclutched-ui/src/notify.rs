//! The little pop-up that slides down from the top of the screen, over the game, when a
//! screenshot, a clip or a match has been saved. It never takes clicks or keys.

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use reclutched_core::rocketleague::settings::Settings;

use crate::overlay::{make_overlay_layer, supported};

/// How a saved match went, for its picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Loss,
    LeftEarly,
}

/// What the pop-up says and shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Popup {
    pub title: &'static str,
    pub subtitle: String,
    /// A picture to show (a screenshot, or the first frame of a clip).
    pub image: Option<PathBuf>,
    /// A drawn badge instead of a picture (matches).
    pub outcome: Option<Outcome>,
}

impl Popup {
    pub fn screenshot(path: PathBuf) -> Self {
        Self {
            title: "Screenshot Saved",
            subtitle: file_name(&path),
            image: Some(path),
            outcome: None,
        }
    }

    /// `frame` is the first frame of the clip, when we have one.
    #[allow(dead_code)] // used once clips can be recorded
    pub fn clip(path: PathBuf, frame: Option<PathBuf>) -> Self {
        Self {
            title: "Clip Saved",
            subtitle: file_name(&path),
            image: frame,
            outcome: None,
        }
    }

    pub fn saved_match(mode: &str, score: &str, outcome: Outcome) -> Self {
        Self {
            title: "Match Saved",
            subtitle: format!("{mode}  \u{00B7}  {score}"),
            image: None,
            outcome: Some(outcome),
        }
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().to_string())
}

/// Which edges of the screen the pop-up hugs for a saved position.
pub fn edges(position: &str) -> (bool, bool) {
    // (left, right); it always hugs the top.
    match position {
        "top-left" => (true, false),
        "top-right" => (false, true),
        _ => (false, false),
    }
}

/// Plays the system's "camera shutter" or "message" sound if there is one, else the bell.
fn play_sound() {
    std::thread::spawn(|| {
        let found = [
            "/usr/share/sounds/freedesktop/stereo/camera-shutter.oga",
            "/usr/share/sounds/freedesktop/stereo/message.oga",
        ]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists());
        let played = found.is_some_and(|file| {
            ["pw-play", "paplay"].iter().any(|player| {
                Command::new(player)
                    .arg(file)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
            })
        });
        if !played {
            // Nothing to play with: a plain terminal bell is better than silence.
            print!("\x07");
        }
    });
}

/// Draws the badge of a match: a golden trophy for a win, "GG" for a loss, a dash for a match
/// you left.
fn badge(outcome: Outcome) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(72)
        .content_height(72)
        .valign(gtk::Align::Center)
        .build();
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (f64::from(w), f64::from(h));
        let tau = std::f64::consts::TAU;
        match outcome {
            Outcome::Win => {
                let gold = (0.96, 0.78, 0.25);
                let dark = (0.78, 0.58, 0.12);
                cr.set_source_rgb(gold.0, gold.1, gold.2);
                // The cup.
                cr.move_to(w * 0.26, h * 0.14);
                cr.line_to(w * 0.74, h * 0.14);
                cr.curve_to(w * 0.74, h * 0.52, w * 0.62, h * 0.60, w * 0.5, h * 0.60);
                cr.curve_to(w * 0.38, h * 0.60, w * 0.26, h * 0.52, w * 0.26, h * 0.14);
                cr.close_path();
                let _ = cr.fill();
                // Handles.
                cr.set_line_width(w * 0.06);
                cr.arc(w * 0.26, h * 0.28, w * 0.12, tau * 0.25, tau * 0.75);
                let _ = cr.stroke();
                cr.arc(w * 0.74, h * 0.28, w * 0.12, -tau * 0.25, tau * 0.25);
                let _ = cr.stroke();
                // Stem and base.
                cr.set_source_rgb(dark.0, dark.1, dark.2);
                cr.rectangle(w * 0.44, h * 0.60, w * 0.12, h * 0.16);
                let _ = cr.fill();
                cr.rectangle(w * 0.30, h * 0.76, w * 0.40, h * 0.10);
                let _ = cr.fill();
                // A shine.
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
                cr.rectangle(w * 0.33, h * 0.20, w * 0.05, h * 0.22);
                let _ = cr.fill();
            }
            Outcome::Loss | Outcome::LeftEarly => {
                let (r, g, b) = if outcome == Outcome::Loss {
                    (0.85, 0.33, 0.31)
                } else {
                    (0.79, 0.60, 0.23)
                };
                cr.set_source_rgba(r, g, b, 0.22);
                cr.arc(w / 2.0, h / 2.0, w * 0.44, 0.0, tau);
                let _ = cr.fill();
                cr.set_source_rgb(r, g, b);
                cr.set_line_width(w * 0.05);
                cr.arc(w / 2.0, h / 2.0, w * 0.44, 0.0, tau);
                let _ = cr.stroke();
                cr.set_font_size(h * 0.34);
                let text = if outcome == Outcome::Loss {
                    "GG"
                } else {
                    "\u{2013}"
                };
                if let Ok(ext) = cr.text_extents(text) {
                    cr.move_to(
                        w / 2.0 - ext.width() / 2.0 - ext.x_bearing(),
                        h / 2.0 - ext.height() / 2.0 - ext.y_bearing(),
                    );
                    let _ = cr.show_text(text);
                }
            }
        }
    });
    area
}

fn card(popup: &Popup) -> gtk::Box {
    let row = gtk::Box::builder()
        .spacing(16)
        .css_classes(["notify-card"])
        .build();
    if let Some(image) = &popup.image {
        let picture = gtk::Picture::for_filename(image);
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        let frame = gtk::Box::builder()
            .css_classes(["notify-thumb"])
            .overflow(gtk::Overflow::Hidden)
            .width_request(160)
            .height_request(90)
            .valign(gtk::Align::Center)
            .build();
        picture.set_hexpand(true);
        frame.append(&picture);
        row.append(&frame);
    } else if let Some(outcome) = popup.outcome {
        row.append(&badge(outcome));
    }
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    text.append(
        &gtk::Label::builder()
            .label(popup.title)
            .xalign(0.0)
            .css_classes(["notify-title"])
            .build(),
    );
    text.append(
        &gtk::Label::builder()
            .label(&popup.subtitle)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(34)
            .css_classes(["notify-sub"])
            .build(),
    );
    row.append(&text);
    // A recorded match shows its first frame on the left and the result badge on the right.
    if popup.image.is_some() {
        if let Some(outcome) = popup.outcome {
            let small = badge(outcome);
            small.set_content_width(56);
            small.set_content_height(56);
            row.append(&small);
        }
    }
    row
}

/// Shows pop-ups, one at a time (a new one replaces the old).
pub struct Notifier {
    app: adw::Application,
    current: RefCell<Option<gtk::Window>>,
}

impl Notifier {
    pub fn new(app: adw::Application) -> Rc<Self> {
        Rc::new(Self {
            app,
            current: RefCell::new(None),
        })
    }

    /// Whether this desktop can show it over other windows; otherwise the caller uses a toast.
    pub fn available() -> bool {
        supported()
    }

    /// Slides the pop-up in, keeps it for the saved time and slides it out again.
    pub fn show(self: &Rc<Self>, popup: &Popup, settings: &Settings) {
        if !settings.notify_enabled || !supported() {
            return;
        }
        if let Some(old) = self.current.borrow_mut().take() {
            old.destroy();
        }
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(320)
            .valign(gtk::Align::Start)
            .halign(gtk::Align::Center)
            .child(&card(popup))
            .build();
        let window = gtk::Window::builder()
            .application(&self.app)
            .title("Reclutched pop-up")
            .child(&revealer)
            .resizable(false)
            .default_width(560)
            .default_height(150)
            .build();
        make_overlay_layer(&window);
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("reclutched-notify"));
        window.set_anchor(Edge::Top, true);
        let (left, right) = edges(&settings.notify_position);
        window.set_anchor(Edge::Left, left);
        window.set_anchor(Edge::Right, right);
        window.set_margin(Edge::Top, 18);
        window.set_margin(Edge::Left, 18);
        window.set_margin(Edge::Right, 18);
        window.present();
        *self.current.borrow_mut() = Some(window.clone());
        if settings.notify_sound {
            play_sound();
        }

        // In, wait, out.
        {
            let revealer = revealer.clone();
            gtk::glib::timeout_add_local_once(Duration::from_millis(60), move || {
                revealer.set_reveal_child(true);
            });
        }
        let stay = u64::from(settings.notify_seconds.clamp(2, 20));
        let (weak_window, weak_revealer) = (window.downgrade(), revealer.downgrade());
        gtk::glib::timeout_add_local_once(
            Duration::from_secs(stay) + Duration::from_millis(380),
            move || {
                if let Some(revealer) = weak_revealer.upgrade() {
                    revealer.set_reveal_child(false);
                }
                let weak_window = weak_window.clone();
                gtk::glib::timeout_add_local_once(Duration::from_millis(420), move || {
                    if let Some(window) = weak_window.upgrade() {
                        window.destroy();
                    }
                });
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_say_what_was_saved() {
        let shot = Popup::screenshot(PathBuf::from("/a/b/Reclutched 2026.png"));
        assert_eq!(shot.title, "Screenshot Saved");
        assert_eq!(shot.subtitle, "Reclutched 2026.png");
        assert!(shot.image.is_some() && shot.outcome.is_none());
        let clip = Popup::clip(PathBuf::from("/a/clip.mp4"), None);
        assert_eq!(clip.title, "Clip Saved");
        assert!(clip.image.is_none());
        let m = Popup::saved_match("Casual Duel", "4 \u{2013} 2", Outcome::Win);
        assert_eq!(m.title, "Match Saved");
        assert_eq!(m.outcome, Some(Outcome::Win));
        assert!(m.subtitle.starts_with("Casual Duel"));
    }

    #[test]
    fn positions_pick_the_edges() {
        assert_eq!(edges("top"), (false, false));
        assert_eq!(edges("top-left"), (true, false));
        assert_eq!(edges("top-right"), (false, true));
        assert_eq!(edges("nonsense"), (false, false));
    }
}
