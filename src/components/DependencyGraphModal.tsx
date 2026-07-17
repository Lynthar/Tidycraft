import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { ModalShell } from "./ModalShell";
import { X } from "lucide-react";
import { ReactFlow, Background, Controls, MarkerType, type Node, type Edge } from "@xyflow/react";
import dagre from "@dagrejs/dagre";
import "@xyflow/react/dist/style.css";
import { useUiStore } from "../stores/uiStore";
import { useProjectStore } from "../stores/projectStore";
import { basename } from "../lib/pathUtils";
import type { DependencyGraph, DependencyNode } from "../types/asset";

const NODE_W = 180;
const NODE_H = 40;
const HOPS = 2;

/// The center's neighborhood, walked DIRECTION-CONSISTENTLY: one sweep
/// follows only outgoing edges ("what it uses", up to `hops` levels) and one
/// only incoming edges ("what uses it") — a path never flips direction
/// midway. Mixed-direction walks read as relevance but deliver siblings: any
/// shared dependency (an atlas, a package shader) is one hop down, and the
/// flip-back hop would drag in every OTHER user of it — on real projects
/// that's the entire material list. Same Uses / Used-by split as Unity's own
/// dependency viewer. Edges between reached nodes are all kept (true
/// information); they just don't expand the set.
///
/// Non-`asset` nodes (package / unresolved / unscanned / missing) stay
/// TERMINALS on top of that: nothing is known about their far side, so
/// expansion stops at them in both sweeps.
function extractSubgraph(
  graph: DependencyGraph,
  centerId: string,
  hops: number
): { nodeIds: Set<string>; edges: DependencyGraph["edges"] } {
  const terminals = new Set(
    graph.nodes.filter((n) => n.kind !== "asset").map((n) => n.id)
  );
  const out = new Map<string, Set<string>>();
  const inc = new Map<string, Set<string>>();
  for (const e of graph.edges) {
    (out.get(e.from) ?? out.set(e.from, new Set()).get(e.from)!).add(e.to);
    (inc.get(e.to) ?? inc.set(e.to, new Set()).get(e.to)!).add(e.from);
  }
  const nodeIds = new Set<string>([centerId]);
  // Per-sweep visited set: a node reached downstream may still need its own
  // upstream walk in the other sweep (cycles aside, the global set would cut
  // the second sweep short).
  const sweep = (adj: Map<string, Set<string>>) => {
    const seen = new Set<string>([centerId]);
    let frontier = [centerId];
    for (let h = 0; h < hops; h++) {
      const next: string[] = [];
      for (const id of frontier) {
        if (terminals.has(id)) continue;
        for (const nb of adj.get(id) ?? []) {
          if (!seen.has(nb)) {
            seen.add(nb);
            nodeIds.add(nb);
            next.push(nb);
          }
        }
      }
      frontier = next;
    }
  };
  sweep(out);
  sweep(inc);
  const edges = graph.edges.filter((e) => nodeIds.has(e.from) && nodeIds.has(e.to));
  return { nodeIds, edges };
}

/// i18n keys per non-asset node kind: the hover explanation and the footer
/// legend label. Colors: unresolved reads as a warning (ambiguous, not
/// asserted broken), missing as an error (confirmed absent from disk),
/// package and unscanned stay neutral with a dashed border — both mean
/// "exists outside the scan", and they never co-occur (package is
/// Unity-only, unscanned Godot-only) so the shared look stays unambiguous.
const KIND_TIP = {
  package: "depGraph.packageTip",
  unresolved: "depGraph.unresolvedTip",
  unscanned: "depGraph.unscannedTip",
  missing: "depGraph.missingTip",
} as const;
const KIND_LEGEND = {
  package: "depGraph.legendPackage",
  unresolved: "depGraph.legendUnresolved",
  unscanned: "depGraph.legendUnscanned",
  missing: "depGraph.legendMissing",
} as const;
type SpecialKind = keyof typeof KIND_TIP;

function kindTone(kind: SpecialKind): string | null {
  return kind === "unresolved" ? "var(--warn)" : kind === "missing" ? "var(--err)" : null;
}
function kindBackground(kind: SpecialKind): string {
  const tone = kindTone(kind);
  return tone ? `color-mix(in oklab, ${tone} 14%, var(--panel-2))` : "var(--panel-2)";
}
function kindBorder(kind: SpecialKind): string {
  const tone = kindTone(kind);
  return tone ? `1px solid ${tone}` : "1px dashed var(--line)";
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
      .map((n) => {
        const isCenter = n.id === center.id;
        if (n.kind !== "asset") {
          // Identity stays visible — a package node shows its file name, a
          // Godot res:// path is actionable as-is, a Unity GUID shortens to
          // a searchable 8-char prefix — and the explanation, the package id
          // (when known) and the full string live in the hover title.
          // (div styles resolve CSS var() fine, unlike SVG edge markers.)
          const label = (
            <span
              title={[t(KIND_TIP[n.kind]), n.detail, n.name].filter(Boolean).join("\n")}
              style={{
                display: "block",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                fontFamily: n.kind === "unresolved" ? "monospace" : undefined,
              }}
            >
              {n.kind === "unresolved" ? `${n.name.slice(0, 8)}…` : n.name}
            </span>
          );
          return {
            id: n.id,
            position: { x: 0, y: 0 },
            data: { label, path: n.path, kind: n.kind },
            style: {
              width: NODE_W,
              fontSize: 11,
              padding: 6,
              borderRadius: 6,
              background: kindBackground(n.kind),
              color: kindTone(n.kind) ?? "var(--text-3)",
              border: kindBorder(n.kind),
              cursor: "default", // path is empty — nothing to locate
            },
          };
        }
        return {
          id: n.id,
          position: { x: 0, y: 0 },
          data: { label: n.name, path: n.path, kind: n.kind },
          style: {
            width: NODE_W,
            fontSize: 11,
            padding: 6,
            borderRadius: 6,
            background: "var(--panel-2)",
            color: "var(--text-3)",
            border: `1px solid ${isCenter ? "var(--primary)" : "var(--line)"}`,
            cursor: "pointer",
          },
        };
      });

    const rfEdges: Edge[] = subEdges.map((e, i) => ({
      id: `e${i}`,
      source: e.from,
      target: e.to,
      // Arrowhead at the target: an edge means "source uses target", so the
      // graph now reads directionally. No custom colour — the SVG marker
      // attribute wouldn't resolve a CSS var() (same gotcha as recharts fills),
      // and React Flow's default grey already matches the edge line.
      markerEnd: { type: MarkerType.ArrowClosed },
    }));

    return { nodes: layoutLR(rfNodes, rfEdges), edges: rfEdges };
  }, [graph, assetPath, t]);

  if (!open) return null;

  const centerName = assetPath ? basename(assetPath) : "";

  // Footer legend, limited to the special kinds actually on screen — no
  // dead swatches, and an all-asset neighborhood shows none at all.
  const specialKinds = (["package", "unresolved", "unscanned", "missing"] as const).filter(
    (k) => nodes.some((n) => (n.data as { kind?: string }).kind === k)
  );

  return (
    <ModalShell
      onClose={() => setOpen(false)}
      ariaLabel={t("depGraph.title", { name: centerName })}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
    >
      <div
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

        <div className="px-4 py-2 text-[11px] text-text-secondary border-t border-border flex items-center gap-4 flex-wrap">
          <span>{t("depGraph.hint")}</span>
          <span style={{ flex: 1 }} />
          {specialKinds.map((k) => (
            <span key={k} className="inline-flex items-center gap-1.5" title={t(KIND_TIP[k])}>
              <span
                style={{
                  width: 10,
                  height: 10,
                  borderRadius: 3,
                  display: "inline-block",
                  background: kindBackground(k),
                  border: kindBorder(k),
                }}
              />
              {t(KIND_LEGEND[k])}
            </span>
          ))}
        </div>
      </div>
    </ModalShell>
  );
}
