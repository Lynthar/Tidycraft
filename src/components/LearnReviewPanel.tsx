import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Sparkles, X, Trash2, Check, AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  useUiStore,
  type AiLearnedRule,
  type AiTagCategory,
  type AiTagGap,
} from "../stores/uiStore";
import { useTagsStore } from "../stores/tagsStore";
import { useProjectStore } from "../stores/projectStore";

const CATEGORY_COLORS: Record<AiTagCategory, string> = {
  type: "#3b82f6",
  style: "#a855f7",
  mood: "#f97316",
  subject: "#10b981",
  other: "#6b7280",
};

/// Reviews an LLM learning result.
///
/// Behaviour on mount: tag-gaps are auto-created (per design discussion
/// — saves the user a click; review panel shows "AI added N tags" with
/// per-row revoke). Rules can be deleted before save; "Save rules"
/// persists the current list to `tidycraft.ai.toml` via `save_ai_rules`.
export function LearnReviewPanel() {
  const { t } = useTranslation();
  const open = useUiStore((s) => s.learnReviewOpen);
  const data = useUiStore((s) => s.learnReviewData);
  const setOpen = useUiStore((s) => s.setLearnReviewOpen);

  const createTag = useTagsStore((s) => s.createTag);
  const deleteTag = useTagsStore((s) => s.deleteTag);
  const userTags = useTagsStore((s) => s.tags);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  // Local mutable copy of the rules so the user can delete entries
  // before saving without forcing the global store to track edits.
  const [rules, setRules] = useState<AiLearnedRule[]>([]);
  // Track which gaps we successfully auto-created so the user can
  // revoke individually. Keyed by gap label for stability.
  const [createdGapTagIds, setCreatedGapTagIds] = useState<
    Record<string, string>
  >({});
  const [saving, setSaving] = useState(false);
  const [savedNotice, setSavedNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Reset + initialize on open.
  useEffect(() => {
    if (!open || !data) return;
    setRules(data.rules);
    setError(null);
    setSavedNotice(null);
    // Auto-create tags for gaps. Skip names that already exist (case
    // insensitive) so re-opening a result doesn't duplicate.
    const lowerExisting = new Set(
      userTags.map((tt) => tt.name.toLowerCase())
    );
    (async () => {
      const created: Record<string, string> = {};
      for (const gap of data.tag_gaps) {
        if (lowerExisting.has(gap.label.toLowerCase())) continue;
        try {
          const tag = await createTag(gap.label, CATEGORY_COLORS[gap.category]);
          if (tag) created[gap.label] = tag.id;
        } catch (e) {
          console.warn("[LearnReview] auto-create gap failed:", gap.label, e);
        }
      }
      setCreatedGapTagIds(created);
    })();
    // We deliberately omit `userTags` from the dep array — re-running
    // the auto-create loop on every tag list change would loop forever.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, data]);

  const handleRevokeGap = async (label: string) => {
    const id = createdGapTagIds[label];
    if (!id) return;
    try {
      await deleteTag(id);
      setCreatedGapTagIds((prev) => {
        const next = { ...prev };
        delete next[label];
        return next;
      });
    } catch (e) {
      console.warn("[LearnReview] revoke gap failed:", label, e);
    }
  };

  const handleDeleteRule = (idx: number) => {
    setRules((prev) => prev.filter((_, i) => i !== idx));
  };

  const handleSave = async () => {
    if (!activeProjectId || saving) return;
    setSaving(true);
    setError(null);
    try {
      await invoke("save_ai_rules", { projectId: activeProjectId, rules });
      setSavedNotice(t("learnReview.savedNotice"));
    } catch (e) {
      console.error("[LearnReview] save failed:", e);
      setError(t("learnReview.saveError"));
    } finally {
      setSaving(false);
    }
  };

  const conventions = data?.inferred_conventions;
  const tagMeanings = useMemo(
    () => Object.entries(conventions?.existing_tag_meanings ?? {}),
    [conventions]
  );

  if (!open || !data) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div
        className="rounded-lg shadow-xl w-full max-w-3xl flex flex-col"
        style={{
          background: "var(--panel)",
          border: "1px solid var(--line)",
          maxHeight: "90vh",
        }}
      >
        <div
          className="flex items-center justify-between px-4 py-3 shrink-0"
          style={{ borderBottom: "1px solid var(--line)" }}
        >
          <div className="flex items-center gap-2">
            <Sparkles size={16} style={{ color: "var(--primary)" }} />
            <h2 className="text-sm font-semibold">{t("learnReview.title")}</h2>
          </div>
          <button onClick={() => setOpen(false)} style={{ color: "var(--text-3)" }}>
            <X size={18} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-4 space-y-4">
          {/* --- Inferred Conventions --- */}
          {conventions &&
            (conventions.naming || conventions.directories || tagMeanings.length > 0) && (
              <section>
                <h3
                  className="text-xs font-semibold uppercase tracking-wide mb-2"
                  style={{ color: "var(--text-3)" }}
                >
                  {t("learnReview.conventions")}
                </h3>
                <div
                  className="rounded p-3 space-y-2"
                  style={{
                    background: "var(--panel-2)",
                    border: "1px solid var(--line)",
                  }}
                >
                  {conventions.naming && (
                    <div className="text-sm">
                      <span style={{ color: "var(--text-3)" }}>
                        {t("learnReview.naming")}:{" "}
                      </span>
                      {conventions.naming}
                    </div>
                  )}
                  {conventions.directories && (
                    <div className="text-sm">
                      <span style={{ color: "var(--text-3)" }}>
                        {t("learnReview.directories")}:{" "}
                      </span>
                      {conventions.directories}
                    </div>
                  )}
                  {tagMeanings.length > 0 && (
                    <details className="text-sm">
                      <summary
                        className="cursor-pointer"
                        style={{ color: "var(--text-3)" }}
                      >
                        {t("learnReview.tagMeanings", { count: tagMeanings.length })}
                      </summary>
                      <ul className="mt-1 space-y-0.5 pl-4">
                        {tagMeanings.map(([name, meaning]) => (
                          <li key={name} className="text-xs">
                            <code>{name}</code>
                            <span style={{ color: "var(--text-3)" }}> — </span>
                            {meaning}
                          </li>
                        ))}
                      </ul>
                    </details>
                  )}
                </div>
              </section>
            )}

          {/* --- Tag Gaps (auto-created) --- */}
          {data.tag_gaps.length > 0 && (
            <section>
              <h3
                className="text-xs font-semibold uppercase tracking-wide mb-2"
                style={{ color: "var(--text-3)" }}
              >
                {t("learnReview.tagGaps", {
                  count: Object.keys(createdGapTagIds).length,
                })}
              </h3>
              <div className="space-y-1.5">
                {data.tag_gaps.map((gap: AiTagGap) => {
                  const wasCreated = !!createdGapTagIds[gap.label];
                  return (
                    <div
                      key={gap.label}
                      className="flex items-start gap-2 rounded p-2"
                      style={{
                        background: "var(--panel-2)",
                        border: "1px solid var(--line)",
                      }}
                    >
                      <span
                        className="rounded-full shrink-0 mt-1"
                        style={{
                          width: 8,
                          height: 8,
                          background: CATEGORY_COLORS[gap.category],
                        }}
                      />
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-sm font-medium">
                            {gap.label}
                          </span>
                          <span
                            className="text-xs"
                            style={{ color: "var(--text-3)" }}
                          >
                            ({gap.category})
                          </span>
                          {wasCreated && (
                            <span
                              className="text-xs flex items-center gap-0.5"
                              style={{ color: "var(--ok, var(--primary))" }}
                            >
                              <Check size={11} />
                              {t("learnReview.gapCreated")}
                            </span>
                          )}
                        </div>
                        <p
                          className="text-xs mt-0.5"
                          style={{ color: "var(--text-3)" }}
                        >
                          {gap.reason}
                        </p>
                      </div>
                      {wasCreated && (
                        <button
                          onClick={() => handleRevokeGap(gap.label)}
                          className="p-1 rounded hover:bg-background"
                          style={{ color: "var(--text-3)" }}
                          title={t("learnReview.gapRevoke")}
                        >
                          <Trash2 size={13} />
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            </section>
          )}

          {/* --- Rules --- */}
          <section>
            <h3
              className="text-xs font-semibold uppercase tracking-wide mb-2"
              style={{ color: "var(--text-3)" }}
            >
              {t("learnReview.rules", { count: rules.length })}
            </h3>
            {rules.length === 0 ? (
              <p
                className="text-sm"
                style={{ color: "var(--text-3)", fontStyle: "italic" }}
              >
                {t("learnReview.rulesEmpty")}
              </p>
            ) : (
              <div className="space-y-1">
                {rules.map((rule, idx) => (
                  <div
                    key={`${rule.kind}-${rule.pattern}-${idx}`}
                    className="flex items-center gap-2 rounded p-2 font-mono text-xs"
                    style={{
                      background: "var(--panel-2)",
                      border: "1px solid var(--line)",
                    }}
                  >
                    <span
                      className="px-1.5 py-0.5 rounded shrink-0"
                      style={{
                        background: "var(--panel)",
                        color: "var(--text-3)",
                        border: "1px solid var(--line)",
                      }}
                    >
                      {rule.kind}
                    </span>
                    <span className="shrink-0" style={{ color: "var(--text-2)" }}>
                      {rule.pattern}
                    </span>
                    <span
                      style={{ color: "var(--text-3)" }}
                      className="shrink-0"
                    >
                      →
                    </span>
                    <span className="flex-1 truncate" title={rule.tags.join(", ")}>
                      {rule.tags.join(", ")}
                    </span>
                    <span
                      className="shrink-0"
                      style={{ color: "var(--text-3)" }}
                    >
                      {(rule.confidence * 100).toFixed(0)}%
                    </span>
                    <button
                      onClick={() => handleDeleteRule(idx)}
                      className="p-1 rounded hover:bg-background shrink-0"
                      style={{ color: "var(--text-3)" }}
                      title={t("learnReview.ruleDelete")}
                    >
                      <Trash2 size={12} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </section>

          {/* --- Sample Tags (compact preview) --- */}
          {data.sample_tags.length > 0 && (
            <section>
              <h3
                className="text-xs font-semibold uppercase tracking-wide mb-2"
                style={{ color: "var(--text-3)" }}
              >
                {t("learnReview.sampleTags", { count: data.sample_tags.length })}
              </h3>
              <div className="space-y-1.5 max-h-64 overflow-y-auto">
                {data.sample_tags.map((s) => (
                  <div
                    key={s.asset_path}
                    className="rounded p-2"
                    style={{
                      background: "var(--panel-2)",
                      border: "1px solid var(--line)",
                    }}
                  >
                    <div
                      className="text-xs font-mono truncate"
                      style={{ color: "var(--text-2)" }}
                      title={s.asset_path}
                    >
                      {s.asset_path}
                    </div>
                    <div className="flex flex-wrap gap-1 mt-1">
                      {s.matched_existing.map((label) => (
                        <span
                          key={`m-${label}`}
                          className="text-xs px-1.5 py-0.5 rounded"
                          style={{
                            background: "var(--panel)",
                            color: "var(--text-2)",
                            border: "1px solid var(--line)",
                          }}
                        >
                          {label}
                        </span>
                      ))}
                      {s.suggested_new.map((nt) => (
                        <span
                          key={`n-${nt.label}`}
                          className="text-xs px-1.5 py-0.5 rounded"
                          style={{
                            background: `${CATEGORY_COLORS[nt.category]}1F`,
                            color: CATEGORY_COLORS[nt.category],
                            border: `1px solid ${CATEGORY_COLORS[nt.category]}55`,
                          }}
                        >
                          + {nt.label}
                        </span>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}
        </div>

        {(savedNotice || error) && (
          <div
            className="px-4 py-2 text-xs shrink-0"
            style={{ borderTop: "1px solid var(--line)" }}
          >
            {error ? (
              <span
                className="flex items-center gap-1"
                style={{ color: "var(--err)" }}
              >
                <AlertTriangle size={11} /> {error}
              </span>
            ) : (
              <span style={{ color: "var(--ok, var(--primary))" }}>
                <Check size={11} className="inline mr-1" />
                {savedNotice}
              </span>
            )}
          </div>
        )}

        <div
          className="flex justify-end gap-2 px-4 py-3 shrink-0"
          style={{ borderTop: "1px solid var(--line)" }}
        >
          <button
            onClick={() => setOpen(false)}
            disabled={saving}
            className="px-3 py-1.5 text-sm rounded disabled:opacity-50"
            style={{
              border: "1px solid var(--line)",
              color: "var(--text-2)",
            }}
          >
            {t("learnReview.close")}
          </button>
          <button
            onClick={handleSave}
            disabled={saving || !activeProjectId}
            className="px-3 py-1.5 text-sm rounded disabled:opacity-50"
            style={{
              background: "var(--primary)",
              color: "var(--on-primary, white)",
            }}
          >
            {saving ? "…" : t("learnReview.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
