import { cn } from "@/lib/utils";

// Renders a horizontal bar 0-1 with a numeric label.
export function ConfidenceBar({
  value,
  className,
  showLabel = true,
}: {
  value: number;
  className?: string;
  showLabel?: boolean;
}) {
  const v = Math.max(0, Math.min(1, value || 0));
  const pct = Math.round(v * 100);
  // Color: red → amber → green linear ramp.
  let bar = "bg-red-500";
  if (v >= 0.75) bar = "bg-emerald-500";
  else if (v >= 0.4) bar = "bg-amber-500";
  return (
    <div className={cn("flex items-center gap-2", className)}>
      <div className="relative h-2 w-24 rounded-full bg-muted overflow-hidden">
        <div
          className={cn("absolute inset-y-0 left-0 rounded-full", bar)}
          style={{ width: `${pct}%` }}
        />
      </div>
      {showLabel ? (
        <span className="text-xs tabular-nums text-muted-foreground">
          {pct}%
        </span>
      ) : null}
    </div>
  );
}
