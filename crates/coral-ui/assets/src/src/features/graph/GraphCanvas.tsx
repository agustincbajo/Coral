import { useEffect, useMemo, useRef } from "react";
import {
  SigmaContainer,
  useLoadGraph,
  useRegisterEvents,
  useSigma,
} from "@react-sigma/core";
import Graph from "graphology";
import forceAtlas2 from "graphology-layout-forceatlas2";
import noverlap from "graphology-layout-noverlap";
import "@react-sigma/core/lib/style.css";

import type { GraphPayload, PageType, Status } from "@/lib/types";
import { STATUS_HEX } from "@/components/StatusBadge";
import { useGraphStore } from "@/stores/graph";

// NOTE(coral-ui frontend): page-type palette mirrors PageTypeBadge tones
// in saturated form so colour-by works in both modes.
const PAGE_TYPE_HEX: Record<PageType, string> = {
  module: "#0284c7",
  concept: "#4f46e5",
  entity: "#10b981",
  flow: "#d97706",
  decision: "#e11d48",
  synthesis: "#a21caf",
  operation: "#0d9488",
  source: "#475569",
  gap: "#ea580c",
  index: "#52525b",
  log: "#ca8a04",
  schema: "#0891b2",
  readme: "#78716c",
  reference: "#7c3aed",
  interface: "#65a30d",
};

function sqrtScale(value: number, min: number, max: number, lo: number, hi: number): number {
  if (max <= min) return (lo + hi) / 2;
  const t = Math.sqrt((value - min) / (max - min));
  return lo + t * (hi - lo);
}

function Loader({ payload }: { payload: GraphPayload }) {
  const loadGraph = useLoadGraph();
  const sigma = useSigma();
  const { layout, colorBy, sizeBy } = useGraphStore();
  const animRef = useRef<number | null>(null);

  useEffect(() => {
    const g = new Graph();

    const degrees = payload.nodes.map((n) => n.degree);
    const minDeg = degrees.length ? Math.min(...degrees) : 0;
    const maxDeg = degrees.length ? Math.max(...degrees) : 1;
    // Polar fallback positions for empty-state graphs that have no layout yet.
    payload.nodes.forEach((n, i) => {
      const angle = (i / Math.max(1, payload.nodes.length)) * Math.PI * 2;
      const colour =
        colorBy === "status"
          ? STATUS_HEX[n.status as Status] ?? "#94a3b8"
          : PAGE_TYPE_HEX[n.page_type as PageType] ?? "#94a3b8";
      const size =
        sizeBy === "degree"
          ? sqrtScale(n.degree, minDeg, maxDeg, 5, 25)
          : sqrtScale(n.confidence, 0, 1, 5, 25);
      g.addNode(n.id, {
        label: n.label || n.id,
        x: Math.cos(angle),
        y: Math.sin(angle),
        size,
        color: colour,
        // NOTE(coral-ui frontend): rgba opacity baked into colour at draw time;
        // Sigma respects per-node alpha through the colour channel.
        confidence: Math.max(0.4, Math.min(1, n.confidence || 0.6)),
      });
    });

    payload.edges.forEach((e, i) => {
      if (g.hasNode(e.source) && g.hasNode(e.target) && !g.hasEdge(e.source, e.target)) {
        g.addEdgeWithKey(`e${i}`, e.source, e.target, {
          color: "#cbd5e1",
          size: 0.8,
        });
      }
    });

    if (layout === "circular" || payload.nodes.length === 0) {
      // already polar
    } else if (layout === "noverlap") {
      noverlap.assign(g, { maxIterations: 50, settings: { margin: 5 } });
    } else {
      const settings = forceAtlas2.inferSettings(g);
      const start = performance.now();
      const step = () => {
        forceAtlas2.assign(g, { iterations: 1, settings });
        if (performance.now() - start < 2000) {
          animRef.current = requestAnimationFrame(step);
        }
      };
      animRef.current = requestAnimationFrame(step);
    }

    loadGraph(g);

    return () => {
      if (animRef.current) cancelAnimationFrame(animRef.current);
    };
  }, [payload, layout, colorBy, sizeBy, loadGraph, sigma]);

  return null;
}

function Events() {
  const setSelectedNode = useGraphStore((s) => s.setSelectedNode);
  const registerEvents = useRegisterEvents();
  useEffect(() => {
    registerEvents({
      clickNode: (e) => setSelectedNode(e.node),
      clickStage: () => setSelectedNode(null),
    });
  }, [registerEvents, setSelectedNode]);
  return null;
}

interface Props {
  payload: GraphPayload;
  height?: number;
}

export function GraphCanvas({ payload, height = 600 }: Props) {
  // remount the SigmaContainer when payload identity changes so layout reruns clean
  const key = useMemo(
    () => `${payload.nodes.length}-${payload.edges.length}`,
    [payload],
  );
  return (
    <div className="rounded-lg border overflow-hidden" style={{ height }}>
      <SigmaContainer
        key={key}
        style={{ height: "100%", width: "100%", background: "transparent" }}
        settings={{
          renderLabels: payload.nodes.length <= 120,
          labelSize: 12,
          labelWeight: "500",
        }}
      >
        <Loader payload={payload} />
        <Events />
      </SigmaContainer>
    </div>
  );
}
