//! 一个项目根现在还能不能用,作为前端可以直接分支的值。
//!
//! 与扫描的错误路径分开:stub 项目从不扫描,所以「文件夹还在不在」必须能在
//! 不跑扫描的前提下回答。形状照 `warning::ScanWarning`——serde-tagged、
//! `kind` 串由测试钉住、`asset.ts` 手抄镜像。

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

/// 两次系统调用是有意的:`metadata` 分开「没了」与「不是目录」,而只有
/// `read_dir` 能发现目录存在却列不动(mode 000)。这样的根扫描必然失败,
/// stat 却成功——只 stat 会把它报成健康。
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

/// 每个入参产出一条报告,重复入参就产出重复报告——命令不去重,前端建 Map
/// 时后写覆盖先写(同路径必然同结论)。**按 path 配对而不是按序号对应**:
/// 同型教训见 LLM 缓存(写回按 `asset_path` 配对,从不按响应顺序)。
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

    /// mode 000 是「存在但列不动」唯一便携的造法，而 chmod 在 Windows 上
    /// 无效——不门住就是主力开发机上一条永远绿的测试。
    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_be_listed_is_unreadable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let status = check(&s(&locked));
        // 先恢复权限，断言失败时 tempdir 仍能清理。
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

        // 乱序 + 重复：报告按入参逐条产出，不去重。
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
