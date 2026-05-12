import { create } from "zustand";

const STORAGE_KEY = "coral.locale";

export type Locale = "en" | "es";

function readLocale(): Locale | null {
  if (typeof window === "undefined") return null;
  try {
    const v = window.localStorage.getItem(STORAGE_KEY);
    return v === "en" || v === "es" ? v : null;
  } catch {
    return null;
  }
}

function writeLocale(loc: Locale): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, loc);
  } catch {
    // ignore
  }
}

interface LocaleState {
  locale: Locale | null; // null = follow detection
  setLocale: (l: Locale) => void;
}

export const useLocaleStore = create<LocaleState>((set) => ({
  locale: readLocale(),
  setLocale: (locale) => {
    writeLocale(locale);
    set({ locale });
  },
}));
