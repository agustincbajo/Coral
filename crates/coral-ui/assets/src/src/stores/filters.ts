import { create } from "zustand";
import type { PageType, Status } from "@/lib/types";

interface FiltersState {
  q: string;
  pageTypes: PageType[];
  statuses: Status[];
  confidenceMin: number;
  confidenceMax: number;
  repo: string;
  validAt: string;
  page: number;
  pageSize: number;
  setQ: (q: string) => void;
  togglePageType: (t: PageType) => void;
  toggleStatus: (s: Status) => void;
  setConfidenceRange: (min: number, max: number) => void;
  setRepo: (r: string) => void;
  setValidAt: (iso: string) => void;
  setPage: (p: number) => void;
  setPageSize: (n: number) => void;
  reset: () => void;
}

const INITIAL = {
  q: "",
  pageTypes: [] as PageType[],
  statuses: [] as Status[],
  confidenceMin: 0,
  confidenceMax: 1,
  repo: "",
  validAt: "",
  page: 0,
  pageSize: 25,
};

export const useFiltersStore = create<FiltersState>((set) => ({
  ...INITIAL,
  setQ: (q) => set({ q, page: 0 }),
  togglePageType: (t) =>
    set((st) => ({
      pageTypes: st.pageTypes.includes(t)
        ? st.pageTypes.filter((x) => x !== t)
        : [...st.pageTypes, t],
      page: 0,
    })),
  toggleStatus: (s) =>
    set((st) => ({
      statuses: st.statuses.includes(s)
        ? st.statuses.filter((x) => x !== s)
        : [...st.statuses, s],
      page: 0,
    })),
  setConfidenceRange: (confidenceMin, confidenceMax) =>
    set({ confidenceMin, confidenceMax, page: 0 }),
  setRepo: (repo) => set({ repo, page: 0 }),
  setValidAt: (validAt) => set({ validAt, page: 0 }),
  setPage: (page) => set({ page }),
  setPageSize: (pageSize) => set({ pageSize, page: 0 }),
  reset: () => set({ ...INITIAL }),
}));
