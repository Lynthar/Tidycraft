import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { ReactFlow, Background, Controls, type Node, type Edge } from "@xyflow/react";
import dagre from "@dagrejs/dagre";
import "@xyflow/react/dist/style.css";
import { useUiStore } from "../stores/uiStore";
import { useProjectStore } from "../stores/projectStore";
import { basename } from "../lib/pathUtils";
import type { DependencyGraph, DependencyNode } from "../types/asset";

const NODE_W = 180;
const NODE_H = 40;
const HOPS = 2;

/// BFS outward from `centerId` up to `hops`, following edges in BOTH directions
/// (we want both what the asset uses and what uses it). Returns the reachable
/// node ids plus the edges that stay within that set.
function extractSubgraph(
  graph: DependencyGraph,
  centerId: string,
  hops: number
): { nodeIds: Set<string>; edges: DependencyGraph["edges"] } {
  const adj = new Map<string, Set<string>>();
  for (const e of graph.edges) {
    (adj.get(e.from) ?? adj.set(e.from, new Set()).get(e.from)!).add(e.to);
    (adj.get(e.to) ?? adj.set(e.to, new Set()).get(e.to)!).add(e.from);
  }
  const nodeIds = new Set<string>([centerId]);
  let frontier = [centerId];
  for (let h = 0; h < hops; h++) {
    const next: string[] = [];
    for (const id of frontier) {
      for (const nb of adj.get(id) ?? []) {
        if (!nodeIds.has(nb)) {
          nodeIds.add(nb);
          next.push(nb);
        }
      }
    }
    frontier = next;
  }
  const edges = graph.edges.filter((e) => nodeIds.has(e.from) && nodeIds.has(e.to));
  return { nodeIds, edges };
}

/// dagre left→right layout: ranks nodes by dependency direction so the graph
/// reads "things that use X" → X → "things X uses". dagre returns center-based
/// coords; React Flow anchors at top-left, so shift by half the node size.
function layoutLR(nodes: Node[], edges: Edge[]): Node[] {
  const g = new dagre.graphlib.Graph();
  g.setDefaultEdgeLabel(() => ({}));
  g.setGraph({ rankdir: "LR", nodesep: 24, ranksep: 80 });
  nodes.forEach((n) => g.setNode(n.id, { width: NODE_W, height: NODE_H }));
  edges.forEach((e) => g.setEdge(e.source, e.target));
  dagre.layout(g);
  return nodes.map((n) => {
    const p = g.node(n.id);
    return { ...n, position: { x: p.x - NODE_W / 2, y: p.y - NODE_H / 2 } };
  });
}

export function DependencyGraphModal() {
  const { t } = useTranslation();
  const open = useUiStore((s) => s.depGraphOpen);
  const assetPath = useUiStore((s) => s.depGraphAssetPath);
  const setOpen = useUiStore((s) => s.setDepGraphOpen);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const scanResult = useProjectStore((s) => s.scanResult);
  const locateAsset = useProjectStore((s) => s.locateAsset);

  const projectType = scanResult?.project_type;
  const [graph, setGraph] = useState<DependencyGraph | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Fetch the full graph when the modal opens; the subgraph is derived locally.
  useEffect(() => {
    if (!open || !activeProjectId) return;
    // Guard against a stale graph resolving after the user switched projects
    // while the modal is open — a late response must not overwrite the current
    // project's graph.
    let cancelled = false;
    const cmd =
      projectType === "godot" ? "get_godot_dependencies" : "get_unity_dependencies";
    setLoading(true);
    setError(null);
    setGraph(null);
    invoke<DependencyGraph>(cmd, { projectId: activeProjectId })
      .then((g) => { if (!cancelled) setGraph(g); })
      .catch((e) => { if (!cancelled) setError(String(e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => {
      cancelled = true;
    };
  }, [open, activeProjectId, projectType]);

  const { nodes, edges } = useMemo(() => {
    const empty = { nodes: [] as Node[], edges: [] as Edge[] };
    if (!graph || !assetPath) return empty;
    const center = graph.nodes.find((n) => n.path === assetPath);
    if (!center) return empty;

    const { nodeIds, edges: subEdges } = extractSubgraph(graph, center.id, HOPS);
    const byId = new Map(graph.nodes.map((n) => [n.id, n]));

    const rfNodes: Node[] = Array.from(nodeIds)
      .map((id) => byId.get(id))
      .filter((n): n is DependencyNode => !!n)
      .map((n) => ({
        id: n.id,
        position: { x: 0, y: 0 },
        data: { label: n.name, path: n.path },
        style: {
          width: NODE_W,
          fontSize: 11,
          padding: 6,
          borderRadius: 6,
          background: "var(--panel-2)",
          color: "var(--text-3)",
          border: `1px solid ${n.id === center.id ? "var(--primary)" : "var(--line)"}`,
        },
      }));

    const rfEdges: Edge[] = subEdges.map((e, i) => ({
      id: `e${i}`,
      source: e.from,
      target: e.to,
    }));

    return { nodes: layoutLR(rfNodes, rfEdges), edges: rfEdges };
  }, [graph, assetPath]);

  if (!open) return null;

  const centerName = assetPath ? basename(assetPath) : "";

  return (
    <div
      onClick={() => setOpen(false)}
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.5)" }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="bg-card-bg border border-border rounded-lg flex flex-col overflow-hidden"
        style={{ width: "82vw", height: "82vh", maxWidth: 1100 }}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <h2 className="text-sm font-semibold text-text-primary truncate">
            {t("depGraph.title", { name: centerName })}
          </h2>
          <button
            onClick={() => setOpen(false)}
            className="text-text-secondary hover:text-text-primary shrink-0"
            title={t("depGraph.close")}
          >
            <X size={16} />
          </button>
        </div>

        <div style={{ flex: 1, minHeight: 0 }}>
          {loading ? (
            <div className="flex items-center justify-center h-full text-text-secondary text-sm">
              {t("depGraph.loading")}
            </div>
          ) : error ? (
            <div className="flex items-center justify-center h-full text-text-secondary text-sm">
              {error}
            </div>
          ) : nodes.length === 0 ? (
            <div className="flex items-center justify-center h-full text-text-secondary text-sm">
              {t("depGraph.empty")}
            </div>
          ) : (
            <ReactFlow
              nodes={nodes}
              edges={edges}
              fitView
              nodesDraggable={false}
              onNodeClick={(_, node) => {
                const p = (node.data as { path?: string }).path;
                if (p) {
                  locateAsset(p);
                  setOpen(false);
                }
              }}
            >
              <Background />
              <Controls showInteractive={false} />
            </ReactFlow>
          )}
        </div>

        <p className="px-4 py-2 text-[11px] text-text-secondary border-t border-border">
          {t("depGraph.hint")}
        </p>
      </div>
    </div>
  );
}
