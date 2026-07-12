import { useRef, useState, useEffect, useMemo } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Image,
  Box,
  Volume2,
  Video,
  File,
  ArrowUp,
  ArrowDown,
  Settings,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useColumnStore, type ColumnId } from "../stores/columnStore";
import { useSettingsStore } from "../stores/settingsStore";
import { cn, formatFileSize } from "../lib/utils";
import type {
  AssetInfo,
  AssetType,
  GitFileStatus,
  Tag,
  AssetTagsMap,
  GitStatusMap,
} from "../types/asset";
import { useProjectStore } from "../stores/projectStore";
import type { SortField, SortDirection } from "../stores/projectStore";
import { TagBadge } from "./TagSelector";
import { GitStatusBadge } from "./GitStatusBadge";
import { DccSourceBadge } from "./DccSourceBadge";

const ROW_HEIGHT = 40; // matches .tc-row + 22px glyph + padding
const MIN_COL_WIDTH = 60;
const MAX_COL_WIDTH = 500;

function AssetIcon({ type }: { type: AssetType }) {
  const icon = (() => {
    const size = 13;
    switch (type) {
      case "texture": return <Image size={size} />;
      case "model":   return <Box size={size} />;
      case "audio":   return <Volume2 size={size} />;
      case "video":   return <Video size={size} />;
      default:        return <File size={size} />;
    }
  })();
  return (
    <span className="tc-asset-glyph" data-type={type}>
      {icon}
    </span>
  );
}

/// Drag-to-resize grab handle on the right edge of a header cell. Uses
/// document-level mousemove/mouseup so cursor leaving the handle mid-drag
/// doesn't abort. Stops propagation to avoid triggering the parent header's
/// sort-on-click.
function ColumnResizeHandle({
  columnId,
  currentWidth,
}: {
  columnId: ColumnId;
  currentWidth: number;
}) {
  const setColumnWidth = useColumnStore((s) => s.setColumnWidth);

  const onMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startWidth = currentWidth;

    const onMove = (me: MouseEvent) => {
      const delta = me.clientX - startX;
      const next = Math.max(
        MIN_COL_WIDTH,
        Math.min(MAX_COL_WIDTH, startWidth + delta)
      );
      setColumnWidth(columnId, next);
    };
    const onUp = () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  };

  return (
    <div
      onMouseDown={onMouseDown}
      onClick={(e) => e.stopPropagation()}
      className="tc-col-resize"
      aria-hidden
    />
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
  showGitStatusIndicators: boolean;
  assetTags: Tag[];
  visibleColumns: ColumnId[];
  columnWidths: Record<string, number>;
  /// Max `vertex_count` across the visible dataset, used to scale the inline
  /// `.tc-bar` viz for each row. 1 acts as a safe divisor when the column
  /// has no models or no vertex data.
  maxVertices: number;
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
  showGitStatusIndicators,
  assetTags,
  visibleColumns,
  columnWidths,
  maxVertices,
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
        return asset.metadata?.sample_rate
          ? `${(asset.metadata.sample_rate / 1000).toFixed(1)} kHz`
          : "-";
      case "extension":
        return asset.extension || "-";
      default:
        return "-";
    }
  };

  return (
    <div
      className="tc-row"
      data-selected={isSelected ? "true" : undefined}
      data-checked={isChecked ? "true" : undefined}
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
      <div
        className="py-2 px-3 shrink-0 min-w-0 overflow-hidden"
        style={{ width: columnWidths.name }}
      >
        <div className="tc-name-cell">
          <AssetIcon type={asset.asset_type} />
          <span className="tc-name">{asset.name}</span>
          {asset.metadata?.dcc_source_kind && (
            <DccSourceBadge kind={asset.metadata.dcc_source_kind} t={t} />
          )}
          {showGitStatusIndicators && gitStatus && gitStatus !== "unchanged" && (
            <GitStatusBadge status={gitStatus} t={t} />
          )}
        </div>
      </div>
      {visibleColumns
        .filter((c) => c !== "name")
        .map((columnId) => {
          const width = columnWidths[columnId];
          if (columnId === "tags") {
            return (
              <div
                key={columnId}
                className="py-2 px-3 shrink-0 overflow-hidden"
                style={{ width }}
              >
                <div className="flex items-center gap-1 overflow-hidden">
                  {assetTags.slice(0, 2).map((tag) => (
                    <TagBadge key={tag.id} tag={tag} />
                  ))}
                  {assetTags.length > 2 && (
                    <span className="text-[10px] text-text-secondary">
                      +{assetTags.length - 2}
                    </span>
                  )}
                  {assetTags.length === 0 && (
                    <span className="text-text-secondary text-xs">-</span>
                  )}
                </div>
              </div>
            );
          }
          if (columnId === "vertices") {
            const count = asset.metadata?.vertex_count;
            return (
              <div
                key={columnId}
                className="py-2 px-3 text-text-secondary shrink-0 overflow-hidden truncate text-right font-mono text-xs"
                style={{ width }}
              >
                {count != null ? (
                  <>
                    <span
                      className="tc-bar"
                      style={
                        {
                          ["--bar"]: `${Math.round((count / maxVertices) * 100)}%`,
                        } as React.CSSProperties
                      }
                    />
                    {count.toLocaleString()}
                  </>
                ) : (
                  "-"
                )}
              </div>
            );
          }
          return (
            <div
              key={columnId}
              className={cn(
                "py-2 px-3 text-text-secondary shrink-0 overflow-hidden truncate",
                columnId !== "type" && "text-right",
                (columnId === "dimensions" || columnId === "faces") &&
                  "font-mono text-xs"
              )}
              style={{ width }}
            >
              {getColumnValue(columnId)}
            </div>
          );
        })}
    </div>
  );
}

function SortIndicator({
  field,
  currentField,
  direction,
}: {
  field: SortField;
  currentField: SortField;
  direction: SortDirection;
}) {
  if (field !== currentField) return null;
  return direction === "asc" ? <ArrowUp size={12} /> : <ArrowDown size={12} />;
}

function ColumnConfigDropdown({ t }: { t: (key: string) => string }) {
  const { columns, setColumnVisible } = useColumnStore();
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(e.target as Node)
      ) {
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

export interface AssetListViewProps {
  assets: AssetInfo[];
  selectedAsset: AssetInfo | null;
  selectedPaths: Set<string>;
  showCheckbox: boolean;
  gitStatuses: GitStatusMap;
  allAssetTags: AssetTagsMap;
  sortField: SortField;
  sortDirection: SortDirection;
  setSortField: (field: SortField) => void;
  onAssetClick: (asset: AssetInfo, index: number, e: React.MouseEvent) => void;
  onContextMenu: (e: React.MouseEvent, asset: AssetInfo) => void;
  onCheckChange: (path: string, checked: boolean) => void;
  getTypeLabel: (type: AssetType) => string;
}

export function AssetListView({
  assets,
  selectedAsset,
  selectedPaths,
  showCheckbox,
  gitStatuses,
  allAssetTags,
  sortField,
  sortDirection,
  setSortField,
  onAssetClick,
  onContextMenu,
  onCheckChange,
  getTypeLabel,
}: AssetListViewProps) {
  const { t } = useTranslation();
  const { columns } = useColumnStore();
  const { showGitStatusIndicators } = useSettingsStore();
  const parentRef = useRef<HTMLDivElement>(null);

  const visibleColumns = columns.filter((c) => c.visible).map((c) => c.id);
  const columnWidths = useMemo<Record<string, number>>(() => {
    const map: Record<string, number> = {};
    for (const c of columns) map[c.id] = c.width;
    return map;
  }, [columns]);

  // Total intrinsic row width = sum of all visible columns + checkbox lane
  // (when active) + the trailing ColumnConfigDropdown wrapper (~36px).
  // Used to size the header and the virtualizer's spacer so the row can
  // exceed the viewport horizontally — critical for column resize: with a
  // shrink-0 layout, growing one column has to make total content wider,
  // not push siblings around.
  const totalRowWidth = useMemo(() => {
    let sum = 0;
    if (showCheckbox) sum += 32;
    for (const id of visibleColumns) sum += columnWidths[id] ?? 0;
    sum += 36; // ColumnConfigDropdown trailing slot
    return sum;
  }, [visibleColumns, columnWidths, showCheckbox]);

  // Max vertex count across the dataset for the inline `.tc-bar` viz.
  // Recomputed when assets change; for 10k+ rows this is a single pass
  // and the result memoizes through React's normal flow.
  const maxVertices = useMemo(() => {
    let max = 0;
    for (const a of assets) {
      const v = a.metadata?.vertex_count;
      if (v && v > max) max = v;
    }
    return max || 1;
  }, [assets]);

  const virtualizer = useVirtualizer({
    count: assets.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10,
  });

  // Scroll the selected row into view on selection change / locate pulse.
  // align:"auto" is a no-op when the row is already visible, so ordinary
  // clicks don't jump the list; the pulse makes a repeat "locate" of the
  // same asset scroll again (assets is deliberately not a dependency —
  // re-filtering while searching must not fight the user's scrolling).
  const locatePulse = useProjectStore((s) => s.locatePulse);
  useEffect(() => {
    if (!selectedAsset) return;
    const idx = assets.findIndex((a) => a.path === selectedAsset.path);
    if (idx >= 0) {
      virtualizer.scrollToIndex(idx, { align: "auto" });
    }
  }, [selectedAsset?.path, locatePulse]);

  const virtualItems = virtualizer.getVirtualItems();

  return (
    <div ref={parentRef} className="tc-list-scroll">
      <div
        className="tc-list-header"
        style={{ width: totalRowWidth, minWidth: "100%" }}
      >
        {showCheckbox && <div className="w-8 py-2 px-2 shrink-0" />}
        <div
          className="py-2 px-3 shrink-0 flex items-center gap-1 cursor-pointer hover:text-text-primary transition-colors select-none relative overflow-hidden"
          style={{ width: columnWidths.name }}
          onClick={() => setSortField("name")}
        >
          <span className="truncate">{t("columns.name")}</span>
          <SortIndicator
            field="name"
            currentField={sortField}
            direction={sortDirection}
          />
          <ColumnResizeHandle
            columnId="name"
            currentWidth={columnWidths.name}
          />
        </div>
        {visibleColumns
          .filter((c) => c !== "name")
          .map((columnId) => {
            const width = columnWidths[columnId];
            const isSortable = columnId !== "tags";
            return (
              <div
                key={columnId}
                className={cn(
                  "py-2 px-3 shrink-0 flex items-center gap-1 transition-colors select-none relative overflow-hidden",
                  columnId !== "type" &&
                    columnId !== "tags" &&
                    "justify-end text-right",
                  isSortable && "cursor-pointer hover:text-text-primary"
                )}
                style={{ width }}
                onClick={() =>
                  isSortable && setSortField(columnId as SortField)
                }
              >
                <span className="truncate">{t(`columns.${columnId}`)}</span>
                {isSortable && (
                  <SortIndicator
                    field={columnId as SortField}
                    currentField={sortField}
                    direction={sortDirection}
                  />
                )}
                <ColumnResizeHandle
                  columnId={columnId}
                  currentWidth={width}
                />
              </div>
            );
          })}
        <div className="py-1 px-2 shrink-0">
          <ColumnConfigDropdown t={t} />
        </div>
      </div>

      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          width: totalRowWidth,
          minWidth: "100%",
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
                onClick={(e) => onAssetClick(asset, virtualItem.index, e)}
                onContextMenu={(e) => onContextMenu(e, asset)}
                onCheckChange={(checked) =>
                  onCheckChange(asset.path, checked)
                }
                typeLabel={getTypeLabel(asset.asset_type)}
                showCheckbox={showCheckbox}
                gitStatus={gitStatus}
                showGitStatusIndicators={showGitStatusIndicators}
                assetTags={assetTags}
                visibleColumns={visibleColumns}
                columnWidths={columnWidths}
                maxVertices={maxVertices}
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
  );
}
