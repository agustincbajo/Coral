import { useQuery, keepPreviousData } from "@tanstack/react-query";
import { api, qs } from "@/lib/api";
import type { PagesEnvelope } from "@/lib/types";
import { useFiltersStore } from "@/stores/filters";

export function usePages() {
  const f = useFiltersStore();
  const params = {
    q: f.q,
    page_type: f.pageTypes,
    status: f.statuses,
    confidence_min: f.confidenceMin > 0 ? f.confidenceMin : undefined,
    confidence_max: f.confidenceMax < 1 ? f.confidenceMax : undefined,
    repo: f.repo,
    valid_at: f.validAt,
    limit: f.pageSize,
    offset: f.page * f.pageSize,
  };
  return useQuery<PagesEnvelope>({
    queryKey: ["pages", params],
    queryFn: () => api<PagesEnvelope>(`/pages${qs(params)}`, { raw: true }),
    placeholderData: keepPreviousData,
  });
}
