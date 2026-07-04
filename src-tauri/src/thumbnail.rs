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

/// Generate a cache key from file path and modification time
fn get_cache_key(path: &Path, max_size: u32) -> Option<String> {
    let metadata = path.metadata().ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;

    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(duration.as_secs().to_le_bytes());
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
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tga"
        | "tiff" | "tif" | "webp" | "hdr" | "exr" => {}
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
#[allow(dead_code)]
pub fn clear_cache() -> Result<(), ThumbnailError> {
    if let Some(cache_dir) = get_cache_dir() {
        if cache_dir.exists() {
            fs::remove_dir_all(&cache_dir)?;
        }
    }
    Ok(())
}

/// Get cache size in bytes
#[allow(dead_code)]
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
