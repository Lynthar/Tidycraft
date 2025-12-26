import { useState } from "react";
import { ChevronRight, ChevronDown, Folder, FolderOpen } from "lucide-react";
import { useProjectStore } from "../stores/projectStore";
import { cn } from "../lib/utils";
import type { DirectoryNode } from "../types/asset";

interface TreeNodeProps {
  node: DirectoryNode;
  level: number;
}

function TreeNode({ node, level }: TreeNodeProps) {
  const [isExpanded, setIsExpanded] = useState(level === 0);
  const { selectedDirectory, setSelectedDirectory } = useProjectStore();

  const isSelected = selectedDirectory === node.path;
  const hasChildren = node.children.length > 0;

  const handleClick = () => {
    setSelectedDirectory(node.path);
  };

  const handleToggle = (e: React.MouseEvent) => {
    e.stopPropagation();
    setIsExpanded(!isExpanded);
  };

  return (
    <div>
      <div
        className={cn(
          "flex items-center gap-1 py-1 px-2 cursor-pointer rounded text-sm",
          "hover:bg-background transition-colors",
          isSelected && "bg-primary/20 text-primary"
        )}
        style={{ paddingLeft: `${level * 12 + 4}px` }}
        onClick={handleClick}
      >
        {hasChildren ? (
          <button
            onClick={handleToggle}
            className="p-0.5 hover:bg-card-bg rounded"
          >
            {isExpanded ? (
              <ChevronDown size={14} />
            ) : (
              <ChevronRight size={14} />
            )}
          </button>
        ) : (
          <span className="w-5" />
        )}

        {isExpanded ? (
          <FolderOpen size={16} className="text-warning shrink-0" />
        ) : (
          <Folder size={16} className="text-warning shrink-0" />
        )}

        <span className="truncate flex-1">{node.name}</span>

        <span className="text-xs text-text-secondary shrink-0">
          {node.file_count}
        </span>
      </div>

      {isExpanded && hasChildren && (
        <div>
          {node.children.map((child) => (
            <TreeNode key={child.path} node={child} level={level + 1} />
          ))}
        </div>
      )}
    </div>
  );
}

export function DirectoryTree() {
  const { scanResult, isScanning } = useProjectStore();

  if (isScanning) {
    return (
      <div className="p-4 text-text-secondary text-sm">
        Scanning...
      </div>
    );
  }

  if (!scanResult) {
    return (
      <div className="p-4 text-text-secondary text-sm">
        Open a folder to start
      </div>
    );
  }

  return (
    <div className="py-2 overflow-auto h-full">
      <TreeNode node={scanResult.directory_tree} level={0} />
    </div>
  );
}
