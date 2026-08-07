use base64::{engine::general_purpose::STANDARD, Engine};
use image::{imageops::FilterType, GenericImageView, ImageFormat};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ThumbnailError {
    #[error("Failed to open image: {0}")]
    ImageOpen(String),
    #[error("Failed to encode thumbnail: {0}")]
    Encode(String),
    #[error("Unsupported format")]
    UnsupportedFormat,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Get the cache directory for thumbnails
fn get_cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|p| p.join("tidycraft").join("thumbnails"))
}

/// Generate a cache key from a file's path, mtime and size.
///
/// The mtime goes in at nanosecond resolution: whole seconds cannot separate
/// a source image from the version that replaced it later in the same second,
/// and the stale thumbnail then survives until the next edit. Size is in the
/// key for the filesystems that only store whole seconds anyway (HFS+, some
/// network mounts), where it is the only part of the key such a rewrite can
/// still move.
fn get_cache_key(path: &Path, max_size: u32) -> Option<String> {
    let metadata = path.metadata().ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;

    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update((duration.as_nanos() as u64).to_le_bytes());
    hasher.update(metadata.len().to_le_bytes());
    hasher.update(max_size.to_le_bytes());

    let hash = hasher.finalize();
    Some(format!("{:x}", hash))
}

/// Try to get thumbnail from cache
fn get_from_cache(cache_key: &str) -> Option<String> {
    let cache_dir = get_cache_dir()?;
    let cache_path = cache_dir.join(format!("{}.png", cache_key));

    if cache_path.exists() {
        let mut file = File::open(&cache_path).ok()?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).ok()?;
        Some(STANDARD.encode(&buffer))
    } else {
        None
    }
}

/// Save thumbnail to cache
fn save_to_cache(cache_key: &str, data: &[u8]) -> Result<(), ThumbnailError> {
    if let Some(cache_dir) = get_cache_dir() {
        // Create cache directory if it doesn't exist
        fs::create_dir_all(&cache_dir)?;

        let cache_path = cache_dir.join(format!("{}.png", cache_key));
        // Atomic (unique temp + rename): two concurrent requests for the
        // same key (e.g. gallery + preview racing on one asset) used to
        // interleave inside one `File::create`, and the torn PNG then
        // stayed cached until the source file's mtime changed.
        crate::fs_atomic::write_atomic(&cache_path, data)?;
    }
    Ok(())
}

/// Generate a thumbnail and return as base64 encoded PNG
/// Uses disk cache to avoid regenerating thumbnails
pub fn get_thumbnail_base64(path: &str, max_size: u32) -> Result<String, ThumbnailError> {
    let path = Path::new(path);

    // Check if file exists and is an image
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Formats the `image` crate can decode with the features enabled in
    // Cargo.toml. PSD/DDS/SVG are intentionally excluded: PSD/SVG aren't
    // supported by `image` at all, and DDS uses our own header-only
    // parser elsewhere (no full decode path). HDR/EXR will lose dynamic
    // range when written out as 8-bit PNG, but a slightly compressed
    // preview is more useful than no preview.
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tga" | "tiff" | "tif" | "webp" | "hdr"
        | "exr" => {}
        _ => return Err(ThumbnailError::UnsupportedFormat),
    }

    // Try to get from cache first
    if let Some(cache_key) = get_cache_key(path, max_size) {
        if let Some(cached) = get_from_cache(&cache_key) {
            return Ok(cached);
        }

        // Generate thumbnail
        let thumbnail_data = generate_thumbnail(path, max_size)?;

        // Save to cache (ignore errors)
        let _ = save_to_cache(&cache_key, &thumbnail_data);

        // Return as base64
        Ok(STANDARD.encode(&thumbnail_data))
    } else {
        // No cache key available, just generate
        let thumbnail_data = generate_thumbnail(path, max_size)?;
        Ok(STANDARD.encode(&thumbnail_data))
    }
}

/// Generate thumbnail bytes (PNG format)
fn generate_thumbnail(path: &Path, max_size: u32) -> Result<Vec<u8>, ThumbnailError> {
    // Open and decode image
    let img = image::open(path).map_err(|e| ThumbnailError::ImageOpen(e.to_string()))?;

    // Calculate thumbnail size maintaining aspect ratio
    let (width, height) = img.dimensions();
    let (new_width, new_height) = if width > height {
        let ratio = max_size as f32 / width as f32;
        (max_size, (height as f32 * ratio) as u32)
    } else {
        let ratio = max_size as f32 / height as f32;
        ((width as f32 * ratio) as u32, max_size)
    };

    // Only resize if image is larger than target
    let thumbnail = if width > max_size || height > max_size {
        img.resize(new_width, new_height, FilterType::Triangle)
    } else {
        img
    };

    // Encode to PNG. PNG supports 8- and 16-bit integer channels but NOT
    // 32-bit float, so HDR/EXR images (which `image::open` decodes to Rgb32F /
    // Rgba32F) must be flattened to 8-bit first — otherwise `PngEncoder`
    // returns `Unsupported` and the thumbnail silently fails (the README's
    // claimed HDR/EXR support never actually rendered). `to_rgba8` naively
    // clamps values > 1.0 (blown highlights), which is fine for a small
    // preview; non-float images pass through untouched.
    let thumbnail = match thumbnail.color() {
        image::ColorType::Rgb32F | image::ColorType::Rgba32F => {
            image::DynamicImage::ImageRgba8(thumbnail.to_rgba8())
        }
        _ => thumbnail,
    };

    let mut buffer = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut buffer, ImageFormat::Png)
        .map_err(|e| ThumbnailError::Encode(e.to_string()))?;

    Ok(buffer.into_inner())
}

/// Clear the thumbnail cache
pub fn clear_cache() -> Result<(), ThumbnailError> {
    if let Some(cache_dir) = get_cache_dir() {
        if cache_dir.exists() {
            fs::remove_dir_all(&cache_dir)?;
        }
    }
    Ok(())
}

/// Ceiling for the thumbnail cache, swept once per launch. Nothing ever
/// removed an entry before this: keys carry the source file's mtime, so every
/// edit to an image left its old thumbnail behind for good, and the only way
/// to reclaim any of it was the Clear button in Settings. A 256px preview runs
/// 30–80 KB, so this holds several thousand of them — the working set of a
/// project or two, which is what a cache is for.
const CACHE_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Delete cache files, oldest first, until the directory fits in `max_bytes`.
/// Returns the bytes reclaimed.
///
/// Age is when the thumbnail was *generated*, not when it was last shown: true
/// LRU would mean writing to a file on every cache hit, and paying a write to
/// record a read is a poor trade for a cache whose miss costs one image
/// decode. The practical difference only shows up for a thumbnail generated
/// long ago and viewed constantly since.
fn prune_dir(dir: &Path, max_bytes: u64) -> std::io::Result<u64> {
    let mut files: Vec<(SystemTime, u64, PathBuf)> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                meta.len(),
                e.path(),
            ))
        })
        .collect();

    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= max_bytes {
        return Ok(0);
    }

    files.sort_by_key(|(modified, _, _)| *modified);

    let mut freed = 0;
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        // A file that won't delete (permissions, another process) must not
        // stop the sweep — skip it and keep going, or one stuck entry pins
        // the whole cache above the cap forever.
        if fs::remove_file(&path).is_ok() {
            total -= len;
            freed += len;
        }
    }
    Ok(freed)
}

/// Bring the thumbnail cache back under its ceiling. Called once at startup,
/// off the main thread — the sweep stats every file in the directory.
pub fn prune_cache() {
    let Some(dir) = get_cache_dir() else { return };
    if !dir.exists() {
        return;
    }
    match prune_dir(&dir, CACHE_MAX_BYTES) {
        Ok(0) => {}
        Ok(freed) => eprintln!("Thumbnail cache: reclaimed {} bytes", freed),
        Err(e) => eprintln!("Thumbnail cache: sweep failed: {}", e),
    }
}

/// Get cache size in bytes
pub fn get_cache_size() -> u64 {
    let cache_dir = match get_cache_dir() {
        Some(dir) => dir,
        None => return 0,
    };

    if !cache_dir.exists() {
        return 0;
    }

    fs::read_dir(&cache_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_mtime(path: &Path, secs: u64, nanos: u32) {
        let file = fs::File::options().write(true).open(path).unwrap();
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::new(secs, nanos);
        file.set_times(fs::FileTimes::new().set_modified(t))
            .unwrap();
    }

    /// Write `bytes` worth of file at `name` and pin its mtime, so a test can
    /// state the eviction order rather than hope for it.
    fn cache_file(dir: &Path, name: &str, bytes: usize, mtime_secs: u64) {
        let path = dir.join(name);
        fs::write(&path, vec![0u8; bytes]).unwrap();
        set_mtime(&path, mtime_secs, 0);
    }

    #[test]
    fn prune_dir_drops_the_oldest_until_it_fits() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        cache_file(d, "old.png", 400, 1_000);
        cache_file(d, "middle.png", 400, 2_000);
        cache_file(d, "recent.png", 400, 3_000);

        let freed = prune_dir(d, 900).unwrap();

        assert_eq!(freed, 400, "one file is enough to get under the cap");
        assert!(!d.join("old.png").exists(), "oldest goes first");
        assert!(d.join("middle.png").exists());
        assert!(d.join("recent.png").exists());
    }

    #[test]
    fn prune_dir_leaves_a_cache_that_fits_alone() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        cache_file(d, "a.png", 100, 1_000);
        cache_file(d, "b.png", 100, 2_000);

        assert_eq!(prune_dir(d, 1_000).unwrap(), 0);
        assert!(d.join("a.png").exists());
        assert!(d.join("b.png").exists());
    }

    #[test]
    fn cache_key_separates_rewrites_inside_one_second() {
        // Re-exporting an image over itself is a single-second operation in
        // any art tool. A key that rounds the mtime down to the second cannot
        // tell the new file from the old, so the preview keeps showing the
        // image that is no longer on disk.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tex.png");

        fs::write(&path, "first").unwrap();
        set_mtime(&path, 1_700_000_000, 100_000_000);
        let before = get_cache_key(&path, 256).unwrap();

        fs::write(&path, "later").unwrap(); // same length, same second
        set_mtime(&path, 1_700_000_000, 900_000_000);
        assert_ne!(before, get_cache_key(&path, 256).unwrap());

        // Filesystems that store only whole seconds leave the mtime halves of
        // the key identical; size is what still moves there.
        fs::write(&path, "longer than before").unwrap();
        set_mtime(&path, 1_700_000_000, 100_000_000);
        assert_ne!(before, get_cache_key(&path, 256).unwrap());
    }

    #[test]
    fn generate_thumbnail_flattens_hdr_float_to_png() {
        // Regression for the HDR/EXR thumbnail bug: `image::open` decodes .hdr
        // to an Rgb32F image, which `PngEncoder` rejects (Unsupported) — so the
        // preview silently failed with an encode error. generate_thumbnail must
        // flatten float pixels to 8-bit before PNG-encoding. A pixel value > 1.0
        // exercises real HDR data (clamped on the way down to 8-bit).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hdr_preview.hdr");
        let img = image::Rgb32FImage::from_pixel(8, 4, image::Rgb([2.5, 0.5, 0.1]));
        image::DynamicImage::ImageRgb32F(img)
            .save_with_format(&path, ImageFormat::Hdr)
            .expect("write test .hdr");

        let bytes =
            generate_thumbnail(&path, 256).expect("HDR thumbnail must encode to PNG, not error");
        // The output is a real PNG (8-byte signature), not an encoder failure.
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }
}
