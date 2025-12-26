import { Image, Box, Volume2, File } from "lucide-react";
import { useProjectStore } from "../stores/projectStore";
import { cn, formatFileSize, getAssetTypeLabel } from "../lib/utils";
import type { AssetInfo, AssetType } from "../types/asset";

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
}

function AssetRow({ asset, isSelected, onClick }: AssetRowProps) {
  const dimensions =
    asset.metadata?.width && asset.metadata?.height
      ? `${asset.metadata.width} x ${asset.metadata.height}`
      : "-";

  return (
    <tr
      className={cn(
        "border-b border-border cursor-pointer transition-colors",
        "hover:bg-background",
        isSelected && "bg-primary/20"
      )}
      onClick={onClick}
    >
      <td className="py-2 px-3">
        <div className="flex items-center gap-2">
          <AssetIcon type={asset.asset_type} />
          <span className="truncate">{asset.name}</span>
        </div>
      </td>
      <td className="py-2 px-3 text-text-secondary">
        {getAssetTypeLabel(asset.asset_type)}
      </td>
      <td className="py-2 px-3 text-text-secondary text-right">
        {formatFileSize(asset.size)}
      </td>
      <td className="py-2 px-3 text-text-secondary text-right font-mono text-xs">
        {dimensions}
      </td>
    </tr>
  );
}

export function AssetList() {
  const { scanResult, selectedAsset, setSelectedAsset, getFilteredAssets, isScanning } =
    useProjectStore();

  const assets = getFilteredAssets();

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

  return (
    <div className="h-full overflow-auto">
      <table className="w-full text-sm">
        <thead className="sticky top-0 bg-card-bg border-b border-border">
          <tr className="text-text-secondary text-left">
            <th className="py-2 px-3 font-medium">Name</th>
            <th className="py-2 px-3 font-medium w-24">Type</th>
            <th className="py-2 px-3 font-medium w-24 text-right">Size</th>
            <th className="py-2 px-3 font-medium w-32 text-right">Dimensions</th>
          </tr>
        </thead>
        <tbody>
          {assets.map((asset) => (
            <AssetRow
              key={asset.path}
              asset={asset}
              isSelected={selectedAsset?.path === asset.path}
              onClick={() => setSelectedAsset(asset)}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}
