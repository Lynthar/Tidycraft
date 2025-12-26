import { DirectoryTree } from "./DirectoryTree";

export function Sidebar() {
  return (
    <aside className="w-64 bg-card-bg border-r border-border flex flex-col shrink-0">
      <div className="h-8 px-3 flex items-center border-b border-border text-xs text-text-secondary font-medium uppercase tracking-wide">
        Explorer
      </div>
      <div className="flex-1 overflow-hidden">
        <DirectoryTree />
      </div>
    </aside>
  );
}
