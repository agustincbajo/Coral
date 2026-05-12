// NOTE(coral-ui frontend): accessors for the runtime config injected by
// the Rust backend. When running under `vite dev` standalone (no backend
// in front), `window.__CORAL_CONFIG__` is undefined; we fall back to
// reasonable defaults so the SPA still boots.

const FALLBACK: CoralConfig = {
  apiBase: "/api/v1",
  authRequired: false,
  writeToolsEnabled: false,
  version: "dev",
  defaultLocale: "en",
};

export function getConfig(): CoralConfig {
  if (typeof window !== "undefined" && window.__CORAL_CONFIG__) {
    return window.__CORAL_CONFIG__;
  }
  return FALLBACK;
}

export function getApiBase(): string {
  return getConfig().apiBase || "/api/v1";
}
