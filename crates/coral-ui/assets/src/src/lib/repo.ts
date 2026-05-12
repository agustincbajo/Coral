// Repository identifier resolution.
//
// v0.32.2: multi-repo dynamic resolution from `/api/v1/manifest`. When
// the manifest lists a `repos` table the first entry's name is used as
// the active repo; the user can override via the FiltersSidebar repo
// input. When the manifest is missing or single-repo we fall back to
// `DEFAULT_REPO` (matches backend's default namespace).

import { useFiltersStore } from "@/stores/filters";
import { useManifest } from "@/features/manifest/useManifest";

export const DEFAULT_REPO = "default" as const;

interface ManifestShape {
  // `coral.toml` -> JSON shape after `toml::Value` -> `serde_json::Value`.
  // The `repos` table (when present) is `{ <repo-name>: { ... }, ... }`.
  repos?: Record<string, unknown>;
  // Some manifests use a flat `[[repo]]` array; tolerate both.
  repo?: Array<{ name?: string }>;
}

function repoFromManifest(m: unknown): string | null {
  if (!m || typeof m !== "object") return null;
  const obj = m as ManifestShape;
  if (obj.repos && typeof obj.repos === "object") {
    const names = Object.keys(obj.repos);
    if (names.length > 0) return names[0];
  }
  if (Array.isArray(obj.repo) && obj.repo.length > 0) {
    const first = obj.repo[0];
    if (first && typeof first.name === "string" && first.name) {
      return first.name;
    }
  }
  return null;
}

/**
 * Returns the repository identifier currently active in the UI.
 *
 * Resolution order:
 *   1. User's explicit override in `useFiltersStore().repo`.
 *   2. First repo entry in `/api/v1/manifest` if available.
 *   3. `DEFAULT_REPO` constant.
 */
export function useCurrentRepo(): string {
  const repoFilter = useFiltersStore((s) => s.repo);
  const { data: manifest } = useManifest();
  if (repoFilter) return repoFilter;
  const fromManifest = repoFromManifest(manifest);
  return fromManifest ?? DEFAULT_REPO;
}
