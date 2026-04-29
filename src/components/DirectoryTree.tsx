import { useState } from "react";
import { ChevronRight, Folder, FolderOpen } from "lucide-react";
import { useProjectStore } from "../stores/projectStore";
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
        className="tc-tree-row"
        data-active={isSelected ? "true" : undefined}
        style={{ paddingLeft: `${level * 14 + 4}px` }}
        onClick={handleClick}
      >
        <button
          className="tc-tree-chev"
          data-open={hasChildren && isExpanded ? "true" : undefined}
          data-leaf={!hasChildren ? "true" : undefined}
          onClick={handleToggle}
          tabIndex={hasChildren ? 0 : -1}
          aria-label={hasChildren ? (isExpanded ? "Collapse" : "Expand") : undefined}
        >
          <ChevronRight size={11} />
        </button>
        <span
          className="tc-tree-icon"
          style={{ color: level === 0 ? "var(--primary)" : "var(--text-3)" }}
        >
          {isExpanded && hasChildren ? <FolderOpen size={13} /> : <Folder size={13} />}
        </span>
        <span className="tc-tree-name" title={node.name}>
          {node.name}
        </span>
        <span className="tc-tree-count">{node.file_count}</span>
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
    return <div className="tc-tree-empty">Scanning...</div>;
  }

  if (!scanResult) {
    return <div className="tc-tree-empty">Open a folder to start</div>;
  }

  return (
    <div className="tc-tree">
      <TreeNode node={scanResult.directory_tree} level={0} />
    </div>
  );
}
