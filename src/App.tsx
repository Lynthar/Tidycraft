import { useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Group, Panel, Separator } from "react-resizable-panels";
import { Header } from "./components/Header";
import { ProjectList } from "./components/ProjectList";
import { Sidebar } from "./components/Sidebar";
import { AssetList } from "./components/AssetList";
import { AssetPreview } from "./components/AssetPreview";
import { IssueList } from "./components/IssueList";
import { StatsDashboard } from "./components/StatsDashboard";
import { StatusBar } from "./components/StatusBar";
import { useProjectStore } from "./stores/projectStore";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";

function App() {
  const {
    selectedAsset,
    scanResult,
    viewMode,
    analysisResult,
    isAnalyzing,
    runAnalysis,
    locateAsset,
    getProjectList,
  } = useProjectStore();

  const projects = getProjectList();

  const searchInputRef = useRef<HTMLInputElement>(null);

  // Initialize keyboard shortcuts
  useKeyboardShortcuts({
    onFocusSearch: () => {
      searchInputRef.current?.focus();
    },
  });

  const showPreview = scanResult && selectedAsset && viewMode === "assets";

  const handleExportJson = async () => {
    try {
      const json = await invoke<string>("export_to_json");
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
    try {
      const csv = await invoke<string>("export_to_csv");
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
    try {
      const html = await invoke<string>("export_to_html");
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

  const renderMainContent = () => {
    switch (viewMode) {
      case "assets":
        return <AssetList />;
      case "issues":
        return (
          <IssueList
            result={analysisResult}
            isAnalyzing={isAnalyzing}
            onAnalyze={runAnalysis}
            onLocate={locateAsset}
          />
        );
      case "stats":
        return (
          <StatsDashboard
            issueCount={analysisResult?.issue_count || 0}
            passCount={scanResult ? scanResult.total_count - (analysisResult?.issue_count || 0) : 0}
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
      <Header searchInputRef={searchInputRef} />

      <div className="flex-1 flex overflow-hidden">
        <Group
          orientation="horizontal"
          id="tidycraft-panels"
          className="flex-1 h-full"
          style={{ height: "100%" }}
        >
          {/* Project List - only show when there are projects */}
          {projects.length > 0 && (
            <>
              <Panel
                id="projects"
                defaultSize="12%"
                minSize="8%"
                maxSize="25%"
                className="overflow-hidden"
              >
                <ProjectList />
              </Panel>
              <Separator className="w-1 bg-border hover:bg-primary/50 active:bg-primary transition-colors cursor-col-resize" />
            </>
          )}

          <Panel
            id="sidebar"
            defaultSize="20%"
            minSize="12%"
            maxSize="40%"
            className="overflow-hidden"
          >
            <Sidebar />
          </Panel>
          <Separator className="w-1 bg-border hover:bg-primary/50 active:bg-primary transition-colors cursor-col-resize" />

          <Panel
            id="main"
            defaultSize={showPreview ? "50%" : "68%"}
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
    </div>
  );
}

export default App;
