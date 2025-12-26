import { Header } from "./components/Header";
import { Sidebar } from "./components/Sidebar";
import { AssetList } from "./components/AssetList";
import { StatusBar } from "./components/StatusBar";

function App() {
  return (
    <div className="h-screen flex flex-col bg-background">
      <Header />

      <div className="flex-1 flex overflow-hidden">
        <Sidebar />

        <main className="flex-1 bg-background overflow-hidden">
          <AssetList />
        </main>
      </div>

      <StatusBar />
    </div>
  );
}

export default App;
