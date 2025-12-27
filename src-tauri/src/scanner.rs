use parking_lot::RwLock;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;
use walkdir::WalkDir;

use crate::cache::{get_modified_time, ScanCache};

#[derive(Error, Debug)]
pub enum ScanError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Path not found: {0}")]
    PathNotFound(String),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("Scan cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    pub path: String,
    pub name: String,
    pub extension: String,
    pub asset_type: AssetType,
    pub size: u64,
    pub metadata: Option<AssetMetadata>,
    pub unity_guid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AssetType {
    Texture,
    Model,
    Audio,
    Animation,
    Material,
    Prefab,
    Scene,
    Script,
    Data,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetMetadata {
    // Image metadata
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_alpha: Option<bool>,
    // Model metadata
    pub vertex_count: Option<u32>,
    pub face_count: Option<u32>,
    pub material_count: Option<u32>,
    // Audio metadata
    pub duration_secs: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub bit_depth: Option<u32>,
}

impl Default for AssetMetadata {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            has_alpha: None,
            vertex_count: None,
            face_count: None,
            material_count: None,
            duration_secs: None,
            sample_rate: None,
            channels: None,
            bit_depth: None,
        }
    }
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
    pub project_type: Option<ProjectType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
    Unity,
    Unreal,
    Godot,
    Generic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    pub current: usize,
    pub total: Option<usize>,
    pub current_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanPhase {
    Discovering,
    Parsing,
    Building,
    Completed,
    Cancelled,
}

/// Shared scan state for cancellation
pub struct ScanState {
    pub cancelled: AtomicBool,
    pub current: AtomicUsize,
    pub total: AtomicUsize,
    pub current_file: RwLock<String>,
    pub phase: RwLock<ScanPhase>,
}

impl ScanState {
    pub fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            current: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
            current_file: RwLock::new(String::new()),
            phase: RwLock::new(ScanPhase::Discovering),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn get_progress(&self) -> ScanProgress {
        ScanProgress {
            phase: self.phase.read().clone(),
            current: self.current.load(Ordering::SeqCst),
            total: Some(self.total.load(Ordering::SeqCst)),
            current_file: self.current_file.read().clone(),
        }
    }
}

impl Default for ScanState {
    fn default() -> Self {
        Self::new()
    }
}

/// Get asset type from file extension
fn get_asset_type(extension: &str) -> AssetType {
    match extension.to_lowercase().as_str() {
        // Textures
        "png" | "jpg" | "jpeg" | "tga" | "psd" | "tiff" | "tif" | "exr" | "hdr" | "webp"
        | "dds" | "bmp" | "gif" => AssetType::Texture,
        // Models
        "fbx" | "obj" | "gltf" | "glb" | "blend" | "dae" | "3ds" | "max" => AssetType::Model,
        // Audio
        "wav" | "mp3" | "ogg" | "flac" | "aiff" | "aac" | "wma" => AssetType::Audio,
        // Unity specific
        "prefab" => AssetType::Prefab,
        "unity" => AssetType::Scene,
        "mat" => AssetType::Material,
        "controller" | "anim" => AssetType::Animation,
        "cs" | "js" => AssetType::Script,
        "asset" | "json" | "xml" | "yaml" | "csv" => AssetType::Data,
        // Other
        _ => AssetType::Other,
    }
}

/// Parse image metadata (dimensions, alpha)
fn parse_image_metadata(path: &Path) -> Option<AssetMetadata> {
    match image::open(path) {
        Ok(img) => {
            let has_alpha = match img.color() {
                image::ColorType::Rgba8
                | image::ColorType::Rgba16
                | image::ColorType::Rgba32F
                | image::ColorType::La8
                | image::ColorType::La16 => true,
                _ => false,
            };
            Some(AssetMetadata {
                width: Some(img.width()),
                height: Some(img.height()),
                has_alpha: Some(has_alpha),
                ..Default::default()
            })
        }
        Err(_) => None,
    }
}

/// Parse glTF model metadata
fn parse_gltf_metadata(path: &Path) -> Option<AssetMetadata> {
    match gltf::Gltf::open(path) {
        Ok(gltf) => {
            let mut vertex_count = 0u32;
            let mut face_count = 0u32;

            for mesh in gltf.meshes() {
                for primitive in mesh.primitives() {
                    if let Some(accessor) = primitive.get(&gltf::Semantic::Positions) {
                        vertex_count += accessor.count() as u32;
                    }
                    if let Some(indices) = primitive.indices() {
                        face_count += (indices.count() / 3) as u32;
                    }
                }
            }

            Some(AssetMetadata {
                vertex_count: Some(vertex_count),
                face_count: Some(face_count),
                material_count: Some(gltf.materials().count() as u32),
                ..Default::default()
            })
        }
        Err(_) => None,
    }
}

/// Parse OBJ model metadata
fn parse_obj_metadata(path: &Path) -> Option<AssetMetadata> {
    match tobj::load_obj(path, &tobj::GPU_LOAD_OPTIONS) {
        Ok((models, _materials)) => {
            let mut vertex_count = 0u32;
            let mut face_count = 0u32;

            for model in &models {
                vertex_count += (model.mesh.positions.len() / 3) as u32;
                face_count += (model.mesh.indices.len() / 3) as u32;
            }

            Some(AssetMetadata {
                vertex_count: Some(vertex_count),
                face_count: Some(face_count),
                material_count: Some(models.len() as u32),
                ..Default::default()
            })
        }
        Err(_) => None,
    }
}

/// Parse audio metadata using symphonia
fn parse_audio_metadata(path: &Path) -> Option<AssetMetadata> {
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension() {
        hint.with_extension(ext.to_str().unwrap_or(""));
    }

    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .ok()?;

    let format = probed.format;

    // Get the default track
    let track = format.default_track()?;
    let codec_params = &track.codec_params;

    let sample_rate = codec_params.sample_rate;
    let channels = codec_params.channels.map(|c| c.count() as u32);
    let bit_depth = codec_params.bits_per_sample;

    // Calculate duration
    let duration_secs = if let (Some(n_frames), Some(sample_rate)) =
        (codec_params.n_frames, codec_params.sample_rate)
    {
        Some(n_frames as f64 / sample_rate as f64)
    } else {
        None
    };

    Some(AssetMetadata {
        duration_secs,
        sample_rate,
        channels,
        bit_depth,
        ..Default::default()
    })
}

/// Parse Unity .meta file to get GUID
fn parse_unity_meta(path: &Path) -> Option<String> {
    let meta_path = path.with_extension(format!(
        "{}.meta",
        path.extension().unwrap_or_default().to_str().unwrap_or("")
    ));

    // Try the standard .meta path
    let meta_file_path = if meta_path.exists() {
        meta_path
    } else {
        // Try appending .meta to full path
        let mut p = path.as_os_str().to_owned();
        p.push(".meta");
        let p = Path::new(&p);
        if p.exists() {
            p.to_path_buf()
        } else {
            return None;
        }
    };

    let content = fs::read_to_string(meta_file_path).ok()?;

    // Parse GUID from meta file (simple regex-like approach)
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("guid:") {
            return Some(line[5..].trim().to_string());
        }
    }

    None
}

/// Detect project type based on marker files
fn detect_project_type(root_path: &Path) -> Option<ProjectType> {
    // Unity: Has ProjectSettings folder or Assets folder with .meta files
    if root_path.join("ProjectSettings").is_dir()
        || root_path.join("Assets").is_dir() && root_path.join("Assets").join("Editor.meta").exists()
    {
        return Some(ProjectType::Unity);
    }

    // Unreal: Has .uproject file
    if fs::read_dir(root_path)
        .ok()?
        .filter_map(|e| e.ok())
        .any(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "uproject")
                .unwrap_or(false)
        })
    {
        return Some(ProjectType::Unreal);
    }

    // Godot: Has project.godot file
    if root_path.join("project.godot").exists() {
        return Some(ProjectType::Godot);
    }

    Some(ProjectType::Generic)
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

/// Scan a directory with optional state for progress tracking and cancellation
pub fn scan_directory_with_state(
    path: &str,
    state: Option<Arc<ScanState>>,
) -> Result<ScanResult, ScanError> {
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

    // Detect project type
    let project_type = detect_project_type(root_path);

    // Phase 1: Discover all files
    if let Some(ref s) = state {
        *s.phase.write() = ScanPhase::Discovering;
    }

    let mut file_paths: Vec<walkdir::DirEntry> = Vec::new();

    for entry in WalkDir::new(root_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if let Some(ref s) = state {
            if s.is_cancelled() {
                return Err(ScanError::Cancelled);
            }
        }

        let entry_path = entry.path();

        // Skip directories, hidden files, and .meta files
        if entry_path.is_dir() {
            continue;
        }

        let file_name = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if file_name.starts_with('.') || file_name.ends_with(".meta") {
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

        file_paths.push(entry);
    }

    let total_files = file_paths.len();
    if let Some(ref s) = state {
        s.total.store(total_files, Ordering::SeqCst);
    }

    // Phase 2: Parse all files in parallel
    if let Some(ref s) = state {
        *s.phase.write() = ScanPhase::Parsing;
    }

    // Parse files in parallel using rayon
    let state_clone = state.clone();
    let project_type_clone = project_type.clone();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let assets: Vec<AssetInfo> = file_paths
        .par_iter()
        .filter_map(|entry| {
            // Check for cancellation periodically
            if let Some(ref s) = state_clone {
                if s.is_cancelled() {
                    return None;
                }
            }

            // Update progress counter
            let current = counter_clone.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(ref s) = state_clone {
                s.current.store(current, Ordering::Relaxed);
                // Only update current_file every 100 files to reduce lock contention
                if current % 100 == 0 {
                    *s.current_file.write() = entry.path().to_string_lossy().to_string();
                }
            }

            let entry_path = entry.path();
            let file_name = entry_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let extension = entry_path
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();

            // Get file metadata
            let metadata = entry_path.metadata().ok();
            let size = metadata.map(|m| m.len()).unwrap_or(0);

            // Determine asset type
            let asset_type = get_asset_type(&extension);

            // Parse metadata based on asset type
            let asset_metadata = match asset_type {
                AssetType::Texture => {
                    let ext_lower = extension.to_lowercase();
                    match ext_lower.as_str() {
                        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "tga" => {
                            parse_image_metadata(entry_path)
                        }
                        _ => None,
                    }
                }
                AssetType::Model => {
                    let ext_lower = extension.to_lowercase();
                    match ext_lower.as_str() {
                        "gltf" | "glb" => parse_gltf_metadata(entry_path),
                        "obj" => parse_obj_metadata(entry_path),
                        _ => None,
                    }
                }
                AssetType::Audio => {
                    let ext_lower = extension.to_lowercase();
                    match ext_lower.as_str() {
                        "mp3" | "ogg" | "wav" => parse_audio_metadata(entry_path),
                        _ => None,
                    }
                }
                _ => None,
            };

            // Try to get Unity GUID if it's a Unity project
            let unity_guid = if matches!(project_type_clone, Some(ProjectType::Unity)) {
                parse_unity_meta(entry_path)
            } else {
                None
            };

            Some(AssetInfo {
                path: entry_path.to_string_lossy().to_string(),
                name: file_name,
                extension,
                asset_type,
                size,
                metadata: asset_metadata,
                unity_guid,
            })
        })
        .collect();

    // Check if cancelled during parallel processing
    if let Some(ref s) = state {
        if s.is_cancelled() {
            return Err(ScanError::Cancelled);
        }
    }

    // Calculate type counts from the results
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    for asset in &assets {
        let type_key = match asset.asset_type {
            AssetType::Texture => "texture",
            AssetType::Model => "model",
            AssetType::Audio => "audio",
            AssetType::Animation => "animation",
            AssetType::Material => "material",
            AssetType::Prefab => "prefab",
            AssetType::Scene => "scene",
            AssetType::Script => "script",
            AssetType::Data => "data",
            AssetType::Other => "other",
        };
        *type_counts.entry(type_key.to_string()).or_insert(0) += 1;
    }

    // Convert to mutable for sorting
    let mut assets = assets;

    // Sort assets by path using parallel sort for large collections
    if assets.len() > 1000 {
        assets.par_sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    } else {
        assets.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    }

    // Phase 3: Build directory tree
    if let Some(ref s) = state {
        *s.phase.write() = ScanPhase::Building;
    }

    let directory_tree = build_directory_tree(root_path, &assets);

    let total_count = assets.len();
    let total_size = assets.iter().map(|a| a.size).sum();

    if let Some(ref s) = state {
        *s.phase.write() = ScanPhase::Completed;
    }

    Ok(ScanResult {
        root_path: path.to_string(),
        directory_tree,
        assets,
        total_count,
        total_size,
        type_counts,
        project_type,
    })
}

/// Parse a single asset file and return AssetInfo
pub fn parse_asset_file(
    path: &Path,
    project_type: &Option<ProjectType>,
) -> Option<AssetInfo> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();

    if extension.is_empty() {
        return None;
    }

    // Get file metadata
    let metadata = path.metadata().ok()?;
    let size = metadata.len();

    // Determine asset type
    let asset_type = get_asset_type(&extension);

    // Parse metadata based on asset type
    let asset_metadata = match asset_type {
        AssetType::Texture => {
            let ext_lower = extension.to_lowercase();
            match ext_lower.as_str() {
                "png" | "jpg" | "jpeg" | "bmp" | "gif" | "tga" => parse_image_metadata(path),
                _ => None,
            }
        }
        AssetType::Model => {
            let ext_lower = extension.to_lowercase();
            match ext_lower.as_str() {
                "gltf" | "glb" => parse_gltf_metadata(path),
                "obj" => parse_obj_metadata(path),
                _ => None,
            }
        }
        AssetType::Audio => {
            let ext_lower = extension.to_lowercase();
            match ext_lower.as_str() {
                "mp3" | "ogg" | "wav" => parse_audio_metadata(path),
                _ => None,
            }
        }
        _ => None,
    };

    // Try to get Unity GUID if it's a Unity project
    let unity_guid = if matches!(project_type, Some(ProjectType::Unity)) {
        parse_unity_meta(path)
    } else {
        None
    };

    Some(AssetInfo {
        path: path.to_string_lossy().to_string(),
        name: file_name,
        extension,
        asset_type,
        size,
        metadata: asset_metadata,
        unity_guid,
    })
}

/// Incremental scan - only re-parse changed files
pub fn scan_directory_incremental(
    path: &str,
    state: Option<Arc<ScanState>>,
) -> Result<(ScanResult, IncrementalStats), ScanError> {
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

    // Load existing cache
    let mut cache = ScanCache::load(path).unwrap_or_else(|| ScanCache::new(path));

    // Detect project type
    let project_type = detect_project_type(root_path);

    // Phase 1: Discover all files
    if let Some(ref s) = state {
        *s.phase.write() = ScanPhase::Discovering;
    }

    let mut file_entries: Vec<(walkdir::DirEntry, u64)> = Vec::new();

    for entry in WalkDir::new(root_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if let Some(ref s) = state {
            if s.is_cancelled() {
                return Err(ScanError::Cancelled);
            }
        }

        let entry_path = entry.path();

        if entry_path.is_dir() {
            continue;
        }

        let file_name = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if file_name.starts_with('.') || file_name.ends_with(".meta") {
            continue;
        }

        let extension = entry_path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();

        if extension.is_empty() {
            continue;
        }

        let modified = get_modified_time(entry_path).unwrap_or(0);
        file_entries.push((entry, modified));
    }

    // Collect all current file paths for pruning
    let current_paths: Vec<String> = file_entries
        .iter()
        .map(|(e, _)| e.path().to_string_lossy().to_string())
        .collect();

    // Prune deleted files from cache
    cache.prune(&current_paths);

    // Determine which files need scanning
    let files_to_scan: Vec<&(walkdir::DirEntry, u64)> = file_entries
        .iter()
        .filter(|(entry, modified)| {
            let path_str = entry.path().to_string_lossy().to_string();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            cache.needs_rescan(&path_str, *modified, size)
        })
        .collect();

    let total_files = file_entries.len();
    let files_to_parse = files_to_scan.len();
    let cached_count = total_files - files_to_parse;

    if let Some(ref s) = state {
        s.total.store(files_to_parse, Ordering::SeqCst);
    }

    // Phase 2: Parse only changed files in parallel
    if let Some(ref s) = state {
        *s.phase.write() = ScanPhase::Parsing;
    }

    let state_clone = state.clone();
    let project_type_clone = project_type.clone();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    // Parse files in parallel and collect results
    let parsed_assets: Vec<(AssetInfo, u64)> = files_to_scan
        .par_iter()
        .filter_map(|(entry, modified)| {
            // Check for cancellation periodically
            if let Some(ref s) = state_clone {
                if s.is_cancelled() {
                    return None;
                }
            }

            // Update progress counter
            let current = counter_clone.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(ref s) = state_clone {
                s.current.store(current, Ordering::Relaxed);
                if current % 100 == 0 {
                    *s.current_file.write() = entry.path().to_string_lossy().to_string();
                }
            }

            parse_asset_file(entry.path(), &project_type_clone)
                .map(|asset| (asset, *modified))
        })
        .collect();

    // Check if cancelled during parallel processing
    if let Some(ref s) = state {
        if s.is_cancelled() {
            return Err(ScanError::Cancelled);
        }
    }

    // Update cache with parsed assets
    for (asset, modified) in parsed_assets {
        cache.update_entry(asset, modified);
    }

    // Get all assets from cache
    let mut assets = cache.get_assets();

    // Sort assets by path using parallel sort for large collections
    if assets.len() > 1000 {
        assets.par_sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    } else {
        assets.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    }

    // Calculate type counts
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    for asset in &assets {
        let type_key = match asset.asset_type {
            AssetType::Texture => "texture",
            AssetType::Model => "model",
            AssetType::Audio => "audio",
            AssetType::Animation => "animation",
            AssetType::Material => "material",
            AssetType::Prefab => "prefab",
            AssetType::Scene => "scene",
            AssetType::Script => "script",
            AssetType::Data => "data",
            AssetType::Other => "other",
        };
        *type_counts.entry(type_key.to_string()).or_insert(0) += 1;
    }

    // Phase 3: Build directory tree
    if let Some(ref s) = state {
        *s.phase.write() = ScanPhase::Building;
    }

    let directory_tree = build_directory_tree(root_path, &assets);

    let total_count = assets.len();
    let total_size = assets.iter().map(|a| a.size).sum();

    // Save updated cache
    let _ = cache.save();

    if let Some(ref s) = state {
        *s.phase.write() = ScanPhase::Completed;
    }

    let result = ScanResult {
        root_path: path.to_string(),
        directory_tree,
        assets,
        total_count,
        total_size,
        type_counts,
        project_type,
    };

    let stats = IncrementalStats {
        total_files,
        cached_files: cached_count,
        rescanned_files: files_to_parse,
    };

    Ok((result, stats))
}

/// Statistics about incremental scan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalStats {
    pub total_files: usize,
    pub cached_files: usize,
    pub rescanned_files: usize,
}
