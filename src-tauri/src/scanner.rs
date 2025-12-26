use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Error, Debug)]
pub enum ScanError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Path not found: {0}")]
    PathNotFound(String),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    pub path: String,
    pub name: String,
    pub extension: String,
    pub asset_type: AssetType,
    pub size: u64,
    pub metadata: Option<AssetMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetType {
    Texture,
    Model,
    Audio,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetMetadata {
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryNode {
    pub name: String,
    pub path: String,
    pub children: Vec<DirectoryNode>,
    pub file_count: usize,
    pub total_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub root_path: String,
    pub directory_tree: DirectoryNode,
    pub assets: Vec<AssetInfo>,
    pub total_count: usize,
    pub total_size: u64,
    pub type_counts: HashMap<String, usize>,
}

/// Get asset type from file extension
fn get_asset_type(extension: &str) -> AssetType {
    match extension.to_lowercase().as_str() {
        // Textures
        "png" | "jpg" | "jpeg" | "tga" | "psd" | "tiff" | "tif" | "exr" | "hdr" | "webp" | "dds" | "bmp" | "gif" => {
            AssetType::Texture
        }
        // Models
        "fbx" | "obj" | "gltf" | "glb" | "blend" | "dae" | "3ds" | "max" => AssetType::Model,
        // Audio
        "wav" | "mp3" | "ogg" | "flac" | "aiff" | "aac" | "wma" => AssetType::Audio,
        // Other
        _ => AssetType::Other,
    }
}

/// Parse image metadata (dimensions)
fn parse_image_metadata(path: &Path) -> Option<AssetMetadata> {
    match image::open(path) {
        Ok(img) => Some(AssetMetadata {
            width: Some(img.width()),
            height: Some(img.height()),
        }),
        Err(_) => None,
    }
}

/// Build directory tree recursively
fn build_directory_tree(path: &Path, assets: &[AssetInfo]) -> DirectoryNode {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let path_str = path.to_string_lossy().to_string();

    // Get direct children directories
    let mut children: Vec<DirectoryNode> = Vec::new();

    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                // Skip hidden directories
                let dir_name = entry_path.file_name().unwrap_or_default().to_string_lossy();
                if !dir_name.starts_with('.') {
                    children.push(build_directory_tree(&entry_path, assets));
                }
            }
        }
    }

    // Sort children by name
    children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    // Count files and size in this directory (not recursive)
    let (file_count, total_size) = assets
        .iter()
        .filter(|a| {
            Path::new(&a.path)
                .parent()
                .map(|p| p == path)
                .unwrap_or(false)
        })
        .fold((0, 0u64), |(count, size), asset| {
            (count + 1, size + asset.size)
        });

    // Add children counts
    let total_file_count = file_count + children.iter().map(|c| c.file_count).sum::<usize>();
    let total_dir_size = total_size + children.iter().map(|c| c.total_size).sum::<u64>();

    DirectoryNode {
        name,
        path: path_str,
        children,
        file_count: total_file_count,
        total_size: total_dir_size,
    }
}

/// Scan a directory and collect asset information
pub fn scan_directory(path: &str) -> Result<ScanResult, ScanError> {
    let root_path = Path::new(path);

    if !root_path.exists() {
        return Err(ScanError::PathNotFound(path.to_string()));
    }

    if !root_path.is_dir() {
        return Err(ScanError::InvalidPath(format!(
            "{} is not a directory",
            path
        )));
    }

    let mut assets: Vec<AssetInfo> = Vec::new();
    let mut type_counts: HashMap<String, usize> = HashMap::new();

    // Walk the directory tree
    for entry in WalkDir::new(root_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let entry_path = entry.path();

        // Skip directories and hidden files
        if entry_path.is_dir() {
            continue;
        }

        let file_name = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if file_name.starts_with('.') {
            continue;
        }

        // Get file extension
        let extension = entry_path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();

        if extension.is_empty() {
            continue;
        }

        // Get file metadata
        let metadata = entry_path.metadata().ok();
        let size = metadata.map(|m| m.len()).unwrap_or(0);

        // Determine asset type
        let asset_type = get_asset_type(&extension);

        // Parse image metadata for textures (PNG/JPG only for MVP)
        let asset_metadata = match asset_type {
            AssetType::Texture => {
                let ext_lower = extension.to_lowercase();
                if ext_lower == "png" || ext_lower == "jpg" || ext_lower == "jpeg" {
                    parse_image_metadata(entry_path)
                } else {
                    None
                }
            }
            _ => None,
        };

        // Update type counts
        let type_key = match asset_type {
            AssetType::Texture => "texture",
            AssetType::Model => "model",
            AssetType::Audio => "audio",
            AssetType::Other => "other",
        };
        *type_counts.entry(type_key.to_string()).or_insert(0) += 1;

        assets.push(AssetInfo {
            path: entry_path.to_string_lossy().to_string(),
            name: file_name,
            extension,
            asset_type,
            size,
            metadata: asset_metadata,
        });
    }

    // Sort assets by path
    assets.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));

    // Build directory tree
    let directory_tree = build_directory_tree(root_path, &assets);

    let total_count = assets.len();
    let total_size = assets.iter().map(|a| a.size).sum();

    Ok(ScanResult {
        root_path: path.to_string(),
        directory_tree,
        assets,
        total_count,
        total_size,
        type_counts,
    })
}
