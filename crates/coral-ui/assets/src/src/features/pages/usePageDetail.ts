import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { PageDetail } from "@/lib/types";

export function usePageDetail(repo: string | undefined, slug: string | undefined) {
  const effectiveRepo = repo || "default";
  return useQuery<PageDetail>({
    queryKey: ["page", effectiveRepo, slug],
    queryFn: () =>
      api<PageDetail>(
        `/pages/${encodeURIComponent(effectiveRepo)}/${encodeURIComponent(slug ?? "")}`,
      ),
    enabled: !!slug,
  });
}
