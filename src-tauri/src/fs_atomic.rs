//! Crash-safe file writes shared by every module that persists state
//! (tags, undo history, AI rule docs, LLM response cache, thumbnails).
//!
//! `fs::write` truncates the destination before writing, so a crash (or a
//! concurrent reader) mid-write observes a torn file. Writing to a unique
//! sibling temp file and `rename(2)`-ing over the destination is atomic on
//! the same filesystem — readers see either the old complete file or the
//! new complete file, never a partial one. The temp name embeds a process-
//! wide counter so two threads writing the SAME key (e.g. rayon workers
//! both generating one thumbnail) can't interleave inside one temp file;
//! last rename wins with a complete payload either way.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `path` atomically (unique temp file + rename).
/// The temp file lands in `path`'s parent directory so the final rename
/// never crosses a filesystem boundary.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?
        .to_owned();

    let mut tmp_name = file_name;
    tmp_name.push(format!(
        ".tmp.{}.{}",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp_path = path.with_file_name(tmp_name);

    fs::write(&tmp_path, contents)?;
    fs::rename(&tmp_path, path).inspect_err(|_| {
        // Failed rename (e.g. destination dir vanished): don't leave the
        // temp file behind.
        let _ = fs::remove_file(&tmp_path);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn writes_and_replaces() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("state.json");
        write_atomic(&target, b"one").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"one");
        write_atomic(&target, b"two").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"two");
        // No temp litter left behind.
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1, "leftover files: {entries:?}");
    }

    #[test]
    fn concurrent_same_key_writes_leave_a_complete_file() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("thumb.png");
        let payload_a = vec![b'a'; 64 * 1024];
        let payload_b = vec![b'b'; 64 * 1024];
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| write_atomic(&target, &payload_a).unwrap());
                s.spawn(|| write_atomic(&target, &payload_b).unwrap());
            }
        });
        let got = fs::read(&target).unwrap();
        // Whichever writer won, the payload must be COMPLETE — all-a or
        // all-b, never interleaved or truncated.
        assert_eq!(got.len(), 64 * 1024);
        assert!(got == payload_a || got == payload_b);
    }

    #[test]
    fn rejects_bare_root() {
        assert!(write_atomic(Path::new("/"), b"x").is_err());
    }
}
