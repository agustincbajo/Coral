import { useQuery, keepPreviousData } from "@tanstack/react-query";
import { api, qs } from "@/lib/api";
import type { GraphPayload } from "@/lib/types";
import { useGraphStore } from "@/stores/graph";

export function useGraph() {
  const { maxNodes, validAt } = useGraphStore();
  const params = {
    max_nodes: maxNodes,
    valid_at: validAt || undefined,
  };
  return useQuery<GraphPayload>({
    queryKey: ["graph", params],
    queryFn: () => api<GraphPayload>(`/graph${qs(params)}`),
    placeholderData: keepPreviousData,
  });
}
