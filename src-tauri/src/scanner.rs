use ignore::WalkBuilder;
use image::ImageDecoder;
use parking_lot::RwLock;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;

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
    /// File mtime as unix seconds (0 when unreadable). Besides display,
    /// this is the frontend's change signal for mounted components:
    /// CardThumb / AssetPreview key their thumbnail effects on it so an
    /// external edit refreshes a card that never left the viewport.
    pub modified: u64,
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

/// Every field is optional and serializes as ABSENT (not `null`) when unset —
/// the frontend's `metadata.field !== undefined` guards rely on this, and
/// `types/asset.ts` declares the mirror fields as `field?: T`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetMetadata {
    // Image metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_alpha: Option<bool>,
    // Model metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertex_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_count: Option<u32>,
    // Audio / video metadata (duration is shared)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bit_depth: Option<u32>,
    // Video-specific
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framerate: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    // Texture color space: "sRGB" | "Linear" | "Unknown". Extracted where
    // the file format exposes it (PNG chunks); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_space: Option<String>,
    // Mipmap level count (DDS). 1 = base only, no mipmaps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mipmap_count: Option<u32>,
    // DCC tool identifier when the file is an authoring/source format
    // (`.blend` / `.ma` / `.psd` / `.spp` / etc). Values are the stable
    // strings returned by `dcc_source_kind_for` — see that function for
    // the canonical list. None for runtime exports (`.fbx` / `.png` / ...)
    // and non-asset files. Informational only: nothing consumes it today —
    // the dcc_source analyzer matches on file extensions from its own
    // config, NOT on this field (a long-standing comment claiming otherwise
    // was wrong). Kept because it's already on the wire (types/asset.ts
    // mirrors it) and a UI "source app" badge is a plausible consumer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dcc_source_kind: Option<String>,
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
            dcc_source_kind: None,
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
        // Textures + texture-source DCC formats. .psb is Photoshop's
        // big-document variant; .spp is Substance Painter's project
        // file (1→N, paired against generated PBR textures); .sbs is
        // Substance Designer's source graph (typically produces .sbsar
        // or PNG output).
        "png" | "jpg" | "jpeg" | "tga" | "psd" | "psb" | "tiff" | "tif" | "exr" | "hdr" | "webp"
        | "dds" | "bmp" | "gif" | "svg" | "spp" | "sbs" => AssetType::Texture,
        // Models + 3D-source DCC formats. ZBrush (ztl/zpr), Maya
        // (ma/mb), 3ds Max (max), Modo (lxo), Houdini (hip/hipnc/hiplc),
        // Cinema 4D (c4d), Marvelous Designer (zprj — garment, exports
        // to obj/fbx). Blender (blend) was already in this list.
        "fbx" | "obj" | "gltf" | "glb" | "blend" | "dae" | "3ds" | "max" | "vox"
        | "ma" | "mb" | "ztl" | "zpr" | "lxo" | "hip" | "hipnc" | "hiplc" | "c4d"
        | "zprj" => AssetType::Model,
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
        // Godot specific. `.tscn` is a scene (like Unity's `.unity`); `.gd` is
        // a script; `.tres` is a serialized resource (material / curve / …) —
        // there's no dedicated Resource type, so it joins the other serialized
        // formats under Data.
        "tscn" => AssetType::Scene,
        "gd" => AssetType::Script,
        "tres" => AssetType::Data,
        // Other
        _ => AssetType::Other,
    }
}

/// Map an extension to a DCC tool identifier when the file is an
/// authoring/source format. Returns `None` for runtime exports
/// (`.fbx` / `.png` / ...) and non-asset files. The string returned is
/// the wire-format value persisted into `AssetMetadata.dcc_source_kind`,
/// which is informational-only (mirrored to the frontend, read by nothing
/// today — the dcc_source analyzer matches extensions from its own config).
/// Keep values stable anyway: they're serialized into scan caches, so a
/// rename would surface as inconsistent metadata across cached vs fresh
/// entries until the cache version is bumped.
///
/// Why a separate function (not just `get_asset_type` returning a
/// richer enum): asset_type is the user-visible category (Texture /
/// Model / etc.); dcc_source_kind is an orthogonal "source vs
/// runtime" axis. A `.blend` is both a Model AND a Blender source —
/// folding the two into one enum would force `.blend` to pick one
/// and break the AssetList type filter for users who expect to see
/// `.blend` under "Models".
pub fn dcc_source_kind_for(extension: &str) -> Option<&'static str> {
    match extension.to_lowercase().as_str() {
        "blend" => Some("blender"),
        "ma" => Some("maya_ascii"),
        "mb" => Some("maya_binary"),
        "max" => Some("max"),
        "ztl" | "zpr" => Some("zbrush"),
        "spp" => Some("substance_painter"),
        "sbs" => Some("substance_designer"),
        "zprj" => Some("marvelous"),
        "psd" | "psb" => Some("photoshop"),
        "lxo" => Some("modo"),
        "hip" | "hipnc" | "hiplc" => Some("houdini"),
        "c4d" => Some("cinema4d"),
        _ => None,
    }
}

/// Dispatch metadata parsing for a single asset based on its type + extension.
/// Used by both the full scan and the incremental per-file reparse so the set
/// of supported formats lives in one place.
///
/// After per-format parsing, files identified as a DCC source by
/// `dcc_source_kind_for` get their `dcc_source_kind` field tagged —
/// this happens even when format-specific parsing returned None (e.g.
/// `.blend` has no metadata extractor, but we still want the kind
/// label so the dcc_source analyzer can find it). For files that are
/// both DCC sources AND parseable (`.psd` parsed via `image` would
/// be such a case if we enabled the feature), the parsed metadata is
/// preserved and the kind field is overlaid.
fn parse_metadata_for(path: &Path, extension: &str, asset_type: &AssetType) -> Option<AssetMetadata> {
    let ext = extension.to_lowercase();
    let parsed: Option<AssetMetadata> = match asset_type {
        AssetType::Texture => match ext.as_str() {
            // PNG gets the color-space chunk scan on top of the image::open pass.
            "png" => parse_image_metadata(path).map(|mut m| {
                m.color_space = parse_png_color_space(path);
                m
            }),
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
    };

    // Tag DCC source kind. Even when format-specific parsing failed
    // (most authoring formats — .blend, .ma, .psd — have no Rust
    // parser), we still produce a metadata entry carrying the kind
    // so the dcc_source analyzer can reason about source/export pairs.
    if let Some(kind) = dcc_source_kind_for(&ext) {
        let mut m = parsed.unwrap_or_default();
        m.dcc_source_kind = Some(kind.to_string());
        return Some(m);
    }
    parsed
}

/// Parse image metadata (dimensions, alpha).
///
/// Reads only the header (dimensions + color type) via the decoder instead of
/// decoding every pixel — a 4K texture is hundreds of ms to fully decode and we
/// only need w/h/alpha. The incremental scan cache means this runs at most once
/// per file version, but the first scan of a texture-heavy project is far
/// cheaper this way. On any header/format error we return None, exactly as the
/// old full-decode path did on `Err`.
fn parse_image_metadata(path: &Path) -> Option<AssetMetadata> {
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    let has_alpha = decoder.color_type().has_alpha();
    Some(AssetMetadata {
        width: Some(width),
        height: Some(height),
        has_alpha: Some(has_alpha),
        ..Default::default()
    })
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
///       80..84: ddspf.dwFlags (DDPF_ALPHAPIXELS = 0x1, DDPF_FOURCC = 0x4)
///       84..88: ddspf.dwFourCC (compressed format tag, e.g. "DXT5"/"DX10")
///   128..148: DDS_HEADER_DXT10 extension, only when FourCC == "DX10"
///       128..132: dxgiFormat
fn parse_dds_metadata(path: &Path) -> Option<AssetMetadata> {
    const DDPF_ALPHAPIXELS: u32 = 0x1;
    const DDPF_FOURCC: u32 = 0x4;

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

    // For FourCC (compressed) formats, alpha is a property of the block
    // format itself — the ALPHAPIXELS bit only describes uncompressed
    // layouts and is typically 0 on compressed files.
    let has_alpha = if (ddspf_flags & DDPF_FOURCC) != 0 {
        match &buf[84..88] {
            b"DXT2" | b"DXT3" | b"DXT4" | b"DXT5" => Some(true),
            // BC1's optional 1-bit alpha isn't recorded in the header;
            // BC4/BC5 (ATI1/ATI2) are 1–2 channel data formats.
            b"DXT1" | b"ATI1" | b"ATI2" | b"BC4U" | b"BC4S" | b"BC5U" | b"BC5S" => Some(false),
            b"DX10" => {
                // The real format lives in the DXT10 extension header
                // directly after the 128 bytes we already consumed.
                let mut dxgi = [0u8; 4];
                std::io::Read::read_exact(&mut file, &mut dxgi)
                    .ok()
                    .map(|_| {
                        matches!(
                            u32::from_le_bytes(dxgi),
                            // block-compressed with alpha: BC2 / BC3 / BC7
                            73..=78 | 97..=99
                            // common uncompressed alpha layouts:
                            // R32G32B32A32*, R16G16B16A16*, R10G10B10A2*,
                            // R8G8B8A8*, A8_UNORM, B8G8R8A8*
                            | 1..=4 | 9..=14 | 23..=25 | 27..=32 | 65 | 87 | 90 | 91
                        )
                    })
            }
            _ => None, // unrecognized compressed format — don't guess
        }
    } else {
        Some((ddspf_flags & DDPF_ALPHAPIXELS) != 0)
    };

    // DDS header reports 0 when the MIPMAPCOUNT flag is absent; treat that
    // as "no mipmaps generated" (effectively 1 level — the base).
    let mipmap_count = Some(raw_mipmaps.max(1));

    Some(AssetMetadata {
        width: Some(width),
        height: Some(height),
        has_alpha,
        mipmap_count,
        ..Default::default()
    })
}

/// Walk PNG chunks looking for color-space signals. An explicit `sRGB` chunk
/// wins; an `iCCP` chunk has its embedded ICC profile parsed and classified
/// ("sRGB" for gamma-encoded transfer curves, "Linear" for identity ones —
/// see `classify_icc_profile`); an unreadable profile yields None (unknown)
/// rather than a guess. None also means "no color-profile info at all" — the
/// colorspace rule only fires on *known* encodings to avoid false positives.
///
/// PNG chunk layout (after 8-byte magic): repeated [4-byte big-endian length]
/// [4-byte type] [length bytes of data] [4-byte CRC]. Chunks before IDAT
/// carry the color metadata we care about.
fn parse_png_color_space(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    // Real iCCP payloads are tens of KB; anything past this is corrupt or
    // hostile and just gets skipped (leaving the classification unknown).
    const MAX_ICCP_CHUNK: u32 = 8 << 20;

    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"\x89PNG\r\n\x1a\n" {
        return None;
    }

    let mut has_srgb = false;
    // `Some(...)` once an iCCP chunk was seen: the inner Option is the
    // profile's classification (None = present but unreadable → unknown).
    let mut iccp: Option<Option<String>> = None;

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
            b"iCCP" if iccp.is_none() && len <= MAX_ICCP_CHUNK => {
                // Consume the chunk data (instead of seeking past it) so the
                // profile can be classified, then skip the 4-byte CRC.
                let mut data = vec![0u8; len as usize];
                if file.read_exact(&mut data).is_err() {
                    break;
                }
                file.seek(SeekFrom::Current(4)).ok()?;
                iccp = Some(classify_iccp_chunk(&data));
                continue;
            }
            // Oversized chunk: mark "seen but unreadable" — without clobbering
            // a classification from an earlier (spec says: the only) iCCP.
            b"iCCP" if iccp.is_none() => iccp = Some(None),
            _ => {}
        }

        // Skip chunk data + 4-byte CRC.
        file.seek(SeekFrom::Current(len as i64 + 4)).ok()?;
    }

    if has_srgb {
        return Some("sRGB".to_string());
    }
    iccp.flatten()
}

/// Decode a PNG `iCCP` chunk payload — `[profile name][NUL][compression
/// method byte][zlib stream]` — and classify the embedded ICC profile.
/// `None` = unreadable/unknown, so the colorspace rule stays silent rather
/// than guessing (the old behavior treated ANY profile as sRGB and mis-warned
/// on deliberately linear profiles).
fn classify_iccp_chunk(data: &[u8]) -> Option<String> {
    // Layout: 1-79 byte profile name, NUL, compression method (0 = zlib),
    // compressed profile bytes.
    let nul = data.iter().position(|&b| b == 0)?;
    if *data.get(nul + 1)? != 0 {
        return None; // unknown compression method
    }
    let compressed = data.get(nul + 2..)?;
    // 4 MB decompressed cap: real profiles are ≤ ~1 MB; the limit defuses
    // decompression bombs in hostile files.
    let profile =
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(compressed, 4 << 20).ok()?;
    classify_icc_profile(&profile)
}

/// Classify a raw ICC profile: `Some("Linear")` when its tone-response curves
/// are (approximately) identity, `Some("sRGB")` when they're gamma-encoded —
/// which is the question the texture colorspace rule actually asks: "will the
/// engine de-gamma this data?". Falls back to the profile description text
/// when no TRC is readable; `None` when the profile tells us nothing.
fn classify_icc_profile(profile: &[u8]) -> Option<String> {
    // 128-byte header ('acsp' signature at 36) + u32 tag count at 128.
    if profile.len() < 132 || &profile[36..40] != b"acsp" {
        return None;
    }
    let tag_count = u32::from_be_bytes(profile[128..132].try_into().ok()?) as usize;
    if tag_count > 1024 {
        return None; // implausible table; don't walk garbage
    }

    // true = linear, one entry per readable tone-response curve.
    let mut trc_verdicts: Vec<bool> = Vec::new();
    let mut desc_text: Option<String> = None;

    for i in 0..tag_count {
        let entry_off = 132 + i * 12;
        let Some(entry) = profile.get(entry_off..entry_off + 12) else {
            break;
        };
        let sig = &entry[0..4];
        let tag_off = u32::from_be_bytes(entry[4..8].try_into().ok()?) as usize;
        let tag_size = u32::from_be_bytes(entry[8..12].try_into().ok()?) as usize;
        let Some(end) = tag_off.checked_add(tag_size) else {
            continue;
        };
        let Some(body) = profile.get(tag_off..end) else {
            continue;
        };
        match sig {
            b"rTRC" | b"gTRC" | b"bTRC" | b"kTRC" => {
                if let Some(linear) = trc_is_linear(body) {
                    trc_verdicts.push(linear);
                }
            }
            b"desc" => desc_text = parse_icc_desc_text(body),
            _ => {}
        }
    }

    // Transfer curves are the ground truth: the colorspace rule's question is
    // literally "will the engine de-gamma these pixels?".
    if !trc_verdicts.is_empty() {
        let all_linear = trc_verdicts.iter().all(|&l| l);
        return Some(if all_linear { "Linear" } else { "sRGB" }.to_string());
    }

    // No readable TRC (e.g. LUT-based v4 profiles): fall back to the
    // human-readable description most authoring tools embed.
    let text = desc_text?.to_lowercase();
    if text.contains("linear") {
        return Some("Linear".to_string());
    }
    if text.contains("srgb") {
        return Some("sRGB".to_string());
    }
    None
}

/// Is this TRC tag (`curv` / `para` body) an identity (γ≈1.0) curve?
/// `None` = unrecognized curve type, treated as "no verdict".
fn trc_is_linear(body: &[u8]) -> Option<bool> {
    const GAMMA_TOLERANCE: f32 = 0.05;
    match body.get(0..4)? {
        b"curv" => {
            let n = u32::from_be_bytes(body.get(8..12)?.try_into().ok()?) as usize;
            match n {
                // Zero entries = identity curve by definition (ICC spec).
                0 => Some(true),
                // One entry = u8Fixed8 gamma value.
                1 => {
                    let g = u16::from_be_bytes(body.get(12..14)?.try_into().ok()?) as f32 / 256.0;
                    Some((g - 1.0).abs() <= GAMMA_TOLERANCE)
                }
                // Sampled LUT: identity has value[mid] ≈ mid/(n-1)·65535
                // (≈32768); an sRGB/γ2.2 curve sits near ~14000 there, so a
                // ±3000 band separates them with a wide margin.
                _ => {
                    let mid = n / 2;
                    let at = 12 + mid * 2;
                    let v = u16::from_be_bytes(body.get(at..at + 2)?.try_into().ok()?) as f32;
                    let expected = mid as f32 / (n - 1) as f32 * 65535.0;
                    Some((v - expected).abs() <= 3000.0)
                }
            }
        }
        b"para" => {
            // parametricCurveType: u16 function id (0-4), 2 reserved bytes,
            // then s15Fixed16 params — the first is always the exponent g
            // (function 3, the sRGB shape, has g = 2.4).
            let function = u16::from_be_bytes(body.get(8..10)?.try_into().ok()?);
            if function > 4 {
                return None;
            }
            let g_raw = i32::from_be_bytes(body.get(12..16)?.try_into().ok()?);
            let g = g_raw as f32 / 65536.0;
            Some((g - 1.0).abs() <= GAMMA_TOLERANCE)
        }
        _ => None,
    }
}

/// Extract the profile description string: v2 `desc` (ASCII, NUL-terminated)
/// or v4 `mluc` (first record, UTF-16BE).
fn parse_icc_desc_text(body: &[u8]) -> Option<String> {
    match body.get(0..4)? {
        b"desc" => {
            let count = u32::from_be_bytes(body.get(8..12)?.try_into().ok()?) as usize;
            let raw = body.get(12..12usize.checked_add(count)?)?;
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            Some(String::from_utf8_lossy(&raw[..end]).into_owned())
        }
        b"mluc" => {
            let len = u32::from_be_bytes(body.get(20..24)?.try_into().ok()?) as usize;
            let off = u32::from_be_bytes(body.get(24..28)?.try_into().ok()?) as usize;
            let raw = body.get(off..off.checked_add(len)?)?;
            let units: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
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
                    let position_count = primitive
                        .get(&gltf::Semantic::Positions)
                        .map(|a| a.count())
                        .unwrap_or(0);
                    vertex_count += position_count as u32;

                    // Non-indexed primitives draw straight from the vertex
                    // stream, so the element count falls back to it. How many
                    // triangles those elements make depends on the topology —
                    // strips/fans share edges, points/lines have no faces.
                    let element_count = primitive
                        .indices()
                        .map(|a| a.count())
                        .unwrap_or(position_count);
                    use gltf::mesh::Mode;
                    face_count += match primitive.mode() {
                        Mode::Triangles => element_count / 3,
                        Mode::TriangleStrip | Mode::TriangleFan => {
                            element_count.saturating_sub(2)
                        }
                        Mode::Points | Mode::Lines | Mode::LineLoop | Mode::LineStrip => 0,
                    } as u32;
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
        Ok((models, materials)) => {
            let mut vertex_count = 0u32;
            let mut face_count = 0u32;

            for model in &models {
                vertex_count += (model.mesh.positions.len() / 3) as u32;
                face_count += (model.mesh.indices.len() / 3) as u32;
            }

            Some(AssetMetadata {
                vertex_count: Some(vertex_count),
                face_count: Some(face_count),
                // `models` are sub-meshes/groups, not materials — a 30-group
                // export with one shared material must report 1, or the
                // max_materials rule fires on every multi-object OBJ. The
                // side-loaded MTL is authoritative; if it can't be read the
                // count is unknown, not zero.
                material_count: materials.ok().map(|m| m.len() as u32),
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

/// Modification time of the Unity sidecar `<file>.meta`, if present.
/// Unity's convention is the full filename plus ".meta" (`foo.png` →
/// `foo.png.meta`). Used by the incremental scan to fold the sidecar
/// into the cache-invalidation key — see [`crate::cache::CacheEntry`].
fn meta_modified_time(path: &Path) -> Option<u64> {
    let mut p = path.as_os_str().to_owned();
    p.push(".meta");
    get_modified_time(Path::new(&p))
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

/// Per-directory direct-file aggregates, keyed by normalized (forward-slash)
/// parent path. Precomputed once in one O(N) pass so the recursive tree
/// build becomes O(D + fs::read_dir) instead of O(D × N).
struct DirStats {
    file_count: usize,
    total_size: u64,
}

fn precompute_dir_stats(assets: &[AssetInfo]) -> HashMap<String, DirStats> {
    // Rough heuristic: ~1 dir per 10 files. Slight overshoot just avoids
    // the first couple of HashMap resizes.
    let mut map: HashMap<String, DirStats> = HashMap::with_capacity(assets.len() / 10 + 16);
    for asset in assets {
        let Some(parent) = Path::new(&asset.path).parent() else {
            continue;
        };
        let key = path_to_string(parent);
        let entry = map
            .entry(key)
            .or_insert(DirStats { file_count: 0, total_size: 0 });
        entry.file_count += 1;
        entry.total_size += asset.size;
    }
    map
}

/// Build the directory tree for the project.
///
/// Historically this was O(D × N) — each of the D directory nodes filtered
/// the full assets slice looking for its direct children. On a 10k-file /
/// 200-dir project that's 2M comparisons per rebuild, and watcher events
/// trigger a full rebuild on every fs-change batch.
///
/// New implementation: one O(N) preprocessing pass groups assets by their
/// parent directory path into a HashMap, then the recursive tree build only
/// does an O(1) hashmap lookup per node for its direct counts.
///
/// `ignore` prunes gitignored directories from the `fs::read_dir` recursion
/// (pass the same matcher the scan/watcher uses; `None` = gitignore off).
/// Without it, a Unity project's `Library/` — 50k+ entries the scan never
/// looks at — got fully re-walked on every scan AND every watcher batch,
/// and showed up in the sidebar tree even though none of its files exist
/// in the scan result.
pub(crate) fn build_directory_tree(
    root: &Path,
    assets: &[AssetInfo],
    ignore: Option<&IgnoreMatcher>,
) -> DirectoryNode {
    let stats = precompute_dir_stats(assets);
    build_dir_node(root, root, &stats, ignore)
}

fn build_dir_node(
    path: &Path,
    root: &Path,
    stats: &HashMap<String, DirStats>,
    ignore: Option<&IgnoreMatcher>,
) -> DirectoryNode {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path_to_string(path));

    let path_str = path_to_string(path);

    // Walk subdirectories via fs::read_dir so empty dirs still appear in the
    // tree (common for fresh project folders the user is organizing).
    let mut children: Vec<DirectoryNode> = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                let dir_name = entry_path.file_name().unwrap_or_default().to_string_lossy();
                if dir_name.starts_with('.') {
                    continue;
                }
                if let (Some(matcher), Ok(rel)) = (ignore, entry_path.strip_prefix(root)) {
                    if matcher.is_ignored(rel, true) {
                        continue;
                    }
                }
                children.push(build_dir_node(&entry_path, root, stats, ignore));
            }
        }
    }
    children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    // O(1) lookup of direct-file counts from the pre-grouped map.
    let direct = stats.get(&path_str);
    let direct_count = direct.map(|s| s.file_count).unwrap_or(0);
    let direct_size = direct.map(|s| s.total_size).unwrap_or(0);

    let total_file_count = direct_count + children.iter().map(|c| c.file_count).sum::<usize>();
    let total_dir_size = direct_size + children.iter().map(|c| c.total_size).sum::<u64>();

    DirectoryNode {
        name,
        path: path_str,
        children,
        file_count: total_file_count,
        total_size: total_dir_size,
    }
}

/// Build the directory walker. When `respect_gitignore` is true the
/// walker honors `.gitignore` (incl. parent dirs and `.git/info/exclude`)
/// and `.ignore` files; `require_git(false)` makes the gitignore rules
/// apply even outside a git repo. Hidden files and directories
/// (`.git/`, `.vscode/`, `.idea/`, etc.) are always skipped — matches
/// the user-visible behavior of the previous walkdir filter (which
/// only checked `starts_with('.')` at the file-name level after
/// recursing wastefully into dot dirs).
fn build_walker(root: &Path, respect_gitignore: bool) -> ignore::Walk {
    let mut builder = WalkBuilder::new(root);
    builder.follow_links(false).hidden(true);
    if respect_gitignore {
        builder
            .git_ignore(true)
            .git_exclude(true)
            .git_global(true)
            .ignore(true)
            .parents(true)
            .require_git(false);
    } else {
        builder
            .git_ignore(false)
            .git_exclude(false)
            .git_global(false)
            .ignore(false)
            .parents(false);
    }
    builder.build()
}

/// A single-path `.gitignore` matcher mirroring `build_walker`'s root-level
/// exclusion sources, for callers that test individual paths instead of
/// walking the tree (the filesystem watcher). Checks both the project-local
/// ignore files and the user's global gitignore.
///
/// NOTE: this loads only the project-root `.gitignore` / `.ignore` /
/// `.git/info/exclude` (+ global) — it does NOT descend into per-directory
/// nested `.gitignore` files the way `WalkBuilder` does. That covers the
/// common case (root-level rules like `Library/`, `Temp/`, `*.tmp`); nested
/// ignore files are a documented gap.
pub struct IgnoreMatcher {
    local: ignore::gitignore::Gitignore,
    global: ignore::gitignore::Gitignore,
}

impl IgnoreMatcher {
    /// True if `rel_path` (relative to the project root) is excluded by either
    /// the local or global ignore rules. `is_dir` lets directory-only patterns
    /// (`Library/`) match correctly, and matching is purely lexical so it
    /// works for paths that no longer exist (deletions).
    pub fn is_ignored(&self, rel_path: &Path, is_dir: bool) -> bool {
        self.local
            .matched_path_or_any_parents(rel_path, is_dir)
            .is_ignore()
            || self
                .global
                .matched_path_or_any_parents(rel_path, is_dir)
                .is_ignore()
    }
}

/// Build an [`IgnoreMatcher`] for `root`, or `None` when `respect_gitignore`
/// is false (the watcher then tracks everything `is_trackable_path` allows).
pub fn build_gitignore_matcher(root: &Path, respect_gitignore: bool) -> Option<IgnoreMatcher> {
    if !respect_gitignore {
        return None;
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    // Mirror build_walker's root-level sources. `add` returns Some(err) for a
    // missing/unreadable file; a missing `.ignore` or git exclude is the common
    // case, so we ignore those and let only genuine pattern errors surface in
    // `build()` below.
    let _ = builder.add(root.join(".gitignore"));
    let _ = builder.add(root.join(".ignore"));
    let _ = builder.add(root.join(".git").join("info").join("exclude"));
    let local = match builder.build() {
        Ok(gi) => gi,
        Err(e) => {
            eprintln!(
                "[scanner] gitignore matcher build failed for {}: {}",
                root.display(),
                e
            );
            return None;
        }
    };
    // `git_global(true)` equivalent — an empty matcher when none is configured.
    let (global, _) = ignore::gitignore::Gitignore::global();
    Some(IgnoreMatcher { local, global })
}

/// Scan a directory with optional state for progress tracking and
/// cancellation. `respect_gitignore=true` honors the user's
/// `.gitignore` / `.ignore` files; `false` re-enables "scan everything".
///
/// The shipped scan path is `scan_directory_incremental`; since the legacy
/// non-incremental commands were removed this full-scan variant survives as
/// the test suite's harness for the discovery/parse/tree pipeline (it skips
/// the disk cache, which tests must not touch).
#[cfg_attr(not(test), allow(dead_code))]
pub fn scan_directory_with_state(
    path: &str,
    state: Option<Arc<ScanState>>,
    respect_gitignore: bool,
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

    let mut file_paths: Vec<PathBuf> = Vec::new();

    for result in build_walker(root_path, respect_gitignore) {
        let entry = match result {
            Ok(e) => e,
            // Walk errors (permission denied on a sibling, transient IO
            // hiccup) shouldn't poison the whole scan — skip and carry on.
            Err(_) => continue,
        };

        if let Some(ref s) = state {
            if s.is_cancelled() {
                *s.phase.write() = ScanPhase::Cancelled;
                return Err(ScanError::Cancelled);
            }
        }

        // Hidden files and dot-directories are filtered upstream by
        // `build_walker(hidden=true)`, so no `starts_with('.')` check
        // is needed here.
        if entry.file_type().map_or(false, |ft| ft.is_dir()) {
            continue;
        }

        let entry_path = entry.path();
        let file_name = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Unity per-asset metadata files — surfaced via the matching
        // asset's `unity_guid`, not as their own asset entries.
        if file_name.ends_with(".meta") {
            continue;
        }

        let extension = entry_path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        if extension.is_empty() {
            continue;
        }

        file_paths.push(entry_path.to_path_buf());
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
        .filter_map(|entry_path| {
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
                    *s.current_file.write() = path_to_string(entry_path);
                }
            }

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
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = metadata
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

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
                modified,
                metadata: asset_metadata,
                unity_guid,
            })
        })
        .collect();

    // Check if cancelled during parallel processing
    if let Some(ref s) = state {
        if s.is_cancelled() {
            *s.phase.write() = ScanPhase::Cancelled;
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

    let tree_ignore = build_gitignore_matcher(root_path, respect_gitignore);
    let directory_tree = build_directory_tree(root_path, &assets, tree_ignore.as_ref());

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
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

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
        modified,
        metadata: asset_metadata,
        unity_guid,
    })
}

/// Incremental scan — only re-parse changed files. Honors the same
/// `respect_gitignore` semantics as `scan_directory_with_state` (they
/// share `build_walker`). Toggling gitignore on after a previous "scan
/// everything" run will cause newly-ignored files to look "deleted"
/// and get pruned from the cache on the next run — desired but worth
/// noting for users who flip the setting.
pub fn scan_directory_incremental(
    path: &str,
    state: Option<Arc<ScanState>>,
    respect_gitignore: bool,
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

    let mut file_entries: Vec<(PathBuf, u64)> = Vec::new();

    for result in build_walker(root_path, respect_gitignore) {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };

        if let Some(ref s) = state {
            if s.is_cancelled() {
                *s.phase.write() = ScanPhase::Cancelled;
                return Err(ScanError::Cancelled);
            }
        }

        if entry.file_type().map_or(false, |ft| ft.is_dir()) {
            continue;
        }

        let entry_path = entry.path();
        let file_name = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Hidden files / dirs filtered upstream by build_walker(hidden=true);
        // .meta is Unity per-asset metadata (surfaced via unity_guid).
        if file_name.ends_with(".meta") {
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
        file_entries.push((entry_path.to_path_buf(), modified));
    }

    // Collect all current file paths for pruning. Use normalized
    // (forward-slash) paths so they align with what's stored in
    // AssetInfo.path — the cache keys off the exact same string.
    let current_paths: Vec<String> = file_entries
        .iter()
        .map(|(p, _)| path_to_string(p))
        .collect();

    // Prune deleted files from cache. Files that just fell out of
    // scope because of a new `.gitignore` rule also count as
    // "deleted" here — see the function's doc comment.
    cache.prune(&current_paths);

    // Determine which files need scanning. Sidecar mtimes only matter for
    // Unity projects (the only place `.meta` is parsed) — everyone else
    // skips the extra stat per file.
    let is_unity = matches!(project_type, Some(ProjectType::Unity));
    let files_to_scan: Vec<&(PathBuf, u64)> = file_entries
        .iter()
        .filter(|(p, modified)| {
            let path_str = path_to_string(p);
            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
            let meta_modified = if is_unity { meta_modified_time(p) } else { None };
            cache.needs_rescan(&path_str, *modified, size, meta_modified)
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
        .filter_map(|(p, modified)| {
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
                    *s.current_file.write() = path_to_string(p);
                }
            }

            parse_asset_file(p, &project_type_clone)
                .map(|asset| (asset, *modified))
        })
        .collect();

    // Check if cancelled during parallel processing
    if let Some(ref s) = state {
        if s.is_cancelled() {
            *s.phase.write() = ScanPhase::Cancelled;
            return Err(ScanError::Cancelled);
        }
    }

    // Update cache with parsed assets. The sidecar mtime is re-stat'ed here
    // rather than carried from the filter pass — if the .meta changed in
    // between, storing the later value just means one more (correct)
    // re-parse next scan.
    for (asset, modified) in parsed_assets {
        let meta_modified = if is_unity {
            meta_modified_time(Path::new(&asset.path))
        } else {
            None
        };
        cache.update_entry(asset, modified, meta_modified);
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

    let tree_ignore = build_gitignore_matcher(root_path, respect_gitignore);
    let directory_tree = build_directory_tree(root_path, &assets, tree_ignore.as_ref());

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
        assert!(matches!(get_asset_type("vox"), AssetType::Model));
        assert!(matches!(get_asset_type("FBX"), AssetType::Model));
    }

    #[test]
    fn test_get_asset_type_dcc_source_models() {
        // Newly-recognized 3D-source DCC formats. They land under
        // AssetType::Model (orthogonal axis from dcc_source_kind).
        for ext in ["ma", "mb", "ztl", "zpr", "lxo", "hip", "hipnc", "hiplc", "c4d", "zprj"] {
            assert!(
                matches!(get_asset_type(ext), AssetType::Model),
                "{ext} should classify as Model"
            );
        }
    }

    #[test]
    fn test_get_asset_type_dcc_source_textures() {
        // Texture-source DCC formats. .psb is Photoshop big-doc;
        // .spp / .sbs are Substance project / graph sources.
        for ext in ["psb", "spp", "sbs"] {
            assert!(
                matches!(get_asset_type(ext), AssetType::Texture),
                "{ext} should classify as Texture"
            );
        }
    }

    #[test]
    fn test_dcc_source_kind_recognizes_authoring_formats() {
        assert_eq!(dcc_source_kind_for("blend"), Some("blender"));
        assert_eq!(dcc_source_kind_for("BLEND"), Some("blender"));
        assert_eq!(dcc_source_kind_for("ma"), Some("maya_ascii"));
        assert_eq!(dcc_source_kind_for("mb"), Some("maya_binary"));
        assert_eq!(dcc_source_kind_for("max"), Some("max"));
        assert_eq!(dcc_source_kind_for("ztl"), Some("zbrush"));
        assert_eq!(dcc_source_kind_for("zpr"), Some("zbrush"));
        assert_eq!(dcc_source_kind_for("spp"), Some("substance_painter"));
        assert_eq!(dcc_source_kind_for("sbs"), Some("substance_designer"));
        assert_eq!(dcc_source_kind_for("zprj"), Some("marvelous"));
        assert_eq!(dcc_source_kind_for("psd"), Some("photoshop"));
        assert_eq!(dcc_source_kind_for("psb"), Some("photoshop"));
        assert_eq!(dcc_source_kind_for("lxo"), Some("modo"));
        assert_eq!(dcc_source_kind_for("hip"), Some("houdini"));
        assert_eq!(dcc_source_kind_for("c4d"), Some("cinema4d"));
    }

    #[test]
    fn test_dcc_source_kind_none_for_runtime_formats() {
        // Runtime exports / regular assets are not labelled — they're
        // identified by being on the "after" side of a source/export pair.
        for ext in ["fbx", "glb", "gltf", "obj", "png", "jpg", "wav", "mp3", "json"] {
            assert_eq!(
                dcc_source_kind_for(ext),
                None,
                "{ext} should not be tagged as a DCC source"
            );
        }
    }

    #[test]
    fn test_parse_metadata_tags_dcc_kind_when_no_parser() {
        // .blend has no Rust metadata parser, but parse_metadata_for
        // should still return Some(metadata) with dcc_source_kind set
        // — the analyzer relies on this.
        let dir = tempdir().unwrap();
        let path = dir.path().join("character.blend");
        fs::write(&path, b"FAKE BLEND HEADER").unwrap();
        let m = parse_metadata_for(&path, "blend", &AssetType::Model).unwrap();
        assert_eq!(m.dcc_source_kind.as_deref(), Some("blender"));
        // Format-specific fields stay None — we have no parser.
        assert!(m.vertex_count.is_none());
    }

    #[test]
    fn test_parse_metadata_no_kind_for_runtime_export() {
        // Sanity check: parsing a runtime format (here, missing
        // file so parser returns None too) doesn't accidentally tag
        // dcc_source_kind.
        let dir = tempdir().unwrap();
        let path = dir.path().join("ghost.fbx");
        // Don't actually write — just confirm a None parse stays None.
        let m = parse_metadata_for(&path, "fbx", &AssetType::Model);
        assert!(m.is_none());
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
    fn test_metadata_none_fields_are_omitted_from_json() {
        let json = serde_json::to_string(&AssetMetadata {
            width: Some(64),
            ..Default::default()
        })
        .unwrap();
        assert!(json.contains("\"width\":64"), "set field must serialize: {json}");
        // None fields must be ABSENT on the wire, not `null` — the frontend's
        // `!== undefined` guards treat null as a real value and render
        // "-bit" / "0.0 kHz" / "null" garbage rows, and types/asset.ts
        // declares these as `field?: T` (absent), not `T | null`.
        assert!(!json.contains("null"), "no field may serialize as null: {json}");
        assert!(!json.contains("sample_rate"), "unset field must be absent: {json}");
    }

    /// Legacy header with a FourCC pixel format (compressed DDS). The
    /// ALPHAPIXELS bit is meaningless for these — alpha is a property of
    /// the block format itself.
    fn make_dds_fourcc_bytes(fourcc: &[u8; 4]) -> Vec<u8> {
        let mut buf = make_dds_bytes(64, 64, false);
        buf[80..84].copy_from_slice(&0x4u32.to_le_bytes()); // DDPF_FOURCC
        buf[84..88].copy_from_slice(fourcc);
        buf
    }

    #[test]
    fn test_parse_dds_dxt5_reports_alpha() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.dds");
        fs::write(&path, make_dds_fourcc_bytes(b"DXT5")).unwrap();

        let meta = parse_dds_metadata(&path).expect("valid DDS should parse");
        // DXT5/BC3 always carries an alpha block; the old ALPHAPIXELS-only
        // check reported every compressed texture as opaque.
        assert_eq!(meta.has_alpha, Some(true));
    }

    #[test]
    fn test_parse_dds_dxt1_reports_opaque() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.dds");
        fs::write(&path, make_dds_fourcc_bytes(b"DXT1")).unwrap();

        let meta = parse_dds_metadata(&path).expect("valid DDS should parse");
        assert_eq!(meta.has_alpha, Some(false));
    }

    #[test]
    fn test_parse_dds_dx10_bc7_reports_alpha() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.dds");
        let mut buf = make_dds_fourcc_bytes(b"DX10");
        buf.extend_from_slice(&98u32.to_le_bytes()); // DXGI_FORMAT_BC7_UNORM
        buf.extend_from_slice(&[0u8; 16]); // rest of the DXT10 header
        fs::write(&path, buf).unwrap();

        let meta = parse_dds_metadata(&path).expect("valid DDS should parse");
        assert_eq!(meta.has_alpha, Some(true));
    }

    #[test]
    fn test_parse_dds_dx10_bc5_reports_no_alpha() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.dds");
        let mut buf = make_dds_fourcc_bytes(b"DX10");
        buf.extend_from_slice(&83u32.to_le_bytes()); // DXGI_FORMAT_BC5_UNORM
        buf.extend_from_slice(&[0u8; 16]);
        fs::write(&path, buf).unwrap();

        let meta = parse_dds_metadata(&path).expect("valid DDS should parse");
        // Two-channel normal-map format — no alpha despite being compressed.
        assert_eq!(meta.has_alpha, Some(false));
    }

    #[test]
    fn test_obj_material_count_counts_materials_not_meshes() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("scene.mtl"), "newmtl shared\nKd 1.0 0.0 0.0\n").unwrap();
        let obj_path = dir.path().join("scene.obj");
        fs::write(
            &obj_path,
            concat!(
                "mtllib scene.mtl\n",
                "o first\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl shared\nf 1 2 3\n",
                "o second\nv 0 0 1\nv 1 0 1\nv 0 1 1\nusemtl shared\nf 4 5 6\n",
                "o third\nv 0 0 2\nv 1 0 2\nv 0 1 2\nusemtl shared\nf 7 8 9\n",
            ),
        )
        .unwrap();

        let meta = parse_obj_metadata(&obj_path).expect("valid OBJ should parse");
        // Three sub-meshes sharing ONE material: the count feeds the
        // max_materials rule, so reporting `models.len()` (3) is a false
        // positive factory on any multi-group export.
        assert_eq!(meta.material_count, Some(1));
        assert_eq!(meta.face_count, Some(3));
    }

    #[test]
    fn test_obj_unloadable_mtl_leaves_material_count_unknown() {
        let dir = tempdir().unwrap();
        let obj_path = dir.path().join("orphan.obj");
        fs::write(
            &obj_path,
            "mtllib missing.mtl\no a\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        )
        .unwrap();

        let meta = parse_obj_metadata(&obj_path).expect("geometry should still parse");
        // The referenced .mtl can't be read — the material count is unknown,
        // not zero and not the mesh count.
        assert_eq!(meta.material_count, None);
        assert_eq!(meta.face_count, Some(1));
    }

    /// Minimal valid glTF JSON: one primitive over `position_count`
    /// positions, optionally indexed (`indices_count`), with the given
    /// topology `mode`. The gltf crate validates accessors, so bufferViews
    /// and POSITION min/max must all be present even though we never read
    /// the (absent) binary payload for counting.
    fn write_gltf(dir: &Path, position_count: u32, indices: Option<(u32, u32)>) -> PathBuf {
        let pos_bytes = position_count * 12;
        let (indices_json, mode_json, idx_view, total) = match indices {
            Some((count, mode)) => (
                format!(r#""indices": 1, "#),
                format!(r#""mode": {}, "#, mode),
                format!(
                    r#", {{"buffer": 0, "byteOffset": {}, "byteLength": {}}}"#,
                    pos_bytes,
                    count * 2
                ),
                pos_bytes + count * 2,
            ),
            None => (String::new(), String::new(), String::new(), pos_bytes),
        };
        let idx_accessor = match indices {
            Some((count, _)) => format!(
                r#", {{"bufferView": 1, "componentType": 5123, "count": {}, "type": "SCALAR"}}"#,
                count
            ),
            None => String::new(),
        };
        let json = format!(
            r#"{{
              "asset": {{"version": "2.0"}},
              "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, {indices_json}{mode_json}"material": 0}}]}}],
              "materials": [{{}}],
              "accessors": [
                {{"bufferView": 0, "componentType": 5126, "count": {position_count}, "type": "VEC3",
                 "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 1.0]}}{idx_accessor}
              ],
              "bufferViews": [{{"buffer": 0, "byteLength": {pos_bytes}}}{idx_view}],
              "buffers": [{{"byteLength": {total}}}]
            }}"#
        );
        let path = dir.join("m.gltf");
        fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn test_gltf_non_indexed_primitive_counts_faces() {
        let dir = tempdir().unwrap();
        let path = write_gltf(dir.path(), 6, None); // TRIANGLES is the default mode

        let meta = parse_gltf_metadata(&path).expect("valid glTF should parse");
        assert_eq!(meta.vertex_count, Some(6));
        // Non-indexed TRIANGLES draws straight from the position stream:
        // 6 positions = 2 triangles (used to be reported as 0).
        assert_eq!(meta.face_count, Some(2));
        assert_eq!(meta.material_count, Some(1));
    }

    #[test]
    fn test_gltf_triangle_strip_counts_shared_edges() {
        let dir = tempdir().unwrap();
        let path = write_gltf(dir.path(), 4, Some((5, 5))); // 5 indices, mode 5 = TRIANGLE_STRIP

        let meta = parse_gltf_metadata(&path).expect("valid glTF should parse");
        // A 5-index strip is 3 triangles, not 5/3 = 1.
        assert_eq!(meta.face_count, Some(3));
    }

    #[test]
    fn test_gltf_line_primitives_contribute_no_faces() {
        let dir = tempdir().unwrap();
        let path = write_gltf(dir.path(), 4, Some((6, 1))); // 6 indices, mode 1 = LINES

        let meta = parse_gltf_metadata(&path).expect("valid glTF should parse");
        // A debug-wireframe / LINES primitive has no faces at all.
        assert_eq!(meta.face_count, Some(0));
    }

    #[test]
    fn test_cancelled_scan_marks_terminal_phase() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.png"), b"x").unwrap();

        let state = Arc::new(ScanState::new());
        state.cancel();
        let err = scan_directory_with_state(dir.path().to_str().unwrap(), Some(state.clone()), true)
            .expect_err("pre-cancelled scan must not complete");
        assert!(matches!(err, ScanError::Cancelled));
        // The progress reporter treats Cancelled as terminal and stops
        // emitting; the scan must actually record it instead of bailing with
        // the phase stuck at Discovering/Parsing.
        assert!(matches!(state.get_progress().phase, ScanPhase::Cancelled));
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
        let result = scan_directory_with_state("/nonexistent/path/123456", None, false);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ScanError::PathNotFound(_)));
    }

    #[test]
    fn test_scan_empty_directory() {
        let dir = tempdir().unwrap();
        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None, false);

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

        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None, false);

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

        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None, false);

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

        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None, false);

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

        let result = scan_directory_with_state(dir.path().to_str().unwrap(), None, false);

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

    // ---- ICC profile classification (PNG iCCP chunk) ----

    /// Minimal valid ICC container: 128-byte header (`acsp` signature) +
    /// tag table + tag bodies appended in order.
    fn build_icc(tags: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let table_end = 132 + tags.len() * 12;
        let mut profile = vec![0u8; 132];
        profile[36..40].copy_from_slice(b"acsp");
        profile[128..132].copy_from_slice(&(tags.len() as u32).to_be_bytes());
        let mut offset = table_end;
        let mut bodies: Vec<u8> = Vec::new();
        for (sig, body) in tags {
            profile.extend_from_slice(*sig);
            profile.extend_from_slice(&(offset as u32).to_be_bytes());
            profile.extend_from_slice(&(body.len() as u32).to_be_bytes());
            offset += body.len();
            bodies.extend_from_slice(body);
        }
        profile.extend_from_slice(&bodies);
        let total = profile.len() as u32;
        profile[0..4].copy_from_slice(&total.to_be_bytes());
        profile
    }

    /// `curv` TRC body with a single u8Fixed8 gamma entry.
    fn curv_gamma(gamma: f32) -> Vec<u8> {
        let mut body = b"curv".to_vec();
        body.extend_from_slice(&[0u8; 4]);
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&((gamma * 256.0) as u16).to_be_bytes());
        body
    }

    /// v2 `desc` (textDescriptionType) body carrying an ASCII name.
    fn desc_body(text: &str) -> Vec<u8> {
        let mut body = b"desc".to_vec();
        body.extend_from_slice(&[0u8; 4]);
        body.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
        body.extend_from_slice(text.as_bytes());
        body.push(0);
        body
    }

    #[test]
    fn icc_gamma_one_trc_classifies_linear() {
        let profile = build_icc(&[(b"rTRC", curv_gamma(1.0))]);
        assert_eq!(classify_icc_profile(&profile).as_deref(), Some("Linear"));
    }

    #[test]
    fn icc_gamma_curve_trc_classifies_srgb() {
        let profile = build_icc(&[(b"rTRC", curv_gamma(2.2))]);
        assert_eq!(classify_icc_profile(&profile).as_deref(), Some("sRGB"));
    }

    #[test]
    fn icc_zero_point_curv_is_identity_linear() {
        // A `curv` with 0 entries is the identity curve per the ICC spec.
        let mut body = b"curv".to_vec();
        body.extend_from_slice(&[0u8; 4]);
        body.extend_from_slice(&0u32.to_be_bytes());
        let profile = build_icc(&[(b"gTRC", body)]);
        assert_eq!(classify_icc_profile(&profile).as_deref(), Some("Linear"));
    }

    #[test]
    fn icc_parametric_srgb_curve_classifies_srgb() {
        // parametricCurveType, function 3 (the sRGB curve), g = 2.4 as
        // s15Fixed16 — the shape a v4 sRGB profile actually uses.
        let mut body = b"para".to_vec();
        body.extend_from_slice(&[0u8; 4]);
        body.extend_from_slice(&3u16.to_be_bytes());
        body.extend_from_slice(&[0u8; 2]);
        body.extend_from_slice(&((2.4f32 * 65536.0) as i32).to_be_bytes());
        let profile = build_icc(&[(b"rTRC", body)]);
        assert_eq!(classify_icc_profile(&profile).as_deref(), Some("sRGB"));
    }

    #[test]
    fn icc_desc_text_breaks_trc_less_ties() {
        // No readable TRC → fall back to the profile description string.
        let linear = build_icc(&[(b"desc", desc_body("Linear Rec.709"))]);
        assert_eq!(classify_icc_profile(&linear).as_deref(), Some("Linear"));
        let srgb = build_icc(&[(b"desc", desc_body("sRGB IEC61966-2.1"))]);
        assert_eq!(classify_icc_profile(&srgb).as_deref(), Some("sRGB"));
    }

    #[test]
    fn icc_garbage_classifies_unknown() {
        assert_eq!(classify_icc_profile(b"not a profile"), None);
        assert_eq!(classify_icc_profile(&[]), None);
    }

    // ---- PNG chunk walk integration ----

    /// PNG magic + raw chunks. CRCs are zeroed — the walker skips them
    /// without validating.
    fn png_with_chunks(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in chunks {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(*kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0u8; 4]);
        }
        out
    }

    fn iccp_chunk_payload(profile: &[u8]) -> Vec<u8> {
        let mut data = b"embedded".to_vec();
        data.push(0); // NUL after profile name
        data.push(0); // compression method 0 = zlib
        data.extend_from_slice(&miniz_oxide::deflate::compress_to_vec_zlib(profile, 6));
        data
    }

    #[test]
    fn png_iccp_linear_profile_reports_linear() {
        // The whole point of parsing the profile: a linear-profiled normal
        // map must NOT be reported as sRGB (the old behavior mis-warned it).
        let dir = tempdir().unwrap();
        let profile = build_icc(&[(b"rTRC", curv_gamma(1.0))]);
        let png = png_with_chunks(&[(b"iCCP", iccp_chunk_payload(&profile)), (b"IEND", vec![])]);
        let path = dir.path().join("linear.png");
        fs::write(&path, png).unwrap();
        assert_eq!(parse_png_color_space(&path).as_deref(), Some("Linear"));
    }

    #[test]
    fn png_iccp_unreadable_profile_reports_unknown() {
        // Unparseable profile → unknown (None), not a blind "sRGB" guess —
        // the colorspace rule then stays silent instead of mis-advising.
        let dir = tempdir().unwrap();
        let mut data = b"junk".to_vec();
        data.push(0);
        data.push(0);
        data.extend_from_slice(b"\x01\x02this is not zlib");
        let png = png_with_chunks(&[(b"iCCP", data), (b"IEND", vec![])]);
        let path = dir.path().join("junk.png");
        fs::write(&path, png).unwrap();
        assert_eq!(parse_png_color_space(&path), None);
    }

    #[test]
    fn png_explicit_srgb_chunk_still_reports_srgb() {
        let dir = tempdir().unwrap();
        let png = png_with_chunks(&[(b"sRGB", vec![0]), (b"IEND", vec![])]);
        let path = dir.path().join("srgb.png");
        fs::write(&path, png).unwrap();
        assert_eq!(parse_png_color_space(&path).as_deref(), Some("sRGB"));
    }

    /// Set a file's mtime a fixed number of seconds into the future so a
    /// rewrite within the same wall-clock second still registers as a
    /// change (cache mtimes have whole-second granularity).
    fn bump_mtime(path: &Path, secs_ahead: u64) {
        let file = fs::File::options().write(true).open(path).unwrap();
        let t = std::time::SystemTime::now() + std::time::Duration::from_secs(secs_ahead);
        file.set_times(fs::FileTimes::new().set_modified(t)).unwrap();
    }

    #[test]
    fn incremental_rescan_picks_up_meta_only_changes() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        // Unity project marker so unity_guid parsing kicks in.
        fs::create_dir_all(dir.path().join("ProjectSettings")).unwrap();
        fs::write(dir.path().join("tex.png"), "png data").unwrap();
        fs::write(
            dir.path().join("tex.png.meta"),
            "fileFormatVersion: 2\nguid: aaaa1111aaaa1111aaaa1111aaaa1111\n",
        )
        .unwrap();

        let (r1, _) = scan_directory_incremental(root, None, false).unwrap();
        assert_eq!(
            r1.assets[0].unity_guid.as_deref(),
            Some("aaaa1111aaaa1111aaaa1111aaaa1111")
        );

        // Rewrite ONLY the sidecar (the asset itself is untouched) — the
        // second scan must re-parse the asset and surface the new GUID.
        fs::write(
            dir.path().join("tex.png.meta"),
            "fileFormatVersion: 2\nguid: bbbb2222bbbb2222bbbb2222bbbb2222\n",
        )
        .unwrap();
        bump_mtime(&dir.path().join("tex.png.meta"), 5);

        let (r2, _) = scan_directory_incremental(root, None, false).unwrap();
        // Clean up the on-disk cache this test created in the user cache dir.
        let _ = crate::cache::ScanCache::clear(root);
        assert_eq!(
            r2.assets[0].unity_guid.as_deref(),
            Some("bbbb2222bbbb2222bbbb2222bbbb2222")
        );
    }

    #[test]
    fn incremental_rescan_notices_meta_created_and_deleted() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        fs::create_dir_all(dir.path().join("ProjectSettings")).unwrap();
        fs::write(dir.path().join("tex.png"), "png data").unwrap();

        // First scan: no sidecar yet.
        let (r1, _) = scan_directory_incremental(root, None, false).unwrap();
        assert_eq!(r1.assets[0].unity_guid, None);

        // Unity generates the sidecar afterwards ("copy asset in, let the
        // editor produce the .meta") — the next scan must pick it up.
        fs::write(
            dir.path().join("tex.png.meta"),
            "fileFormatVersion: 2\nguid: cccc3333cccc3333cccc3333cccc3333\n",
        )
        .unwrap();
        let (r2, _) = scan_directory_incremental(root, None, false).unwrap();
        assert_eq!(
            r2.assets[0].unity_guid.as_deref(),
            Some("cccc3333cccc3333cccc3333cccc3333")
        );

        // Sidecar removed again → guid must clear.
        fs::remove_file(dir.path().join("tex.png.meta")).unwrap();
        let (r3, _) = scan_directory_incremental(root, None, false).unwrap();
        let _ = crate::cache::ScanCache::clear(root);
        assert_eq!(r3.assets[0].unity_guid, None);
    }

    #[test]
    fn directory_tree_prunes_gitignored_dirs() {
        let dir = tempdir().unwrap();

        fs::create_dir_all(dir.path().join("Library").join("Artifacts")).unwrap();
        fs::create_dir_all(dir.path().join("Assets")).unwrap();
        fs::write(dir.path().join(".gitignore"), "Library/\n").unwrap();
        fs::write(dir.path().join("Assets").join("a.png"), "x").unwrap();

        // gitignore respected → Library/ neither walked nor shown.
        let result =
            scan_directory_with_state(dir.path().to_str().unwrap(), None, true).unwrap();
        let names: Vec<&str> = result
            .directory_tree
            .children
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(names.contains(&"Assets"), "tree children: {:?}", names);
        assert!(
            !names.contains(&"Library"),
            "gitignored dir leaked into the tree: {:?}",
            names
        );

        // gitignore off → the dir still appears (scan-everything mode).
        let result_all =
            scan_directory_with_state(dir.path().to_str().unwrap(), None, false).unwrap();
        assert!(result_all
            .directory_tree
            .children
            .iter()
            .any(|c| c.name == "Library"));
    }
}
