//! Engine sidecar carrying for file operations. Unity `.meta` and Godot
//! `.import` / `.uid` travel with their asset on rename, move and delete, and are
//! hidden from the asset list. Copy does not carry them.

use std::path::{Path, PathBuf};

/// Every per-asset sidecar suffix the app knows about, Unity first. Drives both
/// carrying and hiding.
const SIDECAR_SUFFIXES: &[&str] = &[".meta", ".import", ".uid"];

/// `asset_path` with `suffix` appended to the full name (`hero.png` + `.meta`
/// -> `hero.png.meta`). The returned path may not exist.
pub fn sidecar_path(asset_path: &Path, suffix: &str) -> PathBuf {
    let mut os = asset_path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// Whether `file_name` names an engine sidecar rather than an asset. Used by
/// the scanner's and watcher's discovery filters; not gated on project type.
pub fn is_sidecar_name(file_name: &str) -> bool {
    SIDECAR_SUFFIXES.iter().any(|s| file_name.ends_with(s))
}

/// Best-effort: move every sidecar beside `from` to sit beside `to`. `Err`
/// carries one message per suffix that existed but could not be moved. Refuses
/// to clobber a different file at the destination.
pub fn carry_on_rename(from: &Path, to: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    for suffix in SIDECAR_SUFFIXES {
        let src = sidecar_path(from, suffix);
        if !src.exists() {
            continue;
        }
        let dst = sidecar_path(to, suffix);
        // A destination resolving to the source sidecar itself (case-only
        // rename, NFC/NFD variant) is not a clobber.
        if dst.exists() && !crate::undo::paths_are_same_file(&src, &dst) {
            errors.push(format!(
                "destination sidecar already exists, not overwriting: {}",
                dst.display()
            ));
            continue;
        }
        // Independent per suffix: one blocked sidecar must not strand another.
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

/// Pre-flight for callers to run before touching the primary file: every suffix
/// whose source sidecar exists and whose destination is occupied by a different
/// file. Non-empty means the rename would strand or split a sidecar.
pub fn rename_conflicts(from: &Path, to: &Path) -> Vec<String> {
    let mut conflicts = Vec::new();
    for suffix in SIDECAR_SUFFIXES {
        let src = sidecar_path(from, suffix);
        if !src.exists() {
            continue;
        }
        let dst = sidecar_path(to, suffix);
        if dst.exists() && !crate::undo::paths_are_same_file(&src, &dst) {
            conflicts.push(format!("sidecar target already exists: {}", dst.display()));
        }
    }
    conflicts
}

/// Best-effort: send every sidecar beside `path` to the OS trash. `Err` carries
/// one message per sidecar that existed but could not be trashed.
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
    fn carry_on_rename_allows_case_only_rename() {
        // On a case-insensitive filesystem the sidecar destination resolves to
        // the source sidecar itself, which must not count as a clobber.
        let dir = tempdir().unwrap();
        let from = dir.path().join("hero.png");
        let to = dir.path().join("Hero.png");
        fs::write(&from, "x").unwrap();
        fs::write(sidecar_path(&from, ".meta"), "guid: 123").unwrap();
        fs::rename(&from, &to).unwrap();

        carry_on_rename(&from, &to).unwrap();
        let carried = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .find(|n| n.ends_with(".meta"))
            .expect("meta still present");
        // The stored spelling must be the new one, not the old.
        assert_eq!(carried, "Hero.png.meta");
    }

    #[test]
    fn rename_conflicts_reports_only_genuinely_blocked_suffixes() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("a.png");
        let to = dir.path().join("b.png");
        // No sidecars at all → no conflicts.
        assert!(rename_conflicts(&from, &to).is_empty());

        // Source has a .meta, destination name is free → still no conflict.
        fs::write(sidecar_path(&from, ".meta"), "src").unwrap();
        assert!(rename_conflicts(&from, &to).is_empty());

        // A stray .meta squats on the destination → conflict, named.
        fs::write(sidecar_path(&to, ".meta"), "stray").unwrap();
        let conflicts = rename_conflicts(&from, &to);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].contains("b.png.meta"), "{}", conflicts[0]);

        // A stray sidecar with no source counterpart is not a conflict.
        fs::write(sidecar_path(&to, ".uid"), "stray").unwrap();
        assert_eq!(rename_conflicts(&from, &to).len(), 1);
    }

    #[test]
    fn rename_conflicts_allows_case_only_rename() {
        // A same-file destination must not pre-flight-block the item.
        let dir = tempdir().unwrap();
        let from = dir.path().join("hero.png");
        let to = dir.path().join("Hero.png");
        fs::write(&from, "x").unwrap();
        fs::write(sidecar_path(&from, ".meta"), "guid").unwrap();
        assert!(rename_conflicts(&from, &to).is_empty());
    }

    #[test]
    fn carry_on_delete_is_noop_without_sidecar() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.png");
        // No sidecar — silent Ok. The real OS trash is not exercised here.
        assert!(carry_on_delete(&path).is_ok());
    }
}
