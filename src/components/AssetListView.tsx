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
import { cn, formatFileSize, formatDuration } from "../lib/utils";
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
import { intentFromMouse, type SelectIntent } from "../lib/selectIntent";
import { registerAssetListFocus } from "../lib/menuActions";

const ROW_HEIGHT = 40; // matches .tc-row + 22px glyph + padding
/// Height of the sticky header strip. Must track `.tc-list-header`'s `height` in
/// redesign-components.css: the header sits inside the same scroll box as the
/// rows, so this is both the virtualizer's `scrollMargin` and non-list space.
const LIST_HEADER_HEIGHT = 30;
const MIN_COL_WIDTH = 60;
const MAX_COL_WIDTH = 500;

/// DOM id for the row at `index`, referenced by the container's
/// `aria-activedescendant`. Keyed by index, not path: paths may contain spaces,
/// which are illegal in an HTML id.
const rowDomId = (index: number) => `tc-asset-row-${index}`;

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

/// Props that make a header cell operable from the keyboard. The header row is a
/// strip of flex divs rather than a real table, so the role, the tab stop and
/// Enter are spelled out. No `aria-sort`: there is no grid above these divs.
function sortableHeaderProps(activate: () => void) {
  return {
    role: "button",
    tabIndex: 0,
    onClick: activate,
    onKeyDown: (e: React.KeyboardEvent) => {
      // Space as well as Enter: a native button takes both, and the preventDefault
      // is what stops Space scrolling the list out from under the header.
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        activate();
      }
    },
  };
}

/// Drag-to-resize grab handle on the right edge of a header cell. Uses
/// document-level mousemove/mouseup so the cursor leaving the handle mid-drag
/// does not abort, and stops propagation so it never triggers sort-on-click.
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
  domId: string;
  /// 1-based position in the whole list and the length of that list. Under
  /// virtualization only a screenful of rows exists in the DOM, so assistive tech
  /// would otherwise announce "12 of 31" in a ten-thousand-file project.
  posInSet: number;
  setSize: number;
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
  t: (key: string, opts?: Record<string, unknown>) => string;
}

function AssetRow({
  asset,
  domId,
  posInSet,
  setSize,
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
  // Models carry no width/height; surface their vertex count here instead of a
  // bare "-" (the dedicated vertices column is off by default). Images keep the
  // W×H they already had; anything with neither stays "-".
  const dimensions =
    asset.metadata?.width && asset.metadata?.height
      ? `${asset.metadata.width} x ${asset.metadata.height}`
      : asset.metadata?.vertex_count != null
        ? t("assetList.vertsInline", { n: asset.metadata.vertex_count.toLocaleString() })
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
        return asset.metadata?.duration_secs
          ? formatDuration(asset.metadata.duration_secs)
          : "-";
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
      id={domId}
      role="option"
      aria-selected={isSelected}
      aria-posinset={posInSet}
      aria-setsize={setSize}
      className="tc-row"
      data-selected={isSelected ? "true" : undefined}
      data-checked={isChecked ? "true" : undefined}
      style={style}
      onClick={onClick}
      onContextMenu={onContextMenu}
    >
      {/* Batch-select checkbox. The 32px lane is always reserved (no layout
          shift); the box is revealed on row hover, when the row is checked, or
          while a batch selection is active (data-force) — see `.tc-row-check`
          in redesign-components.css. Clicking it toggles batch selection only,
          not the single-click preview. */}
      <div
        className="tc-row-check w-8 py-2 px-2 shrink-0"
        data-force={showCheckbox ? "true" : undefined}
      >
        <input
          type="checkbox"
          checked={isChecked}
          onChange={(e) => {
            e.stopPropagation();
            onCheckChange(e.target.checked);
          }}
          onClick={(e) => e.stopPropagation()}
          // A click must toggle without taking focus: focus lives on the list
          // itself, and parking it on an input here would make the guard at the
          // top of the key handler reject every arrow key.
          onMouseDown={(e) => e.preventDefault()}
          className="w-4 h-4 accent-primary cursor-pointer"
          aria-label={t("assetList.selectForBatch")}
          // Out of the tab order: a screenful of rows is a screenful of
          // identically-labelled checkboxes. Ticking an individual box is a
          // mouse-only action.
          tabIndex={-1}
        />
      </div>
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
                    <span className="text-[10px] text-ink-2">
                      +{assetTags.length - 2}
                    </span>
                  )}
                  {assetTags.length === 0 && (
                    <span className="text-ink-2 text-xs">-</span>
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
                className="py-2 px-3 text-ink-2 shrink-0 overflow-hidden truncate text-right font-mono text-xs"
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
                "py-2 px-3 text-ink-2 shrink-0 overflow-hidden truncate",
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
        className="p-1.5 text-ink-2 hover:text-ink hover:bg-base rounded transition-colors"
        title={t("columns.configure")}
      >
        <Settings size={14} />
      </button>
      {isOpen && (
        <div className="absolute right-0 top-full mt-1 bg-panel border border-line rounded-lg shadow-lg z-50 py-1 min-w-[160px]">
          {columns.map((col) => (
            <label
              key={col.id}
              className="flex items-center gap-2 px-3 py-1.5 text-sm cursor-pointer hover:bg-base transition-colors"
            >
              <input
                type="checkbox"
                checked={col.visible}
                onChange={(e) => setColumnVisible(col.id, e.target.checked)}
                disabled={col.id === "name"}
                className="w-4 h-4 accent-primary"
              />
              <span className={col.id === "name" ? "text-ink-2" : ""}>
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
  onAssetClick: (asset: AssetInfo, index: number, intent: SelectIntent) => void;
  onContextMenu: (e: React.MouseEvent, asset: AssetInfo) => void;
  onCheckChange: (path: string, checked: boolean) => void;
  getTypeLabel: (type: AssetType) => string;
  onActivate: (asset: AssetInfo) => void;
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
  onActivate,
}: AssetListViewProps) {
  const { t } = useTranslation();
  const { columns } = useColumnStore();
  const { showGitStatusIndicators } = useSettingsStore();
  const parentRef = useRef<HTMLDivElement>(null);
  // The rows container, which is what actually takes focus — see the note on
  // its `role`/`tabIndex` below.
  const listRef = useRef<HTMLDivElement>(null);

  const visibleColumns = columns.filter((c) => c.visible).map((c) => c.id);
  const columnWidths = useMemo<Record<string, number>>(() => {
    const map: Record<string, number> = {};
    for (const c of columns) map[c.id] = c.width;
    return map;
  }, [columns]);

  // Total intrinsic row width = visible columns + the 32px checkbox lane + the
  // ~36px column-config wrapper. Sizes the header and the virtualizer's spacer so
  // a resized column widens the content instead of pushing siblings around.
  const totalRowWidth = useMemo(() => {
    let sum = 32; // batch-select checkbox lane (always reserved)
    for (const id of visibleColumns) sum += columnWidths[id] ?? 0;
    sum += 36; // ColumnConfigDropdown trailing slot
    return sum;
  }, [visibleColumns, columnWidths]);

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
    // The rows sit in a sibling of the sticky header inside the same scroll box,
    // so a row's real position is one header height further down. Without this
    // every downward jump scrolls 30px short.
    scrollMargin: LIST_HEADER_HEIGHT,
    // The counterpart on the way up: "top of the scroll box" is behind the sticky
    // header, so this reserves its strip and is what lets `align: "auto"` notice a
    // row hidden under it.
    scrollPaddingStart: LIST_HEADER_HEIGHT,
  });

  // Focus is not its own state: the focused row and the previewed row are the same
  // row. -1 means no cursor right now. Memoized because this component re-renders
  // on every scroll notification and twice per filesystem event.
  const focusedIndex = useMemo(
    () =>
      selectedAsset
        ? assets.findIndex((a) => a.path === selectedAsset.path)
        : -1,
    [assets, selectedAsset?.path]
  );

  // Where the cursor last was, kept so it can be restored if the file under it is
  // deleted or a filter hides it. A ref, not state. -1 means no cursor has ever
  // been placed, which differs from "not on a visible row right now".
  const lastIndexRef = useRef(-1);
  if (focusedIndex >= 0) lastIndexRef.current = focusedIndex;

  // The store publishes "the previewed asset was dropped because its file was
  // removed" as a counter, and this depends on the counter alone: the selection
  // also goes away for reasons that must NOT move the cursor, Escape loudest.
  const selectionRemovedPulse = useProjectStore((s) => s.selectionRemovedPulse);
  // Seeded with whatever the counter already was at mount, so a remount never
  // re-runs the body against a pulse this instance already handled, or one raised
  // while it was unmounted.
  const lastHandledPulseRef = useRef(selectionRemovedPulse);
  useEffect(() => {
    if (selectionRemovedPulse === lastHandledPulseRef.current) return;
    lastHandledPulseRef.current = selectionRemovedPulse;
    if (document.activeElement !== listRef.current) return;
    if (assets.length === 0) return;
    // Clamped at both ends: the top clamp is for a shortened list, the bottom
    // one for a cursor that was never placed, where -1 would index off the
    // front of the array. Landing on the first row is the honest answer there.
    const idx = Math.max(0, Math.min(lastIndexRef.current, assets.length - 1));
    onAssetClick(assets[idx], idx, {
      select: true,
      toggle: false,
      extend: false,
    });
    // Keyed on the counter alone. Adding `assets` would re-run this on every
    // filesystem event and fire long after the deletion that set it.
  }, [selectionRemovedPulse]);

  // One screen minus a row of context, the way list widgets conventionally page.
  // The container's height includes the sticky header, which is not list space.
  // Floored at one row so an unmeasured container still advances.
  const pageStep = () => {
    const h = parentRef.current?.clientHeight ?? ROW_HEIGHT * 10;
    return Math.max(1, Math.floor((h - LIST_HEADER_HEIGHT) / ROW_HEIGHT) - 1);
  };

  // Moving the cursor IS selecting: see the design note on why arrow keys
  // preview as they go. Shift additionally extends the checkbox selection
  // from the anchor, which is why both bits are set.
  const moveTo = (index: number, extend: boolean) => {
    const clamped = Math.max(0, Math.min(index, assets.length - 1));
    onAssetClick(assets[clamped], clamped, {
      select: true,
      toggle: false,
      extend,
    });
  };

  // Where to pick up when there is no cursor on screen: wherever it last was. A
  // search or type filter can take the previewed row out of the list, and jumping
  // to the far end for that is a teleport the user did not ask for.
  const resumeIndex = (fallback: number) =>
    lastIndexRef.current >= 0 ? lastIndexRef.current : fallback;

  // Entering the list from elsewhere — today, a down arrow in the search box. A
  // cursor the filter took away is put back at the same time. The explicit
  // focus-visible mark is needed: WebKit drops that state across a scripted focus.
  const enterList = () => {
    const el = listRef.current;
    if (!el) return;
    el.dataset.kbdFocus = "true";
    // Without this the browser scrolls the newly focused element into view, and
    // the rows container starts exactly where the sticky header ends — so the box
    // scrolls by the header's height and hides the first row behind it.
    el.focus({ preventScroll: true });
    if (focusedIndex < 0 && assets.length > 0) moveTo(resumeIndex(0), false);
  };

  const dropKeyboardMark = () => {
    const el = listRef.current;
    if (el) delete el.dataset.kbdFocus;
  };

  // No dependency array: the registered function closes over the current cursor
  // and the current list, so it has to be replaced whenever either changes.
  // Cleared on unmount so the search box can tell there is no list to enter.
  useEffect(() => {
    registerAssetListFocus(enterList);
    return () => registerAssetListFocus(null);
  });

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Only when the list itself holds focus, not one of its focusable
    // descendants (the row checkbox) whose events would otherwise bubble
    // through here and be treated as a list-level shortcut.
    if (e.target !== e.currentTarget) return;
    if (assets.length === 0) return;
    const cur = focusedIndex;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        moveTo(cur >= 0 ? cur + 1 : resumeIndex(0), e.shiftKey);
        break;
      case "ArrowUp":
        e.preventDefault();
        moveTo(cur >= 0 ? cur - 1 : resumeIndex(assets.length - 1), e.shiftKey);
        break;
      case "Home":
        e.preventDefault();
        moveTo(0, e.shiftKey);
        break;
      case "End":
        e.preventDefault();
        moveTo(assets.length - 1, e.shiftKey);
        break;
      case "PageDown":
        e.preventDefault();
        moveTo((cur < 0 ? 0 : cur) + pageStep(), e.shiftKey);
        break;
      case "PageUp":
        e.preventDefault();
        moveTo((cur < 0 ? 0 : cur) - pageStep(), e.shiftKey);
        break;
      case "Enter":
        e.preventDefault();
        if (cur < 0) return;
        onActivate(assets[cur]);
        break;
    }
  };

  // Scroll the selected row into view on selection change or locate pulse.
  // `align: "auto"` is a no-op when the row is already visible. `assets` is not a
  // dependency — re-filtering must not fight the user's scroll.
  const locatePulse = useProjectStore((s) => s.locatePulse);
  useEffect(() => {
    if (!selectedAsset) return;
    const idx = assets.findIndex((a) => a.path === selectedAsset.path);
    if (idx >= 0) {
      virtualizer.scrollToIndex(idx, { align: "auto" });
    }
  }, [selectedAsset?.path, locatePulse]);

  const virtualItems = virtualizer.getVirtualItems();

  // Only name a row that is actually mounted. A jump from elsewhere in the app
  // commits the new selection a frame before the virtualizer renders its row, and
  // a reference to an id not in the document is invalid and announces nothing.
  const activeRowId =
    focusedIndex >= 0 && virtualItems.some((v) => v.index === focusedIndex)
      ? rowDomId(focusedIndex)
      : undefined;

  return (
    <div className="tc-list-frame">
      <div ref={parentRef} className="tc-list-scroll">
        <div
          className="tc-list-header"
          style={{ width: totalRowWidth, minWidth: "100%" }}
        >
          {/* Header spacer for the always-reserved checkbox lane. */}
          <div className="w-8 py-2 px-2 shrink-0" />
          <div
            className="py-2 px-3 shrink-0 flex items-center gap-1 cursor-pointer hover:text-ink transition-colors select-none relative overflow-hidden"
            style={{ width: columnWidths.name }}
            {...sortableHeaderProps(() => setSortField("name"))}
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
                    isSortable && "cursor-pointer hover:text-ink"
                  )}
                  style={{ width }}
                  {...(isSortable
                    ? sortableHeaderProps(() =>
                        setSortField(columnId as SortField)
                      )
                    : {})}
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

        {/* Focus lives here rather than on the scroll box: a screen reader
            decides whether to hand the arrow keys to the page or to the widget
            from the role of whatever holds focus, and `listbox` is the role that
            asks for the widget. The scroll box carried `group`, which is not in
            that set, so the arrow keys were eaten by the virtual buffer and this
            handler never ran.

            Focusing an element this tall does not scroll the container on its
            own — checked on a real machine, the scroll position did not move. */}
        <div
          ref={listRef}
          role="listbox"
          aria-label={t("assetList.listLabel")}
          tabIndex={0}
          aria-activedescendant={activeRowId}
          onKeyDown={handleKeyDown}
          onBlur={dropKeyboardMark}
          onMouseDown={dropKeyboardMark}
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
                  domId={rowDomId(virtualItem.index)}
                  posInSet={virtualItem.index + 1}
                  setSize={assets.length}
                  isSelected={selectedAsset?.path === asset.path}
                  isChecked={selectedPaths.has(asset.path)}
                  onClick={(e) => onAssetClick(asset, virtualItem.index, intentFromMouse(e))}
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
                    // `start` is measured from the top of the scroll box and so
                    // includes the scroll margin, while these rows sit in a
                    // container that already begins below the header.
                    transform: `translateY(${virtualItem.start - LIST_HEADER_HEIGHT}px)`,
                  }}
                />
              );
            })}
        </div>
      </div>
    </div>
  );
}
