import { useState, useEffect, useRef } from "react";
import { X, AlertCircle, AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ModalShell } from "./ModalShell";
import { invoke } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/projectStore";

interface RenameDialogProps {
  isOpen: boolean;
  onClose: () => void;
  assetPath: string;
  currentName: string;
  onComplete: () => void;
}

export function RenameDialog({
  isOpen,
  onClose,
  assetPath,
  currentName,
  onComplete,
}: RenameDialogProps) {
  const { t } = useTranslation();
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const projectType = useProjectStore((s) => s.scanResult?.project_type);
  const [newName, setNewName] = useState(currentName);
  const [isRenaming, setIsRenaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Godot rename guardrail: `res://` references are path strings, so a
  // rename silently breaks every scene/resource/script (and project.godot
  // entry) that points here. Warn — don't block — with the referencing
  // files. Unity is exempt by design: its references are GUID-based and the
  // .meta sidecar moves with the file.
  const [godotRefs, setGodotRefs] = useState<string[] | null>(null);
  useEffect(() => {
    if (!isOpen || projectType !== "godot" || !activeProjectId) {
      setGodotRefs(null);
      return;
    }
    setGodotRefs(null);
    let cancelled = false;
    (async () => {
      try {
        const map = await invoke<Record<string, string[]>>(
          "godot_asset_references",
          { projectId: activeProjectId, paths: [assetPath] }
        );
        if (!cancelled) setGodotRefs(map[assetPath] ?? []);
      } catch {
        // Check failure must not break renaming — just show no warning.
        if (!cancelled) setGodotRefs(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, projectType, activeProjectId, assetPath]);

  // The input carries the WHOLE filename and the initial selection covers only
  // the base name — Explorer / Finder behaviour, so typing replaces the name
  // and leaves `.png` alone while editing the extension on purpose stays
  // possible. Holding just the base name and appending a static suffix label
  // instead invites `hero.png` -> `hero.png.png`: the habit users arrive with
  // is to type the whole name they can see.
  // `> 0` keeps a dotfile (`.gitignore`) all name and no extension.
  const lastDotIndex = currentName.lastIndexOf(".");

  useEffect(() => {
    if (isOpen) {
      setNewName(currentName);
      setError(null);
      // Focus and select text after a short delay
      setTimeout(() => {
        const el = inputRef.current;
        if (!el) return;
        el.focus();
        if (lastDotIndex > 0) el.setSelectionRange(0, lastDotIndex);
        else el.select();
      }, 50);
    }
  }, [isOpen, currentName, lastDotIndex]);

  const handleRename = async () => {
    const trimmedName = newName.trim();
    if (!trimmedName) {
      setError(t("rename.emptyName", "Name cannot be empty"));
      return;
    }

    if (trimmedName === currentName) {
      onClose();
      return;
    }

    // Check for invalid characters
    if (/[<>:"/\\|?*]/.test(trimmedName)) {
      setError(t("rename.invalidChars", "Name contains invalid characters"));
      return;
    }

    setIsRenaming(true);
    setError(null);

    if (!activeProjectId) {
      setError(t("rename.noProject", "No active project"));
      setIsRenaming(false);
      return;
    }

    try {
      await invoke("rename_file", {
        projectId: activeProjectId,
        oldPath: assetPath,
        newName: trimmedName,
      });
      onComplete();
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setIsRenaming(false);
    }
  };

  // Enter only: Escape belongs to ModalShell, which takes it on the window in
  // the capture phase and stops propagation, so a handler here never sees it.
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !isRenaming) handleRename();
  };

  if (!isOpen) return null;

  return (
    <ModalShell
      onClose={onClose}
      ariaLabel={t("contextMenu.rename")}
      className="fixed inset-0 bg-black/50 flex items-center justify-center z-50"
      disabled={isRenaming}
    >
      <div className="bg-panel border border-line rounded-lg shadow-xl w-[400px] max-w-[90vw]">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-line">
          <h2 className="text-lg font-semibold">{t("contextMenu.rename")}</h2>
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-base text-ink-2 hover:text-ink transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {/* Content */}
        <div className="p-4">
          <div className="mb-4">
            <label className="block text-sm text-ink-2 mb-1">
              {t("rename.currentName", "Current name")}
            </label>
            <div className="text-sm text-ink bg-base px-3 py-2 rounded truncate">
              {currentName}
            </div>
          </div>

          <div className="mb-4">
            <label className="block text-sm text-ink-2 mb-1">
              {t("rename.newName", "New name")}
            </label>
            <input
              ref={inputRef}
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={handleKeyDown}
              disabled={isRenaming}
              className="w-full px-3 py-2 text-sm bg-base border border-line rounded
                         text-ink focus:outline-none focus:border-primary transition-colors
                         disabled:opacity-50"
              placeholder={t("rename.enterName", "Enter new name...")}
            />

          </div>

          {godotRefs && godotRefs.length > 0 && (
            <div
              className="p-2.5 mb-4 rounded text-xs space-y-1"
              style={{
                background: "color-mix(in oklch, var(--warn) 10%, transparent)",
                border: "1px solid color-mix(in oklch, var(--warn) 35%, transparent)",
                color: "var(--text-2)",
              }}
            >
              <div className="flex items-center gap-1.5" style={{ color: "var(--warn)" }}>
                <AlertTriangle size={13} className="shrink-0" />
                {t("rename.godotRefWarning", { count: godotRefs.length })}
              </div>
              <ul className="font-mono pl-5 space-y-0.5">
                {godotRefs.slice(0, 3).map((f) => (
                  <li key={f} className="truncate" title={f}>
                    {f}
                  </li>
                ))}
              </ul>
              {godotRefs.length > 3 && (
                <div className="pl-5" style={{ color: "var(--text-3)" }}>
                  {t("rename.godotRefMore", { count: godotRefs.length - 3 })}
                </div>
              )}
            </div>
          )}

          {error && (
            <div className="flex items-center gap-2 p-2 mb-4 bg-err-soft border border-err rounded text-sm text-err">
              <AlertCircle size={14} />
              {error}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-line">
          <button
            onClick={onClose}
            disabled={isRenaming}
            className="px-4 py-2 text-sm text-ink-2 hover:text-ink transition-colors disabled:opacity-50"
          >
            {t("common.cancel")}
          </button>
          <button
            onClick={handleRename}
            disabled={isRenaming || !newName.trim()}
            className="px-4 py-2 text-sm bg-primary text-[var(--on-primary)] rounded hover:bg-primary-strong transition-colors disabled:opacity-50"
          >
            {isRenaming ? t("batchRename.renaming") : t("contextMenu.rename")}
          </button>
        </div>
      </div>
    </ModalShell>
  );
}
