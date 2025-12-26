import { Header } from "./components/Header";
import { Sidebar } from "./components/Sidebar";
import { AssetList } from "./components/AssetList";
import { AssetPreview } from "./components/AssetPreview";
import { StatusBar } from "./components/StatusBar";
import { useProjectStore } from "./stores/projectStore";

function App() {
  const { selectedAsset, scanResult } = useProjectStore();
  const showPreview = scanResult && selectedAsset;

  return (
    <div className="h-screen flex flex-col bg-background">
      <Header />

      <div className="flex-1 flex overflow-hidden">
        <Sidebar />

        <main className="flex-1 bg-background overflow-hidden">
          <AssetList />
        </main>

        {showPreview && <AssetPreview />}
      </div>

      <StatusBar />
    </div>
  );
}

export default App;
