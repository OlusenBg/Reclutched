//! Recent screenshots and clips as a slideshow on the Main Menu.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use adw::prelude::*;

use crate::carousel::carousel;
use crate::widgets::label;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Screenshot,
    Clip,
}

#[derive(Debug, Clone)]
pub struct Media {
    pub path: PathBuf,
    pub kind: Kind,
    pub modified: SystemTime,
}

const PICTURES: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
const VIDEOS: [&str; 5] = ["mp4", "mkv", "webm", "mov", "ts"];

/// How many pictures the slideshow looks at.
pub const MAX_ITEMS: usize = 12;
/// How tall a picture is, in pixels.
const PICTURE_HEIGHT: i32 = 160;
/// How many sit side by side on one slide.
const PER_SLIDE: usize = 3;

/// The newest screenshots (`<folder>/Screenshots`) and clips (`<folder>/Clips`), newest first.
pub fn recent_media(folder: &Path) -> Vec<Media> {
    let mut found = Vec::new();
    for (sub, kind, extensions) in [
        ("Screenshots", Kind::Screenshot, &PICTURES[..]),
        ("Clips", Kind::Clip, &VIDEOS[..]),
    ] {
        let Ok(entries) = std::fs::read_dir(folder.join(sub)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let wanted = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.contains(&e.to_lowercase().as_str()));
            let Ok(meta) = entry.metadata() else { continue };
            if wanted && meta.is_file() && meta.len() > 0 {
                found.push(Media {
                    path,
                    kind,
                    modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                });
            }
        }
    }
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
    found.truncate(MAX_ITEMS);
    found
}

fn when_text(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    gtk::glib::DateTime::from_unix_local(secs)
        .ok()
        .and_then(|t| t.format("%b %-d, %H:%M").ok())
        .map_or_else(String::new, |s| s.to_string())
}

fn tile(media: &Media) -> gtk::Box {
    let thumb: gtk::Widget = match media.kind {
        Kind::Screenshot => {
            let picture = gtk::Picture::for_filename(&media.path);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_can_shrink(true);
            picture.upcast()
        }
        Kind::Clip => gtk::Image::builder()
            .icon_name("media-playback-start-symbolic")
            .pixel_size(48)
            .css_classes(["gallery-play"])
            .build()
            .upcast(),
    };
    thumb.set_hexpand(true);
    thumb.set_halign(gtk::Align::Fill);
    thumb.set_valign(gtk::Align::Fill);
    // However tall the window is, a picture is never taller than this.
    let clamp = adw::Clamp::builder()
        .orientation(gtk::Orientation::Vertical)
        .maximum_size(PICTURE_HEIGHT)
        .tightening_threshold(PICTURE_HEIGHT)
        .child(&thumb)
        .build();
    let frame = gtk::Box::builder()
        .css_classes(["gallery-thumb"])
        .height_request(PICTURE_HEIGHT)
        .hexpand(true)
        .valign(gtk::Align::Start)
        .overflow(gtk::Overflow::Hidden)
        .build();
    frame.append(&clamp);
    let kind = match media.kind {
        Kind::Screenshot => "Screenshot",
        Kind::Clip => "Clip",
    };
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_hexpand(true);
    column.set_width_request(0);
    column.append(&frame);
    column.append(&label(
        &format!("{kind}  \u{00B7}  {}", when_text(media.modified)),
        &["gallery-caption"],
    ));
    // A click opens the file in your usual viewer.
    let click = gtk::GestureClick::new();
    let path = media.path.clone();
    click.connect_released(move |_, _, _, _| {
        let file = gtk::gio::File::for_path(&path);
        gtk::FileLauncher::new(Some(&file)).launch(
            None::<&gtk::Window>,
            gtk::gio::Cancellable::NONE,
            |_| {},
        );
    });
    column.add_controller(click);
    column.set_cursor_from_name(Some("pointer"));
    column
}

/// The slideshow, or a short sentence when there is nothing yet.
pub fn slideshow(items: &[Media]) -> gtk::Widget {
    if items.is_empty() {
        return label("Very empty isn't it....", &["muted-note"]).upcast();
    }
    let pages: Vec<gtk::Widget> = items
        .chunks(PER_SLIDE)
        .map(|chunk| {
            let row = gtk::Box::builder()
                .spacing(16)
                .homogeneous(true)
                .hexpand(true)
                .build();
            for media in chunk {
                row.append(&tile(media));
            }
            // Empty slots keep a lone picture the same size as when the slide is full.
            for _ in chunk.len()..PER_SLIDE {
                row.append(&gtk::Box::builder().hexpand(true).build());
            }
            row.upcast()
        })
        .collect();
    carousel(&pages, 5).upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pictures_in_screenshots_and_videos_in_clips_count_newest_first() {
        let dir = std::env::temp_dir().join(format!("reclutched-gallery-{}", std::process::id()));
        let (shots, clips) = (dir.join("Screenshots"), dir.join("Clips"));
        std::fs::create_dir_all(&shots).unwrap();
        std::fs::create_dir_all(&clips).unwrap();
        std::fs::write(shots.join("a.png"), b"x").unwrap();
        std::fs::write(shots.join("notes.txt"), b"x").unwrap();
        std::fs::write(shots.join("empty.png"), b"").unwrap();
        std::fs::write(clips.join("b.MP4"), b"x").unwrap();
        std::fs::write(clips.join("c.png"), b"x").unwrap(); // a picture in Clips: ignored
        let found = recent_media(&dir);
        let mut names: Vec<_> = found
            .iter()
            .map(|m| m.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["a.png", "b.MP4"]);
        assert!(found.iter().any(|m| m.kind == Kind::Clip));
        assert!(recent_media(&dir.join("nowhere")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
