//! Whether a project root is currently usable, as a value the frontend can
//! branch on. Answerable without running a scan. serde-tagged; the `kind`
//! strings are pinned by a test and mirrored by hand in `asset.ts`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectPathStatus {
    Ok,
    Missing,
    NotADirectory,
    Unreadable { detail: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectPathReport {
    pub path: String,
    pub status: ProjectPathStatus,
}

/// Two syscalls: `metadata` separates missing from not-a-directory, and only
/// `read_dir` catches a directory that exists but cannot be listed (mode 000).
pub fn check(path: &str) -> ProjectPathStatus {
    let p = Path::new(path);
    match fs::metadata(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProjectPathStatus::Missing,
        Err(e) => ProjectPathStatus::Unreadable {
            detail: e.to_string(),
        },
        Ok(md) if !md.is_dir() => ProjectPathStatus::NotADirectory,
        Ok(_) => match fs::read_dir(p) {
            Ok(_) => ProjectPathStatus::Ok,
            Err(e) => ProjectPathStatus::Unreadable {
                detail: e.to_string(),
            },
        },
    }
}

/// One report per input, duplicates included, paired by `path` rather than by
/// index. Callers building a Map get last-write-wins on repeated paths.
pub fn check_all(paths: &[String]) -> Vec<ProjectPathReport> {
    paths
        .iter()
        .map(|p| ProjectPathReport {
            path: p.clone(),
            status: check(p),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn s(p: &std::path::Path) -> String {
        p.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn a_readable_directory_is_ok() {
        let dir = tempdir().unwrap();
        assert_eq!(check(&s(dir.path())), ProjectPathStatus::Ok);
    }

    #[test]
    fn a_path_that_is_not_there_is_missing() {
        let dir = tempdir().unwrap();
        let gone = dir.path().join("moved-away");
        assert_eq!(check(&s(&gone)), ProjectPathStatus::Missing);
    }

    #[test]
    fn a_file_is_not_a_directory() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("project.txt");
        fs::write(&file, b"x").unwrap();
        assert_eq!(check(&s(&file)), ProjectPathStatus::NotADirectory);
    }

    /// Unix-only: mode 000 is the portable way to make a directory unlistable,
    /// and `chmod` is a no-op on Windows.
    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_be_listed_is_unreadable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let status = check(&s(&locked));
        // Restore permissions first so tempdir can clean up even on failure.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        match status {
            ProjectPathStatus::Unreadable { detail } => assert!(!detail.is_empty()),
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[test]
    fn check_all_pairs_every_input_with_its_own_verdict() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, b"x").unwrap();
        let gone = dir.path().join("nope");

        // Shuffled and duplicated: one report per input, in order, no dedup.
        let inputs = vec![s(&gone), s(dir.path()), s(&file), s(&gone)];
        let reports = check_all(&inputs);

        assert_eq!(
            reports.len(),
            4,
            "one report per input, duplicates included"
        );
        assert_eq!(reports[0].path, s(&gone));
        assert_eq!(reports[0].status, ProjectPathStatus::Missing);
        assert_eq!(reports[1].status, ProjectPathStatus::Ok);
        assert_eq!(reports[2].status, ProjectPathStatus::NotADirectory);
        assert_eq!(reports[3].status, ProjectPathStatus::Missing);
    }

    #[test]
    fn status_wire_shape_matches_the_frontends_mirror() {
        let ok = serde_json::to_value(ProjectPathStatus::Ok).unwrap();
        assert_eq!(ok["kind"], "ok");

        let missing = serde_json::to_value(ProjectPathStatus::Missing).unwrap();
        assert_eq!(missing["kind"], "missing");

        let not_dir = serde_json::to_value(ProjectPathStatus::NotADirectory).unwrap();
        assert_eq!(not_dir["kind"], "not_a_directory");

        let unreadable = serde_json::to_value(ProjectPathStatus::Unreadable {
            detail: "permission denied".into(),
        })
        .unwrap();
        assert_eq!(unreadable["kind"], "unreadable");
        assert_eq!(unreadable["detail"], "permission denied");

        let report = serde_json::to_value(ProjectPathReport {
            path: "/a/b".into(),
            status: ProjectPathStatus::Missing,
        })
        .unwrap();
        assert_eq!(report["path"], "/a/b");
        assert_eq!(report["status"]["kind"], "missing");
    }
}
