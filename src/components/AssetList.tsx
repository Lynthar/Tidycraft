import { useState, useCallback, useEffect, useMemo } from "react";
import { Image, Edit3, X, List, LayoutGrid, Move, Trash2, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useProjectStore } from "../stores/projectStore";
import { useTagsStore } from "../stores/tagsStore";
import { useColumnStore } from "../stores/columnStore";
import { useUiStore, isBlockingOverlayOpen } from "../stores/uiStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useSettingsStore } from "../stores/settingsStore";
import { BatchRenameDialog } from "./BatchRenameDialog";
import { RenameDialog } from "./RenameDialog";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import { MoveCopyDialog } from "./MoveCopyDialog";
import type { FileOpResult } from "../types/asset";
import { BatchTagSelector } from "./TagSelector";
import { ContextMenu } from "./ContextMenu";
import { AssetListView } from "./AssetListView";
import { AssetGalleryView } from "./AssetGalleryView";
import type { AssetInfo, AssetType } from "../types/asset";

/// Canonical asset-type order for the toolbar filter pills. Mirrors the
/// CommandPalette's Filter section so the two entry points stay
/// consistent. Types absent from `scanResult.type_counts` are skipped.
const FILTER_TYPE_ORDER: AssetType[] = [
  "texture",
  "model",
  "audio",
  "video",
  "animation",
  "material",
  "prefab",
  "scene",
  "script",
  "data",
  "other",
];

export function AssetList() {
  const { t } = useTranslation();
  const {
    scanResult,
    selectedAsset,
    setSelectedAsset,
    getFilteredAssets,
    isScanning,
    projectPath,
    openProject,
    sortField,
    sortDirection,
    setSortField,
    toggleSortDirection,
    gitStatuses,
    typeFilter,
    setTypeFilter,
    searchQuery,
    selectedDirectory,
    refreshUndoState,
    advancedFilters,
  } = useProjectStore();
  const { loadTags, assetTags: allAssetTags, tagFilters } = useTagsStore();
  const viewMode = useColumnStore((s) => s.viewMode);
  const setViewMode = useColumnStore((s) => s.setViewMode);
  const setTagManagerOpen = useUiStore((s) => s.setTagManagerOpen);
  const setAiAnalyzeOpen = useUiStore((s) => s.setAiAnalyzeOpen);
  const aiActiveProvider = useSettingsStore((s) => s.aiActiveProvider);
  const aiPerAssetModeEnabled = useSettingsStore(
    (s) => s.aiPerAssetModeEnabled
  );
  // Per-asset entries (multi-select bar button, right-click menu item)
  // are gated on BOTH a configured provider AND the advanced toggle.
  // Learning mode is the recommended path; this entry is opt-in.
  const aiDirectModeAvailable = !!aiActiveProvider && aiPerAssetModeEnabled;

  const selectedPaths = useSelectionStore((s) => s.selectedPaths);
  const setSelectedPaths = useSelectionStore((s) => s.setSelectedPaths);
  const togglePath = useSelectionStore((s) => s.togglePath);
  const addPaths = useSelectionStore((s) => s.addPaths);
  const removePaths = useSelectionStore((s) => s.removePaths);
  const clearSelection = useSelectionStore((s) => s.clearSelection);

  const [showBatchRename, setShowBatchRename] = useState(false);
  const [lastClickedIndex, setLastClickedIndex] = useState<number | null>(null);

  // Single rename state
  const [renameAsset, setRenameAsset] = useState<AssetInfo | null>(null);

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{
    isOpen: boolean;
    position: { x: number; y: number };
    asset: AssetInfo | null;
  }>({ isOpen: false, position: { x: 0, y: 0 }, asset: null });

  // Delete confirmation: list of paths the user has requested to send to trash.
  const [deleteDialogPaths, setDeleteDialogPaths] = useState<string[] | null>(null);

  // Move / Copy dialog: null means closed. `mode` distinguishes the two flows.
  const [moveCopyDialog, setMoveCopyDialog] = useState<
    { mode: "move" | "copy"; paths: string[] } | null
  >(null);

  // Load tags when project is loaded
  useEffect(() => {
    if (scanResult) {
      loadTags();
    }
  }, [scanResult, loadTags]);

  // Get filtered assets with tag filtering applied. `scanResult` is included
  // so watcher-driven asset list changes (add/remove) invalidate this memo —
  // the store-level cache inside getFilteredAssets handles filter+sort itself.
  const assets = useMemo(() => {
    let filteredAssets = getFilteredAssets();

    if (tagFilters.length > 0) {
      filteredAssets = filteredAssets.filter((asset) => {
        const assetTagList = allAssetTags[asset.path] || [];
        const assetTagIds = assetTagList.map((tag) => tag.id);
        return tagFilters.every((filterId) => assetTagIds.includes(filterId));
      });
    }

    return filteredAssets;
  }, [
    scanResult,
    getFilteredAssets,
    tagFilters,
    allAssetTags,
    typeFilter,
    searchQuery,
    selectedDirectory,
    sortField,
    sortDirection,
    advancedFilters,
  ]);

  const getTypeLabel = useCallback(
    (type: AssetType): string => t(`assetTypes.${type}` as const),
    [t]
  );

  const handleAssetClick = useCallback(
    (asset: AssetInfo, index: number, e: React.MouseEvent) => {
      if (e.ctrlKey || e.metaKey) {
        togglePath(asset.path);
        setLastClickedIndex(index);
      } else if (e.shiftKey && lastClickedIndex !== null) {
        const start = Math.min(lastClickedIndex, index);
        const end = Math.max(lastClickedIndex, index);
        const range: string[] = [];
        for (let i = start; i <= end; i++) range.push(assets[i].path);
        addPaths(range);
      } else {
        setSelectedAsset(asset);
        setLastClickedIndex(index);
      }
    },
    [lastClickedIndex, assets, setSelectedAsset, togglePath, addPaths]
  );

  const handleCheckChange = useCallback(
    (path: string, checked: boolean) => {
      if (checked) addPaths([path]);
      else removePaths([path]);
    },
    [addPaths, removePaths]
  );

  const handleSelectAll = useCallback(() => {
    if (selectedPaths.size === assets.length) {
      clearSelection();
    } else {
      setSelectedPaths(assets.map((a) => a.path));
    }
  }, [assets, selectedPaths.size, clearSelection, setSelectedPaths]);

  const handleRenameComplete = useCallback(async () => {
    clearSelection();
    setShowBatchRename(false);
    await refreshUndoState();
    // Tags followed the renamed files on the backend; re-sync the store so the
    // moved bindings show immediately rather than after the watcher's ~500ms
    // scanResult refresh re-triggers loadTags.
    await loadTags();
    if (projectPath) {
      openProject(projectPath);
    }
  }, [projectPath, openProject, refreshUndoState, clearSelection, loadTags]);

  // Context menu handlers
  const handleContextMenu = useCallback(
    (e: React.MouseEvent, asset: AssetInfo) => {
      e.preventDefault();
      setContextMenu({
        isOpen: true,
        position: { x: e.clientX, y: e.clientY },
        asset,
      });
    },
    []
  );

  const closeContextMenu = useCallback(() => {
    setContextMenu((prev) => ({ ...prev, isOpen: false }));
  }, []);

  const handleCopyPath = useCallback(async () => {
    if (contextMenu.asset) {
      try {
        await writeText(contextMenu.asset.path);
      } catch (err) {
        console.error("Failed to copy path:", err);
      }
    }
  }, [contextMenu.asset]);

  const handleRevealInFileManager = useCallback(async () => {
    if (contextMenu.asset) {
      try {
        await invoke("show_in_file_manager", { path: contextMenu.asset.path });
      } catch (err) {
        console.error("Failed to show in file manager:", err);
      }
    }
  }, [contextMenu.asset]);

  const handleOpenWithDefaultApp = useCallback(async () => {
    if (contextMenu.asset) {
      try {
        await invoke("open_with_default_app", { path: contextMenu.asset.path });
      } catch (err) {
        console.error("Failed to open with default app:", err);
      }
    }
  }, [contextMenu.asset]);

  const handleRename = useCallback(() => {
    if (contextMenu.asset) {
      setRenameAsset(contextMenu.asset);
    }
  }, [contextMenu.asset]);

  const handleSingleRenameComplete = useCallback(async () => {
    setRenameAsset(null);
    await refreshUndoState();
    // Tag binding followed the file on the backend — re-sync (see batch handler).
    await loadTags();
    if (projectPath) {
      openProject(projectPath);
    }
  }, [projectPath, openProject, refreshUndoState, loadTags]);

  // Rule shared by delete / move / copy / duplicate: if the right-clicked asset
  // is part of the current multi-selection, operate on the whole selection.
  // Otherwise it's a single-asset op even if other items happen to be selected.
  const targetPathsFromContext = useCallback((): string[] | null => {
    const ctxAsset = contextMenu.asset;
    if (!ctxAsset) return null;
    if (selectedPaths.size > 0 && selectedPaths.has(ctxAsset.path)) {
      return Array.from(selectedPaths);
    }
    return [ctxAsset.path];
  }, [contextMenu.asset, selectedPaths]);

  const handleDelete = useCallback(() => {
    const targets = targetPathsFromContext();
    if (targets) setDeleteDialogPaths(targets);
  }, [targetPathsFromContext]);

  const handleMoveTo = useCallback(() => {
    const targets = targetPathsFromContext();
    if (targets) setMoveCopyDialog({ mode: "move", paths: targets });
  }, [targetPathsFromContext]);

  const handleCopyTo = useCallback(() => {
    const targets = targetPathsFromContext();
    if (targets) setMoveCopyDialog({ mode: "copy", paths: targets });
  }, [targetPathsFromContext]);

  const handleAITag = useCallback(() => {
    const targets = targetPathsFromContext();
    if (targets && targets.length > 0) {
      setAiAnalyzeOpen(true, targets);
    }
  }, [targetPathsFromContext, setAiAnalyzeOpen]);

  // Duplicate is same-dir with auto-suffix — no dialog, no target picker. Just
  // fire-and-forget; watcher propagates the new files into the asset list.
  const handleDuplicate = useCallback(async () => {
    const targets = targetPathsFromContext();
    if (!targets) return;
    try {
      const result = await invoke<FileOpResult>("duplicate_assets", {
        paths: targets,
      });
      if (result.errors.length > 0) {
        console.warn("Duplicate had errors:", result.errors);
      }
    } catch (err) {
      console.error("Failed to duplicate:", err);
    }
  }, [targetPathsFromContext]);

  // Del key triggers delete for the current multi-selection when nothing
  // interactive has focus. We deliberately don't handle Del for the
  // single-click `selectedAsset` — that's ambiguous with simply navigating.
  // Ctrl/Cmd+D triggers same-dir duplicate for the same target set.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA"))
        return;

      // Don't fire destructive list shortcuts (Del / Ctrl+D) while any modal is
      // open — an app-level overlay (Settings, Tag Manager, AI, …) or one of
      // this list's own file-op dialogs (rename / batch-rename / delete / move).
      if (
        isBlockingOverlayOpen() ||
        showBatchRename ||
        renameAsset ||
        moveCopyDialog ||
        deleteDialogPaths
      )
        return;

      if (e.key === "Delete") {
        if (selectedPaths.size === 0) return;
        e.preventDefault();
        setDeleteDialogPaths(Array.from(selectedPaths));
        return;
      }

      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d" && !e.shiftKey) {
        if (selectedPaths.size === 0) return;
        e.preventDefault();
        (async () => {
          try {
            await invoke<FileOpResult>("duplicate_assets", {
              paths: Array.from(selectedPaths),
            });
          } catch (err) {
            console.error("Failed to duplicate:", err);
          }
        })();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedPaths, deleteDialogPaths, showBatchRename, renameAsset, moveCopyDialog]);

  // After delete finishes: clear selection for any paths that were actually
  // sent to trash. The filesystem watcher will remove them from scanResult
  // on its own, so no rescan is needed.
  const handleDeleteDone = useCallback(
    (result: { success_paths: string[] }) => {
      if (result.success_paths.length === 0) return;
      removePaths(result.success_paths);
    },
    [removePaths]
  );

  const showCheckbox = selectedPaths.size > 0;

  if (isScanning) {
    return (
      <div className="flex items-center justify-center h-full text-text-secondary">
        {t("assetList.scanning")}
      </div>
    );
  }

  if (!scanResult) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-text-secondary gap-2">
        <Image size={48} className="opacity-50" />
        <p>{t("assetList.openFolder")}</p>
      </div>
    );
  }

  if (assets.length === 0) {
    return (
      <div className="flex items-center justify-center h-full text-text-secondary">
        {t("assetList.noAssets")}
      </div>
    );
  }

  return (
    <>
      <div className="tc-main">
        {/* Selection Toolbar */}
        {selectedPaths.size > 0 && (
          <div className="tc-batch-bar">
            <span className="tc-batch-count">
              <strong>{selectedPaths.size}</strong>
              {t("assetList.selected", "selected")}
            </span>
            <button onClick={handleSelectAll} className="tc-batch-link">
              {selectedPaths.size === assets.length
                ? t("assetList.deselectAll", "Deselect all")
                : t("assetList.selectAll", "Select all")}
            </button>
            <span className="tc-batch-spacer" />
            <BatchTagSelector
              selectedPaths={Array.from(selectedPaths)}
              onOpenManager={() => setTagManagerOpen(true)}
            />
            {aiDirectModeAvailable && (
              <button
                onClick={() =>
                  setAiAnalyzeOpen(true, Array.from(selectedPaths))
                }
                className="tc-batch-action"
                title={t("aiAnalyze.entryLabel")}
              >
                <Sparkles size={13} />
                {t("aiAnalyze.entryLabel")}
              </button>
            )}
            <button
              onClick={() => setShowBatchRename(true)}
              className="tc-batch-action"
              data-primary="true"
            >
              <Edit3 size={13} />
              {t("assetList.batchRename", "Batch Rename")}
            </button>
            <button
              onClick={() =>
                setMoveCopyDialog({
                  mode: "move",
                  paths: Array.from(selectedPaths),
                })
              }
              className="tc-batch-action"
            >
              <Move size={13} />
              {t("assetList.move")}
            </button>
            <button
              onClick={() => setDeleteDialogPaths(Array.from(selectedPaths))}
              className="tc-batch-action"
              data-danger="true"
            >
              <Trash2 size={13} />
              {t("assetList.delete")}
            </button>
            <button
              onClick={clearSelection}
              className="tc-batch-action"
              title={t("assetList.clearSelection", "Clear selection")}
            >
              <X size={13} />
            </button>
          </div>
        )}

        {/* View toolbar */}
        <div className="tc-main-toolbar">
          <div className="tc-toolbar-pills">
            <button
              className="tc-pill"
              data-active={typeFilter === null ? "true" : undefined}
              onClick={() => setTypeFilter(null)}
            >
              {t("assetTypes.all")}
              <span className="mono">{scanResult.total_count}</span>
            </button>
            {FILTER_TYPE_ORDER.filter(
              (type) => (scanResult.type_counts[type] ?? 0) > 0
            ).map((type) => (
              <button
                key={type}
                className="tc-pill"
                data-active={typeFilter === type ? "true" : undefined}
                onClick={() => setTypeFilter(type)}
              >
                <span
                  style={{
                    width: 6,
                    height: 6,
                    borderRadius: "50%",
                    background: `var(--c-${type})`,
                    display: "inline-block",
                  }}
                />
                {t(`assetTypes.${type}` as const)}
                <span className="mono">{scanResult.type_counts[type]}</span>
              </button>
            ))}
          </div>
          <button
            className="tc-pill"
            onClick={toggleSortDirection}
            title={t("assetList.sortToggle")}
          >
            {t("assetList.sortLabel")}:{" "}
            <strong style={{ color: "var(--text)", marginLeft: 4 }}>
              {t(`columns.${sortField}` as const)}{" "}
              {sortDirection === "asc" ? "↑" : "↓"}
            </strong>
          </button>
          <div className="tc-view-toggle">
            <button
              data-active={viewMode === "list" ? "true" : undefined}
              onClick={() => setViewMode("list")}
              title={t("assetList.viewList", "List view")}
              aria-label={t("assetList.viewList", "List view")}
            >
              <List size={12} />
            </button>
            <button
              data-active={viewMode === "grid" ? "true" : undefined}
              onClick={() => setViewMode("grid")}
              title={t("assetList.viewGrid", "Grid view")}
              aria-label={t("assetList.viewGrid", "Grid view")}
            >
              <LayoutGrid size={12} />
            </button>
          </div>
        </div>

        {viewMode === "list" ? (
          <AssetListView
            assets={assets}
            selectedAsset={selectedAsset}
            selectedPaths={selectedPaths}
            showCheckbox={showCheckbox}
            gitStatuses={gitStatuses}
            allAssetTags={allAssetTags}
            sortField={sortField}
            sortDirection={sortDirection}
            setSortField={setSortField}
            onAssetClick={handleAssetClick}
            onContextMenu={handleContextMenu}
            onCheckChange={handleCheckChange}
            getTypeLabel={getTypeLabel}
          />
        ) : (
          <AssetGalleryView
            assets={assets}
            selectedAsset={selectedAsset}
            selectedPaths={selectedPaths}
            gitStatuses={gitStatuses}
            allAssetTags={allAssetTags}
            onAssetClick={handleAssetClick}
            onContextMenu={handleContextMenu}
            getTypeLabel={getTypeLabel}
          />
        )}
      </div>

      {/* Batch Rename Dialog */}
      <BatchRenameDialog
        isOpen={showBatchRename}
        onClose={() => setShowBatchRename(false)}
        selectedPaths={Array.from(selectedPaths)}
        onComplete={handleRenameComplete}
      />

      {/* Single Rename Dialog */}
      {renameAsset && (
        <RenameDialog
          isOpen={true}
          onClose={() => setRenameAsset(null)}
          assetPath={renameAsset.path}
          currentName={renameAsset.name}
          onComplete={handleSingleRenameComplete}
        />
      )}

      {/* Context Menu */}
      {contextMenu.asset && (
        <ContextMenu
          isOpen={contextMenu.isOpen}
          position={contextMenu.position}
          onClose={closeContextMenu}
          assetPath={contextMenu.asset.path}
          assetTags={allAssetTags[contextMenu.asset.path] || []}
          onCopyPath={handleCopyPath}
          onRevealInFileManager={handleRevealInFileManager}
          onOpenWithDefaultApp={handleOpenWithDefaultApp}
          onRename={handleRename}
          onDuplicate={handleDuplicate}
          onMoveTo={handleMoveTo}
          onCopyTo={handleCopyTo}
          onDelete={handleDelete}
          onOpenTagManager={() => setTagManagerOpen(true)}
          onAITag={aiDirectModeAvailable ? handleAITag : undefined}
        />
      )}

      {/* Delete Confirmation */}
      <DeleteConfirmDialog
        isOpen={deleteDialogPaths !== null}
        paths={deleteDialogPaths ?? []}
        onClose={() => setDeleteDialogPaths(null)}
        onDone={handleDeleteDone}
      />

      {/* Move / Copy */}
      <MoveCopyDialog
        isOpen={moveCopyDialog !== null}
        mode={moveCopyDialog?.mode ?? "move"}
        paths={moveCopyDialog?.paths ?? []}
        onClose={() => setMoveCopyDialog(null)}
        onDone={(result) => {
          // Clear selection for paths that actually moved/copied; the watcher
          // takes care of removing / inserting them in scanResult.assets.
          if (result.successes.length === 0) return;
          removePaths(result.successes.map((s) => s.original_path));
          // Moved files carried their tag bindings on the backend — re-sync.
          // (Copy doesn't carry tags; loadTags is a harmless no-change refetch.)
          loadTags();
        }}
      />
    </>
  );
}
