import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Image, Box, Volume2, File } from "lucide-react";
import { useProjectStore } from "../stores/projectStore";
import { cn, formatFileSize, getAssetTypeLabel } from "../lib/utils";
import type { AssetInfo, AssetType } from "../types/asset";

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

interface AssetRowProps {
  asset: AssetInfo;
  isSelected: boolean;
  onClick: () => void;
  style: React.CSSProperties;
}

function AssetRow({ asset, isSelected, onClick, style }: AssetRowProps) {
  const dimensions =
    asset.metadata?.width && asset.metadata?.height
      ? `${asset.metadata.width} x ${asset.metadata.height}`
      : "-";

  return (
    <div
      className={cn(
        "flex items-center border-b border-border cursor-pointer transition-colors text-sm",
        "hover:bg-background",
        isSelected && "bg-primary/20"
      )}
      style={style}
      onClick={onClick}
    >
      <div className="flex-1 py-2 px-3 min-w-0">
        <div className="flex items-center gap-2">
          <AssetIcon type={asset.asset_type} />
          <span className="truncate">{asset.name}</span>
        </div>
      </div>
      <div className="w-24 py-2 px-3 text-text-secondary shrink-0">
        {getAssetTypeLabel(asset.asset_type)}
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

export function AssetList() {
  const { scanResult, selectedAsset, setSelectedAsset, getFilteredAssets, isScanning } =
    useProjectStore();
  const parentRef = useRef<HTMLDivElement>(null);

  const assets = getFilteredAssets();

  const virtualizer = useVirtualizer({
    count: assets.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10, // Render 10 extra items above/below visible area
  });

  if (isScanning) {
    return (
      <div className="flex items-center justify-center h-full text-text-secondary">
        Scanning project...
      </div>
    );
  }

  if (!scanResult) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-text-secondary gap-2">
        <Image size={48} className="opacity-50" />
        <p>Open a folder to view assets</p>
      </div>
    );
  }

  if (assets.length === 0) {
    return (
      <div className="flex items-center justify-center h-full text-text-secondary">
        No assets in this directory
      </div>
    );
  }

  const virtualItems = virtualizer.getVirtualItems();

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex items-center bg-card-bg border-b border-border text-text-secondary text-sm font-medium shrink-0">
        <div className="flex-1 py-2 px-3">Name</div>
        <div className="w-24 py-2 px-3 shrink-0">Type</div>
        <div className="w-24 py-2 px-3 text-right shrink-0">Size</div>
        <div className="w-32 py-2 px-3 text-right shrink-0">Dimensions</div>
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
            return (
              <AssetRow
                key={asset.path}
                asset={asset}
                isSelected={selectedAsset?.path === asset.path}
                onClick={() => setSelectedAsset(asset)}
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
  );
}
