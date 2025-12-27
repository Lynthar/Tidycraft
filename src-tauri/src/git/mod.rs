use git2::{Repository, Status, StatusOptions};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitFileStatus {
    New,
    Modified,
    Deleted,
    Renamed,
    Typechange,
    Untracked,
    Ignored,
    Conflicted,
    Unchanged,
}

impl From<Status> for GitFileStatus {
    fn from(status: Status) -> Self {
        if status.is_conflicted() {
            GitFileStatus::Conflicted
        } else if status.is_index_new() || status.is_wt_new() {
            GitFileStatus::New
        } else if status.is_index_modified() || status.is_wt_modified() {
            GitFileStatus::Modified
        } else if status.is_index_deleted() || status.is_wt_deleted() {
            GitFileStatus::Deleted
        } else if status.is_index_renamed() || status.is_wt_renamed() {
            GitFileStatus::Renamed
        } else if status.is_index_typechange() || status.is_wt_typechange() {
            GitFileStatus::Typechange
        } else if status.is_ignored() {
            GitFileStatus::Ignored
        } else if status.contains(Status::WT_NEW) {
            GitFileStatus::Untracked
        } else {
            GitFileStatus::Unchanged
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GitInfo {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub has_changes: bool,
    pub ahead: u32,
    pub behind: u32,
}

pub struct GitManager {
    repo: Option<Repository>,
    root_path: PathBuf,
    status_cache: HashMap<PathBuf, GitFileStatus>,
}

impl GitManager {
    /// Try to open a git repository at the given path
    pub fn open(path: &Path) -> Self {
        let repo = Repository::discover(path).ok();
        let root_path = repo
            .as_ref()
            .and_then(|r| r.workdir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| path.to_path_buf());

        GitManager {
            repo,
            root_path,
            status_cache: HashMap::new(),
        }
    }

    /// Check if this is a git repository
    pub fn is_repo(&self) -> bool {
        self.repo.is_some()
    }

    /// Get repository info
    pub fn get_info(&self) -> GitInfo {
        let Some(repo) = &self.repo else {
            return GitInfo {
                is_repo: false,
                branch: None,
                has_changes: false,
                ahead: 0,
                behind: 0,
            };
        };

        let branch = repo
            .head()
            .ok()
            .and_then(|head| head.shorthand().map(String::from));

        let has_changes = repo
            .statuses(None)
            .map(|statuses| !statuses.is_empty())
            .unwrap_or(false);

        // Get ahead/behind counts
        let (ahead, behind) = self.get_ahead_behind(repo);

        GitInfo {
            is_repo: true,
            branch,
            has_changes,
            ahead,
            behind,
        }
    }

    fn get_ahead_behind(&self, repo: &Repository) -> (u32, u32) {
        let head = match repo.head() {
            Ok(h) => h,
            Err(_) => return (0, 0),
        };

        let local_oid = match head.target() {
            Some(oid) => oid,
            None => return (0, 0),
        };

        // Try to find upstream branch
        let branch_name = match head.shorthand() {
            Some(name) => name,
            None => return (0, 0),
        };

        let upstream_name = format!("origin/{}", branch_name);
        let upstream_ref = match repo.find_reference(&format!("refs/remotes/{}", upstream_name)) {
            Ok(r) => r,
            Err(_) => return (0, 0),
        };

        let upstream_oid = match upstream_ref.target() {
            Some(oid) => oid,
            None => return (0, 0),
        };

        repo.graph_ahead_behind(local_oid, upstream_oid)
            .map(|(a, b)| (a as u32, b as u32))
            .unwrap_or((0, 0))
    }

    /// Get all file statuses
    pub fn get_all_statuses(&mut self) -> &HashMap<PathBuf, GitFileStatus> {
        if !self.status_cache.is_empty() {
            return &self.status_cache;
        }

        let Some(repo) = &self.repo else {
            return &self.status_cache;
        };

        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .include_ignored(false)
            .recurse_untracked_dirs(true);

        if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
            for entry in statuses.iter() {
                if let Some(path) = entry.path() {
                    let full_path = self.root_path.join(path);
                    let status = GitFileStatus::from(entry.status());
                    self.status_cache.insert(full_path, status);
                }
            }
        }

        &self.status_cache
    }

    /// Get status for a specific file
    pub fn get_file_status(&self, path: &Path) -> GitFileStatus {
        let Some(repo) = &self.repo else {
            return GitFileStatus::Unchanged;
        };

        // Get relative path from repo root
        let relative_path = match path.strip_prefix(&self.root_path) {
            Ok(p) => p,
            Err(_) => return GitFileStatus::Unchanged,
        };

        match repo.status_file(relative_path) {
            Ok(status) => GitFileStatus::from(status),
            Err(_) => GitFileStatus::Unchanged,
        }
    }

    /// Check if a path should be ignored according to .gitignore
    pub fn is_ignored(&self, path: &Path) -> bool {
        let Some(repo) = &self.repo else {
            return false;
        };

        let relative_path = match path.strip_prefix(&self.root_path) {
            Ok(p) => p,
            Err(_) => return false,
        };

        repo.is_path_ignored(relative_path).unwrap_or(false)
    }

    /// Clear the status cache
    pub fn clear_cache(&mut self) {
        self.status_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_git_directory() {
        let manager = GitManager::open(Path::new("/tmp"));
        assert!(!manager.is_repo());
    }
}
