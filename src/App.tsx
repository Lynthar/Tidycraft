import { useEffect, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Group, Panel, Separator, useDefaultLayout } from "react-resizable-panels";
import { Header } from "./components/Header";
import { Sidebar } from "./components/Sidebar";
import { AssetList } from "./components/AssetList";
import { AssetPreview } from "./components/AssetPreview";
import { IssueList } from "./components/IssueList";
import { StatsDashboard } from "./components/StatsDashboard";
import { StatusBar } from "./components/StatusBar";
import { EmptyState } from "./components/EmptyState";
import { CommandPalette } from "./components/CommandPalette";
import { SettingsModal } from "./components/SettingsModal";
import { TagManager } from "./components/TagManager";
import { AITagPanel } from "./components/AITagPanel";
import { AIAnalyzeModal } from "./components/AIAnalyzeModal";
import { AIResultPanel } from "./components/AIResultPanel";
import { LearnSetupModal } from "./components/LearnSetupModal";
import { LearnReviewPanel } from "./components/LearnReviewPanel";
import { DependencyGraphModal } from "./components/DependencyGraphModal";
import { useProjectStore } from "./stores/projectStore";
import { useUiStore } from "./stores/uiStore";
import { restoreSession } from "./stores/sessionStore";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { isMacOS } from "./lib/platform";
import { version as appVersion } from "../package.json";

function App() {
  const {
    selectedAsset,
    scanResult,
    viewMode,
    analysisResult,
    analysisStale,
    isAnalyzing,
    runAnalysis,
    locateAsset,
    getProjectList,
    activeProjectId,
  } = useProjectStore();

  const projects = getProjectList();
  const isEmpty = projects.length === 0;

  // Live-vs-snapshot arithmetic: scanResult.total_count is watcher-patched
  // while analysisResult.issues is a frozen snapshot, so a naive subtraction
  // could count flagged files that no longer exist and even go negative
  // (delete enough analyzed files and total < flagged). Intersect the flagged
  // set with the live asset paths, and clamp as a belt-and-braces.
  const passCount = useMemo(() => {
    if (!scanResult) return 0;
    if (!analysisResult) return scanResult.total_count;
    const live = new Set(scanResult.assets.map((a) => a.path));
    const flaggedLive = new Set(
      analysisResult.issues.map((i) => i.asset_path).filter((p) => live.has(p))
    );
    return Math.max(0, scanResult.total_count - flaggedLive.size);
  }, [scanResult, analysisResult]);

  const searchInputRef = useRef<HTMLInputElement>(null);

  // On boot, restore the projects that were open in the last session. Runs
  // exactly once — `sessionStore` has an internal `restored` guard so React
  // strict-mode double-mount doesn't trigger a second restore.
  useEffect(() => {
    restoreSession();
  }, []);

  // Add a body class on macOS so CSS can opt the custom titlebar in. The
  // window's `titleBarStyle: Overlay` only takes effect on macOS — on other
  // platforms native chrome handles things and our titlebar stays hidden.
  useEffect(() => {
    if (isMacOS()) {
      document.body.classList.add("tc-platform-macos");
    }
  }, []);

  // Initialize keyboard shortcuts
  useKeyboardShortcuts({
    onFocusSearch: () => {
      searchInputRef.current?.focus();
    },
  });

  const showPreview = !!(scanResult && selectedAsset && viewMode === "assets");

  // Persist panel sizes across reloads, and restore the preview width when it
  // re-mounts after a deselect→reselect. panelIds tracks which Panels are
  // currently mounted so the 2- and 3-panel layouts are saved separately —
  // react-resizable-panels' documented mechanism for conditionally-rendered
  // panels. On first run defaultLayout is undefined and each Panel falls back
  // to its own defaultSize.
  const panelIds = useMemo(
    () => (showPreview ? ["sidebar", "main", "preview"] : ["sidebar", "main"]),
    [showPreview]
  );
  const { defaultLayout, onLayoutChange } = useDefaultLayout({
    id: "tidycraft-panels",
    panelIds,
    storage: localStorage,
  });

  const handleExportJson = async () => {
    if (!activeProjectId) return;
    try {
      const json = await invoke<string>("export_to_json", { projectId: activeProjectId });
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "assets.json";
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error("Failed to export JSON:", err);
    }
  };

  const handleExportCsv = async () => {
    if (!activeProjectId) return;
    try {
      const csv = await invoke<string>("export_to_csv", { projectId: activeProjectId });
      const blob = new Blob([csv], { type: "text/csv" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "assets.csv";
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error("Failed to export CSV:", err);
    }
  };

  const handleExportHtml = async () => {
    if (!activeProjectId) return;
    try {
      const html = await invoke<string>("export_to_html", { projectId: activeProjectId });
      const blob = new Blob([html], { type: "text/html" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "tidycraft-report.html";
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error("Failed to export HTML:", err);
    }
  };

  const dispatchExport = (format: "json" | "csv" | "html") => {
    if (format === "json") handleExportJson();
    else if (format === "csv") handleExportCsv();
    else handleExportHtml();
  };

  const settingsOpen = useUiStore((s) => s.settingsOpen);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const tagManagerOpen = useUiStore((s) => s.tagManagerOpen);
  const setTagManagerOpen = useUiStore((s) => s.setTagManagerOpen);

  const renderMainContent = () => {
    if (isEmpty) {
      return <EmptyState />;
    }
    switch (viewMode) {
      case "assets":
        return <AssetList />;
      case "issues":
        return (
          <IssueList
            result={analysisResult}
            stale={analysisStale}
            isAnalyzing={isAnalyzing}
            onAnalyze={runAnalysis}
            onLocate={locateAsset}
          />
        );
      case "stats":
        return (
          <StatsDashboard
            issueCount={analysisResult?.issue_count || 0}
            passCount={passCount}
            onExportJson={handleExportJson}
            onExportCsv={handleExportCsv}
            onExportHtml={handleExportHtml}
          />
        );
      default:
        return <AssetList />;
    }
  };

  return (
    <div className="h-screen flex flex-col bg-background">
      <div className="tc-titlebar" data-tauri-drag-region>
        <span className="tc-title-text" data-tauri-drag-region>
          Tidycraft
        </span>
        <span className="tc-title-text mono" data-tauri-drag-region>
          v{appVersion}
        </span>
      </div>
      <Header searchInputRef={searchInputRef} />

      <div className="flex-1 flex overflow-hidden">
        <Group
          orientation="horizontal"
          id="tidycraft-panels"
          className="flex-1 h-full"
          style={{ height: "100%" }}
          defaultLayout={defaultLayout}
          onLayoutChange={onLayoutChange}
        >
          <Panel
            id="sidebar"
            defaultSize="22%"
            minSize="14%"
            maxSize="40%"
            className="overflow-hidden"
          >
            <Sidebar />
          </Panel>
          <Separator className="w-1 bg-border hover:bg-primary/50 active:bg-primary transition-colors cursor-col-resize" />

          <Panel
            id="main"
            defaultSize={showPreview ? "60%" : "78%"}
            minSize="30%"
            className="overflow-hidden"
          >
            <main className="h-full bg-background overflow-hidden">
              {renderMainContent()}
            </main>
          </Panel>

          {showPreview && (
            <>
              <Separator className="w-1 bg-border hover:bg-primary/50 active:bg-primary transition-colors cursor-col-resize" />
              <Panel
                id="preview"
                defaultSize="18%"
                minSize="15%"
                maxSize="35%"
                className="overflow-hidden"
              >
                <AssetPreview />
              </Panel>
            </>
          )}
        </Group>
      </div>

      <StatusBar />

      <CommandPalette onExport={dispatchExport} />
      <SettingsModal isOpen={settingsOpen} onClose={() => setSettingsOpen(false)} />
      <TagManager isOpen={tagManagerOpen} onClose={() => setTagManagerOpen(false)} />
      <AITagPanel />
      <AIAnalyzeModal />
      <AIResultPanel />
      <LearnSetupModal />
      <LearnReviewPanel />
      <DependencyGraphModal />
    </div>
  );
}

export default App;
