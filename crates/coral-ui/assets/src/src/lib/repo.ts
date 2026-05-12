// Repository identifier resolution.
//
// M1 ships single-repo: every page lives under the `default` namespace
// on the backend. The constant below is the single place that knows
// this, so the M2 work (multi-repo dynamic resolution from
// `/api/v1/manifest`) only has to touch one file.
//
// NOTE(coral-ui M2): replace the constant with a Zustand store seeded
// from `useManifest()` once `coral.toml` lists multiple repos. The
// signature of `useCurrentRepo()` is forward-compatible.

import { useFiltersStore } from "@/stores/filters";

export const DEFAULT_REPO = "default" as const;

/**
 * Returns the repository identifier currently active in the UI.
 *
 * In M1 this is always `DEFAULT_REPO`, but components should call this
 * hook (or read from the filters store) instead of hard-coding the
 * literal so that multi-repo support in M2 is a single-file change.
 */
export function useCurrentRepo(): string {
  const repoFilter = useFiltersStore((s) => s.repo);
  return repoFilter || DEFAULT_REPO;
}
