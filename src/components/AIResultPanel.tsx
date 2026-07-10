import { useEffect, useState } from "react";
import { Sparkles, X, Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ModalShell } from "./ModalShell";
import {
  useUiStore,
  type AiSuggestedTag,
} from "../stores/uiStore";
import { useTagsStore } from "../stores/tagsStore";
import { useProjectStore } from "../stores/projectStore";

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
///   - ONE Apply button, and it honors the chip selection ("apply
///     everything still selected"). There is deliberately no separate
///     "Apply all": a primary button that ignored the user's chip
///     deselections next to a UI that presents as a selection editor
///     applied exactly the suggestions the user had just excluded.
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
  const paths = useUiStore((s) => s.aiResultPaths);
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

  const apply = async () => {
    if (!data || applying) return;
    setApplying(true);
    setApplyError(null);
    // Pin the project this result belongs to. The panel closes on a
    // project switch (uiStore subscription), but an already-running apply
    // loop survives the unmount — each iteration would then re-snapshot
    // the NEW active project id and write this result's tags into it.
    const startProjectId = useProjectStore.getState().activeProjectId;
    // The response echoes asset paths back; only paths we actually ASKED
    // about are appliable — a hallucinated path would otherwise flow into
    // add_tag_to_assets and mint an orphan binding for a file that isn't
    // in the project (the cache layer already rejects these; the apply
    // side didn't).
    const requested = new Set(paths);
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
        if (!requested.has(s.asset_path)) {
          console.debug(
            "[AIResultPanel] dropping suggestion for path outside the request:",
            s.asset_path
          );
          continue;
        }
        for (const tag of s.tags) {
          const key = tagKey(s.asset_path, tag.label, tag.category);
          // The apply set is exactly the chips still selected — a chip the
          // user toggled off is never applied (Q7 decision: no bypass).
          if (!selected.has(key)) continue;
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

      // Find the `(AI)`-suffixed tag for a label, creating it only if no
      // tag of that exact name exists yet — in the store OR created
      // earlier in this very loop (the backend's create_tag does not
      // dedupe by name, so every create path must check first; the
      // hallucinated-existing fallback used to skip the check and mint
      // duplicate "x (AI)" tags, even twice within one run).
      const findOrCreateSuffixed = async (
        label: string,
        category: AiSuggestedTag["category"]
      ) => {
        const fullName = label + suffix;
        let tag = existingTags.find((tt) => tt.name === fullName);
        if (!tag) {
          const created = await createTag(fullName, CATEGORY_COLORS[category]);
          if (created) {
            tag = created;
            existingTags.push(created);
          }
        }
        return tag;
      };

      let totalApplied = 0;
      for (const { label, category, source, paths } of tagPlan.values()) {
        // Abort if the user switched projects mid-apply — the remaining
        // writes would land in the newly active project's tag store.
        if (useProjectStore.getState().activeProjectId !== startProjectId) {
          console.warn("[AIResultPanel] apply aborted: project switched mid-run");
          break;
        }
        let tag;
        if (source === "existing") {
          // Match the LLM's label to an existing tag by name. Case-
          // sensitive match — system prompt instructs the model to
          // copy the name verbatim. If the lookup fails (model
          // hallucinated `source: existing` against a non-existent
          // label), gracefully fall through to the suffixed-name path
          // so the tag still gets applied rather than dropped.
          tag =
            existingTags.find((tt) => tt.name === label) ??
            (await findOrCreateSuffixed(label, category));
        } else {
          // Brand-new label — append `(AI)` suffix to disambiguate,
          // reusing the tag if any prior run (or an earlier group in
          // this loop) already created it.
          tag = await findOrCreateSuffixed(label, category);
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
    <ModalShell
      onClose={() => setOpen(false)}
      ariaLabel={t("aiResult.title")}
      disabled={applying}
    >
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
                            isExisting
                              ? t("aiResult.chipExisting")
                              : t("aiResult.chipNew")
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
            onClick={() => apply()}
            disabled={
              applying || selected.size === 0 || data.suggestions.length === 0
            }
            className="px-3 py-1.5 text-sm rounded disabled:opacity-50"
            style={{
              background: "var(--primary)",
              color: "var(--on-primary, white)",
            }}
          >
            {t("aiResult.apply", { count: selected.size })}
          </button>
        </div>
      </div>
    </ModalShell>
  );
}
