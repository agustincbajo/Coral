import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { InterfaceSummary } from "@/lib/types";

export function useInterfaces() {
  return useQuery<InterfaceSummary[]>({
    queryKey: ["interfaces"],
    queryFn: () => api<InterfaceSummary[]>("/interfaces"),
  });
}
