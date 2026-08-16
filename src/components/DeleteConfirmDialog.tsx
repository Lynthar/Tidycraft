import { useEffect, useRef, useState } from "react";
import { Trash2, AlertCircle, X } from "lucide-react";
import { ModalShell } from "./ModalShell";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "../lib/utils";
import { basename } from "../lib/pathUtils";
import type { DeleteResult } from "../types/asset";

interface DeleteConfirmDialogProps {
  isOpen: boolean;
  paths: string[];
  onClose: () => void;
  /** Called after the delete finishes, fully successful or with per-path errors,
   *  so the caller can clear the selection or show a toast. The filesystem watcher
   *  updates the asset list on its own — no rescan needed. */
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
  // Initial focus lands on Cancel (via ModalShell), NOT the destructive confirm
  // button: this dialog can open from a bare Delete keypress, and confirm-focused
  // meant a blind Enter deleted files.
  const cancelButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (isOpen) {
      // Reset transient state on every open: the component persists across
      // openings, so leaving `isDeleting` true from a previous delete would render
      // the confirm button as a disabled "Deleting…" the next time around.
      setIsDeleting(false);
      setErrors([]);
    }
  }, [isOpen]);

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
    <ModalShell
      onClose={onClose}
      ariaLabel={title}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      initialFocusRef={cancelButtonRef}
      disabled={isDeleting}
    >
      <div className="bg-panel border border-line rounded-lg shadow-2xl w-[480px] max-h-[80vh] flex flex-col">
        <div className="flex items-center justify-between px-4 py-3 border-b border-line">
          <div className="flex items-center gap-2 text-err">
            <Trash2 size={18} />
            <h3 className="font-medium text-ink">{title}</h3>
          </div>
          <button
            onClick={onClose}
            disabled={isDeleting}
            className="text-ink-2 hover:text-ink disabled:opacity-50"
          >
            <X size={16} />
          </button>
        </div>

        <div className="flex-1 overflow-auto p-4 space-y-3">
          <p className="text-sm text-ink-2">{t("deleteConfirm.hint")}</p>

          <ul className="bg-base border border-line rounded px-3 py-2 text-sm font-mono space-y-0.5">
            {preview.map((p) => (
              <li key={p} className="truncate text-ink" title={p}>
                {basename(p)}
              </li>
            ))}
            {overflow > 0 && (
              <li className="text-ink-2 italic">
                {t("deleteConfirm.andMore", { count: overflow })}
              </li>
            )}
          </ul>

          {errors.length > 0 && (
            <div className="border border-err bg-err-soft rounded p-3 space-y-2">
              <div className="flex items-center gap-2 text-err font-medium text-sm">
                <AlertCircle size={14} />
                {t("deleteConfirm.errorsTitle")}
              </div>
              <ul className="text-xs text-err space-y-1 max-h-32 overflow-auto">
                {errors.map((e, i) => (
                  <li key={i} className="font-mono">
                    <span className="truncate block" title={e.path}>
                      {e.path ? basename(e.path) : "(unknown)"}
                    </span>
                    <span className="text-err">{e.message}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 px-4 py-3 border-t border-line">
          <button
            ref={cancelButtonRef}
            onClick={onClose}
            disabled={isDeleting}
            className="px-3 py-1.5 text-sm rounded hover:bg-base text-ink-2 disabled:opacity-50"
          >
            {errors.length > 0 ? t("common.done") : t("common.cancel")}
          </button>
          {errors.length === 0 && (
            <button
              onClick={handleConfirm}
              disabled={isDeleting}
              className={cn(
                "px-3 py-1.5 text-sm rounded font-medium transition-colors",
                isDeleting
                  ? "bg-err-soft text-err cursor-not-allowed"
                  : "bg-err hover:brightness-105 text-on-error"
              )}
            >
              {isDeleting ? t("deleteConfirm.deleting") : t("deleteConfirm.confirm")}
            </button>
          )}
        </div>
      </div>
    </ModalShell>
  );
}
