import { Header } from "./components/Header";
import { Sidebar } from "./components/Sidebar";
import { AssetList } from "./components/AssetList";
import { AssetPreview } from "./components/AssetPreview";
import { IssueList } from "./components/IssueList";
import { StatusBar } from "./components/StatusBar";
import { useProjectStore } from "./stores/projectStore";

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

  const showPreview = scanResult && selectedAsset && viewMode === "assets";

  return (
    <div className="h-screen flex flex-col bg-background">
      <Header />

      <div className="flex-1 flex overflow-hidden">
        <Sidebar />

        <main className="flex-1 bg-background overflow-hidden">
          {viewMode === "assets" ? (
            <AssetList />
          ) : (
            <IssueList
              result={analysisResult}
              isAnalyzing={isAnalyzing}
              onAnalyze={runAnalysis}
              onLocate={locateAsset}
            />
          )}
        </main>

        {showPreview && <AssetPreview />}
      </div>

      <StatusBar />
    </div>
  );
}

export default App;
