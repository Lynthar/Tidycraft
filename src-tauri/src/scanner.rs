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
    Video,
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
    // Audio / video metadata (duration is shared)
    pub duration_secs: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub bit_depth: Option<u32>,
    // Video-specific
    pub framerate: Option<f32>,
    pub video_codec: Option<String>,
    // Texture color space: "sRGB" | "Linear" | "Unknown". Extracted where
    // the file format exposes it (PNG chunks); absent otherwise.
    pub color_space: Option<String>,
    // Mipmap level count (DDS). 1 = base only, no mipmaps.
    pub mipmap_count: Option<u32>,
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
            framerate: None,
            video_codec: None,
            color_space: None,
            mipmap_count: None,
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

/// Convert a `Path` to a string using forward slashes as the separator.
///
/// All paths we send to the frontend go through this — the frontend expects
/// a single separator so its path filtering, `convertFileSrc`, and all the
/// `lastIndexOf("/")` call sites work the same on Windows as on macOS/Linux.
/// Windows filenames cannot contain `\`, so the replace is lossless there.
pub(crate) fn path_to_string(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s
    }
}

/// Get asset type from file extension
fn get_asset_type(extension: &str) -> AssetType {
    match extension.to_lowercase().as_str() {
        // Textures
        "png" | "jpg" | "jpeg" | "tga" | "psd" | "tiff" | "tif" | "exr" | "hdr" | "webp"
        | "dds" | "bmp" | "gif" | "svg" => AssetType::Texture,
        // Models
        "fbx" | "obj" | "gltf" | "glb" | "blend" | "dae" | "3ds" | "max" => AssetType::Model,
        // Audio
        "wav" | "mp3" | "ogg" | "flac" | "aiff" | "aac" | "wma" => AssetType::Audio,
        // Video
        "mp4" | "mov" | "m4v" | "webm" | "mkv" | "avi" => AssetType::Video,
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

/// Dispatch metadata parsing for a single asset based on its type + extension.
/// Used by both the full scan and the incremental per-file reparse so the set
/// of supported formats lives in one place.
fn parse_metadata_for(path: &Path, extension: &str, asset_type: &AssetType) -> Option<AssetMetadata> {
    let ext = extension.to_lowercase();
    match asset_type {
        AssetType::Texture => match ext.as_str() {
            // PNG gets the color-space chunk scan on top of the image::open pass.
            "png" => {
                let mut m = parse_image_metadata(path)?;
                m.color_space = parse_png_color_space(path);
                Some(m)
            }
            // Other formats the `image` crate fully decodes (enabled via Cargo features).
            "jpg" | "jpeg" | "bmp" | "gif" | "tga"
            | "tif" | "tiff" | "webp" | "hdr" | "exr" => parse_image_metadata(path),
            // DDS has too many compressed sub-formats for `image` to decode
            // reliably; we parse the header ourselves.
            "dds" => parse_dds_metadata(path),
            // SVG is vector XML; we just pull width/height from the root tag.
            "svg" => parse_svg_metadata(path),
            _ => None,
        },
        AssetType::Model => match ext.as_str() {
            "gltf" | "glb" => parse_gltf_metadata(path),
            "obj" => parse_obj_metadata(path),
            "fbx" => parse_fbx_metadata(path),
            _ => None,
        },
        AssetType::Audio => match ext.as_str() {
            "mp3" | "ogg" | "wav" => parse_audio_metadata(path),
            _ => None,
        },
        AssetType::Video => match ext.as_str() {
            "mp4" | "mov" | "m4v" => parse_mp4_metadata(path),
            "webm" | "mkv" => parse_matroska_metadata(path),
            _ => None, // AVI: no pure-Rust parser we ship with yet
        },
        _ => None,
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

/// Extract the value of a quoted XML attribute from a tag body.
/// Handles both single and double quotes. Returns the raw inner text
/// (callers decide what to do with units / whitespace).
fn xml_attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    // Look for `name=` preceded by whitespace or tag start (avoids matching
    // attributes that happen to have `name` as a suffix, e.g. `viewName=`).
    let needle = format!("{}=", name);
    let mut search_from = 0;
    while let Some(rel) = attrs[search_from..].find(&needle) {
        let abs = search_from + rel;
        let before_ok = abs == 0
            || attrs.as_bytes()[abs - 1].is_ascii_whitespace()
            || attrs.as_bytes()[abs - 1] == b'<';
        if !before_ok {
            search_from = abs + needle.len();
            continue;
        }
        let rest = &attrs[abs + needle.len()..];
        let first = rest.chars().next()?;
        if first != '"' && first != '\'' {
            return None;
        }
        let end_rel = rest[1..].find(first)?;
        return Some(&rest[1..1 + end_rel]);
    }
    None
}

/// Parse a numeric SVG length (width / height). Accepts plain numbers and
/// `px`-suffixed values; rejects `%`, `em`, `vw`, etc. so callers fall back
/// to `viewBox` for percentage-sized SVGs.
fn parse_svg_length(raw: &str) -> Option<u32> {
    let trimmed = raw.trim().trim_end_matches("px").trim();
    if trimmed.is_empty() || trimmed.ends_with('%') {
        return None;
    }
    if trimmed
        .chars()
        .any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E')
    {
        return None; // has non-px unit suffix
    }
    let v: f64 = trimmed.parse().ok()?;
    if v.is_finite() && v > 0.0 {
        Some(v.round() as u32)
    } else {
        None
    }
}

/// Parse SVG root tag for width/height. SVG is XML; we don't pull in a full
/// parser — the root element is always near the top of the file and fits
/// in the first few KB.
fn parse_svg_metadata(path: &Path) -> Option<AssetMetadata> {
    use std::io::Read;
    let mut file = File::open(path).ok()?;
    // 16KB covers even heavily-commented SVG headers; root tag is always early.
    let mut buf = Vec::with_capacity(16 * 1024);
    (&mut file).take(16 * 1024).read_to_end(&mut buf).ok()?;
    let content = std::str::from_utf8(&buf).ok()?;

    let svg_start = content.find("<svg").or_else(|| content.find("<SVG"))?;
    let after_tag = &content[svg_start + 4..];
    let tag_end = after_tag.find('>')?;
    let attrs = &after_tag[..tag_end];

    let width = xml_attr(attrs, "width").and_then(parse_svg_length);
    let height = xml_attr(attrs, "height").and_then(parse_svg_length);

    if let (Some(w), Some(h)) = (width, height) {
        return Some(AssetMetadata {
            width: Some(w),
            height: Some(h),
            has_alpha: Some(true),
            ..Default::default()
        });
    }

    // Fallback: viewBox="min-x min-y width height"
    if let Some(vb) = xml_attr(attrs, "viewBox") {
        let nums: Vec<&str> = vb.split_whitespace().collect();
        if nums.len() == 4 {
            let w: f64 = nums[2].parse().ok()?;
            let h: f64 = nums[3].parse().ok()?;
            if w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite() {
                return Some(AssetMetadata {
                    width: Some(w.round() as u32),
                    height: Some(h.round() as u32),
                    has_alpha: Some(true),
                    ..Default::default()
                });
            }
        }
    }

    None
}

/// Parse DDS (DirectDraw Surface) header for width/height/alpha/mipmap count.
///
/// DDS files are very common for game textures (BC1/BC3/BC7 compressed) but
/// the `image` crate's DDS support doesn't cover most of the compressed
/// variants. We only need the header, so a minimal 128-byte reader does the
/// job regardless of the inner format.
///
/// Layout (all little-endian):
///   0..4   : magic "DDS "
///   4..8   : dwSize (must be 124)
///   8..12  : dwFlags
///   12..16 : dwHeight
///   16..20 : dwWidth
///   20..24 : dwPitchOrLinearSize
///   24..28 : dwDepth
///   28..32 : dwMipMapCount
///   ...
///   76..108: DDS_PIXELFORMAT (32-byte struct)
///       80..84: ddspf.dwFlags (DDPF_ALPHAPIXELS = 0x1)
fn parse_dds_metadata(path: &Path) -> Option<AssetMetadata> {
    let mut file = File::open(path).ok()?;
    let mut buf = [0u8; 128];
    std::io::Read::read_exact(&mut file, &mut buf).ok()?;

    if &buf[0..4] != b"DDS " {
        return None;
    }
    let header_size = u32::from_le_bytes(buf[4..8].try_into().ok()?);
    if header_size != 124 {
        return None;
    }

    let height = u32::from_le_bytes(buf[12..16].try_into().ok()?);
    let width = u32::from_le_bytes(buf[16..20].try_into().ok()?);
    let raw_mipmaps = u32::from_le_bytes(buf[28..32].try_into().ok()?);
    let ddspf_flags = u32::from_le_bytes(buf[80..84].try_into().ok()?);
    let has_alpha = (ddspf_flags & 0x1) != 0;

    // DDS header reports 0 when the MIPMAPCOUNT flag is absent; treat that
    // as "no mipmaps generated" (effectively 1 level — the base).
    let mipmap_count = Some(raw_mipmaps.max(1));

    Some(AssetMetadata {
        width: Some(width),
        height: Some(height),
        has_alpha: Some(has_alpha),
        mipmap_count,
        ..Default::default()
    })
}

/// Walk PNG chunks looking for color-space signals. Returns "sRGB" if an
/// explicit sRGB chunk or an iCCP (embedded ICC profile) is present;
/// otherwise None so callers can distinguish "explicitly sRGB" from "no
/// color-profile info at all" — important because the naming rule only
/// fires on *known* sRGB encodings to avoid false positives on old PNGs.
///
/// PNG chunk layout (after 8-byte magic): repeated [4-byte big-endian length]
/// [4-byte type] [length bytes of data] [4-byte CRC]. Chunks before IDAT
/// carry the color metadata we care about.
fn parse_png_color_space(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"\x89PNG\r\n\x1a\n" {
        return None;
    }

    let mut has_srgb = false;
    let mut has_iccp = false;

    // Cap the walk to protect against malformed files.
    for _ in 0..64 {
        let mut head = [0u8; 8];
        if file.read_exact(&mut head).is_err() {
            break;
        }
        let len = u32::from_be_bytes(head[0..4].try_into().ok()?);
        let kind = &head[4..8];

        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        match kind {
            b"sRGB" => has_srgb = true,
            b"iCCP" => has_iccp = true,
            _ => {}
        }

        // Skip chunk data + 4-byte CRC.
        file.seek(SeekFrom::Current(len as i64 + 4)).ok()?;
    }

    if has_srgb || has_iccp {
        Some("sRGB".to_string())
    } else {
        None
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

/// Parse FBX model metadata (vertex/face/material count).
///
/// FBX is Autodesk's proprietary interchange format — both binary (most common
/// today) and ASCII variants exist. `fbxcel-dom` handles both and gives us a
/// typed DOM; we iterate the object list, match on `Geometry::Mesh` + `Material`,
/// and pull raw node children for the cheap counts we need. For polygon count
/// we exploit FBX's convention that `PolygonVertexIndex` uses a negative
/// sentinel (bit-inverted last index) to mark each polygon's end — so the
/// number of negatives equals the face/polygon count regardless of tri/quad/n-gon.
fn parse_fbx_metadata(path: &Path) -> Option<AssetMetadata> {
    use fbxcel_dom::any::AnyDocument;
    use fbxcel_dom::v7400::object::{geometry::TypedGeometryHandle, TypedObjectHandle};
    use std::io::BufReader;

    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let any_doc = AnyDocument::from_seekable_reader(reader).ok()?;

    // AnyDocument is `#[non_exhaustive]` — only V7400 is actually emitted for
    // modern FBX, but we must handle the open enum.
    let doc = match any_doc {
        AnyDocument::V7400(_, doc) => doc,
        _ => return None,
    };

    let mut vertex_count: u64 = 0;
    let mut face_count: u64 = 0;
    let mut material_count: u32 = 0;

    for obj in doc.objects() {
        match obj.get_typed() {
            TypedObjectHandle::Geometry(TypedGeometryHandle::Mesh(mesh)) => {
                // Vertices: flat [x0, y0, z0, x1, y1, z1, ...] f64 array.
                if let Some(verts_node) = mesh.node().children_by_name("Vertices").next() {
                    if let Some(attr) = verts_node.attributes().get(0) {
                        if let Ok(arr) = attr.get_arr_f64_or_type() {
                            vertex_count += (arr.len() / 3) as u64;
                        }
                    }
                }
                // PolygonVertexIndex: flat i32 array; each polygon's last
                // index is XOR'd with -1, so counting negatives = face count.
                if let Some(pvi_node) = mesh.node().children_by_name("PolygonVertexIndex").next() {
                    if let Some(attr) = pvi_node.attributes().get(0) {
                        if let Ok(arr) = attr.get_arr_i32_or_type() {
                            face_count += arr.iter().filter(|&&v| v < 0).count() as u64;
                        }
                    }
                }
            }
            TypedObjectHandle::Material(_) => {
                material_count = material_count.saturating_add(1);
            }
            _ => {}
        }
    }

    if vertex_count == 0 && face_count == 0 && material_count == 0 {
        return None;
    }

    Some(AssetMetadata {
        vertex_count: Some(vertex_count.min(u32::MAX as u64) as u32),
        face_count: Some(face_count.min(u32::MAX as u64) as u32),
        material_count: Some(material_count),
        ..Default::default()
    })
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

/// Parse MP4 / MOV / M4V container metadata: duration, resolution, framerate,
/// and the first video track's codec. Uses the pure-Rust `mp4` crate.
fn parse_mp4_metadata(path: &Path) -> Option<AssetMetadata> {
    use std::io::BufReader;

    let file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let reader = BufReader::new(file);
    let mp4 = mp4::Mp4Reader::read_header(reader, size).ok()?;

    let duration_secs = Some(mp4.duration().as_secs_f64());

    // First video track wins. Audio-only MP4s (rare) yield duration but no
    // resolution / codec / framerate.
    for track in mp4.tracks().values() {
        if track.track_type().ok() == Some(mp4::TrackType::Video) {
            let video_codec = track.media_type().ok().map(|m| m.to_string());
            return Some(AssetMetadata {
                width: Some(track.width() as u32),
                height: Some(track.height() as u32),
                duration_secs,
                framerate: Some(track.frame_rate() as f32),
                video_codec,
                ..Default::default()
            });
        }
    }

    Some(AssetMetadata {
        duration_secs,
        ..Default::default()
    })
}

/// Parse WebM / MKV (Matroska) metadata via `matroska-demuxer`.
fn parse_matroska_metadata(path: &Path) -> Option<AssetMetadata> {
    use matroska_demuxer::{MatroskaFile, TrackType};

    let file = File::open(path).ok()?;
    let mkv = MatroskaFile::open(file).ok()?;

    let info = mkv.info();
    // Matroska stores duration in "timestamp units"; each unit is
    // `timestamp_scale` nanoseconds (default 1e6 = 1ms). Convert to seconds.
    let ts_scale = info.timestamp_scale().get() as f64;
    let duration_secs = info
        .duration()
        .map(|d| d * ts_scale / 1_000_000_000.0);

    for track in mkv.tracks() {
        if track.track_type() == TrackType::Video {
            let codec_id = track.codec_id().to_string();
            if let Some(v) = track.video() {
                return Some(AssetMetadata {
                    width: Some(v.pixel_width().get() as u32),
                    height: Some(v.pixel_height().get() as u32),
                    duration_secs,
                    video_codec: Some(codec_id),
                    ..Default::default()
                });
            }
            // Video track but no Video block (rare/malformed) — still record codec.
            return Some(AssetMetadata {
                duration_secs,
                video_codec: Some(codec_id),
                ..Default::default()
            });
        }
    }

    Some(AssetMetadata {
        duration_secs,
        ..Default::default()
    })
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
pub(crate) fn build_directory_tree(path: &Path, assets: &[AssetInfo]) -> DirectoryNode {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path_to_string(path));

    let path_str = path_to_string(path);

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
                    *s.current_file.write() = path_to_string(entry.path());
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

            let asset_metadata = parse_metadata_for(entry_path, &extension, &asset_type);

            // Try to get Unity GUID if it's a Unity project
            let unity_guid = if matches!(project_type_clone, Some(ProjectType::Unity)) {
                parse_unity_meta(entry_path)
            } else {
                None
            };

            Some(AssetInfo {
                path: path_to_string(entry_path),
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
            AssetType::Video => "video",
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
        root_path: path_to_string(Path::new(path)),
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

    let asset_metadata = parse_metadata_for(path, &extension, &asset_type);

    // Try to get Unity GUID if it's a Unity project
    let unity_guid = if matches!(project_type, Some(ProjectType::Unity)) {
        parse_unity_meta(path)
    } else {
        None
    };

    Some(AssetInfo {
        path: path_to_string(path),
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

    // Collect all current file paths for pruning. Use normalized (forward-slash)
    // paths so they align with what's stored in AssetInfo.path — the cache keys
    // off the exact same string.
    let current_paths: Vec<String> = file_entries
        .iter()
        .map(|(e, _)| path_to_string(e.path()))
        .collect();

    // Prune deleted files from cache
    cache.prune(&current_paths);

    // Determine which files need scanning
    let files_to_scan: Vec<&(walkdir::DirEntry, u64)> = file_entries
        .iter()
        .filter(|(entry, modified)| {
            let path_str = path_to_string(entry.path());
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
                    *s.current_file.write() = path_to_string(entry.path());
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
            AssetType::Video => "video",
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
        root_path: path_to_string(Path::new(path)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_get_asset_type_textures() {
        assert!(matches!(get_asset_type("png"), AssetType::Texture));
        assert!(matches!(get_asset_type("jpg"), AssetType::Texture));
        assert!(matches!(get_asset_type("jpeg"), AssetType::Texture));
        assert!(matches!(get_asset_type("tga"), AssetType::Texture));
        assert!(matches!(get_asset_type("psd"), AssetType::Texture));
        assert!(matches!(get_asset_type("PNG"), AssetType::Texture));
        assert!(matches!(get_asset_type("JPG"), AssetType::Texture));
        assert!(matches!(get_asset_type("svg"), AssetType::Texture));
        assert!(matches!(get_asset_type("dds"), AssetType::Texture));
        assert!(matches!(get_asset_type("webp"), AssetType::Texture));
    }

    #[test]
    fn test_get_asset_type_models() {
        assert!(matches!(get_asset_type("fbx"), AssetType::Model));
        assert!(matches!(get_asset_type("obj"), AssetType::Model));
        assert!(matches!(get_asset_type("gltf"), AssetType::Model));
        assert!(matches!(get_asset_type("glb"), AssetType::Model));
        assert!(matches!(get_asset_type("blend"), AssetType::Model));
        assert!(matches!(get_asset_type("FBX"), AssetType::Model));
    }

    #[test]
    fn test_get_asset_type_audio() {
        assert!(matches!(get_asset_type("wav"), AssetType::Audio));
        assert!(matches!(get_asset_type("mp3"), AssetType::Audio));
        assert!(matches!(get_asset_type("ogg"), AssetType::Audio));
        assert!(matches!(get_asset_type("flac"), AssetType::Audio));
        assert!(matches!(get_asset_type("WAV"), AssetType::Audio));
    }

    #[test]
    fn test_get_asset_type_unity() {
        assert!(matches!(get_asset_type("prefab"), AssetType::Prefab));
        assert!(matches!(get_asset_type("unity"), AssetType::Scene));
        assert!(matches!(get_asset_type("mat"), AssetType::Material));
        assert!(matches!(get_asset_type("controller"), AssetType::Animation));
        assert!(matches!(get_asset_type("anim"), AssetType::Animation));
    }

    #[test]
    fn test_get_asset_type_scripts() {
        assert!(matches!(get_asset_type("cs"), AssetType::Script));
        assert!(matches!(get_asset_type("js"), AssetType::Script));
    }

    #[test]
    fn test_get_asset_type_data() {
        assert!(matches!(get_asset_type("json"), AssetType::Data));
        assert!(matches!(get_asset_type("xml"), AssetType::Data));
        assert!(matches!(get_asset_type("yaml"), AssetType::Data));
        assert!(matches!(get_asset_type("csv"), AssetType::Data));
    }

    #[test]
    fn test_get_asset_type_unknown() {
        assert!(matches!(get_asset_type("xyz"), AssetType::Other));
        assert!(matches!(get_asset_type("unknown"), AssetType::Other));
        assert!(matches!(get_asset_type(""), AssetType::Other));
    }

    fn make_dds_bytes(width: u32, height: u32, alpha: bool) -> Vec<u8> {
        let mut buf = vec![0u8; 128];
        buf[0..4].copy_from_slice(b"DDS ");
        buf[4..8].copy_from_slice(&124u32.to_le_bytes()); // dwSize
        buf[12..16].copy_from_slice(&height.to_le_bytes()); // dwHeight
        buf[16..20].copy_from_slice(&width.to_le_bytes());  // dwWidth
        buf[76..80].copy_from_slice(&32u32.to_le_bytes()); // ddspf.dwSize
        let flags: u32 = if alpha { 0x1 } else { 0 };
        buf[80..84].copy_from_slice(&flags.to_le_bytes());
        buf
    }

    #[test]
    fn test_parse_dds_valid_header() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.dds");
        fs::write(&path, make_dds_bytes(1024, 512, true)).unwrap();

        let meta = parse_dds_metadata(&path).expect("valid DDS should parse");
        assert_eq!(meta.width, Some(1024));
        assert_eq!(meta.height, Some(512));
        assert_eq!(meta.has_alpha, Some(true));
    }

    #[test]
    fn test_parse_dds_no_alpha() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.dds");
        fs::write(&path, make_dds_bytes(256, 256, false)).unwrap();

        let meta = parse_dds_metadata(&path).expect("valid DDS should parse");
        assert_eq!(meta.has_alpha, Some(false));
    }

    #[test]
    fn test_parse_dds_bad_magic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fake.dds");
        let mut buf = make_dds_bytes(64, 64, false);
        buf[0..4].copy_from_slice(b"XXXX");
        fs::write(&path, buf).unwrap();

        assert!(parse_dds_metadata(&path).is_none());
    }

    #[test]
    fn test_parse_dds_truncated() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("short.dds");
        fs::write(&path, b"DDS ").unwrap();

        assert!(parse_dds_metadata(&path).is_none());
    }

    #[test]
    fn test_parse_metadata_dispatch_dds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tex.dds");
        fs::write(&path, make_dds_bytes(128, 64, true)).unwrap();

        let meta = parse_metadata_for(&path, "dds", &AssetType::Texture);
        assert_eq!(meta.and_then(|m| m.width), Some(128));
    }

    #[test]
    fn test_parse_svg_explicit_width_height() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("icon.svg");
        fs::write(
            &path,
            r#"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="48" height="32"><rect/></svg>"#,
        )
        .unwrap();
        let meta = parse_svg_metadata(&path).expect("valid SVG should parse");
        assert_eq!(meta.width, Some(48));
        assert_eq!(meta.height, Some(32));
        assert_eq!(meta.has_alpha, Some(true));
    }

    #[test]
    fn test_parse_svg_px_suffix() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("icon.svg");
        fs::write(
            &path,
            r#"<svg width="100px" height="50px" xmlns="http://www.w3.org/2000/svg"></svg>"#,
        )
        .unwrap();
        let meta = parse_svg_metadata(&path).unwrap();
        assert_eq!(meta.width, Some(100));
        assert_eq!(meta.height, Some(50));
    }

    #[test]
    fn test_parse_svg_viewbox_fallback() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("icon.svg");
        fs::write(
            &path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 16"></svg>"#,
        )
        .unwrap();
        let meta = parse_svg_metadata(&path).unwrap();
        assert_eq!(meta.width, Some(24));
        assert_eq!(meta.height, Some(16));
    }

    #[test]
    fn test_parse_svg_percent_falls_back_to_viewbox() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("icon.svg");
        fs::write(
            &path,
            r#"<svg width="100%" height="100%" viewBox="0 0 200 100"></svg>"#,
        )
        .unwrap();
        let meta = parse_svg_metadata(&path).unwrap();
        assert_eq!(meta.width, Some(200));
        assert_eq!(meta.height, Some(100));
    }

    #[test]
    fn test_parse_svg_missing_all_sizing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("icon.svg");
        fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#).unwrap();
        assert!(parse_svg_metadata(&path).is_none());
    }

    #[test]
    fn test_parse_svg_single_quoted_attrs() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("icon.svg");
        fs::write(&path, r#"<svg width='64' height='64'></svg>"#).unwrap();
        let meta = parse_svg_metadata(&path).unwrap();
        assert_eq!(meta.width, Some(64));
        assert_eq!(meta.height, Some(64));
    }

    #[test]
    fn test_scan_state_cancellation() {
        let state = ScanState::new();

        assert!(!state.is_cancelled());
        state.cancel();
        assert!(state.is_cancelled());
    }

    #[test]
    fn test_scan_state_progress() {
        let state = ScanState::new();

        state.current.store(50, Ordering::SeqCst);
        state.total.store(100, Ordering::SeqCst);
        *state.current_file.write() = "test.png".to_string();
        *state.phase.write() = ScanPhase::Parsing;

        let progress = state.get_progress();

        assert_eq!(progress.current, 50);
        assert_eq!(progress.total, Some(100));
        assert_eq!(progress.current_file, "test.png");
        assert!(matches!(progress.phase, ScanPhase::Parsing));
    }

    #[test]
    fn test_scan_nonexistent_path() {
        let result = scan_directory_with_state("/nonexistent/path/123456", None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ScanError::PathNotFound(_)));
    }

    #[test]
    fn test_scan_empty_directory() {
        let dir = tempdir().unwrap();
        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None);

        assert!(result.is_ok());
        let scan_result = result.unwrap();
        assert_eq!(scan_result.total_count, 0);
        assert_eq!(scan_result.total_size, 0);
    }

    #[test]
    fn test_scan_with_files() {
        let dir = tempdir().unwrap();

        // Create some test files
        fs::write(dir.path().join("test.png"), "fake png data").unwrap();
        fs::write(dir.path().join("test.mp3"), "fake mp3 data").unwrap();
        fs::write(dir.path().join("test.txt"), "some text").unwrap();

        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None);

        assert!(result.is_ok());
        let scan_result = result.unwrap();
        assert_eq!(scan_result.total_count, 3);
        assert!(scan_result.total_size > 0);

        // Check type counts
        assert_eq!(*scan_result.type_counts.get("texture").unwrap_or(&0), 1);
        assert_eq!(*scan_result.type_counts.get("audio").unwrap_or(&0), 1);
        assert_eq!(*scan_result.type_counts.get("other").unwrap_or(&0), 1);
    }

    #[test]
    fn test_scan_skips_hidden_files() {
        let dir = tempdir().unwrap();

        // Create hidden file
        fs::write(dir.path().join(".hidden"), "hidden content").unwrap();
        fs::write(dir.path().join("visible.png"), "visible content").unwrap();

        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None);

        assert!(result.is_ok());
        let scan_result = result.unwrap();
        assert_eq!(scan_result.total_count, 1);
    }

    #[test]
    fn test_scan_skips_meta_files() {
        let dir = tempdir().unwrap();

        // Create Unity-style meta files
        fs::write(dir.path().join("texture.png"), "texture data").unwrap();
        fs::write(dir.path().join("texture.png.meta"), "meta data").unwrap();

        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None);

        assert!(result.is_ok());
        let scan_result = result.unwrap();
        assert_eq!(scan_result.total_count, 1);
    }

    #[test]
    fn test_directory_tree_structure() {
        let dir = tempdir().unwrap();

        // Create nested structure
        fs::create_dir_all(dir.path().join("textures")).unwrap();
        fs::create_dir_all(dir.path().join("models")).unwrap();
        fs::write(dir.path().join("textures/bg.png"), "texture").unwrap();
        fs::write(dir.path().join("models/char.fbx"), "model").unwrap();

        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None);

        assert!(result.is_ok());
        let scan_result = result.unwrap();

        // Check tree has children
        assert_eq!(scan_result.directory_tree.children.len(), 2);
        assert_eq!(scan_result.total_count, 2);
    }

    #[test]
    fn test_asset_metadata() {
        let asset = AssetMetadata::default();

        assert!(asset.width.is_none());
        assert!(asset.height.is_none());
        assert!(asset.has_alpha.is_none());
        assert!(asset.vertex_count.is_none());
        assert!(asset.face_count.is_none());
        assert!(asset.duration_secs.is_none());
    }

    #[test]
    fn test_project_type_detection_unity() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("ProjectSettings")).unwrap();

        let project_type = detect_project_type(dir.path());
        assert!(matches!(project_type, Some(ProjectType::Unity)));
    }

    #[test]
    fn test_project_type_detection_godot() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("project.godot"), "config").unwrap();

        let project_type = detect_project_type(dir.path());
        assert!(matches!(project_type, Some(ProjectType::Godot)));
    }

    #[test]
    fn test_project_type_detection_generic() {
        let dir = tempdir().unwrap();

        let project_type = detect_project_type(dir.path());
        assert!(matches!(project_type, Some(ProjectType::Generic)));
    }
}
