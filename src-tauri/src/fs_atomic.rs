//! Crash-safe file writes shared by every module that persists state (tags,
//! undo history, AI rule docs, LLM response cache, thumbnails): write to a
//! unique sibling temp file, then `rename(2)` over the destination.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `path` atomically. The temp file lands in `path`'s
/// parent so the rename never crosses a filesystem boundary.
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
        // Don't leave the temp file behind when the rename fails.
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
        // Whichever writer won, the payload must be complete.
        assert_eq!(got.len(), 64 * 1024);
        assert!(got == payload_a || got == payload_b);
    }

    #[test]
    fn rejects_bare_root() {
        assert!(write_atomic(Path::new("/"), b"x").is_err());
    }
}
