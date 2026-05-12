import { useCallback, useEffect, useMemo, useRef } from "react";
import {
  SigmaContainer,
  useLoadGraph,
  useRegisterEvents,
  useSigma,
} from "@react-sigma/core";
import { useTranslation } from "react-i18next";
import { Download } from "lucide-react";
import Graph from "graphology";
import forceAtlas2 from "graphology-layout-forceatlas2";
import noverlap from "graphology-layout-noverlap";
import "@react-sigma/core/lib/style.css";

import type { GraphPayload, PageType, Status } from "@/lib/types";
import { STATUS_HEX } from "@/components/StatusBadge";
import { useGraphStore } from "@/stores/graph";
import { Button } from "@/components/ui/button";
import { useToast } from "@/components/ui/toaster";

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

// Convert "#rrggbb" + alpha to "rgba(r, g, b, a)". Sigma's default node
// program reads the alpha channel from the colour string at draw time,
// so this is what gives us opacity-by-confidence.
function hexToRgba(hex: string, alpha: number): string {
  const normalised = hex.startsWith("#") ? hex.slice(1) : hex;
  if (normalised.length !== 6) return `rgba(148, 163, 184, ${alpha})`;
  const r = parseInt(normalised.slice(0, 2), 16);
  const g = parseInt(normalised.slice(2, 4), 16);
  const b = parseInt(normalised.slice(4, 6), 16);
  if (Number.isNaN(r) || Number.isNaN(g) || Number.isNaN(b)) {
    return `rgba(148, 163, 184, ${alpha})`;
  }
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
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
      const hex =
        colorBy === "status"
          ? STATUS_HEX[n.status as Status] ?? "#94a3b8"
          : PAGE_TYPE_HEX[n.page_type as PageType] ?? "#94a3b8";
      const size =
        sizeBy === "degree"
          ? sqrtScale(n.degree, minDeg, maxDeg, 5, 25)
          : sqrtScale(n.confidence, 0, 1, 5, 25);
      // Opacity by confidence — clamped to [0.4, 1] so even Draft pages
      // (confidence ≈ 0.5) stay readable. Sigma honours the alpha
      // channel in rgba() colours natively.
      const alpha = Math.max(0.4, Math.min(1, n.confidence || 0.6));
      const colour = hexToRgba(hex, alpha);
      g.addNode(n.id, {
        label: n.label || n.id,
        x: Math.cos(angle),
        y: Math.sin(angle),
        size,
        color: colour,
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

// Floating export button. Lives *inside* `<SigmaContainer>` so it has
// access to the Sigma instance via `useSigma()`. Composites the WebGL
// canvas + the labels canvas onto a single PNG via `toDataURL` and
// triggers a same-tab download via a synthetic anchor click.
function ExportButton() {
  const { t } = useTranslation();
  const sigma = useSigma();
  const toast = useToast();
  const onClick = useCallback(() => {
    try {
      const renderer = sigma.getCanvases();
      // Layer order in Sigma v3: edges, nodes, labels, hovers, mouse.
      // We composite onto a single offscreen canvas before serialising.
      const sample = renderer.nodes ?? renderer.edges ?? renderer.labels;
      if (!sample) return;
      const w = sample.width;
      const h = sample.height;
      const out = document.createElement("canvas");
      out.width = w;
      out.height = h;
      const ctx = out.getContext("2d");
      if (!ctx) return;
      // White background so dark-mode previews still produce a readable
      // exported image; consumers can re-fill in image editors if they
      // want transparency.
      ctx.fillStyle = "#ffffff";
      ctx.fillRect(0, 0, w, h);
      for (const layer of ["edges", "nodes", "labels"] as const) {
        const c = renderer[layer];
        if (c) ctx.drawImage(c, 0, 0);
      }
      const url = out.toDataURL("image/png");
      const a = document.createElement("a");
      a.href = url;
      a.download = `coral-graph-${new Date().toISOString().slice(0, 10)}.png`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      toast({
        title: t("graph.toast.exported"),
        description: a.download,
        variant: "success",
      });
    } catch (e) {
      // eslint-disable-next-line no-console
      console.warn("[Coral UI] export PNG failed:", e);
      toast({
        title: t("graph.toast.export_failed"),
        description: e instanceof Error ? e.message : String(e),
        variant: "error",
      });
    }
  }, [sigma, toast, t]);
  return (
    <Button
      size="sm"
      variant="secondary"
      onClick={onClick}
      className="absolute right-2 top-2 z-10 shadow"
      title={t("graph.controls.export_png")}
    >
      <Download className="h-4 w-4 mr-1" />
      {t("graph.controls.export_png")}
    </Button>
  );
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
    <div className="relative rounded-lg border overflow-hidden" style={{ height }}>
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
        <ExportButton />
      </SigmaContainer>
    </div>
  );
}
