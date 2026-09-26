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
    use git2::{BranchType, IndexAddOption, ResetType, Signature};
    use std::fs;

    #[test]
    fn test_non_git_directory() {
        let manager = GitManager::open(Path::new("/tmp"));
        assert!(!manager.is_repo());
    }

    /// Stage everything in the work tree and commit it; returns the new HEAD.
    fn commit_all(repo: &Repository, message: &str) -> git2::Oid {
        let mut index = repo.index().expect("index");
        index
            .add_all(["*"], IndexAddOption::DEFAULT, None)
            .expect("stage");
        index.write().expect("write index");
        let tree = repo
            .find_tree(index.write_tree().expect("tree"))
            .expect("find tree");
        let sig = Signature::now("tidycraft tests", "tests@tidycraft.invalid").expect("signature");
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .expect("commit")
    }

    /// The status of the entry named `name`, or `None` when git lists nothing for it.
    fn status_of(manager: &mut GitManager, name: &str) -> Option<GitFileStatus> {
        manager
            .get_all_statuses()
            .iter()
            .find(|(path, _)| path.file_name().is_some_and(|f| f == name))
            .map(|(_, status)| status.clone())
    }

    #[test]
    fn an_untracked_file_counts_as_a_change_and_reports_as_untracked() {
        let dir = tempfile::tempdir().expect("tempdir");
        Repository::init(dir.path()).expect("init");
        fs::write(dir.path().join("new.png"), b"x").expect("write");

        let mut manager = GitManager::open(dir.path());
        assert!(manager.is_repo());
        let info = manager.get_info();
        assert!(info.is_repo);
        assert!(
            info.has_changes,
            "git status lists an untracked file, so must this"
        );
        assert_eq!(
            status_of(&mut manager, "new.png"),
            Some(GitFileStatus::Untracked)
        );
    }

    #[test]
    fn staged_modified_and_deleted_files_map_to_their_own_statuses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init");
        for name in ["kept.txt", "edited.txt", "removed.txt"] {
            fs::write(dir.path().join(name), "a").expect("write");
        }
        commit_all(&repo, "base");

        fs::write(dir.path().join("edited.txt"), "b").expect("edit");
        fs::remove_file(dir.path().join("removed.txt")).expect("remove");
        fs::write(dir.path().join("staged.txt"), "a").expect("write");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("staged.txt")).expect("stage");
        index.write().expect("write index");

        let mut manager = GitManager::open(dir.path());
        assert_eq!(
            status_of(&mut manager, "staged.txt"),
            Some(GitFileStatus::New)
        );
        assert_eq!(
            status_of(&mut manager, "edited.txt"),
            Some(GitFileStatus::Modified)
        );
        assert_eq!(
            status_of(&mut manager, "removed.txt"),
            Some(GitFileStatus::Deleted)
        );
        assert_eq!(
            status_of(&mut manager, "kept.txt"),
            None,
            "an unchanged file has no entry"
        );
    }

    #[test]
    fn ahead_and_behind_follow_the_configured_upstream() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init");
        fs::write(dir.path().join("a.txt"), "a").expect("write");
        let first = commit_all(&repo, "first");
        fs::write(dir.path().join("b.txt"), "b").expect("write");
        let second = commit_all(&repo, "second");
        let head = repo
            .head()
            .expect("head")
            .shorthand()
            .expect("branch name")
            .to_string();

        // A local branch as upstream: `branch.<head>.remote` is `.`, not `origin`.
        repo.branch("base", &repo.find_commit(first).expect("first"), false)
            .expect("branch");
        repo.find_branch(&head, BranchType::Local)
            .expect("head branch")
            .set_upstream(Some("base"))
            .expect("set upstream");
        let info = GitManager::open(dir.path()).get_info();
        assert_eq!((info.ahead, info.behind), (1, 0));

        repo.branch("tip", &repo.find_commit(second).expect("second"), false)
            .expect("branch");
        repo.find_branch(&head, BranchType::Local)
            .expect("head branch")
            .set_upstream(Some("tip"))
            .expect("set upstream");
        repo.reset(
            repo.find_commit(first).expect("first").as_object(),
            ResetType::Hard,
            None,
        )
        .expect("reset");
        let info = GitManager::open(dir.path()).get_info();
        assert_eq!((info.ahead, info.behind), (0, 1));
    }

    #[test]
    fn get_all_statuses_consumes_the_pass_get_info_ran_then_requeries() {
        let dir = tempfile::tempdir().expect("tempdir");
        Repository::init(dir.path()).expect("init");
        fs::write(dir.path().join("first.txt"), "a").expect("write");
        let mut manager = GitManager::open(dir.path());
        assert!(manager.get_info().has_changes);

        // Appeared after get_info's pass: the read that follows reuses that pass.
        fs::write(dir.path().join("second.txt"), "b").expect("write");
        assert_eq!(status_of(&mut manager, "second.txt"), None);
        // Consumed once; a standalone read re-queries and sees it.
        assert_eq!(
            status_of(&mut manager, "second.txt"),
            Some(GitFileStatus::Untracked)
        );
    }
}
