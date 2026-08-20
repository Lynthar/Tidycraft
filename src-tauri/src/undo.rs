//! Undo stack for batch file operations. History is persisted to
//! `{data_dir}/tidycraft/undo/{sha256(root)[..16]}.json` after every record,
//! undo and clear, and read back when the project is registered.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One recorded file operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOperation {
    pub operation_type: OperationType,
    pub original_path: String,
    /// Destination after a rename or move.
    pub new_path: Option<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationType {
    Rename,
    Move,
    /// Not undoable — there is no backup mechanism.
    Delete,
}

/// One recorded batch of file operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOperation {
    pub id: String,
    pub description: String,
    pub operations: Vec<FileOperation>,
    pub timestamp: u64,
    pub undone: bool,
}

/// Outcome of one undo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoResult {
    pub success: bool,
    pub reverted_count: usize,
    pub failed_count: usize,
    pub errors: Vec<String>,
    pub operation_description: String,
    /// The `(original, new)` pairs that were actually restored. The command
    /// layer migrates tag bindings back along these, so a partially failed undo
    /// leaves the still-moved files' tags where they are. Not sent to the frontend.
    #[serde(skip)]
    pub reverted_pairs: Vec<(String, String)>,
}

/// History summary for the interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub description: String,
    pub file_count: usize,
    pub timestamp: u64,
    /// True only for the most recent batch that has not been undone.
    pub can_undo: bool,
}

/// Bounded, disk-persisted undo history for one project.
pub struct UndoManager {
    history: Vec<BatchOperation>,
    max_history: usize,
    /// Disk persistence target. `None` means in-memory only.
    persist_path: Option<PathBuf>,
}

impl UndoManager {
    /// In-memory only, for tests; production goes through `load_for_project`.
    pub const fn new(max_history: usize) -> Self {
        Self {
            history: Vec::new(),
            max_history,
            persist_path: None,
        }
    }

    /// Build a manager for a project, reading its history back from disk and
    /// trimming to `max_history`.
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

    /// Read history from `path`, keeping the newest `max_history` entries. A
    /// file that exists but will not parse is moved aside to `.corrupt` rather
    /// than silently treated as empty.
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
                    // Keep the earliest backup; a later corruption must not replace it.
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

    /// Write the history to `path` atomically, creating the parent directory.
    /// Errors propagate; `save_to_disk` logs them.
    fn write_history_to(path: &Path, history: &[BatchOperation]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(history)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // Atomic (temp + rename): a crash mid-write must not tear the history.
        crate::fs_atomic::write_atomic(path, json.as_bytes())
    }

    /// File name is the SHA256 (first 16 hex) of the project root: stable across
    /// sessions and free of path characters.
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

    /// Best-effort: a failed write never blocks the undo itself, but it must be
    /// logged — silence looks like "the history is empty again after restart".
    fn save_to_disk(&self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        if let Err(e) = Self::write_history_to(path, &self.history) {
            eprintln!(
                "[undo] failed to persist history to {}: {e}",
                path.display()
            );
        }
    }

    /// Record one batch of operations.
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

        // Drop the oldest beyond the limit.
        while self.history.len() > self.max_history {
            self.history.remove(0);
        }

        self.save_to_disk();
        id
    }

    /// Undo the most recent batch that has not been undone. Sidecar carry
    /// failures go out through `sidecar_failures` rather than into `UndoResult`:
    /// the undo itself succeeded — the asset is back where it belongs — so they
    /// must not count as failed items, but the command layer has to see them.
    pub fn undo_last(
        &mut self,
        sidecar_failures: &mut crate::warning::SampledFailures,
    ) -> Option<UndoResult> {
        let index = self.history.iter().rposition(|op| !op.undone)?;

        let batch = &self.history[index];
        let description = batch.description.clone();

        let had_operations = !batch.operations.is_empty();
        let result = execute_batch_undo(&batch.operations, sidecar_failures);

        // A batch where nothing reverted stays retryable: the cause is usually
        // transient. Partial success still consumes it — a re-run would
        // re-attempt files that already moved back. Empty batches are consumed.
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

    /// History, newest first.
    pub fn get_history(&self) -> Vec<HistoryEntry> {
        let last_undoable_index = self.history.iter().rposition(|op| !op.undone);

        self.history
            .iter()
            .enumerate()
            .map(|(i, op)| HistoryEntry {
                id: op.id.clone(),
                description: op.description.clone(),
                file_count: op.operations.len(),
                timestamp: op.timestamp,
                can_undo: last_undoable_index == Some(i) && !op.undone,
            })
            .rev()
            .collect()
    }

    /// Whether any batch can still be undone.
    pub fn can_undo(&self) -> bool {
        self.history.iter().any(|op| !op.undone)
    }

    /// Drop the whole history.
    pub fn clear_history(&mut self) {
        self.history.clear();
        self.save_to_disk();
    }

    /// Description of the most recent batch that has not been undone.
    #[allow(dead_code)]
    pub fn get_last_operation_description(&self) -> Option<String> {
        self.history
            .iter()
            .rfind(|op| !op.undone)
            .map(|op| op.description.clone())
    }

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

/// Whether two paths name the **same file** by filesystem identity (Unix
/// dev+inode, Windows volume serial + file index). False when either path is
/// missing or unreadable, so callers reject conservatively.
pub(crate) fn paths_are_same_file(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

/// Revert a batch, most recent operation first.
fn execute_batch_undo(
    operations: &[FileOperation],
    sidecar_failures: &mut crate::warning::SampledFailures,
) -> UndoResult {
    let mut reverted_count = 0;
    let mut failed_count = 0;
    let mut errors = Vec::new();
    // Only genuinely restored pairs, so the command layer never migrates tags
    // off a file that stayed put.
    let mut reverted_pairs: Vec<(String, String)> = Vec::new();

    for op in operations.iter().rev() {
        match execute_single_undo(op, sidecar_failures) {
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

/// Revert one file operation.
fn execute_single_undo(
    operation: &FileOperation,
    sidecar_failures: &mut crate::warning::SampledFailures,
) -> Result<(), String> {
    match operation.operation_type {
        OperationType::Rename => {
            let new_path = operation
                .new_path
                .as_ref()
                .ok_or("Missing new path for rename operation")?;

            let src = Path::new(new_path);
            let dst = Path::new(&operation.original_path);

            if !src.exists() {
                return Err(format!(
                    "Source file not found: {} (file may have been modified)",
                    new_path
                ));
            }

            // `dst.exists()` may resolve to `src` itself (case-only undo, NFC/NFD
            // variant); only a genuinely different file blocks the undo.
            if dst.exists() && !paths_are_same_file(src, dst) {
                return Err(format!(
                    "Target path already exists: {}",
                    operation.original_path
                ));
            }

            // Sidecars are pre-flighted before the primary file moves, so a blocked
            // destination refuses the whole item instead of splitting asset from identity.
            let sidecar_conflicts = crate::sidecar::rename_conflicts(src, dst);
            if !sidecar_conflicts.is_empty() {
                return Err(sidecar_conflicts.join("; "));
            }

            fs::rename(src, dst).map_err(|e| {
                format!(
                    "Failed to rename '{}' back to '{}': {}",
                    new_path, operation.original_path, e
                )
            })?;
            // Carry the engine sidecars back too; best-effort, logged on failure.
            if let Err(e) = crate::sidecar::carry_on_rename(src, dst) {
                eprintln!(
                    "[undo] engine sidecar not carried back for {}: {}",
                    new_path, e
                );
                sidecar_failures.record(Some(&operation.original_path), &e);
            }
            Ok(())
        }
        OperationType::Move => {
            let new_path = operation
                .new_path
                .as_ref()
                .ok_or("Missing new path for move operation")?;

            let src = Path::new(new_path);
            let dst = Path::new(&operation.original_path);

            if !src.exists() {
                return Err(format!("Source file not found: {}", new_path));
            }

            // Same identity check as the Rename branch above.
            if dst.exists() && !paths_are_same_file(src, dst) {
                return Err(format!(
                    "Target path already exists: {}",
                    operation.original_path
                ));
            }

            // Sidecar pre-flight, as in the Rename branch.
            let sidecar_conflicts = crate::sidecar::rename_conflicts(src, dst);
            if !sidecar_conflicts.is_empty() {
                return Err(sidecar_conflicts.join("; "));
            }

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
            // Carry the engine sidecars back, as in the Rename branch.
            if let Err(e) = crate::sidecar::carry_on_rename(src, dst) {
                eprintln!(
                    "[undo] engine sidecar not carried back for {}: {}",
                    new_path, e
                );
                sidecar_failures.record(Some(&operation.original_path), &e);
            }
            Ok(())
        }
        OperationType::Delete => Err("Undo for delete operations is not yet supported".to_string()),
    }
}

/// Unique operation id (uuid v4).
fn generate_operation_id() -> String {
    format!("op_{}", uuid::Uuid::new_v4().simple())
}

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

    // ---- History persistence ----

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

    /// A history file that exists but will not parse must not degrade to "no
    /// history": the next `record_batch` would save over it.
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

    /// The first backup is the likeliest to be complete, so a later corruption
    /// must not overwrite it.
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

    /// A failed write must reach a log. The manager keeps working in memory,
    /// but a persistently unwritable history loses every undo across restarts.
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

    /// A batch where nothing could be reverted must stay retryable — the usual
    /// causes are transient (files open in another app, drive unavailable).
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

        let result = manager
            .undo_last(&mut Default::default())
            .expect("a batch was recorded");
        assert_eq!(result.reverted_count, 0);
        assert_eq!(result.failed_count, 2);
        assert!(!result.success);

        assert!(
            manager.can_undo(),
            "a batch that reverted nothing must remain undoable"
        );
        assert!(
            manager.undo_last(&mut Default::default()).is_some(),
            "retry must find it again"
        );
    }

    /// Partial success still consumes the entry: a re-run would re-attempt the
    /// files that already moved back. Only a total failure is retryable.
    #[test]
    fn a_partial_undo_still_consumes_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        let moved = create_test_file(dir.path(), "renamed.png");
        let original = dir
            .path()
            .join("original.png")
            .to_string_lossy()
            .to_string();

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

        let result = manager
            .undo_last(&mut Default::default())
            .expect("a batch was recorded");
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

        // The newest three survive.
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

        let original_path = create_test_file(dir.path(), "original.txt");
        let new_path = dir.path().join("renamed.txt");

        // Simulate the forward rename.
        fs::rename(&original_path, &new_path).unwrap();

        let mut manager = UndoManager::new(10);

        let ops = vec![FileOperation {
            operation_type: OperationType::Rename,
            original_path: original_path.clone(),
            new_path: Some(new_path.to_string_lossy().to_string()),
            timestamp: current_timestamp(),
        }];

        manager.record_batch("Rename file".to_string(), ops);

        let result = manager.undo_last(&mut Default::default()).unwrap();

        assert!(result.success);
        assert_eq!(result.reverted_count, 1);
        assert_eq!(result.failed_count, 0);
        assert!(result.errors.is_empty());

        assert!(Path::new(&original_path).exists());
        assert!(!new_path.exists());
    }

    #[test]
    fn test_undo_rename_carries_engine_sidecars() {
        // Undoing a rename must move the engine sidecar back too, or the revert
        // strands it on the new name. Run per suffix so Godot's two are covered.
        for suffix in [".meta", ".import", ".uid"] {
            let dir = tempdir().unwrap();
            let original = dir.path().join("a.txt");
            let renamed = dir.path().join("b.txt");
            let side_of = |p: &Path| crate::sidecar::sidecar_path(p, suffix);

            fs::write(&original, "asset").unwrap();
            fs::write(side_of(&original), "identity").unwrap();
            // Simulate the forward rename having already carried the sidecar.
            fs::rename(&original, &renamed).unwrap();
            fs::rename(side_of(&original), side_of(&renamed)).unwrap();

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

            let result = manager.undo_last(&mut Default::default()).unwrap();
            assert!(result.success, "{suffix}");
            // Both the asset and its sidecar are back at the original name.
            assert!(original.exists(), "{suffix}");
            assert!(side_of(&original).exists(), "{suffix} not carried back");
            assert!(!renamed.exists(), "{suffix}");
            assert!(!side_of(&renamed).exists(), "{suffix} left behind");
        }
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

        manager.history[0].undone = true;

        assert!(manager.undo_last(&mut Default::default()).is_none());
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
        // Nonexistent path → conservatively "not the same file".
        assert!(!paths_are_same_file(
            Path::new(&a),
            &dir.path().join("missing.txt")
        ));
    }

    // POSIX rename() over an existing entry of the *same* file is a documented
    // no-op success; Windows MoveFileEx errors instead, so this check is Unix-only.
    #[cfg(unix)]
    #[test]
    fn undo_allows_target_occupied_by_the_same_file() {
        // The occupant of the original path is the renamed file itself (a hard
        // link here), so the undo must proceed rather than report a conflict.
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

        let result = manager.undo_last(&mut Default::default()).unwrap();
        assert!(
            result.success,
            "the file itself must not count as a conflicting occupant: {:?}",
            result.errors
        );
    }

    // Unix-only for the same POSIX-rename reason as the twin above.
    #[cfg(unix)]
    #[test]
    fn undo_move_allows_target_occupied_by_the_same_file() {
        // Undoing a move whose original path resolves to the moved file itself
        // must succeed; same identity check as the Rename branch.
        let dir = tempdir().unwrap();
        let moved = create_test_file(dir.path(), "moved.txt");
        let original = dir.path().join("orig.txt");
        std::fs::hard_link(&moved, &original).unwrap();

        let mut manager = UndoManager::new(10);
        manager.record_batch(
            "Move".to_string(),
            vec![FileOperation {
                operation_type: OperationType::Move,
                original_path: original.to_string_lossy().to_string(),
                new_path: Some(moved.clone()),
                timestamp: current_timestamp(),
            }],
        );

        let result = manager.undo_last(&mut Default::default()).unwrap();
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

        let result = manager.undo_last(&mut Default::default()).unwrap();
        assert!(result.success);
        // The successfully reverted pair is reported so the command layer can
        // carry its tags back to the restored path.
        assert_eq!(result.reverted_pairs, vec![(ok_original, ok_new_str)]);

        // A rename whose source no longer exists fails to undo and must not
        // appear in reverted_pairs, or the command layer would migrate tags off
        // a file that never moved.
        let mut manager2 = UndoManager::new(10);
        manager2.record_batch(
            "Rename".to_string(),
            vec![FileOperation {
                operation_type: OperationType::Rename,
                original_path: dir
                    .path()
                    .join("gone_orig.txt")
                    .to_string_lossy()
                    .to_string(),
                new_path: Some(
                    dir.path()
                        .join("gone_new.txt")
                        .to_string_lossy()
                        .to_string(),
                ),
                timestamp: current_timestamp(),
            }],
        );
        let result2 = manager2.undo_last(&mut Default::default()).unwrap();
        assert!(!result2.success);
        assert!(result2.reverted_pairs.is_empty());
    }
}
