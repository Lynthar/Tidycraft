//! 撤销管理模块
//!
//! 提供批量文件操作的内存级撤销功能。
//! 历史记录仅在程序运行期间保留，关闭后丢失。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
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
}

impl UndoManager {
    /// 创建新的撤销管理器
    pub const fn new(max_history: usize) -> Self {
        Self {
            history: Vec::new(),
            max_history,
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
        let result = execute_batch_undo(&batch.operations);

        // 标记为已撤销
        self.history[index].undone = true;

        Some(UndoResult {
            success: result.failed_count == 0,
            reverted_count: result.reverted_count,
            failed_count: result.failed_count,
            errors: result.errors,
            operation_description: description,
        })
    }

    /// 撤销指定 ID 的操作
    pub fn undo_by_id(&mut self, id: &str) -> Option<UndoResult> {
        let index = self
            .history
            .iter()
            .position(|op| op.id == id && !op.undone)?;

        let batch = &self.history[index];
        let description = batch.description.clone();

        // 执行撤销
        let result = execute_batch_undo(&batch.operations);

        // 标记为已撤销
        self.history[index].undone = true;

        Some(UndoResult {
            success: result.failed_count == 0,
            reverted_count: result.reverted_count,
            failed_count: result.failed_count,
            errors: result.errors,
            operation_description: description,
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
    }

    /// 获取最近一次操作的描述
    pub fn get_last_operation_description(&self) -> Option<String> {
        self.history
            .iter()
            .filter(|op| !op.undone)
            .last()
            .map(|op| op.description.clone())
    }

    /// 获取历史记录数量
    pub fn history_count(&self) -> usize {
        self.history.len()
    }

    /// 获取可撤销的操作数量
    pub fn undoable_count(&self) -> usize {
        self.history.iter().filter(|op| !op.undone).count()
    }
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new(50)
    }
}

/// 执行批量撤销
fn execute_batch_undo(operations: &[FileOperation]) -> UndoResult {
    let mut reverted_count = 0;
    let mut failed_count = 0;
    let mut errors = Vec::new();

    // 反向遍历操作列表，按相反顺序撤销
    for op in operations.iter().rev() {
        match execute_single_undo(op) {
            Ok(()) => reverted_count += 1,
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

            // 检查目标路径是否已存在
            if dst.exists() {
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
            })
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
            })
        }
        OperationType::Delete => {
            // 删除操作的撤销需要备份机制，目前不支持
            Err("Undo for delete operations is not yet supported".to_string())
        }
    }
}

/// 生成唯一的操作 ID
fn generate_operation_id() -> String {
    let timestamp = current_timestamp();
    let random: u32 = rand_simple();
    format!("op_{:x}_{:08x}", timestamp, random)
}

/// 获取当前时间戳
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 简单的伪随机数生成（不依赖外部库）
fn rand_simple() -> u32 {
    let t = current_timestamp();
    let ptr = &t as *const u64 as usize;
    ((t ^ (ptr as u64)) & 0xFFFFFFFF) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_file(dir: &Path, name: &str) -> String {
        let path = dir.join(name);
        fs::write(&path, "test content").unwrap();
        path.to_string_lossy().to_string()
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
    fn test_undo_by_id() {
        let mut manager = UndoManager::new(10);

        let ops = vec![FileOperation {
            operation_type: OperationType::Rename,
            original_path: "/old.txt".to_string(),
            new_path: Some("/new.txt".to_string()),
            timestamp: current_timestamp(),
        }];

        let id = manager.record_batch("Test".to_string(), ops);

        // 通过 ID 撤销（会失败因为文件不存在，但逻辑测试通过）
        let result = manager.undo_by_id(&id);
        assert!(result.is_some());

        // 验证操作已标记为撤销
        assert!(manager.history[0].undone);
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
    fn test_undoable_count() {
        let mut manager = UndoManager::new(10);

        for i in 0..3 {
            let ops = vec![FileOperation {
                operation_type: OperationType::Rename,
                original_path: format!("/old{}.txt", i),
                new_path: Some(format!("/new{}.txt", i)),
                timestamp: current_timestamp(),
            }];
            manager.record_batch(format!("Op {}", i), ops);
        }

        assert_eq!(manager.undoable_count(), 3);

        // 标记一个为已撤销
        manager.history[0].undone = true;
        assert_eq!(manager.undoable_count(), 2);
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
}
