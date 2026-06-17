import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke } from "@tauri-apps/api/core";
import {
  Image as ImageIcon,
  Box,
  Volume2,
  Video,
  Film,
  Layers,
  Package,
  Mountain,
  Code,
  Database,
  FileText,
  type LucideIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { formatFileSize } from "../lib/utils";
import { useSettingsStore } from "../stores/settingsStore";
import type {
  AssetInfo,
  AssetType,
  AssetTagsMap,
  GitFileStatus,
  GitStatusMap,
} from "../types/asset";
import { GitStatusBadge } from "./GitStatusBadge";
import { peekThumb, hasThumb, putThumb } from "../lib/thumbnailCache";

const CARD_MIN_WIDTH = 168;
const CARD_GAP = 12;
const FOOT_HEIGHT = 52;
/// Thumbnail render size matches AssetPreview so the disk cache is shared.
const THUMB_SIZE = 256;

/// Thumbnails live in the shared `lib/thumbnailCache` LRU (bounded; evicted by
/// projectStore on fs-change so external edits show fresh images). Survives
/// component remounts (e.g. switching list↔grid) so users don't re-pay the
/// invoke roundtrip.

const GLYPH_ICONS: Record<AssetType, LucideIcon> = {
  texture: ImageIcon,
  model: Box,
  audio: Volume2,
  video: Video,
  animation: Film,
  material: Layers,
  prefab: Package,
  scene: Mountain,
  script: Code,
  data: Database,
  other: FileText,
};

interface CardThumbProps {
  asset: AssetInfo;
}

function CardThumb({ asset }: CardThumbProps) {
  const Glyph = GLYPH_ICONS[asset.asset_type] ?? FileText;
  const cached = peekThumb(asset.path);
  const [thumb, setThumb] = useState<string | null | undefined>(cached);

  useEffect(() => {
    // Only textures get a real thumbnail; other types stay on the glyph
    // even if cache.has(path) — the cache map is per-asset, so different
    // types coexist without collision.
    if (asset.asset_type !== "texture") return;
    if (hasThumb(asset.path)) {
      setThumb(peekThumb(asset.path));
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const base64 = await invoke<string>("get_thumbnail", {
          path: asset.path,
          size: THUMB_SIZE,
        });
        putThumb(asset.path, base64);
        if (!cancelled) setThumb(base64);
      } catch {
        putThumb(asset.path, null);
        if (!cancelled) setThumb(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [asset.path, asset.asset_type]);

  const showImage = asset.asset_type === "texture" && typeof thumb === "string";

  return (
    <div className="tc-card-thumb" data-type={asset.asset_type}>
      {showImage ? (
        <img
          src={`data:image/png;base64,${thumb}`}
          alt={asset.name}
          style={{
            position: "absolute",
            inset: 0,
            width: "100%",
            height: "100%",
            objectFit: "cover",
          }}
          draggable={false}
        />
      ) : (
        <div className="tc-card-thumb-glyph">
          <Glyph size={26} />
        </div>
      )}
    </div>
  );
}

interface CardProps {
  asset: AssetInfo;
  index: number;
  isSelected: boolean;
  gitStatus?: GitFileStatus;
  showGitStatusIndicators: boolean;
  assetTags: AssetTagsMap[string];
  onClick: (asset: AssetInfo, index: number, e: React.MouseEvent) => void;
  onContextMenu: (e: React.MouseEvent, asset: AssetInfo) => void;
  typeLabel: string;
  t: (key: string) => string;
}

function Card({
  asset,
  index,
  isSelected,
  gitStatus,
  showGitStatusIndicators,
  assetTags,
  onClick,
  onContextMenu,
  typeLabel,
  t,
}: CardProps) {
  // Split name into stem + extension for muted-extension styling.
  const lastDot = asset.name.lastIndexOf(".");
  const stem = lastDot > 0 ? asset.name.slice(0, lastDot) : asset.name;
  const ext = lastDot > 0 ? asset.name.slice(lastDot) : "";

  const dim =
    asset.metadata?.width && asset.metadata?.height
      ? `${asset.metadata.width}×${asset.metadata.height}`
      : null;
  const verts = asset.metadata?.vertex_count
    ? `${asset.metadata.vertex_count.toLocaleString()} v`
    : null;

  return (
    <div
      className="tc-card"
      data-selected={isSelected ? "true" : undefined}
      onClick={(e) => onClick(asset, index, e)}
      onContextMenu={(e) => onContextMenu(e, asset)}
    >
      <CardThumb asset={asset} />
      <span className="tc-card-typechip" data-type={asset.asset_type}>
        <span className="tc-card-typedot" />
        {typeLabel}
      </span>
      {showGitStatusIndicators && gitStatus && gitStatus !== "unchanged" && (
        <span className="tc-card-gh">
          <GitStatusBadge status={gitStatus} t={t} />
        </span>
      )}
      {dim && <span className="tc-card-meta-tl">{dim}</span>}
      {verts && <span className="tc-card-meta-br">{verts}</span>}
      <div className="tc-card-foot">
        <div className="tc-card-name" title={asset.name}>
          {stem}
          {ext && <span className="tc-ext">{ext}</span>}
        </div>
        <div className="tc-card-meta">
          <span className="tc-num">{formatFileSize(asset.size)}</span>
          {assetTags.length > 0 && (
            <span className="tc-card-tagstrip">
              {assetTags.slice(0, 2).map((tag) => (
                <span
                  key={tag.id}
                  className="tc-card-tagdot"
                  style={{ background: tag.color }}
                  title={tag.name}
                />
              ))}
              {assetTags.length > 2 && (
                <span className="tc-card-tagmore">
                  +{assetTags.length - 2}
                </span>
              )}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

export interface AssetGalleryViewProps {
  assets: AssetInfo[];
  selectedAsset: AssetInfo | null;
  selectedPaths: Set<string>;
  gitStatuses: GitStatusMap;
  allAssetTags: AssetTagsMap;
  onAssetClick: (asset: AssetInfo, index: number, e: React.MouseEvent) => void;
  onContextMenu: (e: React.MouseEvent, asset: AssetInfo) => void;
  getTypeLabel: (type: AssetType) => string;
}

export function AssetGalleryView({
  assets,
  selectedAsset,
  selectedPaths,
  gitStatuses,
  allAssetTags,
  onAssetClick,
  onContextMenu,
  getTypeLabel,
}: AssetGalleryViewProps) {
  const { t } = useTranslation();
  const { showGitStatusIndicators } = useSettingsStore();
  const parentRef = useRef<HTMLDivElement>(null);
  const [contentWidth, setContentWidth] = useState<number>(0);

  // Track the content-box width (already accounts for our 14/16 padding via
  // the browser, since clientWidth is content + padding — minus padding
  // gives us the usable horizontal space for the row's grid).
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    const measure = () => {
      // padding: 14px 16px 22px → horizontal padding total = 32px
      setContentWidth(Math.max(0, el.clientWidth - 32));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Layout math: at minimum CARD_MIN_WIDTH per card with CARD_GAP between.
  // cols = floor((W + GAP) / (CARD_MIN + GAP))  — guaranteed ≥ 1.
  const { cols, cardWidth, rowHeight } = useMemo(() => {
    if (contentWidth <= 0) {
      return { cols: 1, cardWidth: CARD_MIN_WIDTH, rowHeight: CARD_MIN_WIDTH + FOOT_HEIGHT };
    }
    const cols = Math.max(
      1,
      Math.floor((contentWidth + CARD_GAP) / (CARD_MIN_WIDTH + CARD_GAP))
    );
    const cardWidth = (contentWidth - (cols - 1) * CARD_GAP) / cols;
    // Row stride = card height (square thumb + foot) + vertical gap to next row.
    const rowHeight = cardWidth + FOOT_HEIGHT + CARD_GAP;
    return { cols, cardWidth, rowHeight };
  }, [contentWidth]);

  const rowCount = Math.ceil(assets.length / cols);

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 2,
  });

  // Re-measure when row stride changes (resize, asset count change).
  useEffect(() => {
    virtualizer.measure();
  }, [rowHeight, rowCount, virtualizer]);

  if (contentWidth <= 0) {
    // First paint: container hasn't been measured yet. Render the bare
    // scroll element so the layout effect can read its width.
    return <div ref={parentRef} className="tc-gallery" />;
  }

  return (
    <div ref={parentRef} className="tc-gallery">
      <div
        style={{
          height: virtualizer.getTotalSize(),
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const startIdx = virtualRow.index * cols;
          const endIdx = Math.min(startIdx + cols, assets.length);
          return (
            <div
              key={virtualRow.key}
              className="tc-gallery-row"
              style={{
                top: virtualRow.start,
                // Row content height (without the bottom gap baked into stride).
                height: cardWidth + FOOT_HEIGHT,
                gridTemplateColumns: `repeat(${cols}, 1fr)`,
              }}
            >
              {assets.slice(startIdx, endIdx).map((asset, i) => {
                const idx = startIdx + i;
                const isSelected =
                  selectedAsset?.path === asset.path ||
                  selectedPaths.has(asset.path);
                return (
                  <Card
                    key={asset.path}
                    asset={asset}
                    index={idx}
                    isSelected={isSelected}
                    gitStatus={gitStatuses[asset.path]}
                    showGitStatusIndicators={showGitStatusIndicators}
                    assetTags={allAssetTags[asset.path] || []}
                    onClick={onAssetClick}
                    onContextMenu={onContextMenu}
                    typeLabel={getTypeLabel(asset.asset_type)}
                    t={t}
                  />
                );
              })}
            </div>
          );
        })}
      </div>
    </div>
  );
}
