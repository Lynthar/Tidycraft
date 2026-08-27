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
        } else if status.is_index_new() {
            // Staged addition; a worktree-only new file is Untracked below.
            GitFileStatus::New
        } else if status.is_wt_new() {
            GitFileStatus::Untracked
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
    /// True while `status_cache` holds an unconsumed full-status pass.
    /// `get_info` fills it as a side effect; the `get_all_statuses` that follows
    /// consumes it. One-shot — consuming resets the flag.
    statuses_fresh: bool,
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
            statuses_fresh: false,
        }
    }

    /// Check if this is a git repository
    #[allow(dead_code)]
    pub fn is_repo(&self) -> bool {
        self.repo.is_some()
    }

    /// Get repository info. Runs one full status pass (for `has_changes`) and
    /// leaves it in `status_cache` for the `get_all_statuses` that follows.
    pub fn get_info(&mut self) -> GitInfo {
        if self.repo.is_none() {
            return GitInfo {
                is_repo: false,
                branch: None,
                has_changes: false,
                ahead: 0,
                behind: 0,
            };
        };

        self.load_statuses();
        // Includes untracked files, matching `git status` and the per-file
        // badges this map feeds.
        let has_changes = !self.status_cache.is_empty();

        let repo = self.repo.as_ref().unwrap();

        let branch = repo
            .head()
            .ok()
            .and_then(|head| head.shorthand().map(String::from));

        // Get ahead/behind counts
        let (ahead, behind) = Self::get_ahead_behind(repo);

        GitInfo {
            is_repo: true,
            branch,
            has_changes,
            ahead,
            behind,
        }
    }

    fn get_ahead_behind(repo: &Repository) -> (u32, u32) {
        let head = match repo.head() {
            Ok(h) => h,
            Err(_) => return (0, 0),
        };

        let local_oid = match head.target() {
            Some(oid) => oid,
            None => return (0, 0),
        };

        let branch_name = match head.shorthand() {
            Some(name) => name,
            None => return (0, 0),
        };

        // Resolve the configured upstream (branch.<name>.remote + .merge)
        // rather than assuming `origin/<branch>`.
        let local_branch = match repo.find_branch(branch_name, git2::BranchType::Local) {
            Ok(b) => b,
            Err(_) => return (0, 0),
        };
        let upstream = match local_branch.upstream() {
            Ok(u) => u,
            Err(_) => return (0, 0), // no upstream configured
        };
        let upstream_oid = match upstream.get().target() {
            Some(oid) => oid,
            None => return (0, 0),
        };

        repo.graph_ahead_behind(local_oid, upstream_oid)
            .map(|(a, b)| (a as u32, b as u32))
            .unwrap_or((0, 0))
    }

    /// Run the full status query into `status_cache` and mark it fresh.
    fn load_statuses(&mut self) {
        self.status_cache.clear();
        self.statuses_fresh = true;

        let Some(repo) = &self.repo else {
            return;
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
    }

    /// Get all file statuses. Fresh from the caller's perspective: either the
    /// pass `get_info` ran in the same refresh (consumed exactly once), or a
    /// re-query. A `GitManager` lives for one refresh.
    pub fn get_all_statuses(&mut self) -> &HashMap<PathBuf, GitFileStatus> {
        // Consume the pass `get_info` just ran; re-query only when standalone.
        if !self.statuses_fresh {
            self.load_statuses();
        }
        self.statuses_fresh = false;
        &self.status_cache
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
