import { useRef, useState, useCallback, useEffect } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Image, Box, Volume2, File, Edit3, X, ArrowUp, ArrowDown, Plus, Pencil, Trash2, AlertCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { useTagsStore } from "../stores/tagsStore";
import { cn, formatFileSize } from "../lib/utils";
import { BatchRenameDialog } from "./BatchRenameDialog";
import { BatchTagSelector, TagBadge } from "./TagSelector";
import { TagManager } from "./TagManager";
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
  onCheckChange: (checked: boolean) => void;
  style: React.CSSProperties;
  typeLabel: string;
  showCheckbox: boolean;
  gitStatus?: GitFileStatus;
  assetTags: Tag[];
  t: (key: string) => string;
}

function AssetRow({
  asset,
  isSelected,
  isChecked,
  onClick,
  onCheckChange,
  style,
  typeLabel,
  showCheckbox,
  gitStatus,
  assetTags,
  t,
}: AssetRowProps) {
  const dimensions =
    asset.metadata?.width && asset.metadata?.height
      ? `${asset.metadata.width} x ${asset.metadata.height}`
      : "-";

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
      <div className="w-24 py-2 px-3 text-text-secondary shrink-0">
        {typeLabel}
      </div>
      <div className="w-24 py-2 px-3 text-text-secondary text-right shrink-0">
        {formatFileSize(asset.size)}
      </div>
      <div className="w-32 py-2 px-3 text-text-secondary text-right font-mono text-xs shrink-0">
        {dimensions}
      </div>
    </div>
  );
}

function SortIndicator({ field, currentField, direction }: { field: SortField; currentField: SortField; direction: "asc" | "desc" }) {
  if (field !== currentField) return null;
  return direction === "asc" ? <ArrowUp size={12} /> : <ArrowDown size={12} />;
}

export function AssetList() {
  const { t } = useTranslation();
  const { scanResult, selectedAsset, setSelectedAsset, getFilteredAssets, isScanning, projectPath, openProject, sortField, sortDirection, setSortField, gitStatuses } =
    useProjectStore();
  const { loadTags, assetTags: allAssetTags } = useTagsStore();
  const parentRef = useRef<HTMLDivElement>(null);
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [showBatchRename, setShowBatchRename] = useState(false);
  const [showTagManager, setShowTagManager] = useState(false);
  const [lastClickedIndex, setLastClickedIndex] = useState<number | null>(null);

  // Load tags when project is loaded
  useEffect(() => {
    if (scanResult) {
      loadTags();
    }
  }, [scanResult, loadTags]);

  const assets = getFilteredAssets();

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

  const handleRenameComplete = useCallback(() => {
    setSelectedPaths(new Set());
    setShowBatchRename(false);
    // Rescan to refresh the file list
    if (projectPath) {
      openProject(projectPath);
    }
  }, [projectPath, openProject]);

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
            {t("assetList.name")}
            <SortIndicator field="name" currentField={sortField} direction={sortDirection} />
          </div>
          <div
            className="w-24 py-2 px-3 shrink-0 flex items-center gap-1 cursor-pointer hover:text-text-primary transition-colors select-none"
            onClick={() => setSortField("type")}
          >
            {t("assetList.type")}
            <SortIndicator field="type" currentField={sortField} direction={sortDirection} />
          </div>
          <div
            className="w-24 py-2 px-3 text-right shrink-0 flex items-center justify-end gap-1 cursor-pointer hover:text-text-primary transition-colors select-none"
            onClick={() => setSortField("size")}
          >
            {t("assetList.size")}
            <SortIndicator field="size" currentField={sortField} direction={sortDirection} />
          </div>
          <div
            className="w-32 py-2 px-3 text-right shrink-0 flex items-center justify-end gap-1 cursor-pointer hover:text-text-primary transition-colors select-none"
            onClick={() => setSortField("dimensions")}
          >
            {t("assetList.dimensions")}
            <SortIndicator field="dimensions" currentField={sortField} direction={sortDirection} />
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
                  onCheckChange={(checked) => handleCheckChange(asset.path, checked)}
                  typeLabel={getTypeLabel(asset.asset_type)}
                  showCheckbox={showCheckbox}
                  gitStatus={gitStatus}
                  assetTags={assetTags}
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

      {/* Tag Manager Dialog */}
      <TagManager
        isOpen={showTagManager}
        onClose={() => setShowTagManager(false)}
      />
    </>
  );
}
