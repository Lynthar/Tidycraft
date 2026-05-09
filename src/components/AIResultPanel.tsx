import { useEffect, useState } from "react";
import { Sparkles, X, Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  useUiStore,
  type AiSuggestedTag,
} from "../stores/uiStore";
import { useTagsStore } from "../stores/tagsStore";

/// Per-category color so the user can scan a card and see at a glance
/// which tags are types vs styles vs mood. These hex values match the
/// rough vibe of `--c-texture` etc. but aren't tied to existing CSS
/// vars because we apply them as `${color}1F` (12% alpha) backgrounds
/// which CSS vars don't support directly.
const CATEGORY_COLORS: Record<AiSuggestedTag["category"], string> = {
  type: "#3b82f6", // blue
  style: "#a855f7", // purple
  mood: "#f97316", // orange
  subject: "#10b981", // green
  other: "#6b7280", // gray
};

function tagKey(assetPath: string, label: string, category: string): string {
  return `${assetPath}::${category}::${label}`;
}

/// Reviews the LLM's tag suggestions and applies the user's selection
/// via existing `tagsStore.createTag` + `addTagToAssets`.
///
/// Behavior:
///   - Default state: every returned suggestion is pre-selected.
///   - Click a chip to toggle its inclusion in the apply set.
///   - Tag names get a `(AI)` suffix to disambiguate from heuristic
///     suggestions (which use `(suggested)`) and from manual tags.
///     Same suffixed name from a previous AI run is reused, not
///     duplicated.
///   - Apply groups (label + category) → batched `addTagToAssets`
///     so 50 assets × 3 tags is 3 IPC calls, not 150.
export function AIResultPanel() {
  const { t } = useTranslation();
  const open = useUiStore((s) => s.aiResultOpen);
  const data = useUiStore((s) => s.aiResultData);
  const setOpen = useUiStore((s) => s.setAiResultOpen);

  const createTag = useTagsStore((s) => s.createTag);
  const addTagToAssets = useTagsStore((s) => s.addTagToAssets);
  // Subscribed copy of the user's tag list — used to color "existing"
  // chips with the user's chosen color so the panel stays consistent
  // with how the tag will look after apply.
  const userTags = useTagsStore((s) => s.tags);

  // Selected (asset, tag) pairs. Set keyed by
  // `${path}::${category}::${label}` so toggling a tag on one asset
  // doesn't ripple to a same-named tag on another asset.
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [applying, setApplying] = useState(false);
  const [appliedCount, setAppliedCount] = useState<number | null>(null);
  const [applyError, setApplyError] = useState<string | null>(null);

  // Reset selection state when a new response comes in.
  useEffect(() => {
    if (!data) {
      setSelected(new Set());
      setAppliedCount(null);
      setApplyError(null);
      return;
    }
    const all = new Set<string>();
    for (const s of data.suggestions) {
      for (const tag of s.tags) {
        all.add(tagKey(s.asset_path, tag.label, tag.category));
      }
    }
    setSelected(all);
    setAppliedCount(null);
    setApplyError(null);
  }, [data]);

  const toggleTag = (assetPath: string, tag: AiSuggestedTag) => {
    const key = tagKey(assetPath, tag.label, tag.category);
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const apply = async (mode: "all" | "selected") => {
    if (!data || applying) return;
    setApplying(true);
    setApplyError(null);
    try {
      // Snapshot tags once; manually push to the local array as we
      // create new ones so subsequent dedupe finds them without
      // re-querying the store. Note this is a local mutation, not a
      // React state update — purely a lookup cache for this loop.
      const existingTags = [...useTagsStore.getState().tags];
      const suffix = " " + t("aiResult.tagSuffix"); // " (AI)"

      // Group (label, category, source) → list of asset paths so a
      // single tag applied to N assets goes through one batched
      // `addTagToAssets` IPC call instead of N.
      //
      // `source` is part of the group key because two suggestions
      // labeled "hero" with different `source` values mean different
      // things: existing → use the user's existing "hero" tag id;
      // new → create a fresh "hero (AI)" tag. We must not collapse
      // them into one group.
      type Group = {
        label: string;
        category: AiSuggestedTag["category"];
        source: "existing" | "new";
        paths: string[];
      };
      const tagPlan = new Map<string, Group>();

      for (const s of data.suggestions) {
        for (const tag of s.tags) {
          const key = tagKey(s.asset_path, tag.label, tag.category);
          if (mode === "selected" && !selected.has(key)) continue;
          const source = tag.source ?? "new";
          const groupKey = `${source}::${tag.category}::${tag.label}`;
          let group = tagPlan.get(groupKey);
          if (!group) {
            group = {
              label: tag.label,
              category: tag.category,
              source,
              paths: [],
            };
            tagPlan.set(groupKey, group);
          }
          group.paths.push(s.asset_path);
        }
      }

      let totalApplied = 0;
      for (const { label, category, source, paths } of tagPlan.values()) {
        let tag;
        if (source === "existing") {
          // Match the LLM's label to an existing tag by name. Case-
          // sensitive match — system prompt instructs the model to
          // copy the name verbatim. If the lookup fails (model
          // hallucinated `source: existing` against a non-existent
          // label), gracefully fall through to the "new" branch so
          // the tag still gets applied rather than dropped.
          tag = existingTags.find((tt) => tt.name === label);
          if (!tag) {
            const created = await createTag(
              label + suffix,
              CATEGORY_COLORS[category]
            );
            if (created) {
              tag = created;
              existingTags.push(created);
            }
          }
        } else {
          // Brand-new label — append `(AI)` suffix to disambiguate.
          // If the same suffixed name exists from a prior AI run,
          // reuse it instead of duplicating.
          const fullName = label + suffix;
          tag = existingTags.find((tt) => tt.name === fullName);
          if (!tag) {
            const created = await createTag(fullName, CATEGORY_COLORS[category]);
            if (created) {
              tag = created;
              existingTags.push(created);
            }
          }
        }
        if (tag) {
          await addTagToAssets(paths, tag.id);
          totalApplied += paths.length;
        }
      }

      setAppliedCount(totalApplied);
    } catch (err) {
      console.error("[AIResultPanel] apply failed:", err);
      setApplyError(t("aiResult.applyError"));
    } finally {
      setApplying(false);
    }
  };

  if (!open || !data) return null;

  const summary = data.usage.cached
    ? t("aiResult.summaryCached", { count: data.suggestions.length })
    : t("aiResult.summary", {
        count: data.suggestions.length,
        tokens: data.usage.input_tokens + data.usage.output_tokens,
      });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div
        className="rounded-lg shadow-xl w-full max-w-2xl flex flex-col"
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
            <h2 className="text-sm font-semibold">{t("aiResult.title")}</h2>
          </div>
          <button
            onClick={() => setOpen(false)}
            disabled={applying}
            className="disabled:opacity-50"
            style={{ color: "var(--text-3)" }}
          >
            <X size={18} />
          </button>
        </div>

        <div
          className="px-4 py-2 text-xs shrink-0"
          style={{
            color: "var(--text-3)",
            borderBottom: "1px solid var(--line)",
          }}
        >
          {summary}
        </div>

        <div className="flex-1 overflow-y-auto p-4 space-y-2">
          {data.suggestions.length === 0 ? (
            <div
              className="text-sm text-center py-8"
              style={{ color: "var(--text-3)" }}
            >
              {t("aiResult.empty")}
            </div>
          ) : (
            data.suggestions.map((s) => (
              <div
                key={s.asset_path}
                className="rounded p-3"
                style={{
                  background: "var(--panel-2)",
                  border: "1px solid var(--line)",
                }}
              >
                <div
                  className="text-xs font-mono truncate mb-2"
                  style={{ color: "var(--text-2)" }}
                  title={s.asset_path}
                >
                  {s.asset_path}
                </div>
                <div className="flex flex-wrap gap-1.5">
                  {s.tags.length === 0 ? (
                    <span
                      className="text-xs"
                      style={{ color: "var(--text-3)", fontStyle: "italic" }}
                    >
                      —
                    </span>
                  ) : (
                    s.tags.map((tag) => {
                      const key = tagKey(s.asset_path, tag.label, tag.category);
                      const isSelected = selected.has(key);
                      const isExisting = (tag.source ?? "new") === "existing";
                      // For `existing` chips, use the user's stored color
                      // for the matching tag so the chip matches the
                      // post-apply look. Fall back to category color
                      // if the lookup fails (model labeled something
                      // `existing` but the name doesn't match any tag).
                      const matchedTag = isExisting
                        ? userTags.find((tt) => tt.name === tag.label)
                        : undefined;
                      const color =
                        matchedTag?.color ?? CATEGORY_COLORS[tag.category];
                      return (
                        <button
                          key={key}
                          onClick={() => toggleTag(s.asset_path, tag)}
                          className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs"
                          style={{
                            background: isSelected ? `${color}1F` : "transparent",
                            color: isSelected ? color : "var(--text-3)",
                            border: `1px solid ${
                              isSelected ? `${color}55` : "var(--line)"
                            }`,
                            cursor: "pointer",
                          }}
                          title={`${tag.category} · ${(
                            tag.confidence * 100
                          ).toFixed(0)}% · ${
                            isExisting ? "existing tag" : "new tag"
                          }`}
                        >
                          {isSelected && <Check size={10} />}
                          {/* Visual hint for existing-tag chips: a
                              small dot using the existing tag's color
                              echoes the way TagManager renders them. */}
                          {isExisting && matchedTag && (
                            <span
                              className="rounded-full shrink-0"
                              style={{
                                width: 5,
                                height: 5,
                                background: color,
                              }}
                            />
                          )}
                          <span>{tag.label}</span>
                          <span style={{ opacity: 0.6, fontSize: 10 }}>
                            {isExisting ? "↩" : tag.category}
                          </span>
                        </button>
                      );
                    })
                  )}
                </div>
              </div>
            ))
          )}
        </div>

        {(appliedCount !== null || applyError) && (
          <div
            className="px-4 py-2 text-xs shrink-0"
            style={{ borderTop: "1px solid var(--line)" }}
          >
            {applyError ? (
              <span style={{ color: "var(--err)" }}>{applyError}</span>
            ) : (
              <span style={{ color: "var(--ok, var(--primary))" }}>
                {t("aiResult.applyDone", { count: appliedCount ?? 0 })}
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
            disabled={applying}
            className="px-3 py-1.5 text-sm rounded disabled:opacity-50"
            style={{
              border: "1px solid var(--line)",
              color: "var(--text-2)",
            }}
          >
            {t("aiResult.close")}
          </button>
          <button
            onClick={() => apply("selected")}
            disabled={
              applying || selected.size === 0 || data.suggestions.length === 0
            }
            className="px-3 py-1.5 text-sm rounded disabled:opacity-50"
            style={{
              border: "1px solid var(--line)",
              color: "var(--text)",
            }}
          >
            {t("aiResult.applySelected")}
          </button>
          <button
            onClick={() => apply("all")}
            disabled={applying || data.suggestions.length === 0}
            className="px-3 py-1.5 text-sm rounded disabled:opacity-50"
            style={{
              background: "var(--primary)",
              color: "var(--on-primary, white)",
            }}
          >
            {t("aiResult.applyAll")}
          </button>
        </div>
      </div>
    </div>
  );
}
