//! The rolling recording on disk: short MPEG-TS pieces (segments) written one after the other.
//! A clip is the last few pieces glued together and turned into an MP4; a full match is every
//! piece from its start. Plain file work, tested without any video.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// How long one piece is.
pub const SEGMENT_SECONDS: u32 = 2;
/// The pieces kept for clips: the longest clip (50 s) plus a margin, in pieces.
pub const KEEP_PIECES: u64 = 34;
/// MPEG-TS is made of packets of this many bytes; a half-written file is cut to whole packets.
const TS_PACKET: u64 = 188;

pub fn segment_name(index: u64) -> String {
    format!("seg{index:08}.ts")
}

/// The `location=` pattern for the recorder.
pub fn segment_pattern(dir: &Path) -> String {
    dir.join("seg%08d.ts").to_string_lossy().to_string()
}

/// (index, path, size) of the pieces in `dir`, oldest first.
pub fn list(dir: &Path) -> Vec<(u64, PathBuf, u64)> {
    let mut found: Vec<(u64, PathBuf, u64)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let index = name
                .strip_prefix("seg")?
                .strip_suffix(".ts")?
                .parse::<u64>()
                .ok()?;
            Some((index, entry.path(), entry.metadata().ok()?.len()))
        })
        .collect();
    found.sort_by_key(|(i, _, _)| *i);
    found
}

/// The number of the newest piece (the one being written).
pub fn latest(dir: &Path) -> Option<u64> {
    list(dir).last().map(|(i, _, _)| *i)
}

/// Deletes every piece before `keep_from`.
pub fn prune_before(dir: &Path, keep_from: u64) {
    for (index, path, _) in list(dir) {
        if index < keep_from {
            let _ = fs::remove_file(path);
        }
    }
}

/// Which piece a clip of the last `seconds` seconds starts with, given the newest piece.
pub fn clip_start(latest: u64, seconds: u32) -> u64 {
    let pieces = u64::from(seconds.div_ceil(SEGMENT_SECONDS));
    latest.saturating_sub(pieces)
}

/// Where to prune to when only the last [`KEEP_PIECES`] pieces matter, unless a full-match
/// recording started earlier and still needs its pieces.
pub fn keep_from(latest: u64, match_started_at: Option<u64>) -> u64 {
    let window = latest.saturating_sub(KEEP_PIECES);
    match match_started_at {
        Some(start) => window.min(start),
        None => window,
    }
}

/// Glues the pieces `from..=to` into one TS file. Returns how many bytes it wrote.
pub fn join(dir: &Path, from: u64, to: u64, out: &Path) -> io::Result<u64> {
    let mut output = File::create(out)?;
    let mut total = 0;
    for (index, path, _) in list(dir) {
        if index < from || index > to {
            continue;
        }
        let mut bytes = Vec::new();
        File::open(&path)?.read_to_end(&mut bytes)?;
        // The newest piece may still be growing: keep only whole packets.
        let whole = bytes.len() as u64 / TS_PACKET * TS_PACKET;
        output.write_all(&bytes[..whole as usize])?;
        total += whole;
    }
    output.flush()?;
    if total == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "nothing has been recorded yet",
        ));
    }
    Ok(total)
}

/// The little picture taken with piece `index` (one every [`SEGMENT_SECONDS`]).
pub fn thumb_path(thumbs: &Path, index: u64) -> PathBuf {
    thumbs.join(format!("th{index:08}.jpg"))
}

pub fn thumb_pattern(thumbs: &Path) -> String {
    thumbs.join("th%08d.jpg").to_string_lossy().to_string()
}

/// The picture closest to the start of a clip that starts with piece `from`: that piece's, or
/// the next one that exists.
pub fn thumb_for(thumbs: &Path, from: u64, latest: u64) -> Option<PathBuf> {
    (from..=latest.max(from) + 1)
        .map(|i| thumb_path(thumbs, i))
        .find(|p| p.is_file())
}

/// Deletes the pictures before `keep_from`.
pub fn prune_thumbs(thumbs: &Path, keep_from: u64) {
    for entry in fs::read_dir(thumbs).into_iter().flatten().flatten() {
        let index = entry
            .file_name()
            .to_string_lossy()
            .strip_prefix("th")
            .and_then(|n| n.strip_suffix(".jpg"))
            .and_then(|n| n.parse::<u64>().ok());
        if index.is_some_and(|i| i < keep_from) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Deletes the files in `dir` older than `days` days (0 keeps everything). Returns how many.
pub fn delete_older_than(dir: &Path, days: u32) -> usize {
    if days == 0 {
        return 0;
    }
    let limit = std::time::Duration::from_secs(u64::from(days) * 86_400);
    let mut gone = 0;
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > limit);
        if old && entry.path().is_file() && fs::remove_file(entry.path()).is_ok() {
            gone += 1;
        }
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("reclutched-ring-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, index: u64, packets: usize, extra: usize) {
        let mut bytes = vec![index as u8; packets * 188];
        bytes.extend(vec![0xEE; extra]);
        fs::write(dir.join(segment_name(index)), bytes).unwrap();
    }

    #[test]
    fn pieces_are_listed_in_order_and_pruned() {
        let dir = temp("list");
        for i in [3, 1, 2, 10] {
            write(&dir, i, 2, 0);
        }
        fs::write(dir.join("notes.txt"), b"x").unwrap();
        let indexes: Vec<u64> = list(&dir).iter().map(|(i, _, _)| *i).collect();
        assert_eq!(indexes, [1, 2, 3, 10]);
        assert_eq!(latest(&dir), Some(10));
        prune_before(&dir, 3);
        let indexes: Vec<u64> = list(&dir).iter().map(|(i, _, _)| *i).collect();
        assert_eq!(indexes, [3, 10]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_clip_starts_enough_pieces_back() {
        assert_eq!(clip_start(100, 30), 85);
        assert_eq!(clip_start(100, 31), 84);
        assert_eq!(clip_start(100, 10), 95);
        assert_eq!(clip_start(3, 30), 0);
    }

    #[test]
    fn pruning_keeps_the_window_and_a_running_match() {
        assert_eq!(keep_from(100, None), 100 - KEEP_PIECES);
        // A match that began long ago holds its pieces.
        assert_eq!(keep_from(500, Some(120)), 120);
        // A recent match start does not shorten the window.
        assert_eq!(keep_from(500, Some(499)), 500 - KEEP_PIECES);
        assert_eq!(keep_from(5, None), 0);
    }

    #[test]
    fn joining_cuts_the_growing_piece_to_whole_packets() {
        let dir = temp("join");
        write(&dir, 1, 2, 0);
        write(&dir, 2, 3, 100); // still being written: 100 stray bytes
        write(&dir, 3, 1, 0);
        let out = dir.join("out.ts");
        let bytes = join(&dir, 2, 3, &out).unwrap();
        assert_eq!(bytes, (3 + 1) * 188);
        assert_eq!(fs::metadata(&out).unwrap().len(), bytes);
        assert!(join(&dir, 50, 60, &dir.join("none.ts")).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_clip_picture_is_the_closest_one_at_or_after_its_start() {
        let dir = temp("thumbs");
        for i in [5, 7, 8] {
            fs::write(thumb_path(&dir, i), b"j").unwrap();
        }
        assert_eq!(thumb_for(&dir, 5, 9), Some(thumb_path(&dir, 5)));
        assert_eq!(thumb_for(&dir, 6, 9), Some(thumb_path(&dir, 7)));
        assert_eq!(thumb_for(&dir, 9, 9), None);
        prune_thumbs(&dir, 8);
        assert!(!thumb_path(&dir, 5).exists() && thumb_path(&dir, 8).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn old_files_are_deleted_by_age() {
        let dir = temp("old");
        fs::write(dir.join("a.mp4"), b"x").unwrap();
        // Nothing is older than a day, and 0 means keep everything.
        assert_eq!(delete_older_than(&dir, 1), 0);
        assert_eq!(delete_older_than(&dir, 0), 0);
        assert!(dir.join("a.mp4").exists());
        let _ = fs::remove_dir_all(dir);
    }
}
