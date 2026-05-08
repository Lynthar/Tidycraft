import { useEffect, useRef, useState } from "react";
import { Trash2, AlertCircle, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "../lib/utils";
import { basename } from "../lib/pathUtils";
import type { DeleteResult } from "../types/asset";

interface DeleteConfirmDialogProps {
  isOpen: boolean;
  paths: string[];
  onClose: () => void;
  /**
   * Called after the delete finishes, whether fully successful or with per-path errors.
   * Gives caller a chance to clear selection / display toast / etc. The filesystem
   * watcher will update the asset list on its own — no rescan needed.
   */
  onDone: (result: DeleteResult) => void;
}

const PREVIEW_LIMIT = 5;

export function DeleteConfirmDialog({
  isOpen,
  paths,
  onClose,
  onDone,
}: DeleteConfirmDialogProps) {
  const { t } = useTranslation();
  const [isDeleting, setIsDeleting] = useState(false);
  const [errors, setErrors] = useState<DeleteResult["errors"]>([]);
  const confirmButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (isOpen) {
      // Reset transient state on every open: the component persists across
      // openings (parent only toggles `isOpen`), so leaving `isDeleting` true
      // from a previous successful delete would make the confirm button render
      // as a disabled "Deleting..." the next time around.
      setIsDeleting(false);
      setErrors([]);
      // Focus the confirm button so Enter submits and Esc (handled on overlay) cancels.
      setTimeout(() => confirmButtonRef.current?.focus(), 0);
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !isDeleting) {
        onClose();
      }
    };
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [isOpen, isDeleting, onClose]);

  if (!isOpen) return null;

  const count = paths.length;
  const preview = paths.slice(0, PREVIEW_LIMIT);
  const overflow = count - preview.length;

  const title =
    count === 1
      ? t("deleteConfirm.titleSingle")
      : t("deleteConfirm.titleBatch", { count });

  const handleConfirm = async () => {
    setIsDeleting(true);
    try {
      const result = await invoke<DeleteResult>("delete_assets", { paths });
      if (result.errors.length > 0) {
        // Show errors inline; don't dismiss. User sees what failed and can close.
        setErrors(result.errors);
        setIsDeleting(false);
        onDone(result);
        return;
      }
      onDone(result);
      onClose();
    } catch (err) {
      console.error("Failed to delete:", err);
      setErrors([{ path: "", message: String(err) }]);
      setIsDeleting(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={(e) => {
        if (!isDeleting && e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-card-bg border border-border rounded-lg shadow-2xl w-[480px] max-h-[80vh] flex flex-col">
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <div className="flex items-center gap-2 text-red-400">
            <Trash2 size={18} />
            <h3 className="font-medium text-text-primary">{title}</h3>
          </div>
          <button
            onClick={onClose}
            disabled={isDeleting}
            className="text-text-secondary hover:text-text-primary disabled:opacity-50"
          >
            <X size={16} />
          </button>
        </div>

        <div className="flex-1 overflow-auto p-4 space-y-3">
          <p className="text-sm text-text-secondary">{t("deleteConfirm.hint")}</p>

          <ul className="bg-background border border-border rounded px-3 py-2 text-sm font-mono space-y-0.5">
            {preview.map((p) => (
              <li key={p} className="truncate text-text-primary" title={p}>
                {basename(p)}
              </li>
            ))}
            {overflow > 0 && (
              <li className="text-text-secondary italic">
                {t("deleteConfirm.andMore", { count: overflow })}
              </li>
            )}
          </ul>

          {errors.length > 0 && (
            <div className="border border-red-400/40 bg-red-400/10 rounded p-3 space-y-2">
              <div className="flex items-center gap-2 text-red-400 font-medium text-sm">
                <AlertCircle size={14} />
                {t("deleteConfirm.errorsTitle")}
              </div>
              <ul className="text-xs text-red-300 space-y-1 max-h-32 overflow-auto">
                {errors.map((e, i) => (
                  <li key={i} className="font-mono">
                    <span className="truncate block" title={e.path}>
                      {e.path ? basename(e.path) : "(unknown)"}
                    </span>
                    <span className="text-red-400/80">{e.message}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 px-4 py-3 border-t border-border">
          <button
            onClick={onClose}
            disabled={isDeleting}
            className="px-3 py-1.5 text-sm rounded hover:bg-background text-text-secondary disabled:opacity-50"
          >
            {errors.length > 0 ? t("common.done") : t("common.cancel")}
          </button>
          {errors.length === 0 && (
            <button
              ref={confirmButtonRef}
              onClick={handleConfirm}
              disabled={isDeleting}
              className={cn(
                "px-3 py-1.5 text-sm rounded font-medium transition-colors",
                isDeleting
                  ? "bg-red-400/50 text-white cursor-not-allowed"
                  : "bg-red-500 hover:bg-red-600 text-white"
              )}
            >
              {isDeleting ? t("deleteConfirm.deleting") : t("deleteConfirm.confirm")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
