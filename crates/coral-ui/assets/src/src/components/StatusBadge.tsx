import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import type { Status } from "@/lib/types";
import { cn } from "@/lib/utils";

// NOTE(coral-ui frontend): the same color map is used in GraphCanvas.
export const STATUS_HEX: Record<Status, string> = {
  draft: "#9ca3af",
  reviewed: "#3b82f6",
  verified: "#22c55e",
  stale: "#f59e0b",
  archived: "#ef4444",
  reference: "#8b5cf6",
};

const TONES: Record<Status, string> = {
  draft: "bg-gray-200 text-gray-900",
  reviewed: "bg-blue-100 text-blue-900",
  verified: "bg-green-100 text-green-900",
  stale: "bg-amber-100 text-amber-900",
  archived: "bg-red-100 text-red-900",
  reference: "bg-violet-100 text-violet-900",
};

export function StatusBadge({
  status,
  className,
  large,
}: {
  status: Status;
  className?: string;
  large?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <Badge
      variant="outline"
      className={cn(
        "border-transparent",
        TONES[status],
        large && "text-sm px-3 py-1",
        className,
      )}
    >
      {t(`status.${status}`)}
    </Badge>
  );
}
