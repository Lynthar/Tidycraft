import { Files, AlertTriangle, Play } from "lucide-react";
import { DirectoryTree } from "./DirectoryTree";
import { useProjectStore } from "../stores/projectStore";
import { cn } from "../lib/utils";

export function Sidebar() {
  const {
    scanResult,
    viewMode,
    setViewMode,
    analysisResult,
    isAnalyzing,
    runAnalysis,
  } = useProjectStore();

  return (
    <aside className="w-64 bg-card-bg border-r border-border flex flex-col shrink-0">
      {/* View Mode Tabs */}
      <div className="flex border-b border-border shrink-0">
        <button
          onClick={() => setViewMode("assets")}
          className={cn(
            "flex-1 flex items-center justify-center gap-2 py-2 text-xs font-medium transition-colors",
            viewMode === "assets"
              ? "text-primary border-b-2 border-primary"
              : "text-text-secondary hover:text-text-primary"
          )}
        >
          <Files size={14} />
          Assets
        </button>
        <button
          onClick={() => setViewMode("issues")}
          className={cn(
            "flex-1 flex items-center justify-center gap-2 py-2 text-xs font-medium transition-colors",
            viewMode === "issues"
              ? "text-primary border-b-2 border-primary"
              : "text-text-secondary hover:text-text-primary"
          )}
        >
          <AlertTriangle size={14} />
          Issues
          {analysisResult && analysisResult.issue_count > 0 && (
            <span className="px-1.5 py-0.5 bg-warning/20 text-warning text-[10px] rounded-full">
              {analysisResult.issue_count}
            </span>
          )}
        </button>
      </div>

      {/* Analyze Button */}
      {scanResult && (
        <div className="p-2 border-b border-border shrink-0">
          <button
            onClick={runAnalysis}
            disabled={isAnalyzing}
            className={cn(
              "w-full flex items-center justify-center gap-2 px-3 py-1.5 rounded text-sm font-medium transition-colors",
              isAnalyzing
                ? "bg-primary/50 text-white cursor-not-allowed"
                : "bg-primary text-white hover:bg-primary/90"
            )}
          >
            <Play size={14} />
            {isAnalyzing ? "Analyzing..." : "Run Analysis"}
          </button>
        </div>
      )}

      {/* Directory Tree */}
      <div className="h-8 px-3 flex items-center border-b border-border text-xs text-text-secondary font-medium uppercase tracking-wide">
        Explorer
      </div>
      <div className="flex-1 overflow-hidden">
        <DirectoryTree />
      </div>
    </aside>
  );
}
