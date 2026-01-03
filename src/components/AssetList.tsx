import { useRef, useState, useCallback, useEffect, useMemo } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Image, Box, Volume2, File, Edit3, X, ArrowUp, ArrowDown, Plus, Pencil, Trash2, AlertCircle, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useProjectStore } from "../stores/projectStore";
import { useTagsStore } from "../stores/tagsStore";
import { useColumnStore, type ColumnId } from "../stores/columnStore";
import { cn, formatFileSize } from "../lib/utils";
import { BatchRenameDialog } from "./BatchRenameDialog";
import { RenameDialog } from "./RenameDialog";
import { BatchTagSelector, TagBadge } from "./TagSelector";
import { TagManager } from "./TagManager";
import { ContextMenu } from "./ContextMenu";
import type { AssetInfo, AssetType, GitFileStatus, Tag } from "../types/asset";
import type { SortField } from "../stores/projectStore";

const ROW_HEIGHT = 36; // Height of each row in pixels

function AssetIcon({ type }: { type: AssetType }) {
  const iconProps = { size: 16, className: "shrink-0" };

  switch (type) {
    case "texture":
      return <Image {...iconProps} className="text-green-400" />;
    case "model":
      return <Box {...iconProps} className="text-blue-400" />;
    case "audio":
      return <Volume2 {...iconProps} className="text-yellow-400" />;
    default:
      return <File {...iconProps} className="text-gray-400" />;
  }
}

function GitStatusBadge({ status, t }: { status: GitFileStatus; t: (key: string) => string }) {
  const configs: Record<GitFileStatus, { icon: React.ReactNode; color: string; bg: string } | null> = {
    new: { icon: <Plus size={10} />, color: "text-green-400", bg: "bg-green-400/20" },
    modified: { icon: <Pencil size={10} />, color: "text-yellow-400", bg: "bg-yellow-400/20" },
    deleted: { icon: <Trash2 size={10} />, color: "text-red-400", bg: "bg-red-400/20" },
    renamed: { icon: <Pencil size={10} />, color: "text-blue-400", bg: "bg-blue-400/20" },
    untracked: { icon: <Plus size={10} />, color: "text-gray-400", bg: "bg-gray-400/20" },
    conflicted: { icon: <AlertCircle size={10} />, color: "text-red-500", bg: "bg-red-500/20" },
    typechange: null,
    ignored: null,
    unchanged: null,
  };

  const config = configs[status];
  if (!config) return null;

  return (
    <span
      className={cn("inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[10px] font-medium", config.color, config.bg)}
      title={t(`git.status.${status}`)}
    >
      {config.icon}
    </span>
  );
}

interface AssetRowProps {
  asset: AssetInfo;
  isSelected: boolean;
  isChecked: boolean;
  onClick: (e: React.MouseEvent) => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onCheckChange: (checked: boolean) => void;
  style: React.CSSProperties;
  typeLabel: string;
  showCheckbox: boolean;
  gitStatus?: GitFileStatus;
  assetTags: Tag[];
  visibleColumns: ColumnId[];
  t: (key: string) => string;
}

function AssetRow({
  asset,
  isSelected,
  isChecked,
  onClick,
  onContextMenu,
  onCheckChange,
  style,
  typeLabel,
  showCheckbox,
  gitStatus,
  assetTags,
  visibleColumns,
  t,
}: AssetRowProps) {
  const dimensions =
    asset.metadata?.width && asset.metadata?.height
      ? `${asset.metadata.width} x ${asset.metadata.height}`
      : "-";

  const getColumnValue = (columnId: ColumnId): string => {
    switch (columnId) {
      case "type":
        return typeLabel;
      case "size":
        return formatFileSize(asset.size);
      case "dimensions":
        return dimensions;
      case "vertices":
        return asset.metadata?.vertex_count?.toLocaleString() ?? "-";
      case "faces":
        return asset.metadata?.face_count?.toLocaleString() ?? "-";
      case "duration":
        if (asset.metadata?.duration_secs) {
          const sec = asset.metadata.duration_secs;
          const min = Math.floor(sec / 60);
          const s = Math.floor(sec % 60);
          return `${min}:${s.toString().padStart(2, "0")}`;
        }
        return "-";
      case "sampleRate":
        return asset.metadata?.sample_rate ? `${(asset.metadata.sample_rate / 1000).toFixed(1)} kHz` : "-";
      case "extension":
        return asset.extension || "-";
      default:
        return "-";
    }
  };

  const getColumnWidth = (columnId: ColumnId): string => {
    switch (columnId) {
      case "type": return "w-24";
      case "size": return "w-24";
      case "dimensions": return "w-32";
      case "vertices": return "w-24";
      case "faces": return "w-24";
      case "duration": return "w-20";
      case "sampleRate": return "w-24";
      case "extension": return "w-20";
      default: return "w-24";
    }
  };

  return (
    <div
      className={cn(
        "flex items-center border-b border-border cursor-pointer transition-colors text-sm",
        "hover:bg-background",
        isSelected && "bg-primary/20",
        isChecked && "bg-primary/10"
      )}
      style={style}
      onClick={onClick}
      onContextMenu={onContextMenu}
    >
      {showCheckbox && (
        <div className="w-8 py-2 px-2 shrink-0">
          <input
            type="checkbox"
            checked={isChecked}
            onChange={(e) => {
              e.stopPropagation();
              onCheckChange(e.target.checked);
            }}
            className="w-4 h-4 accent-primary cursor-pointer"
          />
        </div>
      )}
      <div className="flex-1 py-2 px-3 min-w-0">
        <div className="flex items-center gap-2">
          <AssetIcon type={asset.asset_type} />
          <span className="truncate">{asset.name}</span>
          {gitStatus && gitStatus !== "unchanged" && <GitStatusBadge status={gitStatus} t={t} />}
          {assetTags.slice(0, 2).map((tag) => (
            <TagBadge key={tag.id} tag={tag} />
          ))}
          {assetTags.length > 2 && (
            <span className="text-[10px] text-text-secondary">+{assetTags.length - 2}</span>
          )}
        </div>
      </div>
      {visibleColumns.filter(c => c !== "name").map((columnId) => (
        <div
          key={columnId}
          className={cn(
            "py-2 px-3 text-text-secondary shrink-0",
            getColumnWidth(columnId),
            columnId !== "type" && "text-right",
            (columnId === "dimensions" || columnId === "vertices" || columnId === "faces") && "font-mono text-xs"
          )}
        >
          {getColumnValue(columnId)}
        </div>
      ))}
    </div>
  );
}

function SortIndicator({ field, currentField, direction }: { field: SortField; currentField: SortField; direction: "asc" | "desc" }) {
  if (field !== currentField) return null;
  return direction === "asc" ? <ArrowUp size={12} /> : <ArrowDown size={12} />;
}

// Column configuration dropdown
function ColumnConfigDropdown({ t }: { t: (key: string) => string }) {
  const { columns, setColumnVisible } = useColumnStore();
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="p-1.5 text-text-secondary hover:text-text-primary hover:bg-background rounded transition-colors"
        title={t("columns.configure")}
      >
        <Settings size={14} />
      </button>
      {isOpen && (
        <div className="absolute right-0 top-full mt-1 bg-card-bg border border-border rounded-lg shadow-lg z-50 py-1 min-w-[160px]">
          {columns.map((col) => (
            <label
              key={col.id}
              className="flex items-center gap-2 px-3 py-1.5 text-sm cursor-pointer hover:bg-background transition-colors"
            >
              <input
                type="checkbox"
                checked={col.visible}
                onChange={(e) => setColumnVisible(col.id, e.target.checked)}
                disabled={col.id === "name"}
                className="w-4 h-4 accent-primary"
              />
              <span className={col.id === "name" ? "text-text-secondary" : ""}>
                {t(`columns.${col.id}`)}
              </span>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

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
    gitStatuses,
    typeFilter,
    searchQuery,
    selectedDirectory,
    refreshUndoState,
  } = useProjectStore();
  const { loadTags, assetTags: allAssetTags, tagFilter } = useTagsStore();
  const { columns } = useColumnStore();
  const parentRef = useRef<HTMLDivElement>(null);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [showBatchRename, setShowBatchRename] = useState(false);
  const [showTagManager, setShowTagManager] = useState(false);
  const [lastClickedIndex, setLastClickedIndex] = useState<number | null>(null);

  // Single rename state
  const [renameAsset, setRenameAsset] = useState<AssetInfo | null>(null);

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{
    isOpen: boolean;
    position: { x: number; y: number };
    asset: AssetInfo | null;
  }>({ isOpen: false, position: { x: 0, y: 0 }, asset: null });

  // Get visible columns
  const visibleColumns = columns.filter((c) => c.visible).map((c) => c.id);

  // Load tags when project is loaded
  useEffect(() => {
    if (scanResult) {
      loadTags();
    }
  }, [scanResult, loadTags]);

  // Get filtered assets with tag filtering applied
  // Note: Include typeFilter, searchQuery, selectedDirectory, sortField, sortDirection in deps
  // to ensure re-render when these change (getFilteredAssets reads them internally)
  const assets = useMemo(() => {
    let filteredAssets = getFilteredAssets();

    // Apply tag filter if active
    if (tagFilter) {
      filteredAssets = filteredAssets.filter((asset) => {
        const assetTagList = allAssetTags[asset.path] || [];
        return assetTagList.some((tag) => tag.id === tagFilter);
      });
    }

    return filteredAssets;
  }, [getFilteredAssets, tagFilter, allAssetTags, typeFilter, searchQuery, selectedDirectory, sortField, sortDirection]);

  const virtualizer = useVirtualizer({
    count: assets.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10, // Render 10 extra items above/below visible area
  });

  const getTypeLabel = (type: AssetType): string => {
    const key = `assetTypes.${type}` as const;
    return t(key);
  };

  const handleAssetClick = useCallback(
    (asset: AssetInfo, index: number, e: React.MouseEvent) => {
      if (e.ctrlKey || e.metaKey) {
        // Toggle selection with Ctrl/Cmd
        setSelectedPaths((prev) => {
          const next = new Set(prev);
          if (next.has(asset.path)) {
            next.delete(asset.path);
          } else {
            next.add(asset.path);
          }
          return next;
        });
        setLastClickedIndex(index);
      } else if (e.shiftKey && lastClickedIndex !== null) {
        // Range selection with Shift
        const start = Math.min(lastClickedIndex, index);
        const end = Math.max(lastClickedIndex, index);
        setSelectedPaths((prev) => {
          const next = new Set(prev);
          for (let i = start; i <= end; i++) {
            next.add(assets[i].path);
          }
          return next;
        });
      } else {
        // Normal click - select single asset
        setSelectedAsset(asset);
        setLastClickedIndex(index);
      }
    },
    [lastClickedIndex, assets, setSelectedAsset]
  );

  const handleCheckChange = useCallback((path: string, checked: boolean) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (checked) {
        next.add(path);
      } else {
        next.delete(path);
      }
      return next;
    });
  }, []);

  const handleSelectAll = useCallback(() => {
    if (selectedPaths.size === assets.length) {
      setSelectedPaths(new Set());
    } else {
      setSelectedPaths(new Set(assets.map((a) => a.path)));
    }
  }, [assets, selectedPaths.size]);

  const clearSelection = useCallback(() => {
    setSelectedPaths(new Set());
  }, []);

  const handleRenameComplete = useCallback(async () => {
    setSelectedPaths(new Set());
    setShowBatchRename(false);
    // Refresh undo state after rename
    await refreshUndoState();
    // Rescan to refresh the file list
    if (projectPath) {
      openProject(projectPath);
    }
  }, [projectPath, openProject, refreshUndoState]);

  // Context menu handlers
  const handleContextMenu = useCallback((e: React.MouseEvent, asset: AssetInfo) => {
    e.preventDefault();
    setContextMenu({
      isOpen: true,
      position: { x: e.clientX, y: e.clientY },
      asset,
    });
  }, []);

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

  const handleRevealInFinder = useCallback(async () => {
    if (contextMenu.asset) {
      try {
        await invoke("reveal_in_finder", { path: contextMenu.asset.path });
      } catch (err) {
        console.error("Failed to reveal in finder:", err);
      }
    }
  }, [contextMenu.asset]);

  const handleRename = useCallback(() => {
    if (contextMenu.asset) {
      // Use single rename dialog for individual asset
      setRenameAsset(contextMenu.asset);
    }
  }, [contextMenu.asset]);

  const handleSingleRenameComplete = useCallback(async () => {
    setRenameAsset(null);
    // Refresh undo state after rename
    await refreshUndoState();
    // Rescan to refresh the file list
    if (projectPath) {
      openProject(projectPath);
    }
  }, [projectPath, openProject, refreshUndoState]);

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

  const virtualItems = virtualizer.getVirtualItems();

  return (
    <>
      <div className="h-full flex flex-col overflow-hidden">
        {/* Selection Toolbar */}
        {selectedPaths.size > 0 && (
          <div className="flex items-center justify-between bg-primary/10 border-b border-primary/30 px-3 py-2 shrink-0">
            <div className="flex items-center gap-3">
              <span className="text-sm text-primary font-medium">
                {selectedPaths.size} {t("assetList.selected", "selected")}
              </span>
              <button
                onClick={handleSelectAll}
                className="text-sm text-text-secondary hover:text-text-primary transition-colors"
              >
                {selectedPaths.size === assets.length
                  ? t("assetList.deselectAll", "Deselect all")
                  : t("assetList.selectAll", "Select all")}
              </button>
            </div>
            <div className="flex items-center gap-2">
              <BatchTagSelector
                selectedPaths={Array.from(selectedPaths)}
                onOpenManager={() => setShowTagManager(true)}
              />
              <button
                onClick={() => setShowBatchRename(true)}
                className="flex items-center gap-1.5 px-3 py-1.5 text-sm bg-primary text-white rounded hover:bg-primary/90 transition-colors"
              >
                <Edit3 size={14} />
                {t("assetList.batchRename", "Batch Rename")}
              </button>
              <button
                onClick={clearSelection}
                className="p-1.5 text-text-secondary hover:text-text-primary transition-colors"
                title={t("assetList.clearSelection", "Clear selection")}
              >
                <X size={16} />
              </button>
            </div>
          </div>
        )}

        {/* Header */}
        <div className="flex items-center bg-card-bg border-b border-border text-text-secondary text-sm font-medium shrink-0">
          {showCheckbox && <div className="w-8 py-2 px-2 shrink-0" />}
          <div
            className="flex-1 py-2 px-3 flex items-center gap-1 cursor-pointer hover:text-text-primary transition-colors select-none"
            onClick={() => setSortField("name")}
          >
            {t("columns.name")}
            <SortIndicator field="name" currentField={sortField} direction={sortDirection} />
          </div>
          {visibleColumns.filter(c => c !== "name").map((columnId) => {
            const getWidth = (id: ColumnId) => {
              switch (id) {
                case "type": return "w-24";
                case "size": return "w-24";
                case "dimensions": return "w-32";
                case "vertices": return "w-24";
                case "faces": return "w-24";
                case "duration": return "w-20";
                case "sampleRate": return "w-24";
                case "extension": return "w-20";
                default: return "w-24";
              }
            };
            return (
              <div
                key={columnId}
                className={cn(
                  "py-2 px-3 shrink-0 flex items-center gap-1 transition-colors select-none cursor-pointer hover:text-text-primary",
                  getWidth(columnId),
                  columnId !== "type" && "justify-end text-right"
                )}
                onClick={() => setSortField(columnId as SortField)}
              >
                {t(`columns.${columnId}`)}
                <SortIndicator field={columnId as SortField} currentField={sortField} direction={sortDirection} />
              </div>
            );
          })}
          <div className="py-1 px-2 shrink-0">
            <ColumnConfigDropdown t={t} />
          </div>
        </div>

        {/* Virtual List */}
        <div ref={parentRef} className="flex-1 overflow-auto">
          <div
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              width: "100%",
              position: "relative",
            }}
          >
            {virtualItems.map((virtualItem) => {
              const asset = assets[virtualItem.index];
              const gitStatus = gitStatuses[asset.path];
              const assetTags = allAssetTags[asset.path] || [];
              return (
                <AssetRow
                  key={asset.path}
                  asset={asset}
                  isSelected={selectedAsset?.path === asset.path}
                  isChecked={selectedPaths.has(asset.path)}
                  onClick={(e) => handleAssetClick(asset, virtualItem.index, e)}
                  onContextMenu={(e) => handleContextMenu(e, asset)}
                  onCheckChange={(checked) => handleCheckChange(asset.path, checked)}
                  typeLabel={getTypeLabel(asset.asset_type)}
                  showCheckbox={showCheckbox}
                  gitStatus={gitStatus}
                  assetTags={assetTags}
                  visibleColumns={visibleColumns}
                  t={t}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    height: `${virtualItem.size}px`,
                    transform: `translateY(${virtualItem.start}px)`,
                  }}
                />
              );
            })}
          </div>
        </div>
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

      {/* Tag Manager Dialog */}
      <TagManager
        isOpen={showTagManager}
        onClose={() => setShowTagManager(false)}
      />

      {/* Context Menu */}
      {contextMenu.asset && (
        <ContextMenu
          isOpen={contextMenu.isOpen}
          position={contextMenu.position}
          onClose={closeContextMenu}
          assetPath={contextMenu.asset.path}
          assetTags={allAssetTags[contextMenu.asset.path] || []}
          onCopyPath={handleCopyPath}
          onRevealInFinder={handleRevealInFinder}
          onRename={handleRename}
          onOpenTagManager={() => setShowTagManager(true)}
        />
      )}
    </>
  );
}
