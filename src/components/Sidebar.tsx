import { Files, AlertTriangle, Play, BarChart3 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { DirectoryTree } from "./DirectoryTree";
import { TagFilterPanel } from "./TagFilterPanel";
import { useProjectStore } from "../stores/projectStore";
import { cn } from "../lib/utils";

export function Sidebar() {
  const { t } = useTranslation();
  const {
    scanResult,
    viewMode,
    setViewMode,
    analysisResult,
    isAnalyzing,
    runAnalysis,
  } = useProjectStore();

  return (
    <aside className="w-full h-full bg-card-bg border-r border-border flex flex-col">
      {/* View Mode Tabs */}
      <div className="flex border-b border-border shrink-0">
        <button
          onClick={() => setViewMode("assets")}
          className={cn(
            "flex-1 flex items-center justify-center gap-1.5 py-2 text-xs font-medium transition-colors",
            viewMode === "assets"
              ? "text-primary border-b-2 border-primary"
              : "text-text-secondary hover:text-text-primary"
          )}
        >
          <Files size={14} />
          {t("sidebar.assets")}
        </button>
        <button
          onClick={() => setViewMode("issues")}
          className={cn(
            "flex-1 flex items-center justify-center gap-1.5 py-2 text-xs font-medium transition-colors",
            viewMode === "issues"
              ? "text-primary border-b-2 border-primary"
              : "text-text-secondary hover:text-text-primary"
          )}
        >
          <AlertTriangle size={14} />
          {t("sidebar.issues")}
          {analysisResult && analysisResult.issue_count > 0 && (
            <span className="px-1.5 py-0.5 bg-warning/20 text-warning text-[10px] rounded-full">
              {analysisResult.issue_count}
            </span>
          )}
        </button>
        <button
          onClick={() => setViewMode("stats")}
          className={cn(
            "flex-1 flex items-center justify-center gap-1.5 py-2 text-xs font-medium transition-colors",
            viewMode === "stats"
              ? "text-primary border-b-2 border-primary"
              : "text-text-secondary hover:text-text-primary"
          )}
        >
          <BarChart3 size={14} />
          Stats
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
            {isAnalyzing ? t("sidebar.analyzing") : t("sidebar.runAnalysis")}
          </button>
        </div>
      )}

      {/* Tag Filter Panel */}
      {scanResult && <TagFilterPanel />}

      {/* Directory Tree */}
      <div className="h-8 px-3 flex items-center border-b border-border text-xs text-text-secondary font-medium uppercase tracking-wide">
        {t("sidebar.explorer")}
      </div>
      <div className="flex-1 overflow-hidden">
        <DirectoryTree />
      </div>
    </aside>
  );
}
