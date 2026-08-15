//! Warnings that make "could not read it" distinct from "nothing wrong".
//!
//! Same shape as `analyzer::rule_suggest::RuleWarning`, for the same reason:
//! these failures used to reach `eprintln!` and nowhere else, and a
//! Finder-launched .app has no stderr attached to anything. They ride to the
//! frontend as serde-tagged values whose `kind` strings a test pins, because
//! `asset.ts` mirrors them by hand.

use serde::{Deserialize, Serialize};
use tauri::Emitter;

/// Cap on example paths carried per warning: enough to recognize the pattern
/// ("it's all under Vendor/"), few enough that a project with a whole
/// unreadable subtree ships a count, not fifty thousand strings.
pub const SAMPLE_CAP: usize = 5;

/// A non-fatal problem in ONE scan. Rides on `ScanResult.warnings`, so the
/// command return, the cached scan and the JSON export carry the admission
/// automatically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScanWarning {
    /// The directory walk failed to enter or list something (permissions,
    /// transient IO). Whatever lives there is missing from the results.
    TreeWalkFailed {
        skipped: usize,
        sample: Vec<String>,
        detail: String,
    },
    /// Files that were discovered but whose metadata could not be read. The
    /// incremental scan drops them; the full scan keeps them with zeroed
    /// size/mtime. `affected`, not `skipped`: the records may still be
    /// present — they are just not to be trusted.
    AssetUnreadable {
        affected: usize,
        sample: Vec<String>,
        detail: String,
    },
    /// The gitignore matcher failed to build; the directory tree stops
    /// filtering ignored entries.
    IgnoreRulesUnusable { detail: String },
    /// The scan cache could not be written; the next scan is a full one.
    CacheNotSaved { detail: String },
}

/// A non-fatal problem of the RUNNING SESSION, pushed over
/// `project-warning-{project_id}`. Kept out of `ScanWarning` on purpose:
/// these belong to no scan, so they must not ride into a cached scan or an
/// exported report. The frontend adds a third member to its mirror of this
/// enum (`watcher_start_failed`) that is born on its side of the IPC — the
/// failed invoke itself is the signal, so the backend never sends it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectWarning {
    /// notify handed the watcher an error batch; those events are gone and
    /// the cached scan may no longer match the disk. `batches` is cumulative
    /// since the watcher started — the frontend replaces, never adds.
    WatcherEventsDropped { batches: usize, detail: String },
    /// Tag bindings were migrated in memory but could not be written to
    /// disk. Surfaced as a toast, not in the warning list — it is tied to
    /// the one operation the user just performed.
    TagsNotSaved { detail: String },
}

/// Emit `w` on this project's warning channel. Failures to emit are ignored:
/// the warning channel must never fail the operation it reports on.
pub fn emit_project_warning(app: &tauri::AppHandle, project_id: &str, w: &ProjectWarning) {
    let _ = app.emit(&format!("project-warning-{project_id}"), w);
}

/// Accumulates one class of failure during a scan: full count, first
/// [`SAMPLE_CAP`] paths, first error text. The parallel parse stage wraps one
/// in a `Mutex` — it locks only when something fails, so the happy path pays
/// nothing.
#[derive(Debug, Default)]
pub struct SampledFailures {
    pub count: usize,
    pub sample: Vec<String>,
    pub detail: Option<String>,
}

impl SampledFailures {
    pub fn record(&mut self, path: Option<&str>, detail: &str) {
        self.count += 1;
        if let Some(p) = path {
            if self.sample.len() < SAMPLE_CAP {
                self.sample.push(p.to_string());
            }
        }
        if self.detail.is_none() {
            self.detail = Some(detail.to_string());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `asset.ts` mirrors both enums by hand and branches on the exact `kind`
    /// strings — no codegen between the two sides. Same discipline (and same
    /// test shape) as
    /// `rule_suggest::warning_wire_shape_matches_the_frontends_mirror`.
    #[test]
    fn warning_wire_shape_matches_the_frontends_mirror() {
        let walk = serde_json::to_value(ScanWarning::TreeWalkFailed {
            skipped: 3,
            sample: vec!["a/b".into()],
            detail: "permission denied".into(),
        })
        .unwrap();
        assert_eq!(walk["kind"], "tree_walk_failed");
        assert_eq!(walk["skipped"], 3);
        assert_eq!(walk["sample"][0], "a/b");
        assert_eq!(walk["detail"], "permission denied");

        let unreadable = serde_json::to_value(ScanWarning::AssetUnreadable {
            affected: 2,
            sample: vec![],
            detail: "gone".into(),
        })
        .unwrap();
        assert_eq!(unreadable["kind"], "asset_unreadable");
        assert_eq!(unreadable["affected"], 2);

        let ignore = serde_json::to_value(ScanWarning::IgnoreRulesUnusable {
            detail: "bad pattern".into(),
        })
        .unwrap();
        assert_eq!(ignore["kind"], "ignore_rules_unusable");

        let cache = serde_json::to_value(ScanWarning::CacheNotSaved {
            detail: "read-only".into(),
        })
        .unwrap();
        assert_eq!(cache["kind"], "cache_not_saved");

        let dropped = serde_json::to_value(ProjectWarning::WatcherEventsDropped {
            batches: 4,
            detail: "queue overflow".into(),
        })
        .unwrap();
        assert_eq!(dropped["kind"], "watcher_events_dropped");
        assert_eq!(dropped["batches"], 4);

        let tags = serde_json::to_value(ProjectWarning::TagsNotSaved {
            detail: "disk full".into(),
        })
        .unwrap();
        assert_eq!(tags["kind"], "tags_not_saved");
    }

    #[test]
    fn sampled_failures_cap_the_sample_not_the_count() {
        let mut f = SampledFailures::default();
        for i in 0..20 {
            f.record(Some(&format!("p{i}")), "denied");
        }
        assert_eq!(f.count, 20);
        assert_eq!(f.sample.len(), SAMPLE_CAP);
        assert_eq!(f.detail.as_deref(), Some("denied"));
        assert!(!f.is_empty());
        assert!(SampledFailures::default().is_empty());
    }
}
