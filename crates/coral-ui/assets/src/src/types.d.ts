// NOTE(coral-ui frontend): runtime config is injected into index.html
// by the Rust backend by replacing `<!-- __CORAL_RUNTIME_CONFIG__ -->`
// with a `<script>window.__CORAL_CONFIG__ = {...}</script>` block.
// When loaded standalone (vite dev with no backend), it is undefined
// and the SPA falls back to compile-time defaults — see lib/config.ts.

export {};

declare global {
  interface CoralConfig {
    apiBase: string;
    authRequired: boolean;
    writeToolsEnabled: boolean;
    version: string;
    defaultLocale: "en" | "es";
  }

  interface Window {
    __CORAL_CONFIG__?: CoralConfig;
  }
}
