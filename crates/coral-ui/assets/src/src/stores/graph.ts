import { create } from "zustand";

export type Layout = "forceatlas2" | "circular" | "noverlap";
export type ColorBy = "status" | "page_type";
export type SizeBy = "degree" | "confidence";

interface GraphState {
  layout: Layout;
  colorBy: ColorBy;
  sizeBy: SizeBy;
  maxNodes: number;
  validAt: string; // ISO datetime; empty means "now"
  selectedNode: string | null;
  setLayout: (l: Layout) => void;
  setColorBy: (c: ColorBy) => void;
  setSizeBy: (s: SizeBy) => void;
  setMaxNodes: (n: number) => void;
  setValidAt: (iso: string) => void;
  setSelectedNode: (id: string | null) => void;
}

export const useGraphStore = create<GraphState>((set) => ({
  layout: "forceatlas2",
  colorBy: "status",
  sizeBy: "degree",
  maxNodes: 500,
  validAt: "",
  selectedNode: null,
  setLayout: (layout) => set({ layout }),
  setColorBy: (colorBy) => set({ colorBy }),
  setSizeBy: (sizeBy) => set({ sizeBy }),
  setMaxNodes: (maxNodes) => set({ maxNodes }),
  setValidAt: (validAt) => set({ validAt }),
  setSelectedNode: (selectedNode) => set({ selectedNode }),
}));
