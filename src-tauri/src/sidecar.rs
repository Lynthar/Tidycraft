//! Engine sidecar carrying for file operations.
//!
//! Unity and Godot both keep per-asset metadata in a sidecar file that sits
//! next to the asset and carries its full name plus a suffix. Both engines
//! tolerate moves and renames *as long as the sidecar travels with its asset*:
//!
//! - **`.meta`** (Unity) — GUID + import settings. References across a project
//!   are stored by GUID, so a stranded `.meta` makes Unity mint a fresh GUID
//!   for the renamed file and **every reference to it breaks**.
//! - **`.uid`** (Godot 4.4+) — the `uid://…` identity of a script or shader.
//!   Scenes store the UID rather than the path, so a stranded `.uid` breaks
//!   every scene that referenced the script. Godot's exact analogue of `.meta`.
//! - **`.import`** (Godot) — per-asset import settings. Losing it doesn't break
//!   references, but Godot regenerates the file **with default parameters**, so
//!   the asset's compression / mipmap / filter choices are silently lost.
//!
//! `.import`'s `[deps]` block (`source_file=`, and `dest_files=` pointing under
//! `.godot/imported/`) goes stale when the asset moves, and we could not pin
//! down from Godot's docs whether the editor rewrites those entries or
//! re-imports from scratch. Carrying it is still right, because the two
//! outcomes are not symmetric: the worst case for carrying is one re-import,
//! while the worst case for stranding it is a settings loss the user never
//! sees.
//!
//! So the app's own rename / move / delete must carry the sidecars. These
//! helpers are best-effort: a missing sidecar (wrong engine, or an asset the
//! editor hasn't imported yet) is a silent no-op, and a carry failure is
//! returned to the caller — which logs it — without rolling back the
//! already-succeeded primary op (rollback can itself fail and leave a more
//! confusing half-state). **Each suffix is attempted independently**, so one
//! blocked sidecar never decides another's fate.
//!
//! Copy / duplicate deliberately do NOT carry sidecars: a duplicated asset must
//! receive a fresh identity, so copying `.meta` (GUID), `.uid`, or `.import`
//! (which also records a Godot UID) would create a collision. Those paths are
//! left untouched on purpose.
//!
//! The same suffix list drives [`is_sidecar_name`], which the scanner and
//! watcher use to keep sidecars out of the asset list. Carrying and hiding have
//! to agree: a sidecar the user can select and rename on its own is one the
//! file ops would then try to carry a sidecar *for*.

use std::path::{Path, PathBuf};

/// Every per-asset sidecar suffix the app knows about, Unity first.
///
/// Adding one here makes it carried by rename / move / delete **and** hidden
/// from the asset list in the same edit — see the module docs for why those two
/// must move together.
const SIDECAR_SUFFIXES: &[&str] = &[".meta", ".import", ".uid"];

/// The sidecar path for `asset_path` with `suffix` appended to the full name
/// (`hero.png` + `.meta` -> `hero.png.meta`, `Models` + `.meta` ->
/// `Models.meta`). Appends rather than replacing the extension, matching both
/// engines' convention. The returned path may not exist.
pub fn sidecar_path(asset_path: &Path, suffix: &str) -> PathBuf {
    let mut os = asset_path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// Whether `file_name` names an engine sidecar rather than an asset. Used by
/// the scanner's and watcher's discovery filters: a Godot project carries one
/// `.import` per imported asset (and, since 4.4, one `.uid` per script), so
/// listing them roughly doubles the asset count with files the user can neither
/// preview nor meaningfully act on.
///
/// Deliberately not gated on the detected project type, matching how `.meta`
/// has always been filtered. The suffixes are engine-specific enough that a
/// genuine asset named `*.import` is rarer than the cost of threading a project
/// type into the watcher's path-shape filter, which has none.
pub fn is_sidecar_name(file_name: &str) -> bool {
    SIDECAR_SUFFIXES.iter().any(|s| file_name.ends_with(s))
}

/// Best-effort: move every sidecar that exists beside `from` to sit beside
/// `to`. `Ok(())` both when sidecars were moved and when there were none
/// (an asset of another engine — nothing to carry). `Err` carries one message
/// per suffix that existed but could not be moved; the caller has already
/// renamed the primary file and just logs this. Refuses to clobber an existing
/// sidecar at the destination.
pub fn carry_on_rename(from: &Path, to: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    for suffix in SIDECAR_SUFFIXES {
        let src = sidecar_path(from, suffix);
        if !src.exists() {
            continue;
        }
        let dst = sidecar_path(to, suffix);
        if dst.exists() {
            errors.push(format!(
                "destination sidecar already exists, not overwriting: {}",
                dst.display()
            ));
            continue;
        }
        // `continue` rather than an early return: a Godot asset can have both
        // an `.import` and a `.uid`, and one being blocked must not strand the
        // other — that one is the reference-breaking half.
        if let Err(e) = std::fs::rename(&src, &dst) {
            errors.push(format!("failed to move sidecar {}: {}", src.display(), e));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Best-effort: send every sidecar beside `path` to the OS trash too, so
/// deleting an asset doesn't strand them. `Ok(())` when trashed or when there
/// are none; `Err` carries one message per sidecar that existed but could not
/// be trashed (caller logs it — the primary file is already gone).
pub fn carry_on_delete(path: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    for suffix in SIDECAR_SUFFIXES {
        let side = sidecar_path(path, suffix);
        if !side.exists() {
            continue;
        }
        if let Err(e) = trash::delete(&side) {
            errors.push(format!("failed to trash sidecar {}: {}", side.display(), e));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn sidecar_path_appends_suffix_to_full_name() {
        assert_eq!(
            sidecar_path(Path::new("a/hero.png"), ".meta"),
            PathBuf::from("a/hero.png.meta")
        );
        // Folders carry a sidecar too — append, don't touch the "extension".
        assert_eq!(
            sidecar_path(Path::new("a/Models"), ".meta"),
            PathBuf::from("a/Models.meta")
        );
        // Godot's two sit beside the asset exactly the same way.
        assert_eq!(
            sidecar_path(Path::new("a/hero.png"), ".import"),
            PathBuf::from("a/hero.png.import")
        );
        assert_eq!(
            sidecar_path(Path::new("a/player.gd"), ".uid"),
            PathBuf::from("a/player.gd.uid")
        );
    }

    #[test]
    fn is_sidecar_name_covers_every_carried_suffix() {
        // Asserted against SIDECAR_SUFFIXES rather than a second hand-written
        // list, so hiding can never drift out of step with carrying.
        for suffix in SIDECAR_SUFFIXES {
            assert!(is_sidecar_name(&format!("hero.png{suffix}")), "{suffix}");
        }
        assert!(!is_sidecar_name("hero.png"));
        assert!(!is_sidecar_name("player.gd"));
        // Suffix match, not substring: `.importer` is a different name.
        assert!(!is_sidecar_name("hero.png.importer"));
    }

    #[test]
    fn carry_on_rename_moves_existing_sidecar() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("a.png");
        let to = dir.path().join("b.png");
        fs::write(&from, "x").unwrap();
        fs::write(sidecar_path(&from, ".meta"), "guid: 123").unwrap();
        // Caller has already renamed the primary file; we only carry the meta.
        fs::rename(&from, &to).unwrap();

        carry_on_rename(&from, &to).unwrap();
        assert!(sidecar_path(&to, ".meta").exists());
        assert!(!sidecar_path(&from, ".meta").exists());
    }

    #[test]
    fn carry_on_rename_moves_both_godot_sidecars() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("hero.png");
        let to = dir.path().join("villain.png");
        fs::write(&from, "x").unwrap();
        fs::write(sidecar_path(&from, ".import"), "[remap]\n").unwrap();
        fs::write(sidecar_path(&from, ".uid"), "uid://abcdef\n").unwrap();
        fs::rename(&from, &to).unwrap();

        carry_on_rename(&from, &to).unwrap();
        for suffix in [".import", ".uid"] {
            assert!(sidecar_path(&to, suffix).exists(), "{suffix} not carried");
            assert!(
                !sidecar_path(&from, suffix).exists(),
                "{suffix} left behind"
            );
        }
    }

    #[test]
    fn carry_on_rename_is_noop_without_sidecar() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("a.png");
        let to = dir.path().join("b.png");
        // No sidecar at all (an engine that writes none) — must be a silent Ok.
        assert!(carry_on_rename(&from, &to).is_ok());
        assert!(!sidecar_path(&to, ".meta").exists());
    }

    #[test]
    fn carry_on_rename_refuses_to_clobber_destination() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("a.png");
        let to = dir.path().join("b.png");
        fs::write(sidecar_path(&from, ".meta"), "src").unwrap();
        fs::write(sidecar_path(&to, ".meta"), "existing").unwrap();
        // A stray sidecar already sits at the destination — don't overwrite it.
        assert!(carry_on_rename(&from, &to).is_err());
        assert!(sidecar_path(&from, ".meta").exists()); // source sidecar untouched
        assert_eq!(
            fs::read_to_string(sidecar_path(&to, ".meta")).unwrap(),
            "existing"
        );
    }

    #[test]
    fn carry_on_rename_carries_the_rest_when_one_is_blocked() {
        // The whole reason each suffix is attempted independently: `.meta`
        // sorts first, and its failure must not strand the `.uid` behind it.
        let dir = tempdir().unwrap();
        let from = dir.path().join("a.png");
        let to = dir.path().join("b.png");
        fs::write(sidecar_path(&from, ".meta"), "src").unwrap();
        fs::write(sidecar_path(&to, ".meta"), "existing").unwrap(); // blocks
        fs::write(sidecar_path(&from, ".uid"), "uid://abcdef").unwrap();

        let err = carry_on_rename(&from, &to).unwrap_err();
        assert!(
            err.contains(".meta"),
            "blocked suffix must be reported: {err}"
        );
        assert!(
            !err.contains(".uid"),
            "the carried suffix must not be reported as failed: {err}"
        );

        // The unblocked sidecar still moved…
        assert!(sidecar_path(&to, ".uid").exists());
        assert!(!sidecar_path(&from, ".uid").exists());
        // …and the blocked one is untouched at both ends.
        assert!(sidecar_path(&from, ".meta").exists());
        assert_eq!(
            fs::read_to_string(sidecar_path(&to, ".meta")).unwrap(),
            "existing"
        );
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
