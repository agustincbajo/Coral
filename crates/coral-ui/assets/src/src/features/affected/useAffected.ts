import { useQuery } from "@tanstack/react-query";
import { api, qs } from "@/lib/api";
import type { AffectedEnvelope } from "@/lib/types";

/**
 * Fires a query for affected repos given a git ref.
 *
 * The hook stays disabled until `since` is non-empty, so a fresh mount
 * doesn't issue a call. Call `refetch()` after the user clicks compute.
 */
export function useAffected(since: string) {
  return useQuery<AffectedEnvelope>({
    queryKey: ["affected", since],
    queryFn: () =>
      api<AffectedEnvelope>(`/affected${qs({ since })}`, { raw: true }),
    enabled: false,
    retry: false,
  });
}
