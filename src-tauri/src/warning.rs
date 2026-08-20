//! Warnings that make "could not read it" distinct from "nothing wrong". They
//! ride to the frontend as serde-tagged values whose `kind` strings a test
//! pins, because `asset.ts` mirrors them by hand.

use serde::{Deserialize, Serialize};
use tauri::Emitter;

/// Cap on example paths carried per warning: enough to recognize a pattern,
/// few enough that an unreadable subtree ships a count rather than the paths.
pub const SAMPLE_CAP: usize = 5;

/// A non-fatal problem in ONE scan. Rides on `ScanResult.warnings`, so the
/// command return, the cached scan and the JSON export all carry it.
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
    /// Files discovered but whose metadata could not be read. Neither scan drops
    /// the file: the full scan keeps it with zeroed size/mtime, and the
    /// incremental one leaves the cache entry from before the change standing, so
    /// what is shown for it is stale rather than absent. Both are what the
    /// `asset_unreadable` copy means by "missing or unreliable".
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

/// A non-fatal problem of the running session, pushed over
/// `project-warning-{project_id}`. Never rides into a cached scan or an
/// exported report. The frontend's mirror adds a `watcher_start_failed` member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectWarning {
    /// notify handed the watcher an error batch; those events are gone and the
    /// cached scan may no longer match the disk. `batches` is cumulative.
    WatcherEventsDropped { batches: usize, detail: String },
    /// Tag bindings were migrated in memory but could not be written to disk.
    /// Surfaced as a toast rather than in the warning list.
    TagsNotSaved { detail: String },
    /// Engine sidecars (`.meta`, `.import`, `.uid`) that did not follow their
    /// asset through a rename, move or undo. The asset moved and the file
    /// carrying its identity did not, so the engine treats it as a new asset on
    /// its next import: Unity mints a fresh GUID and every existing reference to
    /// it breaks. The rename is not rolled back — the asset is where the user
    /// asked for it — so saying so is the only way this becomes visible.
    ///
    /// `sample` holds asset paths, not sidecar paths: the asset is what the user
    /// named. Deletion is deliberately not reported through this — see the
    /// `carry_on_delete` call in `delete_assets`.
    SidecarNotCarried {
        affected: usize,
        sample: Vec<String>,
        detail: String,
    },
}

/// Emit `w` on this project's warning channel. Failures to emit are ignored:
/// the warning channel must never fail the operation it reports on.
pub fn emit_project_warning(app: &tauri::AppHandle, project_id: &str, w: &ProjectWarning) {
    let _ = app.emit(&format!("project-warning-{project_id}"), w);
}

/// Accumulates one class of failure during a scan: full count, first
/// [`SAMPLE_CAP`] paths, first error text.
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
    /// strings — there is no codegen between the two sides.
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

        let sidecar = serde_json::to_value(ProjectWarning::SidecarNotCarried {
            affected: 2,
            sample: vec!["Assets/hero.png".into()],
            detail: "failed to move sidecar".into(),
        })
        .unwrap();
        assert_eq!(sidecar["kind"], "sidecar_not_carried");
        assert_eq!(sidecar["affected"], 2);
        assert_eq!(sidecar["sample"][0], "Assets/hero.png");
        assert_eq!(sidecar["detail"], "failed to move sidecar");
    }

    /// The status bar renders one entry per `kind`, so a `ProjectWarning` sharing
    /// a `kind` with a `ScanWarning` would collide on the React key and show one
    /// where two are meant. Nothing else checks this.
    #[test]
    fn project_and_scan_warning_kinds_do_not_overlap() {
        fn kind(v: serde_json::Value) -> String {
            v["kind"].as_str().expect("tagged enum").to_string()
        }
        let scan_kinds: Vec<String> = vec![
            kind(
                serde_json::to_value(ScanWarning::TreeWalkFailed {
                    skipped: 0,
                    sample: vec![],
                    detail: String::new(),
                })
                .unwrap(),
            ),
            kind(
                serde_json::to_value(ScanWarning::AssetUnreadable {
                    affected: 0,
                    sample: vec![],
                    detail: String::new(),
                })
                .unwrap(),
            ),
            kind(
                serde_json::to_value(ScanWarning::IgnoreRulesUnusable {
                    detail: String::new(),
                })
                .unwrap(),
            ),
            kind(
                serde_json::to_value(ScanWarning::CacheNotSaved {
                    detail: String::new(),
                })
                .unwrap(),
            ),
        ];
        let project_kinds: Vec<String> = vec![
            kind(
                serde_json::to_value(ProjectWarning::WatcherEventsDropped {
                    batches: 0,
                    detail: String::new(),
                })
                .unwrap(),
            ),
            kind(
                serde_json::to_value(ProjectWarning::TagsNotSaved {
                    detail: String::new(),
                })
                .unwrap(),
            ),
            kind(
                serde_json::to_value(ProjectWarning::SidecarNotCarried {
                    affected: 0,
                    sample: vec![],
                    detail: String::new(),
                })
                .unwrap(),
            ),
        ];
        for p in &project_kinds {
            assert!(
                !scan_kinds.contains(p),
                "`{p}` is used by both enums; the status bar keys entries by kind"
            );
        }
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
