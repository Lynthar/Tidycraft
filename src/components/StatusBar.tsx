import { X } from "lucide-react";
import { useProjectStore } from "../stores/projectStore";
import { formatFileSize } from "../lib/utils";

function getPhaseLabel(phase: string): string {
  switch (phase) {
    case "discovering":
      return "Discovering files";
    case "parsing":
      return "Parsing assets";
    case "building":
      return "Building tree";
    case "completed":
      return "Completed";
    case "cancelled":
      return "Cancelled";
    default:
      return "Scanning";
  }
}

export function StatusBar() {
  const { scanResult, isScanning, error, scanProgress, cancelScan, getFilteredAssets } =
    useProjectStore();

  const filteredAssets = getFilteredAssets();
  const filteredSize = filteredAssets.reduce((sum, a) => sum + a.size, 0);

  if (error) {
    return (
      <footer className="h-6 bg-error/20 border-t border-error px-4 flex items-center text-xs text-error">
        Error: {error}
      </footer>
    );
  }

  if (isScanning && scanProgress) {
    const progressPercent =
      scanProgress.total && scanProgress.total > 0
        ? Math.round((scanProgress.current / scanProgress.total) * 100)
        : 0;

    const currentFile = scanProgress.current_file
      ? scanProgress.current_file.split("/").pop() || scanProgress.current_file
      : "";

    return (
      <footer className="h-6 bg-card-bg border-t border-border px-4 flex items-center text-xs text-text-secondary">
        <div className="flex items-center gap-3 flex-1">
          <span className="text-primary">{getPhaseLabel(scanProgress.phase)}</span>
          {scanProgress.total && scanProgress.total > 0 && (
            <>
              <span>|</span>
              <span>
                {scanProgress.current} / {scanProgress.total} ({progressPercent}%)
              </span>
              <div className="w-32 h-1.5 bg-background rounded-full overflow-hidden">
                <div
                  className="h-full bg-primary transition-all duration-200"
                  style={{ width: `${progressPercent}%` }}
                />
              </div>
            </>
          )}
          {currentFile && (
            <>
              <span>|</span>
              <span className="truncate max-w-64">{currentFile}</span>
            </>
          )}
        </div>
        <button
          onClick={cancelScan}
          className="flex items-center gap-1 px-2 py-0.5 hover:bg-error/20 hover:text-error rounded transition-colors"
        >
          <X size={12} />
          Cancel
        </button>
      </footer>
    );
  }

  if (isScanning) {
    return (
      <footer className="h-6 bg-card-bg border-t border-border px-4 flex items-center text-xs text-text-secondary">
        <span>Starting scan...</span>
        <button
          onClick={cancelScan}
          className="ml-auto flex items-center gap-1 px-2 py-0.5 hover:bg-error/20 hover:text-error rounded transition-colors"
        >
          <X size={12} />
          Cancel
        </button>
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
  const projectType = scanResult.project_type;

  return (
    <footer className="h-6 bg-card-bg border-t border-border px-4 flex items-center justify-between text-xs">
      <div className="flex items-center gap-4 text-text-secondary">
        {projectType && projectType !== "generic" && (
          <>
            <span className="px-1.5 py-0.5 bg-primary/20 text-primary rounded text-[10px] uppercase font-medium">
              {projectType}
            </span>
            <span>|</span>
          </>
        )}
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
