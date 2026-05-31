//! Unity `.meta` sidecar carrying for file operations.
//!
//! Unity writes a `<asset>.meta` file next to every asset (and folder) holding
//! that asset's GUID + import settings. References across a project are stored
//! by GUID, so the engine tolerates moves/renames *as long as the .meta travels
//! with its asset*. If the sidecar is left behind, Unity regenerates a fresh
//! GUID for the renamed/moved file and every reference to it breaks.
//!
//! So the app's own rename / move / delete must carry the sidecar. These
//! helpers are best-effort: a missing sidecar (non-Unity project) is a silent
//! no-op, and a carry failure is returned to the caller — which logs it —
//! without rolling back the already-succeeded primary op (rollback can itself
//! fail and leave a more confusing half-state).
//!
//! Copy / duplicate deliberately do NOT carry the sidecar: a duplicated asset
//! must receive a fresh GUID, so copying the .meta (and its GUID) would create
//! a collision. Those paths are left untouched on purpose.

use std::path::{Path, PathBuf};

/// The Unity sidecar path for `asset_path`: the asset path with `.meta`
/// appended (`hero.png` -> `hero.png.meta`, `Models` -> `Models.meta`).
/// Appends to the full name rather than replacing the extension, matching
/// Unity's convention. The returned path may not exist (non-Unity asset).
pub fn sidecar_path(asset_path: &Path) -> PathBuf {
    let mut os = asset_path.as_os_str().to_os_string();
    os.push(".meta");
    PathBuf::from(os)
}

/// Best-effort: when `from` has a `.meta` sidecar, rename it to sit beside
/// `to`. `Ok(())` both when the sidecar was moved and when there was none
/// (non-Unity asset — nothing to carry). `Err` only when a sidecar exists but
/// couldn't be moved; the caller has already renamed the primary file and just
/// logs this. Refuses to clobber an existing sidecar at the destination.
pub fn carry_on_rename(from: &Path, to: &Path) -> Result<(), String> {
    let src = sidecar_path(from);
    if !src.exists() {
        return Ok(());
    }
    let dst = sidecar_path(to);
    if dst.exists() {
        return Err(format!(
            "destination sidecar already exists, not overwriting: {}",
            dst.display()
        ));
    }
    std::fs::rename(&src, &dst)
        .map_err(|e| format!("failed to move sidecar {}: {}", src.display(), e))
}

/// Best-effort: when `path` has a `.meta` sidecar, send it to the OS trash too,
/// so deleting an asset doesn't strand its sidecar. `Ok(())` when trashed or
/// when there's no sidecar; `Err` only when a sidecar exists but couldn't be
/// trashed (caller logs it — the primary file is already gone).
pub fn carry_on_delete(path: &Path) -> Result<(), String> {
    let meta = sidecar_path(path);
    if !meta.exists() {
        return Ok(());
    }
    trash::delete(&meta).map_err(|e| format!("failed to trash sidecar {}: {}", meta.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn sidecar_path_appends_meta_to_full_name() {
        assert_eq!(
            sidecar_path(Path::new("a/hero.png")),
            PathBuf::from("a/hero.png.meta")
        );
        // Folders carry a sidecar too — append, don't touch the "extension".
        assert_eq!(
            sidecar_path(Path::new("a/Models")),
            PathBuf::from("a/Models.meta")
        );
    }

    #[test]
    fn carry_on_rename_moves_existing_sidecar() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("a.png");
        let to = dir.path().join("b.png");
        fs::write(&from, "x").unwrap();
        fs::write(sidecar_path(&from), "guid: 123").unwrap();
        // Caller has already renamed the primary file; we only carry the meta.
        fs::rename(&from, &to).unwrap();

        carry_on_rename(&from, &to).unwrap();
        assert!(sidecar_path(&to).exists());
        assert!(!sidecar_path(&from).exists());
    }

    #[test]
    fn carry_on_rename_is_noop_without_sidecar() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("a.png");
        let to = dir.path().join("b.png");
        // No sidecar at all (non-Unity asset) — must be a silent Ok.
        assert!(carry_on_rename(&from, &to).is_ok());
        assert!(!sidecar_path(&to).exists());
    }

    #[test]
    fn carry_on_rename_refuses_to_clobber_destination() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("a.png");
        let to = dir.path().join("b.png");
        fs::write(sidecar_path(&from), "src").unwrap();
        fs::write(sidecar_path(&to), "existing").unwrap();
        // A stray sidecar already sits at the destination — don't overwrite it.
        assert!(carry_on_rename(&from, &to).is_err());
        assert!(sidecar_path(&from).exists()); // source sidecar untouched
    }

    #[test]
    fn carry_on_delete_is_noop_without_sidecar() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.png");
        // No sidecar — silent Ok. We don't exercise the real OS trash here,
        // matching the project's convention of not unit-testing trash effects.
        assert!(carry_on_delete(&path).is_ok());
    }
}
