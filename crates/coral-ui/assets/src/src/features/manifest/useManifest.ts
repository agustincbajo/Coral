import { useQuery } from "@tanstack/react-query";
import { api, ApiError } from "@/lib/api";

export function useManifest() {
  return useQuery<unknown>({
    queryKey: ["manifest"],
    queryFn: async () => {
      try {
        return await api<unknown>("/manifest");
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) return null;
        throw e;
      }
    },
    retry: false,
  });
}

export function useLock() {
  return useQuery<unknown>({
    queryKey: ["lock"],
    queryFn: async () => {
      try {
        return await api<unknown>("/lock");
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) return null;
        throw e;
      }
    },
    retry: false,
  });
}
