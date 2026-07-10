import { useEffect, useMemo, useRef, useState } from "react";
import {
  FolderInput,
  CopyPlus,
  Folder,
  FolderOpen,
  ChevronRight,
  ChevronDown,
  X,
  AlertCircle,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { ModalShell } from "./ModalShell";
import { invoke } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/projectStore";
import { cn } from "../lib/utils";
import { basename, dirname } from "../lib/pathUtils";
import type { DirectoryNode, FileOpResult } from "../types/asset";

interface MoveCopyDialogProps {
  isOpen: boolean;
  /** 'move' records undo and disallows same-dir targets; 'copy' is additive. */
  mode: "move" | "copy";
  /** Absolute paths of the files to operate on. */
  paths: string[];
  onClose: () => void;
  /** Called after backend returns — regardless of per-path errors. Gives caller
   * a chance to clear selection etc. Watcher handles UI asset-list refresh. */
  onDone: (result: FileOpResult) => void;
}

const PREVIEW_LIMIT = 5;

/**
 * Recursive row for the directory tree. Parent owns the expanded set so that
 * the state persists across node re-creations (e.g. on fs-change events).
 */
function TreeRow({
  node,
  depth,
  expanded,
  toggleExpanded,
  selectedPath,
  onSelect,
  disabledDirs,
}: {
  node: DirectoryNode;
  depth: number;
  expanded: Set<string>;
  toggleExpanded: (path: string) => void;
  selectedPath: string | null;
  onSelect: (path: string) => void;
  disabledDirs: Set<string>;
}) {
  const { t } = useTranslation();
  const hasChildren = node.children.length > 0;
  const isExpanded = expanded.has(node.path);
  const isSelected = selectedPath === node.path;
  const isDisabled = disabledDirs.has(node.path);

  return (
    <>
      <div
        role="button"
        tabIndex={isDisabled ? -1 : 0}
        onClick={() => !isDisabled && onSelect(node.path)}
        onKeyDown={(e) => {
          if (isDisabled) return;
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onSelect(node.path);
          }
        }}
        className={cn(
          "flex items-center gap-1 px-2 py-1 text-sm rounded cursor-pointer select-none",
          isSelected && "bg-primary/20 text-primary",
          !isSelected && !isDisabled && "hover:bg-background text-text-primary",
          isDisabled && "opacity-40 cursor-not-allowed"
        )}
        style={{ paddingLeft: 8 + depth * 14 }}
        title={isDisabled ? t("moveCopy.disabledTarget") : node.path}
      >
        {hasChildren ? (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              toggleExpanded(node.path);
            }}
            className="p-0.5 rounded hover:bg-card-bg"
          >
            {isExpanded ? (
              <ChevronDown size={12} className="text-text-secondary" />
            ) : (
              <ChevronRight size={12} className="text-text-secondary" />
            )}
          </button>
        ) : (
          <span className="w-4" />
        )}
        {isExpanded && hasChildren ? (
          <FolderOpen size={14} className="text-yellow-400 shrink-0" />
        ) : (
          <Folder size={14} className="text-yellow-400/80 shrink-0" />
        )}
        <span className="truncate flex-1">{node.name}</span>
        <span className="text-xs text-text-secondary">{node.file_count}</span>
      </div>
      {isExpanded &&
        hasChildren &&
        node.children.map((c) => (
          <TreeRow
            key={c.path}
            node={c}
            depth={depth + 1}
            expanded={expanded}
            toggleExpanded={toggleExpanded}
            selectedPath={selectedPath}
            onSelect={onSelect}
            disabledDirs={disabledDirs}
          />
        ))}
    </>
  );
}

export function MoveCopyDialog({
  isOpen,
  mode,
  paths,
  onClose,
  onDone,
}: MoveCopyDialogProps) {
  const { t } = useTranslation();
  const { scanResult, activeProjectId } = useProjectStore();

  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [isWorking, setIsWorking] = useState(false);
  const [errors, setErrors] = useState<FileOpResult["errors"]>([]);
  const confirmButtonRef = useRef<HTMLButtonElement>(null);

  // Compute disabled dirs: for move, can't move a file into the folder it's
  // already in (that's a no-op at best, confusing at worst). For copy, allow
  // same-dir — the backend will surface a "target already exists" error and
  // the user can read that clearly.
  const disabledDirs = useMemo(() => {
    if (mode !== "move") return new Set<string>();
    return new Set(paths.map(dirname));
  }, [mode, paths]);

  useEffect(() => {
    if (isOpen) {
      setIsWorking(false);
      setErrors([]);
      setSelectedPath(null);
      // Start with just the project root expanded.
      setExpanded(scanResult ? new Set([scanResult.directory_tree.path]) : new Set());
      setTimeout(() => confirmButtonRef.current?.focus(), 0);
    }
  }, [isOpen, scanResult]);

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !isWorking) onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [isOpen, isWorking, onClose]);

  if (!isOpen) return null;

  const count = paths.length;
  const preview = paths.slice(0, PREVIEW_LIMIT);
  const overflow = count - preview.length;

  const toggleExpanded = (path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const handleConfirm = async () => {
    if (!selectedPath || isWorking) return;
    setIsWorking(true);
    try {
      const result =
        mode === "move"
          ? await invoke<FileOpResult>("move_assets", {
              projectId: activeProjectId,
              paths,
              targetDir: selectedPath,
            })
          : await invoke<FileOpResult>("copy_assets", {
              paths,
              targetDir: selectedPath,
            });

      if (result.errors.length > 0) {
        setErrors(result.errors);
        setIsWorking(false);
        onDone(result);
        return;
      }
      onDone(result);
      onClose();
    } catch (err) {
      console.error(`Failed to ${mode}:`, err);
      setErrors([{ path: "", message: String(err) }]);
      setIsWorking(false);
    }
  };

  const title =
    mode === "move" ? t("moveCopy.titleMove", { count }) : t("moveCopy.titleCopy", { count });
  const confirmLabel =
    mode === "move" ? t("moveCopy.confirmMove") : t("moveCopy.confirmCopy");
  const workingLabel =
    mode === "move" ? t("moveCopy.moving") : t("moveCopy.copying");

  return (
    <ModalShell
      onClose={onClose}
      ariaLabel={title}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      disabled={isWorking}
    >
      <div className="bg-card-bg border border-border rounded-lg shadow-2xl w-[560px] max-h-[80vh] flex flex-col">
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <div className="flex items-center gap-2 text-primary">
            {mode === "move" ? <FolderInput size={18} /> : <CopyPlus size={18} />}
            <h3 className="font-medium text-text-primary">{title}</h3>
          </div>
          <button
            onClick={onClose}
            disabled={isWorking}
            className="text-text-secondary hover:text-text-primary disabled:opacity-50"
          >
            <X size={16} />
          </button>
        </div>

        <div className="flex-1 overflow-hidden flex flex-col gap-3 p-4 min-h-0">
          {/* Source list */}
          <div>
            <div className="text-xs text-text-secondary mb-1">
              {t("moveCopy.sourceList")}
            </div>
            <ul className="bg-background border border-border rounded px-3 py-2 text-sm font-mono space-y-0.5 max-h-28 overflow-auto">
              {preview.map((p) => (
                <li key={p} className="truncate text-text-primary" title={p}>
                  {basename(p)}
                </li>
              ))}
              {overflow > 0 && (
                <li className="text-text-secondary italic">
                  {t("moveCopy.andMore", { count: overflow })}
                </li>
              )}
            </ul>
          </div>

          {/* Target picker */}
          <div className="flex-1 flex flex-col min-h-0">
            <div className="text-xs text-text-secondary mb-1">
              {t("moveCopy.pickTarget")}
            </div>
            <div className="flex-1 bg-background border border-border rounded overflow-auto py-1">
              {scanResult ? (
                <TreeRow
                  node={scanResult.directory_tree}
                  depth={0}
                  expanded={expanded}
                  toggleExpanded={toggleExpanded}
                  selectedPath={selectedPath}
                  onSelect={setSelectedPath}
                  disabledDirs={disabledDirs}
                />
              ) : (
                <div className="p-3 text-sm text-text-secondary">
                  {t("moveCopy.noTree", "No project loaded")}
                </div>
              )}
            </div>
            {selectedPath && (
              <div className="mt-2 text-xs text-text-secondary truncate" title={selectedPath}>
                → {selectedPath}
              </div>
            )}
          </div>

          {errors.length > 0 && (
            <div className="border border-red-400/40 bg-red-400/10 rounded p-3 space-y-2">
              <div className="flex items-center gap-2 text-red-400 font-medium text-sm">
                <AlertCircle size={14} />
                {t("moveCopy.errorsTitle")}
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
            disabled={isWorking}
            className="px-3 py-1.5 text-sm rounded hover:bg-background text-text-secondary disabled:opacity-50"
          >
            {errors.length > 0 ? t("common.done") : t("common.cancel")}
          </button>
          {errors.length === 0 && (
            <button
              ref={confirmButtonRef}
              onClick={handleConfirm}
              disabled={isWorking || !selectedPath}
              className={cn(
                "px-3 py-1.5 text-sm rounded font-medium transition-colors",
                isWorking || !selectedPath
                  ? "bg-primary/50 text-[var(--on-primary)] cursor-not-allowed"
                  : "bg-primary hover:bg-primary/90 text-[var(--on-primary)]"
              )}
            >
              {isWorking ? workingLabel : confirmLabel}
            </button>
          )}
        </div>
      </div>
    </ModalShell>
  );
}
