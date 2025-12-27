//! Unreal Engine project support
//!
//! This module provides parsing for Unreal Engine assets, including:
//! - .uasset file header parsing
//! - .uproject file parsing
//! - Asset metadata extraction

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Unreal Engine package file magic number
const PACKAGE_FILE_MAGIC: u32 = 0x9E2A83C1;

/// Minimum header size for reading basic info
const MIN_HEADER_SIZE: usize = 32;

/// Unreal asset info extracted from .uasset files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnrealAssetInfo {
    /// Package version
    pub package_version: u32,
    /// Licensee version
    pub licensee_version: u32,
    /// Engine version (major.minor.patch)
    pub engine_version: Option<String>,
    /// Asset class name (e.g., "Texture2D", "Blueprint", "StaticMesh")
    pub asset_class: Option<String>,
    /// Total header size
    pub header_size: u32,
    /// Package flags
    pub package_flags: u32,
    /// Is cooked for distribution
    pub is_cooked: bool,
}

/// Unreal project info from .uproject file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnrealProjectInfo {
    /// Project file version
    pub file_version: u32,
    /// Engine version string
    pub engine_association: Option<String>,
    /// Project category
    pub category: Option<String>,
    /// Project description
    pub description: Option<String>,
    /// List of enabled plugins
    pub plugins: Vec<String>,
    /// Target platforms
    pub target_platforms: Vec<String>,
}

/// Parse .uproject file to get project info
pub fn parse_uproject(path: &Path) -> Option<UnrealProjectInfo> {
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let file_version = json.get("FileVersion")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(0);

    let engine_association = json.get("EngineAssociation")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let category = json.get("Category")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let description = json.get("Description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Parse plugins list
    let plugins = json.get("Plugins")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let enabled = p.get("Enabled").and_then(|e| e.as_bool()).unwrap_or(false);
                    if enabled {
                        p.get("Name").and_then(|n| n.as_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Parse target platforms
    let target_platforms = json.get("TargetPlatforms")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Some(UnrealProjectInfo {
        file_version,
        engine_association,
        category,
        description,
        plugins,
        target_platforms,
    })
}

/// Parse .uasset file header to extract metadata
///
/// .uasset file structure (simplified):
/// - Magic (4 bytes): 0x9E2A83C1
/// - Legacy Version (4 bytes)
/// - Legacy UE3 Version (4 bytes)
/// - Package Version (4 bytes)
/// - Licensee Version (4 bytes)
/// - Custom Versions (variable)
/// - Total Header Size (4 bytes)
/// - Folder Name (FString)
/// - Package Flags (4 bytes)
/// - Name Count/Offset (4+4 bytes)
/// - Export Count/Offset (4+4 bytes)
/// - Import Count/Offset (4+4 bytes)
pub fn parse_uasset(path: &Path) -> Option<UnrealAssetInfo> {
    let mut file = File::open(path).ok()?;
    let mut buffer = [0u8; 64];

    // Read initial header
    if file.read(&mut buffer).ok()? < MIN_HEADER_SIZE {
        return None;
    }

    // Check magic number
    let magic = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
    if magic != PACKAGE_FILE_MAGIC {
        return None;
    }

    // Parse header fields
    let legacy_version = i32::from_le_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]);
    let _legacy_ue3_version = i32::from_le_bytes([buffer[8], buffer[9], buffer[10], buffer[11]]);
    let package_version = u32::from_le_bytes([buffer[12], buffer[13], buffer[14], buffer[15]]);
    let licensee_version = u32::from_le_bytes([buffer[16], buffer[17], buffer[18], buffer[19]]);

    // Skip custom versions based on legacy version
    let custom_versions_offset = if legacy_version < -7 {
        // Has custom version container
        let custom_versions_count = u32::from_le_bytes([buffer[20], buffer[21], buffer[22], buffer[23]]);
        24 + (custom_versions_count as usize * 20) // GUID (16) + Version (4)
    } else {
        20
    };

    // Read more if needed for header size
    let header_size = if custom_versions_offset + 4 <= 64 {
        u32::from_le_bytes([
            buffer[custom_versions_offset],
            buffer[custom_versions_offset + 1],
            buffer[custom_versions_offset + 2],
            buffer[custom_versions_offset + 3],
        ])
    } else {
        // Need to seek and read
        file.seek(SeekFrom::Start(custom_versions_offset as u64)).ok()?;
        let mut size_buf = [0u8; 4];
        file.read_exact(&mut size_buf).ok()?;
        u32::from_le_bytes(size_buf)
    };

    // Read package flags (after folder name - complex to parse, use fixed offset approximation)
    let package_flags = read_package_flags(&mut file, header_size).unwrap_or(0);

    // Check if cooked
    let is_cooked = (package_flags & 0x00000008) != 0; // PKG_Cooked flag

    // Try to determine engine version from package version
    let engine_version = guess_engine_version(package_version);

    // Try to get asset class from exports
    let asset_class = get_asset_class(&mut file, header_size);

    Some(UnrealAssetInfo {
        package_version,
        licensee_version,
        engine_version,
        asset_class,
        header_size,
        package_flags,
        is_cooked,
    })
}

/// Try to read package flags from the file
fn read_package_flags(file: &mut File, header_size: u32) -> Option<u32> {
    // Package flags are typically after folder name
    // This is a simplified approach - real parsing would need to read FString
    if header_size > 100 {
        // Seek to approximate location
        file.seek(SeekFrom::Start(60)).ok()?;
        let mut buf = [0u8; 4];

        // Read several positions to find package flags
        for _ in 0..10 {
            if file.read_exact(&mut buf).is_ok() {
                let val = u32::from_le_bytes(buf);
                // Package flags typically have specific patterns
                if val > 0 && val < 0xFFFFFFFF && (val & 0xFF000000) == 0 {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Guess engine version from package file version
fn guess_engine_version(package_version: u32) -> Option<String> {
    // Package versions map approximately to engine versions
    // These are rough estimates based on UE4/UE5 versioning
    match package_version {
        // UE5 versions
        1000..=1009 => Some("5.0".to_string()),
        1010..=1019 => Some("5.1".to_string()),
        1020..=1029 => Some("5.2".to_string()),
        1030..=1039 => Some("5.3".to_string()),
        1040..=1049 => Some("5.4".to_string()),
        1050..=1099 => Some("5.5+".to_string()),
        // UE4 versions (approximated)
        500..=599 => Some("4.24-4.27".to_string()),
        400..=499 => Some("4.18-4.23".to_string()),
        300..=399 => Some("4.10-4.17".to_string()),
        _ => None,
    }
}

/// Try to get the primary asset class from exports
fn get_asset_class(file: &mut File, header_size: u32) -> Option<String> {
    // This is a simplified approach - full implementation would parse name table and exports
    // For now, try to infer from file size and common patterns

    // Seek to exports section (approximate)
    let exports_offset = header_size as u64;
    if file.seek(SeekFrom::Start(exports_offset)).is_err() {
        return None;
    }

    // Read some bytes to look for class name patterns
    let mut buffer = [0u8; 256];
    if file.read(&mut buffer).is_err() {
        return None;
    }

    // Look for common class name patterns in the data
    let data = String::from_utf8_lossy(&buffer);

    // Check for common asset classes
    let classes = [
        ("Texture2D", "Texture2D"),
        ("StaticMesh", "StaticMesh"),
        ("SkeletalMesh", "SkeletalMesh"),
        ("Blueprint", "Blueprint"),
        ("Material", "Material"),
        ("MaterialInstance", "MaterialInstance"),
        ("SoundWave", "SoundWave"),
        ("SoundCue", "SoundCue"),
        ("AnimSequence", "AnimSequence"),
        ("AnimMontage", "AnimMontage"),
        ("ParticleSystem", "ParticleSystem"),
        ("NiagaraSystem", "NiagaraSystem"),
        ("DataTable", "DataTable"),
        ("CurveTable", "CurveTable"),
        ("World", "World"),
        ("Level", "Level"),
    ];

    for (pattern, class_name) in classes {
        if data.contains(pattern) {
            return Some(class_name.to_string());
        }
    }

    None
}

/// Get Unreal asset type from file extension
pub fn get_unreal_asset_type(extension: &str) -> UnrealAssetType {
    match extension.to_lowercase().as_str() {
        "uasset" => UnrealAssetType::Asset,
        "umap" => UnrealAssetType::Map,
        "uexp" => UnrealAssetType::BulkData,
        "ubulk" => UnrealAssetType::BulkData,
        "uplugin" => UnrealAssetType::Plugin,
        "upluginmanifest" => UnrealAssetType::PluginManifest,
        "uproject" => UnrealAssetType::Project,
        "ini" => UnrealAssetType::Config,
        "ushaderbytecode" => UnrealAssetType::Shader,
        _ => UnrealAssetType::Other,
    }
}

/// Unreal-specific asset types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UnrealAssetType {
    /// .uasset - compiled asset
    Asset,
    /// .umap - level/map file
    Map,
    /// .uexp/.ubulk - bulk data
    BulkData,
    /// .uplugin - plugin descriptor
    Plugin,
    /// Plugin manifest
    PluginManifest,
    /// .uproject - project file
    Project,
    /// .ini - config file
    Config,
    /// Shader bytecode
    Shader,
    /// Other file type
    Other,
}

/// Detect if a directory is an Unreal Engine project
pub fn detect_unreal_project(path: &Path) -> Option<UnrealProjectInfo> {
    // Look for .uproject file
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.extension().map(|e| e == "uproject").unwrap_or(false) {
                return parse_uproject(&entry_path);
            }
        }
    }

    // Check for typical UE folder structure without .uproject
    let content_dir = path.join("Content");
    let source_dir = path.join("Source");
    let config_dir = path.join("Config");

    if content_dir.is_dir() && (source_dir.is_dir() || config_dir.is_dir()) {
        // Likely an Unreal project or plugin
        return Some(UnrealProjectInfo {
            file_version: 0,
            engine_association: None,
            category: None,
            description: None,
            plugins: vec![],
            target_platforms: vec![],
        });
    }

    None
}

/// Check if a file is an Unreal Engine asset file
pub fn is_unreal_asset(extension: &str) -> bool {
    matches!(
        extension.to_lowercase().as_str(),
        "uasset" | "umap" | "uexp" | "ubulk"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_unreal_asset_type() {
        assert!(matches!(get_unreal_asset_type("uasset"), UnrealAssetType::Asset));
        assert!(matches!(get_unreal_asset_type("UASSET"), UnrealAssetType::Asset));
        assert!(matches!(get_unreal_asset_type("umap"), UnrealAssetType::Map));
        assert!(matches!(get_unreal_asset_type("uexp"), UnrealAssetType::BulkData));
        assert!(matches!(get_unreal_asset_type("ubulk"), UnrealAssetType::BulkData));
        assert!(matches!(get_unreal_asset_type("uproject"), UnrealAssetType::Project));
        assert!(matches!(get_unreal_asset_type("ini"), UnrealAssetType::Config));
        assert!(matches!(get_unreal_asset_type("unknown"), UnrealAssetType::Other));
    }

    #[test]
    fn test_is_unreal_asset() {
        assert!(is_unreal_asset("uasset"));
        assert!(is_unreal_asset("UASSET"));
        assert!(is_unreal_asset("umap"));
        assert!(is_unreal_asset("uexp"));
        assert!(is_unreal_asset("ubulk"));
        assert!(!is_unreal_asset("png"));
        assert!(!is_unreal_asset("fbx"));
    }

    #[test]
    fn test_guess_engine_version() {
        assert_eq!(guess_engine_version(1000), Some("5.0".to_string()));
        assert_eq!(guess_engine_version(1015), Some("5.1".to_string()));
        assert_eq!(guess_engine_version(1025), Some("5.2".to_string()));
        assert_eq!(guess_engine_version(500), Some("4.24-4.27".to_string()));
        assert_eq!(guess_engine_version(100), None);
    }

    #[test]
    fn test_package_file_magic() {
        // Verify magic constant
        assert_eq!(PACKAGE_FILE_MAGIC, 0x9E2A83C1);
    }
}
