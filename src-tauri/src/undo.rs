//! 撤销管理模块
//!
//! 批量文件操作的撤销栈。v2 (Phase 3.1): 历史持久化到
//! `{data_dir}/tidycraft/undo/{sha256(root)[..16]}.json`,每次
//! record_batch / undo / clear 后落盘,register_project 时回读。
//! 文件在用户/磁盘操作失败时只记录不崩,保证撤销功能本身不阻塞主流程。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 单个文件操作记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOperation {
    /// 操作类型
    pub operation_type: OperationType,
    /// 原始路径
    pub original_path: String,
    /// 新路径（重命名/移动后）
    pub new_path: Option<String>,
    /// 操作时间戳
    pub timestamp: u64,
}

/// 操作类型枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationType {
    /// 重命名操作
    Rename,
    /// 移动操作（预留）
    Move,
    /// 删除操作（预留，需要备份机制）
    Delete,
}

/// 批量操作记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOperation {
    /// 唯一标识符
    pub id: String,
    /// 操作描述
    pub description: String,
    /// 包含的文件操作列表
    pub operations: Vec<FileOperation>,
    /// 操作时间戳
    pub timestamp: u64,
    /// 是否已撤销
    pub undone: bool,
}

/// 撤销操作结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoResult {
    /// 是否成功
    pub success: bool,
    /// 成功撤销的文件数
    pub reverted_count: usize,
    /// 失败的文件数
    pub failed_count: usize,
    /// 错误信息列表
    pub errors: Vec<String>,
    /// 被撤销的操作描述
    pub operation_description: String,
    /// 本次**实际还原成功**的 `(原路径, 新路径)` 对。命令层用它把标签绑定迁回
    /// (new_path → original)——只迁移真正搬回去了的文件,所以部分失败的撤销不会把
    /// 仍停在 new_path 的文件的标签剥走(旧实现用 `original.exists()` 猜测,在
    /// 「original 被无关占位文件顶替」时会误判)。不序列化给前端。
    #[serde(skip)]
    pub reverted_pairs: Vec<(String, String)>,
}

/// 历史记录摘要（用于 UI 显示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// 操作 ID
    pub id: String,
    /// 操作描述
    pub description: String,
    /// 文件数量
    pub file_count: usize,
    /// 时间戳
    pub timestamp: u64,
    /// 是否可撤销（未被撤销且是最近的操作）
    pub can_undo: bool,
}

/// 撤销历史管理器
pub struct UndoManager {
    /// 操作历史栈
    history: Vec<BatchOperation>,
    /// 最大历史记录数
    max_history: usize,
    /// 磁盘持久化路径。`None` 表示纯内存(测试 / fallback)。
    persist_path: Option<PathBuf>,
}

impl UndoManager {
    /// 创建纯内存的撤销管理器(无持久化)。主要给测试用;生产代码走
    /// `load_for_project`。
    pub const fn new(max_history: usize) -> Self {
        Self {
            history: Vec::new(),
            max_history,
            persist_path: None,
        }
    }

    /// 为某个项目构造 UndoManager,并从磁盘读回历史(如果存在)。
    /// 回读后会按 `max_history` trim 掉过旧的批次。
    pub fn load_for_project(project_root: &Path, max_history: usize) -> Self {
        let persist_path = Self::persist_path_for(project_root);
        let history = persist_path
            .as_deref()
            .map(|p| Self::read_history_from(p, max_history))
            .unwrap_or_default();

        Self {
            history,
            max_history,
            persist_path,
        }
    }

    /// 从 `path` 读回历史并按 `max_history` 保留最新的若干条。
    ///
    /// 文件缺失是正常的「还没有历史」→ 空。但**存在却解析不了**的文件不能
    /// 静默退化为空:下一次 `record_batch` 的 `save_to_disk` 会盖掉它,用户
    /// 那份(很可能还能救的)历史就永久没了,且全程无任何痕迹。所以先把损坏
    /// 文件挪到 `.corrupt` 备份再从空开始——与 `tags.rs::load` 同一套纪律。
    fn read_history_from(path: &Path, max_history: usize) -> Vec<BatchOperation> {
        if !path.exists() {
            return Vec::new();
        }
        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Vec<BatchOperation>>(&content) {
                Ok(loaded) => {
                    let start = loaded.len().saturating_sub(max_history);
                    return loaded[start..].to_vec();
                }
                Err(e) => {
                    // 保留最早那份备份(最可能是完整的),别让后一次损坏覆盖它。
                    let backup = path.with_extension("json.corrupt");
                    if !backup.exists() {
                        let _ = fs::rename(path, &backup);
                    }
                    eprintln!(
                        "[undo] {} failed to parse ({e}); backed up to {}",
                        path.display(),
                        backup.display()
                    );
                }
            },
            Err(e) => eprintln!("[undo] failed to read {}: {e}", path.display()),
        }
        Vec::new()
    }

    /// 把历史原子写入 `path`(建父目录)。错误**向上传播**,由 `save_to_disk`
    /// 负责记日志——写盘持续失败意味着每次重启都丢光撤销历史,静默吞掉它
    /// 等于让这种故障永远查不出来。
    fn write_history_to(path: &Path, history: &[BatchOperation]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(history)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // Atomic (temp + rename): a crash mid-write must not tear the
        // persisted undo history — same discipline as tags.rs.
        crate::fs_atomic::write_atomic(path, json.as_bytes())
    }

    /// 以项目根路径的 SHA256(前 16 hex) 做文件名,避免路径特殊字符 /
    /// 冲突问题,也能跨 app 会话稳定命中同一文件。
    fn persist_path_for(project_root: &Path) -> Option<PathBuf> {
        let mut hasher = Sha256::new();
        hasher.update(project_root.to_string_lossy().as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        dirs::data_dir().map(|d| {
            d.join("tidycraft")
                .join("undo")
                .join(format!("{}.json", &hash[..16]))
        })
    }

    /// 写盘 best-effort:失败不阻塞撤销操作(内存里的历史照常可用),但**必须
    /// 留下日志**——否则一个持续写不进去的数据目录表现为「每次重启撤销历史
    /// 都空了」,没有任何线索可查。
    fn save_to_disk(&self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        if let Err(e) = Self::write_history_to(path, &self.history) {
            eprintln!("[undo] failed to persist history to {}: {e}", path.display());
        }
    }

    /// 记录一次批量操作
    pub fn record_batch(&mut self, description: String, operations: Vec<FileOperation>) -> String {
        let id = generate_operation_id();
        let timestamp = current_timestamp();

        let batch = BatchOperation {
            id: id.clone(),
            description,
            operations,
            timestamp,
            undone: false,
        };

        self.history.push(batch);

        // 超过最大历史记录数时移除最旧的
        while self.history.len() > self.max_history {
            self.history.remove(0);
        }

        self.save_to_disk();
        id
    }

    /// 撤销最近一次未撤销的操作
    pub fn undo_last(&mut self) -> Option<UndoResult> {
        // 查找最近一个未撤销的操作
        let index = self
            .history
            .iter()
            .rposition(|op| !op.undone)?;

        let batch = &self.history[index];
        let description = batch.description.clone();

        // 执行撤销
        let had_operations = !batch.operations.is_empty();
        let result = execute_batch_undo(&batch.operations);

        // 标记为已撤销。全败(一个都没回滚)时**不**标记:失败几乎总是暂时且
        // 可修复的——文件被 Photoshop/Unity 占用、盘符临时不可用——烧掉条目
        // 就等于在用户关掉那个程序之后再也回不去了。部分成功仍然标记:重跑
        // 会对已经回滚过的文件再次尝试并报错,不是可用的重试语义。
        // `had_operations` 守住空批次,否则它会永远卡在栈顶。
        if result.reverted_count > 0 || !had_operations {
            self.history[index].undone = true;
            self.save_to_disk();
        }

        Some(UndoResult {
            success: result.failed_count == 0,
            reverted_count: result.reverted_count,
            failed_count: result.failed_count,
            errors: result.errors,
            operation_description: description,
            reverted_pairs: result.reverted_pairs,
        })
    }

    /// 获取撤销历史列表
    pub fn get_history(&self) -> Vec<HistoryEntry> {
        // 找到最近一个未撤销的操作的索引
        let last_undoable_index = self
            .history
            .iter()
            .rposition(|op| !op.undone);

        self.history
            .iter()
            .enumerate()
            .map(|(i, op)| HistoryEntry {
                id: op.id.clone(),
                description: op.description.clone(),
                file_count: op.operations.len(),
                timestamp: op.timestamp,
                // 只有最近一个未撤销的操作可以撤销
                can_undo: last_undoable_index == Some(i) && !op.undone,
            })
            .rev() // 最新的在前面
            .collect()
    }

    /// 检查是否有可撤销的操作
    pub fn can_undo(&self) -> bool {
        self.history.iter().any(|op| !op.undone)
    }

    /// 清空历史记录
    pub fn clear_history(&mut self) {
        self.history.clear();
        self.save_to_disk();
    }

    /// 获取最近一次操作的描述
    #[allow(dead_code)]
    pub fn get_last_operation_description(&self) -> Option<String> {
        self.history
            .iter()
            .filter(|op| !op.undone)
            .last()
            .map(|op| op.description.clone())
    }

    /// 获取历史记录数量
    #[allow(dead_code)]
    pub fn history_count(&self) -> usize {
        self.history.len()
    }
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new(50)
    }
}

/// 判断两个路径是否指向**同一个文件**——按文件系统身份(Unix: dev+inode,
/// Windows: 卷序列号+文件索引,经 `same-file` crate),而不是按文件名猜测。
/// 任一路径不存在或元数据不可读时返回 false,调用方随即按「目标已存在」保守拒绝。
///
/// 用于改名/撤销的「目标已存在」守卫:目标 `exists()` 可能命中源文件自身
/// (大小写不敏感文件系统上的纯大小写改名、macOS 的 NFC/NFD Unicode 变体),
/// 这种改名必须放行;而大小写**敏感**文件系统(Linux)上 `foo.png` 与 `FOO.PNG`
/// 可共存,旧的「文件名仅大小写不同即放行」在那里会让 `fs::rename` 静默覆盖
/// 目标文件。按身份判断在两类文件系统上都正确。
pub(crate) fn paths_are_same_file(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

/// 执行批量撤销
fn execute_batch_undo(operations: &[FileOperation]) -> UndoResult {
    let mut reverted_count = 0;
    let mut failed_count = 0;
    let mut errors = Vec::new();
    // 只收集真正还原成功的 (原路径, 新路径) 对,供命令层把标签迁回。失败的操作
    // (源丢失 / 目标被占用)不进此列表,其标签因此留在 new_path 不被误迁。
    let mut reverted_pairs: Vec<(String, String)> = Vec::new();

    // 反向遍历操作列表，按相反顺序撤销
    for op in operations.iter().rev() {
        match execute_single_undo(op) {
            Ok(()) => {
                reverted_count += 1;
                if let Some(np) = &op.new_path {
                    reverted_pairs.push((op.original_path.clone(), np.clone()));
                }
            }
            Err(e) => {
                failed_count += 1;
                errors.push(e);
            }
        }
    }

    UndoResult {
        success: failed_count == 0,
        reverted_count,
        failed_count,
        errors,
        operation_description: String::new(),
        reverted_pairs,
    }
}

/// 执行单个文件撤销操作
fn execute_single_undo(operation: &FileOperation) -> Result<(), String> {
    match operation.operation_type {
        OperationType::Rename => {
            let new_path = operation
                .new_path
                .as_ref()
                .ok_or("Missing new path for rename operation")?;

            let src = Path::new(new_path);
            let dst = Path::new(&operation.original_path);

            // 检查源文件是否存在
            if !src.exists() {
                return Err(format!(
                    "Source file not found: {} (file may have been modified)",
                    new_path
                ));
            }

            // 检查目标路径是否已存在。dst.exists() 可能命中 src 自身(大小写不敏感
            // 文件系统上撤销纯大小写改名、NFC/NFD 变体),这种情况放行——fs::rename
            // 能正确改名;只有 dst 确实是**另一个文件**时才拒绝(大小写敏感文件系统
            // 上同名异例可共存,按文件名猜测会静默覆盖它,按身份判断不会)。
            if dst.exists() && !paths_are_same_file(src, dst) {
                return Err(format!(
                    "Target path already exists: {}",
                    operation.original_path
                ));
            }

            // 执行重命名
            fs::rename(src, dst).map_err(|e| {
                format!(
                    "Failed to rename '{}' back to '{}': {}",
                    new_path, operation.original_path, e
                )
            })?;
            // 撤销也要对称连带 Unity .meta —— 否则把主文件名改回去却把
            // sidecar 留在新名上,反而制造孤儿 + 断引用。Best-effort:
            // 连带失败只记录,不回滚已成功的主文件还原。
            if let Err(e) = crate::meta_sidecar::carry_on_rename(src, dst) {
                eprintln!(
                    "[undo] .meta sidecar not carried back for {}: {}",
                    new_path, e
                );
            }
            Ok(())
        }
        OperationType::Move => {
            // 移动操作的撤销与重命名类似
            let new_path = operation
                .new_path
                .as_ref()
                .ok_or("Missing new path for move operation")?;

            let src = Path::new(new_path);
            let dst = Path::new(&operation.original_path);

            if !src.exists() {
                return Err(format!("Source file not found: {}", new_path));
            }

            if dst.exists() {
                return Err(format!(
                    "Target path already exists: {}",
                    operation.original_path
                ));
            }

            // 确保目标目录存在
            if let Some(parent) = dst.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).map_err(|e| {
                        format!("Failed to create directory '{}': {}", parent.display(), e)
                    })?;
                }
            }

            fs::rename(src, dst).map_err(|e| {
                format!(
                    "Failed to move '{}' back to '{}': {}",
                    new_path, operation.original_path, e
                )
            })?;
            // 移动撤销同样对称连带 Unity .meta(见 Rename 分支说明)。
            if let Err(e) = crate::meta_sidecar::carry_on_rename(src, dst) {
                eprintln!(
                    "[undo] .meta sidecar not carried back for {}: {}",
                    new_path, e
                );
            }
            Ok(())
        }
        OperationType::Delete => {
            // 删除操作的撤销需要备份机制，目前不支持
            Err("Undo for delete operations is not yet supported".to_string())
        }
    }
}

/// 生成唯一的操作 ID。用 uuid v4 —— 旧实现是 `秒级时间戳 ^ 栈地址`,而同一
/// 调用点的栈地址通常不变,于是同一秒内记录的两批操作会生成相同 id,
/// 按 id 查找的路径(如 undo 历史列表)命中第一个 → 关联到错误的批次。
fn generate_operation_id() -> String {
    format!("op_{}", uuid::Uuid::new_v4().simple())
}

/// 获取当前时间戳
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ---- History persistence (the same discipline tags.rs already keeps) ----

    fn a_batch(description: &str) -> BatchOperation {
        BatchOperation {
            id: "id-1".to_string(),
            description: description.to_string(),
            operations: vec![FileOperation {
                operation_type: OperationType::Rename,
                original_path: "/p/a.png".to_string(),
                new_path: Some("/p/b.png".to_string()),
                timestamp: 1,
            }],
            timestamp: 1,
            undone: false,
        }
    }

    /// A history file that exists but won't parse must not degrade to "no
    /// history": the next `record_batch` saves over it, and the user's route
    /// back is gone with no way to tell it ever existed. `tags.rs` already
    /// backs its file up for exactly this reason; this path did not.
    #[test]
    fn a_corrupt_history_file_is_preserved_rather_than_silently_dropped() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");
        fs::write(&path, "{ truncated not json").unwrap();

        let history = UndoManager::read_history_from(&path, 50);

        assert!(history.is_empty(), "unparseable history yields no entries");
        let backup = dir.path().join("history.json.corrupt");
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "{ truncated not json",
            "the corrupt file must survive for recovery"
        );
        assert!(!path.exists(), "it is moved aside, not copied");
    }

    /// Second corruption keeps the FIRST backup — that one is the likeliest
    /// to be complete, and a fresh empty file overwriting it would finish the
    /// job the corruption started.
    #[test]
    fn an_existing_corrupt_backup_is_not_overwritten() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");
        let backup = dir.path().join("history.json.corrupt");
        fs::write(&backup, "the original").unwrap();
        fs::write(&path, "later garbage").unwrap();

        UndoManager::read_history_from(&path, 50);

        assert_eq!(fs::read_to_string(&backup).unwrap(), "the original");
    }

    #[test]
    fn a_readable_history_round_trips_and_trims_to_the_limit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("history.json");
        let batches: Vec<BatchOperation> = ["oldest", "middle", "newest"]
            .iter()
            .map(|d| a_batch(d))
            .collect();

        UndoManager::write_history_to(&path, &batches).expect("write succeeds");
        let loaded = UndoManager::read_history_from(&path, 2);

        // Trimming keeps the newest entries.
        let descriptions: Vec<&str> = loaded.iter().map(|b| b.description.as_str()).collect();
        assert_eq!(descriptions, ["middle", "newest"]);
    }

    /// A write that fails must reach a log, not `let _ =`. The manager keeps
    /// working in memory either way, but a persistently unwritable history is
    /// a silent loss of every undo across restarts.
    #[test]
    fn a_failed_history_write_is_reported_rather_than_swallowed() {
        let dir = tempdir().unwrap();
        // A *file* sits where the history's parent directory would go, so
        // `create_dir_all` cannot succeed.
        let blocker = dir.path().join("undo");
        fs::write(&blocker, "not a directory").unwrap();

        assert!(UndoManager::write_history_to(&blocker.join("history.json"), &[]).is_err());
    }

    fn create_test_file(dir: &Path, name: &str) -> String {
        let path = dir.join(name);
        fs::write(&path, "test content").unwrap();
        path.to_string_lossy().to_string()
    }

    /// A batch where nothing could be reverted must stay retryable. The usual
    /// cause is transient and fixable — the files are open in Photoshop/Unity,
    /// or a drive is momentarily unavailable — so burning the entry destroys
    /// the user's only route back once they close the other app.
    #[test]
    fn a_totally_failed_undo_leaves_the_batch_retryable() {
        let mut manager = UndoManager::new(10);
        // new_path doesn't exist, so every revert fails.
        manager.record_batch(
            "Rename 2 files".to_string(),
            vec![
                FileOperation {
                    operation_type: OperationType::Rename,
                    original_path: "/nowhere/a.png".to_string(),
                    new_path: Some("/nowhere/a_new.png".to_string()),
                    timestamp: current_timestamp(),
                },
                FileOperation {
                    operation_type: OperationType::Rename,
                    original_path: "/nowhere/b.png".to_string(),
                    new_path: Some("/nowhere/b_new.png".to_string()),
                    timestamp: current_timestamp(),
                },
            ],
        );

        let result = manager.undo_last().expect("a batch was recorded");
        assert_eq!(result.reverted_count, 0);
        assert_eq!(result.failed_count, 2);
        assert!(!result.success);

        assert!(
            manager.can_undo(),
            "a batch that reverted nothing must remain undoable"
        );
        assert!(manager.undo_last().is_some(), "retry must find it again");
    }

    /// Partial success still consumes the entry: re-running would re-attempt
    /// the files that already moved back and error on them. Only a *total*
    /// failure is retryable.
    #[test]
    fn a_partial_undo_still_consumes_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        let moved = create_test_file(dir.path(), "renamed.png");
        let original = dir.path().join("original.png").to_string_lossy().to_string();

        let mut manager = UndoManager::new(10);
        manager.record_batch(
            "Mixed".to_string(),
            vec![
                FileOperation {
                    operation_type: OperationType::Rename,
                    original_path: original,
                    new_path: Some(moved),
                    timestamp: current_timestamp(),
                },
                FileOperation {
                    operation_type: OperationType::Rename,
                    original_path: "/nowhere/b.png".to_string(),
                    new_path: Some("/nowhere/b_new.png".to_string()),
                    timestamp: current_timestamp(),
                },
            ],
        );

        let result = manager.undo_last().expect("a batch was recorded");
        assert_eq!(result.reverted_count, 1);
        assert_eq!(result.failed_count, 1);
        assert!(!manager.can_undo());
    }

    #[test]
    fn test_undo_manager_new() {
        let manager = UndoManager::new(10);
        assert_eq!(manager.max_history, 10);
        assert!(manager.history.is_empty());
        assert!(!manager.can_undo());
    }

    #[test]
    fn test_record_batch() {
        let mut manager = UndoManager::new(10);

        let ops = vec![FileOperation {
            operation_type: OperationType::Rename,
            original_path: "/old/path.txt".to_string(),
            new_path: Some("/new/path.txt".to_string()),
            timestamp: current_timestamp(),
        }];

        let id = manager.record_batch("Test operation".to_string(), ops);

        assert!(!id.is_empty());
        assert!(id.starts_with("op_"));
        assert_eq!(manager.history_count(), 1);
        assert!(manager.can_undo());
    }

    #[test]
    fn test_history_limit() {
        let mut manager = UndoManager::new(3);

        for i in 0..5 {
            let ops = vec![FileOperation {
                operation_type: OperationType::Rename,
                original_path: format!("/old/{}.txt", i),
                new_path: Some(format!("/new/{}.txt", i)),
                timestamp: current_timestamp(),
            }];
            manager.record_batch(format!("Operation {}", i), ops);
        }

        assert_eq!(manager.history_count(), 3);

        // 确保保留的是最新的 3 个操作
        let history = manager.get_history();
        assert_eq!(history.len(), 3);
        assert!(history[0].description.contains('4'));
        assert!(history[1].description.contains('3'));
        assert!(history[2].description.contains('2'));
    }

    #[test]
    fn test_get_history() {
        let mut manager = UndoManager::new(10);

        let ops = vec![
            FileOperation {
                operation_type: OperationType::Rename,
                original_path: "/a.txt".to_string(),
                new_path: Some("/b.txt".to_string()),
                timestamp: current_timestamp(),
            },
            FileOperation {
                operation_type: OperationType::Rename,
                original_path: "/c.txt".to_string(),
                new_path: Some("/d.txt".to_string()),
                timestamp: current_timestamp(),
            },
        ];

        manager.record_batch("Rename 2 files".to_string(), ops);

        let history = manager.get_history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].file_count, 2);
        assert_eq!(history[0].description, "Rename 2 files");
        assert!(history[0].can_undo);
    }

    #[test]
    fn test_undo_rename() {
        let dir = tempdir().unwrap();

        // 创建原始文件
        let original_path = create_test_file(dir.path(), "original.txt");
        let new_path = dir.path().join("renamed.txt");

        // 模拟重命名操作
        fs::rename(&original_path, &new_path).unwrap();

        let mut manager = UndoManager::new(10);

        let ops = vec![FileOperation {
            operation_type: OperationType::Rename,
            original_path: original_path.clone(),
            new_path: Some(new_path.to_string_lossy().to_string()),
            timestamp: current_timestamp(),
        }];

        manager.record_batch("Rename file".to_string(), ops);

        // 执行撤销
        let result = manager.undo_last().unwrap();

        assert!(result.success);
        assert_eq!(result.reverted_count, 1);
        assert_eq!(result.failed_count, 0);
        assert!(result.errors.is_empty());

        // 验证文件已恢复原名
        assert!(Path::new(&original_path).exists());
        assert!(!new_path.exists());
    }

    #[test]
    fn test_undo_rename_carries_meta_sidecar() {
        // Undoing a rename must move the Unity .meta sidecar back too —
        // otherwise the revert strands the sidecar on the new name and breaks
        // GUID references, the very thing the forward op was careful to avoid.
        let dir = tempdir().unwrap();
        let original = dir.path().join("a.txt");
        let renamed = dir.path().join("b.txt");
        fs::write(&original, "asset").unwrap();
        fs::write(crate::meta_sidecar::sidecar_path(&original), "guid: 1").unwrap();
        // Simulate the forward rename having already carried the sidecar.
        fs::rename(&original, &renamed).unwrap();
        fs::rename(
            crate::meta_sidecar::sidecar_path(&original),
            crate::meta_sidecar::sidecar_path(&renamed),
        )
        .unwrap();

        let mut manager = UndoManager::new(10);
        manager.record_batch(
            "Rename".to_string(),
            vec![FileOperation {
                operation_type: OperationType::Rename,
                original_path: original.to_string_lossy().to_string(),
                new_path: Some(renamed.to_string_lossy().to_string()),
                timestamp: current_timestamp(),
            }],
        );

        let result = manager.undo_last().unwrap();
        assert!(result.success);
        // Both the asset and its sidecar are back at the original name.
        assert!(original.exists());
        assert!(crate::meta_sidecar::sidecar_path(&original).exists());
        assert!(!renamed.exists());
        assert!(!crate::meta_sidecar::sidecar_path(&renamed).exists());
    }

    #[test]
    fn test_undo_already_undone() {
        let mut manager = UndoManager::new(10);

        let ops = vec![FileOperation {
            operation_type: OperationType::Rename,
            original_path: "/old.txt".to_string(),
            new_path: Some("/new.txt".to_string()),
            timestamp: current_timestamp(),
        }];

        manager.record_batch("Test".to_string(), ops);

        // 标记为已撤销
        manager.history[0].undone = true;

        // 尝试撤销应该返回 None
        assert!(manager.undo_last().is_none());
        assert!(!manager.can_undo());
    }

    #[test]
    fn test_clear_history() {
        let mut manager = UndoManager::new(10);

        let ops = vec![FileOperation {
            operation_type: OperationType::Rename,
            original_path: "/old.txt".to_string(),
            new_path: Some("/new.txt".to_string()),
            timestamp: current_timestamp(),
        }];

        manager.record_batch("Test".to_string(), ops);
        assert_eq!(manager.history_count(), 1);

        manager.clear_history();
        assert_eq!(manager.history_count(), 0);
        assert!(!manager.can_undo());
    }

    #[test]
    fn test_get_last_operation_description() {
        let mut manager = UndoManager::new(10);
        assert!(manager.get_last_operation_description().is_none());

        let ops = vec![FileOperation {
            operation_type: OperationType::Rename,
            original_path: "/a.txt".to_string(),
            new_path: Some("/b.txt".to_string()),
            timestamp: current_timestamp(),
        }];

        manager.record_batch("First operation".to_string(), ops.clone());
        assert_eq!(
            manager.get_last_operation_description(),
            Some("First operation".to_string())
        );

        manager.record_batch("Second operation".to_string(), ops);
        assert_eq!(
            manager.get_last_operation_description(),
            Some("Second operation".to_string())
        );
    }

    #[test]
    fn test_operation_type_serialization() {
        let rename = OperationType::Rename;
        let json = serde_json::to_string(&rename).unwrap();
        assert_eq!(json, "\"rename\"");

        let parsed: OperationType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, OperationType::Rename);
    }

    #[test]
    fn generated_ids_do_not_collide_within_the_same_second() {
        // The old id was `timestamp_secs ^ stack_addr`, so two batches recorded
        // in the same second produced the same id and undo_by_id targeted the
        // wrong one. uuid v4 ids must differ even back-to-back.
        let a = generate_operation_id();
        let b = generate_operation_id();
        assert_ne!(a, b);
        assert!(a.starts_with("op_") && b.starts_with("op_"));
    }

    #[test]
    fn paths_are_same_file_matches_identity_not_names() {
        let dir = tempdir().unwrap();
        let a = create_test_file(dir.path(), "a.txt");
        let b = create_test_file(dir.path(), "b.txt");

        // Same path twice → trivially the same file.
        assert!(paths_are_same_file(Path::new(&a), Path::new(&a)));
        // Two distinct files in the same directory → not the same.
        assert!(!paths_are_same_file(Path::new(&a), Path::new(&b)));
        // A hard link is the same file under a different name — only an
        // identity check can know that; any name-based guess says "different".
        let link = dir.path().join("a_link.txt");
        std::fs::hard_link(&a, &link).unwrap();
        assert!(paths_are_same_file(Path::new(&a), &link));
        // Nonexistent path → conservatively "not the same file" (the guard
        // then rejects, never silently proceeds).
        assert!(!paths_are_same_file(
            Path::new(&a),
            &dir.path().join("missing.txt")
        ));
    }

    // POSIX rename() over an existing directory entry of the *same* file is a
    // documented no-op success; on Windows MoveFileEx errors instead, so this
    // behavioral check is Unix-only (the helper itself is tested above on all
    // platforms).
    #[cfg(unix)]
    #[test]
    fn undo_allows_target_occupied_by_the_same_file() {
        // If the occupant of the original path is the renamed file ITSELF
        // (here via a hard link; on case-insensitive filesystems via a case
        // variant), the undo must proceed rather than report "target already
        // exists" — that guard exists to protect a *different* file's data.
        let dir = tempdir().unwrap();
        let renamed = create_test_file(dir.path(), "renamed.txt");
        let original = dir.path().join("orig.txt");
        std::fs::hard_link(&renamed, &original).unwrap();

        let mut manager = UndoManager::new(10);
        manager.record_batch(
            "Rename".to_string(),
            vec![FileOperation {
                operation_type: OperationType::Rename,
                original_path: original.to_string_lossy().to_string(),
                new_path: Some(renamed.clone()),
                timestamp: current_timestamp(),
            }],
        );

        let result = manager.undo_last().unwrap();
        assert!(
            result.success,
            "the file itself must not count as a conflicting occupant: {:?}",
            result.errors
        );
    }

    #[test]
    fn undo_reports_reverted_pairs_for_success_and_omits_failures() {
        let dir = tempdir().unwrap();

        // A real rename we can undo successfully.
        let ok_original = create_test_file(dir.path(), "ok_orig.txt");
        let ok_new = dir.path().join("ok_new.txt");
        fs::rename(&ok_original, &ok_new).unwrap();
        let ok_new_str = ok_new.to_string_lossy().to_string();

        let mut manager = UndoManager::new(10);
        manager.record_batch(
            "Rename".to_string(),
            vec![FileOperation {
                operation_type: OperationType::Rename,
                original_path: ok_original.clone(),
                new_path: Some(ok_new_str.clone()),
                timestamp: current_timestamp(),
            }],
        );

        let result = manager.undo_last().unwrap();
        assert!(result.success);
        // The successfully reverted pair is reported so the command layer can
        // carry its tags back to the restored path.
        assert_eq!(result.reverted_pairs, vec![(ok_original, ok_new_str)]);

        // A rename whose source (new_path) no longer exists fails to undo and
        // must NOT appear in reverted_pairs — otherwise the command layer would
        // migrate tags off a file that never moved (the #7 bug).
        let mut manager2 = UndoManager::new(10);
        manager2.record_batch(
            "Rename".to_string(),
            vec![FileOperation {
                operation_type: OperationType::Rename,
                original_path: dir.path().join("gone_orig.txt").to_string_lossy().to_string(),
                new_path: Some(dir.path().join("gone_new.txt").to_string_lossy().to_string()),
                timestamp: current_timestamp(),
            }],
        );
        let result2 = manager2.undo_last().unwrap();
        assert!(!result2.success);
        assert!(result2.reverted_pairs.is_empty());
    }
}
