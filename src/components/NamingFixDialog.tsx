import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { X, RefreshCw, Check, AlertCircle, AlertTriangle, Wand2 } from "lucide-react";
import { ModalShell } from "./ModalShell";
import { useProjectStore } from "../stores/projectStore";
import type { NamingFixPreview, NamingFix, BatchRenameResult } from "../types/asset";

interface NamingFixDialogProps {
  isOpen: boolean;
  onClose: () => void;
  /// null = every auto-fixable asset in the project; a list = only those paths
  /// (the per-row "Fix" action passes a single path).
  scopePaths: string[] | null;
  /// Fired after >=1 file was renamed. `fullySucceeded` lets the parent decide
  /// whether to toast + treat as done; `renamedCount` is for the message. On a
  /// partial failure the dialog stays open to show the per-file errors.
  onComplete: (fullySucceeded: boolean, renamedCount: number) => void;
}

/// Cap on rendered rows. A strict project (Kenney with a `T_` prefix rule) can
/// have tens of thousands of auto-fixable issues; rendering them all as
/// non-virtualized table rows stalls the webview (the duplicate-group lesson).
/// Apply still operates on the full included set — only the display is capped.
const RENDER_CAP = 200;

/// Parent directory + case-folded name — the key on which two proposed renames
/// would collide (only the first would land). A JSON tuple keeps the two parts
/// unambiguous, so a name can't forge the boundary between them.
function targetKey(path: string, name: string): string {
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const parent = slash >= 0 ? path.slice(0, slash) : "";
  return JSON.stringify([parent.toLowerCase(), name.toLowerCase()]);
}

/// Review + apply auto-fixable naming renames. Fetches proposals from
/// `preview_naming_fixes` (same config the analysis used), lets the user
/// exclude or hand-edit any target, warns about Godot `res://` references, and
/// applies through `apply_naming_fixes` — one undo batch, tags carried, .meta
/// sidecars carried. Mounted inline by IssueList (like DeleteConfirmDialog).
export function NamingFixDialog({ isOpen, onClose, scopePaths, onComplete }: NamingFixDialogProps) {
  const { t } = useTranslation();
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const projectType = useProjectStore((s) => s.scanResult?.project_type);

  const [previews, setPreviews] = useState<NamingFixPreview[]>([]);
  const [edited, setEdited] = useState<Record<string, string>>({});
  const [excluded, setExcluded] = useState<Set<string>>(new Set());
  const [godotRefs, setGodotRefs] = useState<Record<string, string[]> | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isApplying, setIsApplying] = useState(false);
  const [result, setResult] = useState<BatchRenameResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Load proposals on open, using the same tidycraft.toml the analysis ran with
  // so suggestions line up with the issues that surfaced them.
  useEffect(() => {
    if (!isOpen || !activeProjectId) return;
    let cancelled = false;
    setIsLoading(true);
    setError(null);
    setResult(null);
    setEdited({});
    setExcluded(new Set());
    (async () => {
      try {
        const config = await invoke<string | null>("read_project_config", {
          projectId: activeProjectId,
        });
        const all = await invoke<NamingFixPreview[]>("preview_naming_fixes", {
          projectId: activeProjectId,
          configToml: config ?? null,
        });
        const scoped = scopePaths ? all.filter((p) => scopePaths.includes(p.path)) : all;
        if (!cancelled) setPreviews(scoped);
      } catch (err) {
        if (!cancelled) {
          setError(String(err));
          setPreviews([]);
        }
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, activeProjectId, scopePaths]);

  // Godot guardrail: which of these files are referenced by `res://` path. One
  // fetch per open (the set can't change while the modal is up). See
  // RenameDialog for the rationale — Unity is exempt (GUID + .meta carry).
  useEffect(() => {
    if (!isOpen || projectType !== "godot" || !activeProjectId || previews.length === 0) {
      setGodotRefs(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const map = await invoke<Record<string, string[]>>("godot_asset_references", {
          projectId: activeProjectId,
          paths: previews.map((p) => p.path),
        });
        if (!cancelled) setGodotRefs(map);
      } catch {
        if (!cancelled) setGodotRefs(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, projectType, activeProjectId, previews]);

  // Clear transient state on close so a reopen (possibly for a different scope)
  // doesn't flash the previous run's rows/result.
  useEffect(() => {
    if (!isOpen) {
      setPreviews([]);
      setResult(null);
      setError(null);
      setGodotRefs(null);
    }
  }, [isOpen]);

  const effectiveName = (p: NamingFixPreview) => edited[p.path] ?? p.suggested_name;
  const isIncluded = (p: NamingFixPreview) => {
    const name = effectiveName(p);
    return !excluded.has(p.path) && name.trim() !== "" && name !== p.original_name;
  };

  // Recompute collisions from the CURRENT (possibly edited) names among the
  // included rows, so editing a name to resolve a clash clears the warning.
  const collidingPaths = useMemo(() => {
    const counts = new Map<string, number>();
    for (const p of previews) {
      if (!isIncluded(p)) continue;
      const k = targetKey(p.path, effectiveName(p));
      counts.set(k, (counts.get(k) ?? 0) + 1);
    }
    const set = new Set<string>();
    for (const p of previews) {
      if (isIncluded(p) && (counts.get(targetKey(p.path, effectiveName(p))) ?? 0) > 1) {
        set.add(p.path);
      }
    }
    return set;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [previews, edited, excluded]);

  const includedList = previews.filter(isIncluded);
  const changedCount = includedList.length;

  // Godot: included files that are referenced, and total referencing files.
  const referencedIncluded = godotRefs
    ? includedList.filter((p) => (godotRefs[p.path]?.length ?? 0) > 0)
    : [];
  const referencedRefTotal = referencedIncluded.reduce(
    (n, p) => n + (godotRefs?.[p.path]?.length ?? 0),
    0
  );

  const handleApply = async () => {
    if (!activeProjectId) return;
    setIsApplying(true);
    setError(null);
    try {
      const fixes: NamingFix[] = includedList.map((p) => ({
        path: p.path,
        new_name: effectiveName(p),
      }));
      const res = await invoke<BatchRenameResult>("apply_naming_fixes", {
        projectId: activeProjectId,
        fixes,
      });
      setResult(res);
      if (res.success_count > 0) onComplete(res.error_count === 0, res.success_count);
      // Full success: close (the toast confirms). Any failure keeps the dialog
      // open so the result banner + per-file errors actually render.
      if (res.error_count === 0) onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setIsApplying(false);
    }
  };

  if (!isOpen) return null;

  return (
    <ModalShell
      onClose={onClose}
      ariaLabel={t("namingFix.title")}
      className="fixed inset-0 bg-black/50 flex items-center justify-center z-50"
      disabled={isApplying}
    >
      <div className="bg-card-bg border border-border rounded-lg w-[620px] max-w-[92vw] max-h-[82vh] flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-border">
          <h2 className="text-lg font-semibold flex items-center gap-2">
            <Wand2 size={17} style={{ color: "var(--primary)" }} />
            {t("namingFix.title")}
          </h2>
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-background text-text-secondary"
          >
            <X size={20} />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-auto p-4 space-y-4">
          <p className="text-xs text-text-secondary">{t("namingFix.desc")}</p>

          {isLoading ? (
            <div className="p-6 text-center text-text-secondary text-sm">
              {t("namingFix.loading")}
            </div>
          ) : previews.length === 0 ? (
            <div className="p-6 text-center text-text-secondary text-sm">
              {t("namingFix.nothingToFix")}
            </div>
          ) : (
            <div className="border border-border rounded overflow-hidden">
              <table className="w-full text-sm">
                <thead className="bg-background sticky top-0">
                  <tr>
                    <th className="w-8 p-2 border-b border-border"></th>
                    <th className="text-left p-2 border-b border-border">
                      {t("namingFix.original")}
                    </th>
                    <th className="text-left p-2 border-b border-border">
                      {t("namingFix.newName")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {previews.slice(0, RENDER_CAP).map((p) => {
                    const included = isIncluded(p);
                    const collides = collidingPaths.has(p.path);
                    return (
                      <tr key={p.path} style={{ opacity: included ? 1 : 0.5 }}>
                        <td className="p-2 border-b border-border align-top">
                          <input
                            type="checkbox"
                            checked={!excluded.has(p.path)}
                            onChange={() =>
                              setExcluded((prev) => {
                                const next = new Set(prev);
                                if (next.has(p.path)) next.delete(p.path);
                                else next.add(p.path);
                                return next;
                              })
                            }
                            className="accent-primary cursor-pointer"
                            style={{ width: 13, height: 13, marginTop: 4 }}
                          />
                        </td>
                        <td className="p-2 border-b border-border align-top">
                          <div
                            className="truncate max-w-[210px] text-text-secondary"
                            title={p.path}
                          >
                            {p.original_name}
                          </div>
                        </td>
                        <td className="p-2 border-b border-border align-top">
                          <input
                            type="text"
                            value={edited[p.path] ?? p.suggested_name}
                            onChange={(e) =>
                              setEdited((prev) => ({ ...prev, [p.path]: e.target.value }))
                            }
                            disabled={excluded.has(p.path)}
                            className="w-full px-2 py-1 bg-background border rounded text-text-primary focus:outline-none focus:border-primary disabled:opacity-50"
                            style={{ borderColor: collides ? "var(--warn)" : "var(--line)" }}
                          />
                          {collides && (
                            <div
                              className="mt-1 text-[11px] flex items-center gap-1"
                              style={{ color: "var(--warn)" }}
                            >
                              <AlertTriangle size={11} />
                              {t("namingFix.collision")}
                            </div>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}

          {previews.length > RENDER_CAP && (
            <p className="text-[11px]" style={{ color: "var(--text-3)" }}>
              {t("namingFix.truncated", { shown: RENDER_CAP, total: previews.length })}
            </p>
          )}

          {/* Godot guardrail: renamed files other files reference by path. */}
          {referencedIncluded.length > 0 && (
            <div
              className="flex items-start gap-2 p-3 rounded text-xs"
              style={{
                background: "color-mix(in oklch, var(--warn) 10%, transparent)",
                border: "1px solid color-mix(in oklch, var(--warn) 35%, transparent)",
                color: "var(--text-2)",
              }}
            >
              <AlertTriangle size={14} className="shrink-0" style={{ color: "var(--warn)" }} />
              <span>
                {t("batchRename.godotRefWarning", {
                  files: referencedIncluded.length,
                  refs: referencedRefTotal,
                })}
              </span>
            </div>
          )}

          {error && (
            <div className="flex items-center gap-2 p-3 bg-error/10 border border-error/30 rounded text-error text-sm">
              <AlertCircle size={16} />
              {error}
            </div>
          )}

          {result && (
            <div
              className={`flex items-center gap-2 p-3 rounded text-sm ${
                result.error_count > 0
                  ? "bg-warning/10 border border-warning/30 text-warning"
                  : "bg-green-500/10 border border-green-500/30 text-green-400"
              }`}
            >
              <Check size={16} />
              <span>
                {t("namingFix.renamed", { count: result.success_count })}
                {result.error_count > 0 &&
                  `, ${t("namingFix.failed", { count: result.error_count })}`}
              </span>
            </div>
          )}

          {result && result.errors.length > 0 && (
            <div className="max-h-32 overflow-auto p-3 bg-error/10 border border-error/30 rounded text-error text-sm">
              <div className="font-medium mb-1">{t("namingFix.errors")}</div>
              <ul className="space-y-1">
                {result.errors.map((e, i) => (
                  <li key={i} className="break-all">
                    {e}
                  </li>
                ))}
              </ul>
            </div>
          )}

          {result && (
            <p className="text-xs text-text-secondary">{t("namingFix.reopenToRetry")}</p>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 p-4 border-t border-border">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary"
          >
            {t("common.cancel")}
          </button>
          {/* `result` lingers only after a partial failure (full success closes
              the dialog) — and by then the table is stale: renamed rows would
              error again if re-sent. Apply locks; reopening refetches a truthful
              preview (fixed files gone, failed ones back with suggestions). */}
          <button
            onClick={handleApply}
            disabled={isApplying || changedCount === 0 || result !== null}
            className="flex items-center gap-2 px-4 py-2 text-sm bg-primary text-[var(--on-primary)] rounded hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {isApplying ? (
              <>
                <RefreshCw size={14} className="animate-spin" />
                {t("namingFix.applying")}
              </>
            ) : (
              <>
                <Check size={14} />
                {t("namingFix.apply", { count: changedCount })}
              </>
            )}
          </button>
        </div>
      </div>
    </ModalShell>
  );
}
