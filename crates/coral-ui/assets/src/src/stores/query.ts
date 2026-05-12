import { create } from "zustand";

export type QueryMode = "local" | "global" | "hybrid";

export interface QueryTurn {
  id: string;
  question: string;
  mode: QueryMode;
  answer: string;
  sources: string[];
  status: "pending" | "streaming" | "done" | "error";
  errorMessage?: string;
}

interface QueryHistoryState {
  turns: QueryTurn[];
  push: (turn: QueryTurn) => void;
  appendToken: (id: string, text: string) => void;
  addSource: (id: string, slug: string) => void;
  finish: (id: string, status: "done" | "error", message?: string) => void;
  clear: () => void;
}

export const useQueryHistory = create<QueryHistoryState>((set) => ({
  turns: [],
  push: (turn) => set((s) => ({ turns: [...s.turns, turn] })),
  appendToken: (id, text) =>
    set((s) => ({
      turns: s.turns.map((t) =>
        t.id === id ? { ...t, answer: t.answer + text, status: "streaming" } : t,
      ),
    })),
  addSource: (id, slug) =>
    set((s) => ({
      turns: s.turns.map((t) =>
        t.id === id
          ? { ...t, sources: t.sources.includes(slug) ? t.sources : [...t.sources, slug] }
          : t,
      ),
    })),
  finish: (id, status, errorMessage) =>
    set((s) => ({
      turns: s.turns.map((t) => (t.id === id ? { ...t, status, errorMessage } : t)),
    })),
  clear: () => set({ turns: [] }),
}));
