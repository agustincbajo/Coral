import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Search, X } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { useFiltersStore } from "@/stores/filters";
import { ALL_PAGE_TYPES, ALL_STATUSES } from "@/lib/types";
import { cn } from "@/lib/utils";

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setV(value), ms);
    return () => clearTimeout(id);
  }, [value, ms]);
  return v;
}

export function FiltersSidebar() {
  const { t } = useTranslation();
  const f = useFiltersStore();

  // Debounced search input
  const [rawQ, setRawQ] = useState(f.q);
  const debouncedQ = useDebounced(rawQ, 200);
  useEffect(() => {
    if (debouncedQ !== f.q) f.setQ(debouncedQ);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [debouncedQ]);

  const [rawRepo, setRawRepo] = useState(f.repo);
  const debouncedRepo = useDebounced(rawRepo, 200);
  useEffect(() => {
    if (debouncedRepo !== f.repo) f.setRepo(debouncedRepo);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [debouncedRepo]);

  const activeCount =
    (f.q ? 1 : 0) +
    f.pageTypes.length +
    f.statuses.length +
    (f.confidenceMin > 0 || f.confidenceMax < 1 ? 1 : 0) +
    (f.repo ? 1 : 0) +
    (f.validAt ? 1 : 0);

  return (
    <aside className="w-64 shrink-0 space-y-5 sticky top-20 self-start">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold">{t("pages.filters.title")}</h2>
        {activeCount > 0 ? (
          <Badge variant="secondary" className="text-[10px]">
            {t("pages.filters.applied_count", { count: activeCount })}
          </Badge>
        ) : null}
      </div>

      <div className="relative">
        <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
        <Input
          className="pl-8"
          placeholder={t("pages.filters.search_placeholder")}
          value={rawQ}
          onChange={(e) => setRawQ(e.target.value)}
        />
      </div>

      <div className="space-y-2">
        <Label className="text-xs uppercase text-muted-foreground">
          {t("pages.filters.page_type")}
        </Label>
        <div className="flex flex-wrap gap-1">
          {ALL_PAGE_TYPES.map((pt) => {
            const active = f.pageTypes.includes(pt);
            return (
              <button
                key={pt}
                type="button"
                onClick={() => f.togglePageType(pt)}
                className={cn(
                  "text-xs rounded-md border px-2 py-0.5 transition-colors",
                  active
                    ? "bg-primary text-primary-foreground border-primary"
                    : "bg-background hover:bg-accent",
                )}
              >
                {t(`pageType.${pt}`)}
              </button>
            );
          })}
        </div>
      </div>

      <div className="space-y-2">
        <Label className="text-xs uppercase text-muted-foreground">
          {t("pages.filters.status")}
        </Label>
        <div className="flex flex-wrap gap-1">
          {ALL_STATUSES.map((s) => {
            const active = f.statuses.includes(s);
            return (
              <button
                key={s}
                type="button"
                onClick={() => f.toggleStatus(s)}
                className={cn(
                  "text-xs rounded-md border px-2 py-0.5 transition-colors",
                  active
                    ? "bg-primary text-primary-foreground border-primary"
                    : "bg-background hover:bg-accent",
                )}
              >
                {t(`status.${s}`)}
              </button>
            );
          })}
        </div>
      </div>

      <div className="space-y-2">
        <Label className="text-xs uppercase text-muted-foreground">
          {t("pages.filters.confidence_range")}
        </Label>
        <Slider
          min={0}
          max={1}
          step={0.05}
          value={[f.confidenceMin, f.confidenceMax]}
          onValueChange={(v) => {
            if (v.length === 2) f.setConfidenceRange(v[0], v[1]);
          }}
        />
        <div className="flex justify-between text-xs text-muted-foreground tabular-nums">
          <span>{Math.round(f.confidenceMin * 100)}%</span>
          <span>{Math.round(f.confidenceMax * 100)}%</span>
        </div>
      </div>

      <div className="space-y-2">
        <Label className="text-xs uppercase text-muted-foreground">
          {t("pages.filters.repo")}
        </Label>
        <Input
          placeholder={t("pages.filters.repo_placeholder")}
          value={rawRepo}
          onChange={(e) => setRawRepo(e.target.value)}
        />
      </div>

      <div className="space-y-2">
        <Label className="text-xs uppercase text-muted-foreground">
          {t("pages.filters.valid_at")}
        </Label>
        <Input
          type="date"
          value={f.validAt}
          onChange={(e) => f.setValidAt(e.target.value)}
        />
        <p className="text-[11px] text-muted-foreground">
          {t("pages.filters.valid_at_helper")}
        </p>
      </div>

      <Separator />

      <Button
        variant="ghost"
        size="sm"
        onClick={() => {
          setRawQ("");
          setRawRepo("");
          f.reset();
        }}
        className="w-full"
      >
        <X className="mr-1 h-4 w-4" />
        {t("pages.filters.reset")}
      </Button>
    </aside>
  );
}
