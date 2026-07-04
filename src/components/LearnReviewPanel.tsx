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

/// Validate a regex pattern via JS `RegExp`. Pattern syntax is close
/// enough between JS and Rust's `regex` crate for the simple anchors /
/// char classes the LLM emits that this catches the vast majority of
/// invalid patterns up-front. False positives (JS-valid, Rust-invalid —
/// e.g. the LLM emits `(?P<name>...)` Python-style group) silent-skip
/// in the backend; we'd surface them via a "0 matches" indicator in
/// v2 if it becomes a problem.
const isValidRegex = (pattern: string): boolean => {
  try {
    new RegExp(pattern);
    return true;
  } catch {
    return false;
  }
};

/// Reviews an LLM learning result.
///
/// Nothing is committed until Save: gap tags are created for the rows the
/// user keeps checked, and the (possibly edited) rule list is persisted to
/// `tidycraft.ai.toml` via `save_ai_rules` — which takes the pending doc the
/// learn command staged in backend memory. Closing the panel without saving
/// discards the run entirely; unreviewed rules never affect suggestions.
export function LearnReviewPanel() {
  const { t } = useTranslation();
  const open = useUiStore((s) => s.learnReviewOpen);
  const data = useUiStore((s) => s.learnReviewData);
  const setOpen = useUiStore((s) => s.setLearnReviewOpen);

  const createTag = useTagsStore((s) => s.createTag);
  const userTags = useTagsStore((s) => s.tags);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  // Local mutable copy of the rules so the user can delete entries
  // before saving without forcing the global store to track edits.
  const [rules, setRules] = useState<AiLearnedRule[]>([]);
  // Which tag-gaps the user has selected to create. Nothing is written to
  // the project until Save — opening the panel has no side effects. Gaps are
  // proposals, pre-selected for convenience but fully opt-out.
  const [selectedGaps, setSelectedGaps] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);
  const [savedNotice, setSavedNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Reset + initialize on open. No side effects: we only pre-select the
  // gaps that don't already exist as tags. Tags are created on Save, not
  // here — opening (or closing) the panel never writes to the project.
  useEffect(() => {
    if (!open || !data) return;
    setRules(data.rules);
    setError(null);
    setSavedNotice(null);
    const lowerExisting = new Set(userTags.map((tt) => tt.name.toLowerCase()));
    setSelectedGaps(
      new Set(
        data.tag_gaps
          .map((g) => g.label)
          .filter((label) => !lowerExisting.has(label.toLowerCase()))
      )
    );
    // `userTags` is deliberately omitted: the pre-selection is snapshotted
    // once per open, so toggling tags afterward doesn't reset the user's
    // checkbox choices.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, data]);

  const handleToggleGap = (label: string) => {
    setSelectedGaps((prev) => {
      const next = new Set(prev);
      if (next.has(label)) next.delete(label);
      else next.add(label);
      return next;
    });
  };

  // Tag names that already exist (case-insensitive). These gaps can't be
  // re-created, so the UI shows them as "already exists" and disables their
  // checkbox. Recomputed as tags change (e.g. right after Save).
  const existingTagNamesLower = useMemo(
    () => new Set(userTags.map((tt) => tt.name.toLowerCase())),
    [userTags]
  );

  const handleDeleteRule = (idx: number) => {
    setRules((prev) => prev.filter((_, i) => i !== idx));
  };

  /// Update one rule's confidence in-place. The slider clamps to
  /// [0.5, 1.0] in the markup; we don't double-clamp here to keep
  /// programmatic callers (none today) honest about the range they pass.
  const handleEditConfidence = (idx: number, next: number) => {
    setRules((prev) =>
      prev.map((r, i) => (i === idx ? { ...r, confidence: next } : r))
    );
  };

  /// Per-rule validity flags. Only `filename_regex` rules can be
  /// invalid; others always pass. Memoized on `rules` so re-renders
  /// from slider edits don't re-run RegExp constructor for every row.
  const ruleValidity = useMemo(
    () =>
      rules.map((r) =>
        r.kind === "filename_regex" ? isValidRegex(r.pattern) : true
      ),
    [rules]
  );

  const handleSave = async () => {
    if (!activeProjectId || saving || !data) return;
    setSaving(true);
    setError(null);
    try {
      // Explicit commit point: create the gap tags the user kept checked
      // (skipping any that already exist), then persist the rules. Closing
      // without Save writes nothing.
      let createdCount = 0;
      for (const gap of data.tag_gaps) {
        if (!selectedGaps.has(gap.label)) continue;
        if (existingTagNamesLower.has(gap.label.toLowerCase())) continue;
        try {
          const tag = await createTag(gap.label, CATEGORY_COLORS[gap.category]);
          if (tag) createdCount += 1;
        } catch (e) {
          console.warn("[LearnReview] create gap failed:", gap.label, e);
        }
      }
      await invoke("save_ai_rules", { projectId: activeProjectId, rules });
      setSavedNotice(
        createdCount > 0
          ? t("learnReview.savedNoticeWithTags", { count: createdCount })
          : t("learnReview.savedNotice")
      );
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

          {/* --- Tag Gaps (created on Save; pre-selected, opt-out) --- */}
          {data.tag_gaps.length > 0 && (
            <section>
              <h3
                className="text-xs font-semibold uppercase tracking-wide mb-2"
                style={{ color: "var(--text-3)" }}
              >
                {t("learnReview.tagGaps", { count: selectedGaps.size })}
              </h3>
              <div className="space-y-1.5">
                {data.tag_gaps.map((gap: AiTagGap) => {
                  const exists = existingTagNamesLower.has(
                    gap.label.toLowerCase()
                  );
                  const selected = selectedGaps.has(gap.label);
                  return (
                    <label
                      key={gap.label}
                      className={`flex items-start gap-2 rounded p-2 ${
                        exists ? "" : "cursor-pointer"
                      }`}
                      style={{
                        background: "var(--panel-2)",
                        border: "1px solid var(--line)",
                      }}
                    >
                      <input
                        type="checkbox"
                        className="mt-1 shrink-0"
                        checked={selected && !exists}
                        disabled={exists}
                        onChange={() => handleToggleGap(gap.label)}
                      />
                      <span
                        className="rounded-full shrink-0 mt-1.5"
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
                          {exists && (
                            <span
                              className="text-xs"
                              style={{ color: "var(--text-3)" }}
                            >
                              {t("learnReview.gapExists")}
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
                    </label>
                  );
                })}
              </div>
              <p className="text-xs mt-1.5" style={{ color: "var(--text-3)" }}>
                {t("learnReview.gapCreateHint")}
              </p>
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
                {rules.map((rule, idx) => {
                  const valid = ruleValidity[idx];
                  return (
                    <div
                      key={`${rule.kind}-${rule.pattern}-${idx}`}
                      className="flex items-center gap-2 rounded p-2 font-mono text-xs"
                      style={{
                        background: "var(--panel-2)",
                        border: "1px solid var(--line)",
                      }}
                    >
                      <span
                        className="px-1.5 py-0.5 rounded shrink-0 flex items-center gap-1"
                        style={{
                          background: "var(--panel)",
                          color: valid ? "var(--text-3)" : "var(--err)",
                          border: valid
                            ? "1px solid var(--line)"
                            : "1px solid color-mix(in oklch, var(--err) 35%, transparent)",
                        }}
                        title={valid ? undefined : t("learnReview.ruleInvalidRegex")}
                      >
                        {!valid && <AlertTriangle size={10} />}
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
                      <input
                        type="range"
                        min={0.5}
                        max={1}
                        step={0.05}
                        value={rule.confidence}
                        onChange={(e) =>
                          handleEditConfidence(idx, parseFloat(e.target.value))
                        }
                        className="shrink-0"
                        style={{ width: 64 }}
                        aria-label={t("learnReview.ruleConfidence")}
                        title={t("learnReview.ruleConfidence")}
                      />
                      <span
                        className="shrink-0 tabular-nums text-right"
                        style={{ color: "var(--text-3)", minWidth: "2.5rem" }}
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
                  );
                })}
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
