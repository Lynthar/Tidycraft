import { useEffect, useState } from "react";
import { Sparkles, X, RotateCw, Eye, Play } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useUiStore, type AiRulesDoc, type AiLearningResult } from "../stores/uiStore";
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
  const setLearnSetupOpen = useUiStore((s) => s.setLearnSetupOpen);
  const setLearnReviewOpen = useUiStore((s) => s.setLearnReviewOpen);
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
  /// AI rules document loaded from `tidycraft.ai.toml`. `undefined` =
  /// not yet probed; `null` = probed and absent (not yet learned).
  const [rulesDoc, setRulesDoc] = useState<AiRulesDoc | null | undefined>(
    undefined
  );

  // Probe rule store on open + when active project changes so the
  // status badge reflects the current state.
  useEffect(() => {
    if (!aiPanelOpen || !activeProjectId) {
      setRulesDoc(undefined);
      return;
    }
    let cancelled = false;
    invoke<AiRulesDoc | null>("read_ai_rules", { projectId: activeProjectId })
      .then((d) => {
        if (!cancelled) setRulesDoc(d);
      })
      .catch((e) => {
        console.warn("[AITagPanel] read_ai_rules failed:", e);
        if (!cancelled) setRulesDoc(null);
      });
    return () => {
      cancelled = true;
    };
  }, [aiPanelOpen, activeProjectId]);

  const handleReview = async () => {
    if (!activeProjectId || !rulesDoc) return;
    // Synthesize a minimal LearningResult from the saved rules so
    // LearnReviewPanel can render. We don't have the original
    // conventions / sample_tags / tag_gaps after persistence — only
    // rules survived the round-trip — so those sections render empty.
    const synthesized: AiLearningResult = {
      inferred_conventions: {
        naming: "",
        directories: "",
        existing_tag_meanings: {},
      },
      sample_tags: [],
      tag_gaps: [],
      rules: rulesDoc.rules,
      usage: { input_tokens: 0, output_tokens: 0, cached: true },
    };
    setLearnReviewOpen(true, synthesized);
  };

  // Days-since helper for "5d ago" display. Negative / future
  // timestamps fall back to "today" rather than rendering nonsense.
  const daysAgo = (iso: string): string => {
    const t0 = new Date(iso).getTime();
    if (!isFinite(t0)) return "?";
    const diffDays = Math.max(
      0,
      Math.floor((Date.now() - t0) / (1000 * 60 * 60 * 24))
    );
    if (diffDays === 0) return t("aiTagPanel.today");
    return t("aiTagPanel.daysAgo", { count: diffDays });
  };

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
      // Abort between the two writes if the user switched projects — the
      // tag landed in the original project; the asset bindings must not
      // resolve against the newly active one.
      if (useProjectStore.getState().activeProjectId !== activeProjectId) {
        console.warn("[AITagPanel] apply aborted: project switched mid-run");
        return false;
      }
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
    // Snapshot the project this apply-all targets. If the user switches projects
    // mid-loop (Sidebar / Switcher), abort the remaining groups — the tags-store
    // actions resolve the active project id LIVE, so they'd otherwise create the
    // rest of the tags in the newly-active project. (The panel itself also closes
    // on switch via uiStore's subscription; this stops the loop already running.)
    const targetProjectId = useProjectStore.getState().activeProjectId;
    if (!targetProjectId) return;
    setApplyingAll(true);
    // Snapshot the list — applyGroup mutates state.groups as it goes.
    const snapshot = [...groups];
    for (const group of snapshot) {
      if (useProjectStore.getState().activeProjectId !== targetProjectId) break;
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
          <div style={{ display: "flex", flexDirection: "column", minWidth: 0, flex: 1 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 6, minWidth: 0 }}>
              <span className="tc-aitag-title">{t("aiTagPanel.title")}</span>
              {/* Source badge: AI rules / Heuristic. Compact pill that
                  always tells the user where suggestions came from. */}
              {rulesDoc ? (
                <span
                  style={{
                    fontSize: 10,
                    padding: "1px 6px",
                    borderRadius: 999,
                    background: "color-mix(in oklch, var(--primary) 12%, transparent)",
                    color: "var(--primary)",
                    border: "1px solid color-mix(in oklch, var(--primary) 30%, transparent)",
                    whiteSpace: "nowrap",
                  }}
                  title={t("aiTagPanel.badgeAiTitle", {
                    when: daysAgo(rulesDoc.last_learned),
                    count: rulesDoc.rules.length,
                  })}
                >
                  🧠 AI · {daysAgo(rulesDoc.last_learned)}
                </span>
              ) : rulesDoc === null ? (
                <span
                  style={{
                    fontSize: 10,
                    padding: "1px 6px",
                    borderRadius: 999,
                    background: "var(--panel-2)",
                    color: "var(--text-3)",
                    border: "1px solid var(--line)",
                    whiteSpace: "nowrap",
                  }}
                  title={t("aiTagPanel.badgeHeuristicTitle")}
                >
                  {t("aiTagPanel.badgeHeuristic")}
                </span>
              ) : null}
            </div>
            <span className="tc-aitag-sub">
              {generatedAt
                ? t("aiTagPanel.subtitle", {
                    time: generatedAt.toLocaleTimeString(),
                  })
                : t("aiTagPanel.subtitleLoading")}
            </span>
          </div>
          {/* Learning controls. Minimal inline buttons (not a dropdown)
              for now; if it gets crowded we can collapse later. Run is
              shown when no rules exist; Re-learn + Review when they do. */}
          {activeProjectId && (
            <div style={{ display: "flex", gap: 2 }}>
              {!rulesDoc ? (
                <button
                  className="tc-aitag-close"
                  onClick={() => setLearnSetupOpen(true)}
                  title={t("aiTagPanel.runLearning")}
                  aria-label={t("aiTagPanel.runLearning")}
                >
                  <Play size={13} />
                </button>
              ) : (
                <>
                  <button
                    className="tc-aitag-close"
                    onClick={handleReview}
                    title={t("aiTagPanel.reviewRules")}
                    aria-label={t("aiTagPanel.reviewRules")}
                  >
                    <Eye size={13} />
                  </button>
                  <button
                    className="tc-aitag-close"
                    onClick={() => setLearnSetupOpen(true)}
                    title={t("aiTagPanel.relearn")}
                    aria-label={t("aiTagPanel.relearn")}
                  >
                    <RotateCw size={13} />
                  </button>
                </>
              )}
            </div>
          )}
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
          {/* "Run AI Learning" CTA — shown only when the rules probe
              has completed AND no rules exist yet. Sits above the
              heuristic suggestions (which still render below it as
              fallback) so users see the upgrade path before they
              scroll through token/dimension/path groups, which most
              find too generic to act on. Hidden when AI Learning has
              run (rulesDoc !== null) — at that point the panel is
              showing RuleSuggester output, no need for the CTA. */}
          {!loading && rulesDoc === null && activeProjectId && (
            <div
              className="rounded-md p-3 mb-2"
              style={{
                background:
                  "color-mix(in oklch, var(--primary) 6%, transparent)",
                border:
                  "1px solid color-mix(in oklch, var(--primary) 28%, transparent)",
              }}
            >
              <div className="flex items-start gap-2">
                <Sparkles
                  size={14}
                  className="shrink-0 mt-0.5"
                  style={{ color: "var(--primary)" }}
                />
                <div className="flex-1 min-w-0">
                  <div
                    className="text-sm font-semibold"
                    style={{ color: "var(--text)" }}
                  >
                    {t("aiTagPanel.ctaTitle")}
                  </div>
                  <p
                    className="text-xs mt-1"
                    style={{ color: "var(--text-2)" }}
                  >
                    {t("aiTagPanel.ctaBody")}
                  </p>
                  <button
                    onClick={() => setLearnSetupOpen(true)}
                    className="mt-2 px-2.5 py-1 text-xs rounded inline-flex items-center gap-1"
                    style={{
                      background: "var(--primary)",
                      color: "var(--on-primary, white)",
                    }}
                  >
                    <Play size={11} />
                    {t("aiTagPanel.ctaButton")}
                  </button>
                </div>
              </div>
            </div>
          )}
          {/* Heuristic-fallback divider — only when CTA above + groups
              below are both rendering, to make it visually clear that
              what follows is the lower-quality fallback set the CTA is
              offering to replace. */}
          {!loading && rulesDoc === null && groups && groups.length > 0 && (
            <div
              className="text-xs flex items-center gap-2 mb-1.5 mt-1"
              style={{ color: "var(--text-3)" }}
            >
              <span>{t("aiTagPanel.heuristicDivider")}</span>
              <span
                className="flex-1"
                style={{ height: 1, background: "var(--line)" }}
              />
            </div>
          )}
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
                      className="tc-aitag-card-btn"
                      onClick={() => handleSkip(group)}
                      disabled={isPending}
                    >
                      {t("aiTagPanel.skip")}
                    </button>
                    <button
                      className="tc-aitag-card-btn"
                      onClick={() => handlePreview(group)}
                      disabled={isPending}
                    >
                      {t("aiTagPanel.preview")}
                    </button>
                    <button
                      className="tc-aitag-card-btn"
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
