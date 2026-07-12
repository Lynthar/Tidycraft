import { useEffect, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Group, Panel, Separator, useDefaultLayout, type PanelImperativeHandle } from "react-resizable-panels";
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
import { useSettingsStore } from "./stores/settingsStore";
import { restoreSession } from "./stores/sessionStore";
import { Toasts } from "./components/Toasts";
import { exportTextFile } from "./lib/exportFile";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useShallow } from "zustand/react/shallow";
import { isMacOS } from "./lib/platform";
import { version as appVersion } from "../package.json";

/// Every panel id the layout can ever contain, in order. Must stay constant
/// across renders (module scope) — see the useDefaultLayout call in App.
const ALL_PANEL_IDS = ["sidebar", "main", "preview"];

function App() {
  const {
    scanResult,
    viewMode,
    analysisResult,
    analysisStale,
    isAnalyzing,
    runAnalysis,
    locateAsset,
    getProjectList,
    activeProjectId,
  } = useProjectStore(
    useShallow((s) => ({ scanResult: s.scanResult, viewMode: s.viewMode, analysisResult: s.analysisResult, analysisStale: s.analysisStale, isAnalyzing: s.isAnalyzing, runAnalysis: s.runAnalysis, locateAsset: s.locateAsset, getProjectList: s.getProjectList, activeProjectId: s.activeProjectId, }))
  );

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

  // The preview panel is expanded on the assets view of any scanned project
  // (AssetPreview renders its own "select an asset" empty state when nothing
  // is selected) and collapsed to zero width on issues/stats and when no
  // project is open.
  const showPreview = !!(scanResult && viewMode === "assets" && !isEmpty);

  // All three Panels stay PERMANENTLY mounted; visibility is driven through
  // the collapse/expand imperative API below. react-resizable-panels 4.2
  // does not re-consult defaultLayout / defaultSize when a Panel mounts into
  // an existing Group — a conditionally-rendered preview panel came up at
  // near-zero width (even below its minSize) and every mount/unmount cycle
  // shifted the other panels. A fixed panel set sidesteps that entire
  // behavior class and keeps the persisted layout key stable.
  // Plain useRef instead of the library's usePanelRef: its return type is
  // RefObject<Handle | null>, which React 18's Ref<Handle> prop type rejects.
  const previewPanelRef = useRef<PanelImperativeHandle>(null);
  useEffect(() => {
    // Deferred a frame: on first mount this effect fires before the Group
    // has registered the panel's constraints, and collapse()/expand() then
    // throw "Panel constraints not found". The catch covers HMR/unmount
    // races; the next run settles the state either way.
    const raf = requestAnimationFrame(() => {
      const handle = previewPanelRef.current;
      if (!handle) return;
      try {
        if (showPreview) handle.expand();
        else handle.collapse();
      } catch {
        /* group not registered yet — re-runs on next showPreview change */
      }
    });
    return () => cancelAnimationFrame(raf);
  }, [showPreview]);

  const { defaultLayout, onLayoutChange } = useDefaultLayout({
    id: "tidycraft-panels",
    panelIds: ALL_PANEL_IDS,
    storage: localStorage,
  });

  // Exports run through the shared native-save-dialog flow (lib/exportFile):
  // destination picked by the user, success/failure surfaced as a toast. The
  // export command itself only runs after a destination is chosen.
  const handleExportJson = () => {
    if (!activeProjectId) return;
    exportTextFile({
      defaultName: "assets.json",
      filterName: "JSON",
      extensions: ["json"],
      fetchContents: () => invoke<string>("export_to_json", { projectId: activeProjectId }),
    });
  };

  const handleExportCsv = () => {
    if (!activeProjectId) return;
    exportTextFile({
      defaultName: "assets.csv",
      filterName: "CSV",
      extensions: ["csv"],
      fetchContents: () => invoke<string>("export_to_csv", { projectId: activeProjectId }),
    });
  };

  const handleExportHtml = () => {
    if (!activeProjectId) return;
    // Row caps come from Settings → Export (0 = unlimited). Read at click
    // time via getState() — App doesn't need to re-render on settings edits.
    const { htmlReportIssueLimit, htmlReportAssetLimit } =
      useSettingsStore.getState();
    exportTextFile({
      defaultName: "tidycraft-report.html",
      filterName: "HTML",
      extensions: ["html"],
      fetchContents: () =>
        invoke<string>("export_to_html", {
          projectId: activeProjectId,
          issueLimit: htmlReportIssueLimit,
          assetLimit: htmlReportAssetLimit,
        }),
    });
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
            defaultSize="60%"
            minSize="30%"
            className="overflow-hidden"
          >
            <main className="h-full bg-background overflow-hidden">
              {renderMainContent()}
            </main>
          </Panel>

          <Separator className="w-1 bg-border hover:bg-primary/50 active:bg-primary transition-colors cursor-col-resize" />
          <Panel
            id="preview"
            panelRef={previewPanelRef}
            defaultSize="18%"
            minSize="15%"
            maxSize="35%"
            collapsible
            collapsedSize={0}
            className="overflow-hidden"
          >
            <AssetPreview />
          </Panel>
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
      <Toasts />
    </div>
  );
}

export default App;
