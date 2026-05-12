import { useQuery } from "@tanstack/react-query";
import { api, qs } from "@/lib/api";
import type { GuaranteeEnvelope } from "@/lib/types";

interface UseGuaranteeArgs {
  env: string;
  strict: boolean;
}

/**
 * Disabled by default — the route component calls `refetch()` when the
 * user clicks Check. The key includes both args so two distinct
 * (env, strict) combinations cache independently.
 */
export function useGuarantee({ env, strict }: UseGuaranteeArgs) {
  return useQuery<GuaranteeEnvelope>({
    queryKey: ["guarantee", env, strict],
    queryFn: () =>
      api<GuaranteeEnvelope>(
        `/guarantee${qs({ env: env || undefined, strict })}`,
        { raw: true },
      ),
    enabled: false,
    retry: false,
  });
}
