import { Files, AlertTriangle, Play, BarChart3, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";
import { DirectoryTree } from "./DirectoryTree";
import { TagFilterPanel } from "./TagFilterPanel";
import { useProjectStore } from "../stores/projectStore";
import { useUiStore } from "../stores/uiStore";
import { formatShortcut, SHORTCUTS } from "../hooks/useKeyboardShortcuts";

export function Sidebar() {
  const { t } = useTranslation();
  const {
    scanResult,
    viewMode,
    setViewMode,
    analysisResult,
    isAnalyzing,
    runAnalysis,
    hasCustomConfig,
  } = useProjectStore();

  const setAiPanelOpen = useUiStore((s) => s.setAiPanelOpen);

  const issueCount = analysisResult?.issue_count ?? 0;
  const errCount = analysisResult?.error_count ?? 0;
  const warnCount = analysisResult?.warning_count ?? 0;
  const issueTone: "err" | "warn" | undefined =
    errCount > 0 ? "err" : warnCount > 0 ? "warn" : undefined;
  const totalAssets = scanResult?.total_count ?? 0;

  return (
    <aside className="tc-sidebar">
      <div className="tc-tabs">
        <button
          className="tc-tab"
          data-active={viewMode === "assets" ? "true" : undefined}
          onClick={() => setViewMode("assets")}
        >
          <Files size={12} />
          {t("sidebar.assets")}
          {totalAssets > 0 && <span className="tc-tab-count">{totalAssets}</span>}
        </button>

        <button
          className="tc-tab"
          data-active={viewMode === "issues" ? "true" : undefined}
          onClick={() => setViewMode("issues")}
        >
          <AlertTriangle size={12} />
          {t("sidebar.issues")}
          {issueCount > 0 ? (
            <span className="tc-tab-count" data-tone={issueTone}>
              {issueCount}
            </span>
          ) : analysisResult ? (
            <span className="tc-tab-count">0</span>
          ) : null}
        </button>

        <button
          className="tc-tab"
          data-active={viewMode === "stats" ? "true" : undefined}
          onClick={() => setViewMode("stats")}
        >
          <BarChart3 size={12} />
          {t("sidebar.stats")}
        </button>
      </div>

      {scanResult && (
        <div className="tc-run-block">
          <button
            onClick={runAnalysis}
            disabled={isAnalyzing}
            className="tc-run-btn"
            title={
              hasCustomConfig
                ? `${t("sidebar.runAnalysis")} (${formatShortcut(SHORTCUTS.analyze)}) — ${t("sidebar.customConfigLoaded")}`
                : `${t("sidebar.runAnalysis")} (${formatShortcut(SHORTCUTS.analyze)})`
            }
          >
            {isAnalyzing ? (
              <span
                className="tc-spinner"
                style={{ borderTopColor: "var(--on-primary)" }}
                aria-hidden
              />
            ) : (
              <Play size={13} />
            )}
            {isAnalyzing ? t("sidebar.analyzing") : t("sidebar.runAnalysis")}
            {hasCustomConfig && !isAnalyzing && (
              <span
                aria-hidden
                title={t("sidebar.customConfigLoaded")}
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  background: "var(--on-primary)",
                  boxShadow:
                    "0 0 0 2px color-mix(in oklch, var(--on-primary) 25%, transparent)",
                  display: "inline-block",
                }}
              />
            )}
            {!isAnalyzing && (
              <span className="mono">{formatShortcut(SHORTCUTS.analyze)}</span>
            )}
          </button>

          {/* Secondary action: open the heuristic AI Tag panel. Outlined
              styling keeps Run Analysis the visual primary; this is an
              entry point to a *view*, not a batch operation. */}
          <button
            onClick={() => setAiPanelOpen(true)}
            className="tc-aitag-btn"
            title={t("sidebar.aiTagSuggestions")}
          >
            <Sparkles size={12} />
            {t("sidebar.aiTagSuggestions")}
          </button>
        </div>
      )}

      {scanResult && <TagFilterPanel />}

      <div className="tc-section" style={{ paddingBottom: 0 }}>
        <div className="tc-section-head">
          <div className="tc-section-title">{t("sidebar.explorer")}</div>
        </div>
      </div>
      <div style={{ flex: 1, overflow: "hidden", minHeight: 0 }}>
        <DirectoryTree />
      </div>
    </aside>
  );
}
