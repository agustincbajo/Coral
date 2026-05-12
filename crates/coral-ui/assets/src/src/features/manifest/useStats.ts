import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { Stats } from "@/lib/types";

export function useStats() {
  return useQuery<Stats>({
    queryKey: ["stats"],
    queryFn: () => api<Stats>("/stats"),
  });
}
