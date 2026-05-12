import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";
import { useGraph } from "@/features/graph/useGraph";
import { GraphCanvas } from "@/features/graph/GraphCanvas";
import {
  GraphErrorBoundary,
  GraphFallback,
  hasWebGL2,
} from "@/features/graph/GraphErrorBoundary";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Slider } from "@/components/ui/slider";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { PageTypeBadge } from "@/components/PageTypeBadge";
import { StatusBadge } from "@/components/StatusBadge";
import { ConfidenceBar } from "@/components/ConfidenceBar";
import { usePageDetail } from "@/features/pages/usePageDetail";
import { useGraphStore, type ColorBy, type Layout, type SizeBy } from "@/stores/graph";
import { formatDate } from "@/lib/utils";

function dayToIso(day: number): string {
  return new Date(day * 86_400_000).toISOString();
}
function isoToDay(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const d = new Date(iso).getTime();
  if (Number.isNaN(d)) return null;
  return Math.floor(d / 86_400_000);
}

export function GraphView() {
  const { t } = useTranslation();
  const {
    layout,
    setLayout,
    colorBy,
    setColorBy,
    sizeBy,
    setSizeBy,
    validAt,
    setValidAt,
    selectedNode,
    setSelectedNode,
  } = useGraphStore();

  const { data, isLoading, isError, error, refetch } = useGraph();
  const [layoutSeed, setLayoutSeed] = useState(0);

  // Compute slider range from the loaded graph (min valid_from → max valid_to/today).
  const { minDay, maxDay } = useMemo(() => {
    const today = Math.floor(Date.now() / 86_400_000);
    if (!data || data.nodes.length === 0) {
      return { minDay: today - 365, maxDay: today };
    }
    let lo = today;
    let hi = today;
    for (const n of data.nodes) {
      const f = isoToDay(n.valid_from);
      const to = isoToDay(n.valid_to);
      if (f !== null && f < lo) lo = f;
      if (to !== null && to > hi) hi = to;
    }
    return { minDay: lo, maxDay: hi };
  }, [data]);

  const currentDay = isoToDay(validAt) ?? maxDay;
  const [draftDay, setDraftDay] = useState(currentDay);

  // Sync draft when underlying validAt updates (e.g., reset).
  useEffect(() => {
    setDraftDay(isoToDay(validAt) ?? maxDay);
  }, [validAt, maxDay]);

  // Debounce slider → setValidAt → refetch.
  useEffect(() => {
    const id = setTimeout(() => {
      const iso = dayToIso(draftDay);
      if (iso !== validAt && draftDay !== maxDay) {
        setValidAt(iso);
      } else if (draftDay === maxDay && validAt !== "") {
        setValidAt("");
      }
    }, 200);
    return () => clearTimeout(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftDay]);

  return (
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-4 flex-wrap">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">
            {t("graph.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("graph.subtitle")}
          </p>
        </div>
        {data ? (
          <div className="text-xs text-muted-foreground">
            {t("graph.stats.nodes", { count: data.nodes.length })} ·{" "}
            {t("graph.stats.edges", { count: data.edges.length })}
          </div>
        ) : null}
      </div>

      <div className="grid md:grid-cols-3 gap-3">
        <div className="space-y-1">
          <Label className="text-xs uppercase text-muted-foreground">
            {t("graph.controls.layout")}
          </Label>
          <Select value={layout} onValueChange={(v) => setLayout(v as Layout)}>
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="forceatlas2">
                {t("graph.layouts.forceatlas2")}
              </SelectItem>
              <SelectItem value="circular">
                {t("graph.layouts.circular")}
              </SelectItem>
              <SelectItem value="noverlap">
                {t("graph.layouts.noverlap")}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1">
          <Label className="text-xs uppercase text-muted-foreground">
            {t("graph.controls.color_by")}
          </Label>
          <Select
            value={colorBy}
            onValueChange={(v) => setColorBy(v as ColorBy)}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="status">
                {t("graph.color_by.status")}
              </SelectItem>
              <SelectItem value="page_type">
                {t("graph.color_by.page_type")}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1">
          <Label className="text-xs uppercase text-muted-foreground">
            {t("graph.controls.size_by")}
          </Label>
          <Select value={sizeBy} onValueChange={(v) => setSizeBy(v as SizeBy)}>
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="degree">{t("graph.size_by.degree")}</SelectItem>
              <SelectItem value="confidence">
                {t("graph.size_by.confidence")}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      <div className="rounded-lg border p-3 space-y-2 bg-muted/30">
        <div className="flex items-center justify-between text-xs">
          <Label className="text-xs uppercase text-muted-foreground">
            {t("graph.controls.valid_at")}
          </Label>
          <span className="tabular-nums">
            {t("graph.controls.as_of", {
              date:
                draftDay === maxDay
                  ? t("graph.controls.now")
                  : formatDate(dayToIso(draftDay)),
            })}
          </span>
        </div>
        <Slider
          min={minDay}
          max={maxDay}
          step={1}
          value={[draftDay]}
          onValueChange={(v) => v.length === 1 && setDraftDay(v[0])}
        />
        <p className="text-[11px] text-muted-foreground">
          {t("graph.controls.valid_at_helper")}
        </p>
      </div>

      <div className="flex gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            setLayoutSeed((n) => n + 1);
            refetch();
          }}
        >
          <RefreshCw className="mr-1 h-4 w-4" />
          {t("graph.controls.rerun_layout")}
        </Button>
      </div>

      <div className="grid lg:grid-cols-[1fr_320px] gap-4 items-start">
        <div>
          {isLoading && !data ? (
            <Skeleton className="h-[600px]" />
          ) : isError ? (
            <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm">
              <div className="font-medium text-destructive">
                {t("common.error")}
              </div>
              <div className="text-xs text-destructive/80">
                {(error as Error)?.message}
              </div>
            </div>
          ) : data && data.nodes.length > 0 ? (
            hasWebGL2() ? (
              <GraphErrorBoundary fallback={<GraphFallback reason="render-error" />}>
                <GraphCanvas key={layoutSeed} payload={data} />
              </GraphErrorBoundary>
            ) : (
              <GraphFallback reason="no-webgl2" />
            )
          ) : (
            <div className="rounded-lg border p-12 text-center text-muted-foreground">
              {t("graph.empty")}
            </div>
          )}
        </div>
        <NodePreview
          slug={selectedNode}
          onClose={() => setSelectedNode(null)}
        />
      </div>
    </div>
  );
}

function NodePreview({
  slug,
  onClose,
}: {
  slug: string | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const { data, isLoading } = usePageDetail("default", slug ?? undefined);
  if (!slug) return null;

  const fm = data?.frontmatter;
  const preview = (data?.body ?? "").slice(0, 500);

  return (
    <aside className="rounded-lg border p-4 space-y-3 lg:sticky lg:top-20">
      <div className="flex items-start justify-between">
        <div className="min-w-0">
          <div className="text-xs text-muted-foreground">Slug</div>
          <div className="font-medium truncate">{slug}</div>
        </div>
        <Button variant="ghost" size="sm" onClick={onClose}>
          {t("common.close")}
        </Button>
      </div>

      {isLoading ? (
        <Skeleton className="h-32" />
      ) : data ? (
        <>
          <div className="flex flex-wrap gap-2">
            {fm?.status ? <StatusBadge status={fm.status} /> : null}
            {fm?.page_type ? <PageTypeBadge type={fm.page_type} /> : null}
          </div>
          {typeof fm?.confidence === "number" ? (
            <ConfidenceBar value={fm.confidence} />
          ) : null}
          <p className="text-xs text-muted-foreground whitespace-pre-wrap">
            {preview || t("graph.node_preview.no_body")}
            {(data.body?.length ?? 0) > 500 ? "…" : ""}
          </p>
          <Button asChild size="sm" className="w-full">
            <Link to={`/pages/default/${encodeURIComponent(slug)}`}>
              {t("graph.node_preview.open_page")}
            </Link>
          </Button>
        </>
      ) : (
        <div className="text-xs text-muted-foreground">
          {t("graph.node_preview.loading")}
        </div>
      )}
    </aside>
  );
}
