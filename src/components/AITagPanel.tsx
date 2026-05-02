import { useEffect, useState } from "react";
import { Sparkles, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useUiStore } from "../stores/uiStore";
import { useProjectStore } from "../stores/projectStore";
import { useTagsStore } from "../stores/tagsStore";
import { useSelectionStore } from "../stores/selectionStore";

interface TagGroup {
  name: string;
  color: string;
  file_paths: string[];
  confidence: number;
  hint: string;
  samples: string[];
}

/// Map the Rust-side hint string to an i18n key. Backend uses fixed English
/// hint values, frontend translates so Chinese users see localized hints.
const HINT_KEYS: Record<string, string> = {
  "filename token": "aiTagPanel.hintFilename",
  dimension: "aiTagPanel.hintDimension",
  "path segment": "aiTagPanel.hintPath",
};

export function AITagPanel() {
  const { t } = useTranslation();
  const aiPanelOpen = useUiStore((s) => s.aiPanelOpen);
  const setAiPanelOpen = useUiStore((s) => s.setAiPanelOpen);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const scanResult = useProjectStore((s) => s.scanResult);
  const setViewMode = useProjectStore((s) => s.setViewMode);
  const createTag = useTagsStore((s) => s.createTag);
  const addTagToAssets = useTagsStore((s) => s.addTagToAssets);
  const setSelectedPaths = useSelectionStore((s) => s.setSelectedPaths);

  const [groups, setGroups] = useState<TagGroup[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /// Per-card pending state — disables buttons while an Apply / Apply-all is
  /// in flight so users can't double-click and create duplicate tags.
  const [pending, setPending] = useState<Set<string>>(new Set());
  const [applyingAll, setApplyingAll] = useState(false);
  const [generatedAt, setGeneratedAt] = useState<Date | null>(null);

  useEffect(() => {
    if (!aiPanelOpen) return;
    if (!activeProjectId || !scanResult) {
      setGroups([]);
      setGeneratedAt(new Date());
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    (async () => {
      try {
        const result = await invoke<TagGroup[]>("suggest_tags", {
          projectId: activeProjectId,
        });
        if (cancelled) return;
        setGroups(result);
        setGeneratedAt(new Date());
      } catch (err) {
        if (cancelled) return;
        setError(String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [aiPanelOpen, activeProjectId, scanResult]);

  // Reset transient state when the panel closes so re-opening is fresh.
  useEffect(() => {
    if (!aiPanelOpen) {
      setGroups(null);
      setError(null);
      setPending(new Set());
      setApplyingAll(false);
      setGeneratedAt(null);
    }
  }, [aiPanelOpen]);

  if (!aiPanelOpen) return null;

  const close = () => setAiPanelOpen(false);

  /// Apply a single group: create a `(suggested)`-suffixed tag, batch-tag all
  /// matching assets, then drop the group from the panel list. Resolves to
  /// `true` on success so callers (Apply all) can sequence.
  const applyGroup = async (group: TagGroup): Promise<boolean> => {
    if (!activeProjectId) return false;
    setPending((prev) => new Set(prev).add(group.name));
    try {
      const tagName = `${group.name} (suggested)`;
      const tag = await createTag(tagName, group.color);
      if (!tag) return false;
      await addTagToAssets(group.file_paths, tag.id);
      setGroups((prev) => (prev ? prev.filter((g) => g.name !== group.name) : prev));
      return true;
    } catch (err) {
      console.error("Failed to apply tag group:", err);
      return false;
    } finally {
      setPending((prev) => {
        const next = new Set(prev);
        next.delete(group.name);
        return next;
      });
    }
  };

  const handleApplyAll = async () => {
    if (!groups || groups.length === 0) return;
    setApplyingAll(true);
    // Snapshot the list — applyGroup mutates state.groups as it goes.
    const snapshot = [...groups];
    for (const group of snapshot) {
      // eslint-disable-next-line no-await-in-loop
      await applyGroup(group);
    }
    setApplyingAll(false);
  };

  const handlePreview = (group: TagGroup) => {
    // Selection lives on the project's asset list; switch to assets view so
    // the user actually sees the highlight.
    setViewMode("assets");
    setSelectedPaths(group.file_paths);
  };

  const handleSkip = (group: TagGroup) => {
    setGroups((prev) => (prev ? prev.filter((g) => g.name !== group.name) : prev));
  };

  const totalSuggested = groups?.reduce((sum, g) => sum + g.file_paths.length, 0) ?? 0;

  return (
    <div className="tc-aitag-anchor">
      <div className="tc-aitag" role="dialog" aria-label={t("aiTagPanel.title")}>
        <div className="tc-aitag-head">
          <span className="tc-aitag-spark">
            <Sparkles size={14} />
          </span>
          <div style={{ display: "flex", flexDirection: "column", minWidth: 0 }}>
            <span className="tc-aitag-title">{t("aiTagPanel.title")}</span>
            <span className="tc-aitag-sub">
              {generatedAt
                ? t("aiTagPanel.subtitle", {
                    time: generatedAt.toLocaleTimeString(),
                  })
                : t("aiTagPanel.subtitleLoading")}
            </span>
          </div>
          <button
            className="tc-aitag-close"
            onClick={close}
            title={t("common.close", "Close")}
            aria-label={t("common.close", "Close")}
          >
            <X size={14} />
          </button>
        </div>

        <div className="tc-aitag-list">
          {loading && (
            <div className="tc-aitag-loading">{t("aiTagPanel.loading")}</div>
          )}
          {!loading && error && (
            <div className="tc-aitag-empty">{error}</div>
          )}
          {!loading && !error && groups && groups.length === 0 && (
            <div className="tc-aitag-empty">{t("aiTagPanel.empty")}</div>
          )}
          {!loading &&
            !error &&
            groups &&
            groups.map((group) => {
              const isPending = pending.has(group.name) || applyingAll;
              const hintKey = HINT_KEYS[group.hint];
              return (
                <div key={group.name} className="tc-aitag-card">
                  <div className="tc-aitag-card-head">
                    <span
                      className="tc-aitag-card-dot"
                      style={{ background: group.color }}
                    />
                    <span className="tc-aitag-card-name">{group.name}</span>
                    <span className="tc-aitag-card-count">
                      {t("aiTagPanel.count", { count: group.file_paths.length })}
                    </span>
                  </div>
                  <div className="tc-aitag-card-conf">
                    <div className="tc-aitag-bar">
                      <div
                        className="tc-aitag-bar-fill"
                        style={{
                          width: `${Math.round(group.confidence * 100)}%`,
                          background: group.color,
                        }}
                      />
                    </div>
                    <span>{Math.round(group.confidence * 100)}%</span>
                  </div>
                  {hintKey && (
                    <div className="tc-aitag-card-hint">
                      {t(hintKey)}
                      {group.samples.length > 0 && (
                        <>
                          {" · "}
                          {group.samples.join(", ")}
                          {group.file_paths.length > group.samples.length &&
                            ` …`}
                        </>
                      )}
                    </div>
                  )}
                  <div className="tc-aitag-card-actions">
                    <button
                      className="tc-aitag-btn"
                      onClick={() => handleSkip(group)}
                      disabled={isPending}
                    >
                      {t("aiTagPanel.skip")}
                    </button>
                    <button
                      className="tc-aitag-btn"
                      onClick={() => handlePreview(group)}
                      disabled={isPending}
                    >
                      {t("aiTagPanel.preview")}
                    </button>
                    <button
                      className="tc-aitag-btn"
                      data-primary="true"
                      onClick={() => applyGroup(group)}
                      disabled={isPending}
                    >
                      {t("aiTagPanel.apply")}
                    </button>
                  </div>
                </div>
              );
            })}
        </div>

        <div className="tc-aitag-foot">
          <span className="tc-aitag-foot-text">
            {t("aiTagPanel.footSummary", {
              groupCount: groups?.length ?? 0,
              fileCount: totalSuggested,
            })}
          </span>
          <button
            className="tc-aitag-cta"
            onClick={handleApplyAll}
            disabled={!groups || groups.length === 0 || applyingAll || loading}
          >
            {applyingAll
              ? t("aiTagPanel.applyingAll")
              : t("aiTagPanel.applyAll")}
          </button>
        </div>
      </div>
    </div>
  );
}
