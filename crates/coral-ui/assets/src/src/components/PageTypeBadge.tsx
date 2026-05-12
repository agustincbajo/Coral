import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import type { PageType } from "@/lib/types";
import { cn } from "@/lib/utils";

// NOTE(coral-ui frontend): keep semantic tone-classes here so badges
// read at-a-glance even without locale text.
const TONES: Record<PageType, string> = {
  module: "bg-sky-100 text-sky-900",
  concept: "bg-indigo-100 text-indigo-900",
  entity: "bg-emerald-100 text-emerald-900",
  flow: "bg-amber-100 text-amber-900",
  decision: "bg-rose-100 text-rose-900",
  synthesis: "bg-fuchsia-100 text-fuchsia-900",
  operation: "bg-teal-100 text-teal-900",
  source: "bg-slate-100 text-slate-900",
  gap: "bg-orange-100 text-orange-900",
  index: "bg-zinc-200 text-zinc-900",
  log: "bg-yellow-100 text-yellow-900",
  schema: "bg-cyan-100 text-cyan-900",
  readme: "bg-stone-100 text-stone-900",
  reference: "bg-violet-100 text-violet-900",
  interface: "bg-lime-100 text-lime-900",
};

export function PageTypeBadge({
  type,
  className,
}: {
  type: PageType;
  className?: string;
}) {
  const { t } = useTranslation();
  return (
    <Badge
      variant="outline"
      className={cn("border-transparent", TONES[type], className)}
    >
      {t(`pageType.${type}`)}
    </Badge>
  );
}
