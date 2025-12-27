import { useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Header } from "./components/Header";
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
  } = useProjectStore();

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
        <Sidebar />

        <main className="flex-1 bg-background overflow-hidden">
          {renderMainContent()}
        </main>

        {showPreview && <AssetPreview />}
      </div>

      <StatusBar />
    </div>
  );
}

export default App;
