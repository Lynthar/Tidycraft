import { useProjectStore } from "../stores/projectStore";
import { formatFileSize } from "../lib/utils";

export function StatusBar() {
  const { scanResult, isScanning, error, getFilteredAssets } = useProjectStore();

  const filteredAssets = getFilteredAssets();
  const filteredSize = filteredAssets.reduce((sum, a) => sum + a.size, 0);

  if (error) {
    return (
      <footer className="h-6 bg-error/20 border-t border-error px-4 flex items-center text-xs text-error">
        Error: {error}
      </footer>
    );
  }

  if (isScanning) {
    return (
      <footer className="h-6 bg-card-bg border-t border-border px-4 flex items-center text-xs text-text-secondary">
        Scanning...
      </footer>
    );
  }

  if (!scanResult) {
    return (
      <footer className="h-6 bg-card-bg border-t border-border px-4 flex items-center text-xs text-text-secondary">
        Ready
      </footer>
    );
  }

  const typeCounts = scanResult.type_counts;

  return (
    <footer className="h-6 bg-card-bg border-t border-border px-4 flex items-center justify-between text-xs">
      <div className="flex items-center gap-4 text-text-secondary">
        <span>
          Total: <span className="text-text-primary">{scanResult.total_count}</span> assets
        </span>
        <span>|</span>
        <span>
          Size: <span className="text-text-primary">{formatFileSize(scanResult.total_size)}</span>
        </span>
        <span>|</span>
        <span className="flex gap-3">
          {typeCounts.texture && (
            <span>
              <span className="text-green-400">Textures:</span> {typeCounts.texture}
            </span>
          )}
          {typeCounts.model && (
            <span>
              <span className="text-blue-400">Models:</span> {typeCounts.model}
            </span>
          )}
          {typeCounts.audio && (
            <span>
              <span className="text-yellow-400">Audio:</span> {typeCounts.audio}
            </span>
          )}
        </span>
      </div>

      <div className="text-text-secondary">
        Showing: <span className="text-text-primary">{filteredAssets.length}</span> assets (
        {formatFileSize(filteredSize)})
      </div>
    </footer>
  );
}
