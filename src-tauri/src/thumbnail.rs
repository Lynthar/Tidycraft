use base64::{engine::general_purpose::STANDARD, Engine};
use image::{imageops::FilterType, GenericImageView, ImageFormat};
use std::io::Cursor;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ThumbnailError {
    #[error("Failed to open image: {0}")]
    ImageOpen(String),
    #[error("Failed to encode thumbnail: {0}")]
    Encode(String),
    #[error("Unsupported format")]
    UnsupportedFormat,
}

/// Generate a thumbnail and return as base64 encoded PNG
pub fn get_thumbnail_base64(path: &str, max_size: u32) -> Result<String, ThumbnailError> {
    let path = Path::new(path);

    // Check if file exists and is an image
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Only support common image formats
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tga" => {}
        _ => return Err(ThumbnailError::UnsupportedFormat),
    }

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

    // Encode to PNG
    let mut buffer = Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut buffer, ImageFormat::Png)
        .map_err(|e| ThumbnailError::Encode(e.to_string()))?;

    // Convert to base64
    let base64_str = STANDARD.encode(buffer.into_inner());

    Ok(base64_str)
}
