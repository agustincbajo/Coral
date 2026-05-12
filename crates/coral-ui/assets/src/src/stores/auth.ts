import { create } from "zustand";

// NOTE(coral-ui frontend): bearer token persists across reloads in
// localStorage. We avoid the zustand `persist` middleware to keep the
// bundle minimal and the contract explicit.

const STORAGE_KEY = "coral.auth.token";

function readToken(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeToken(token: string | null): void {
  if (typeof window === "undefined") return;
  try {
    if (token) window.localStorage.setItem(STORAGE_KEY, token);
    else window.localStorage.removeItem(STORAGE_KEY);
  } catch {
    // ignore quota / privacy-mode errors
  }
}

interface AuthState {
  token: string | null;
  setToken: (token: string | null) => void;
  clear: () => void;
}

export const useAuthStore = create<AuthState>((set) => ({
  token: readToken(),
  setToken: (token) => {
    writeToken(token);
    set({ token });
  },
  clear: () => {
    writeToken(null);
    set({ token: null });
  },
}));
