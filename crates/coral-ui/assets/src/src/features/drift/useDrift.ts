import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { DriftReport } from "@/lib/types";

export function useDrift() {
  return useQuery<DriftReport[]>({
    queryKey: ["contract_status"],
    queryFn: () => api<DriftReport[]>("/contract_status"),
  });
}
