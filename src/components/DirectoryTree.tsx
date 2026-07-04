import { useState } from "react";
import { ChevronRight, Folder, FolderOpen } from "lucide-react";
import { useProjectStore } from "../stores/projectStore";
import { useShallow } from "zustand/react/shallow";
import type { DirectoryNode } from "../types/asset";
import { useTranslation } from "react-i18next";

interface TreeNodeProps {
  node: DirectoryNode;
  level: number;
  // Passed down rather than pulled via useTranslation per node — the tree can
  // render many nodes and per-node i18n subscriptions would be wasted work
  // (same rationale as GitStatusBadge).
  t: (key: string) => string;
}

function TreeNode({ node, level, t }: TreeNodeProps) {
  const [isExpanded, setIsExpanded] = useState(level === 0);
  // Selector subscription: TreeNode instances are per-directory and the
  // tree can hold hundreds of them — a whole-store subscribe re-rendered
  // every node at 10Hz during any background scan.
  const { selectedDirectory, setSelectedDirectory } = useProjectStore(
    useShallow((s) => ({ selectedDirectory: s.selectedDirectory, setSelectedDirectory: s.setSelectedDirectory }))
  );

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
          aria-label={hasChildren ? (isExpanded ? t("directoryTree.collapse") : t("directoryTree.expand")) : undefined}
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
            <TreeNode key={child.path} node={child} level={level + 1} t={t} />
          ))}
        </div>
      )}
    </div>
  );
}

export function DirectoryTree() {
  const { scanResult, isScanning } = useProjectStore(
    useShallow((s) => ({ scanResult: s.scanResult, isScanning: s.isScanning }))
  );
  const { t } = useTranslation();

  if (isScanning) {
    return <div className="tc-tree-empty">{t("directoryTree.scanning")}</div>;
  }

  if (!scanResult) {
    return <div className="tc-tree-empty">{t("directoryTree.empty")}</div>;
  }

  return (
    <div className="tc-tree">
      <TreeNode node={scanResult.directory_tree} level={0} t={t} />
    </div>
  );
}
