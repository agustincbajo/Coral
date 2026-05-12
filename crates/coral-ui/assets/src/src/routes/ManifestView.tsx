import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, ChevronDown } from "lucide-react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useManifest, useLock } from "@/features/manifest/useManifest";
import { useStats } from "@/features/manifest/useStats";
import { cn } from "@/lib/utils";

function JsonTree({ value, depth = 0 }: { value: unknown; depth?: number }) {
  const [open, setOpen] = useState(depth < 1);
  if (value === null) return <span className="text-muted-foreground">null</span>;
  if (typeof value === "string")
    return <span className="text-emerald-700 dark:text-emerald-300">"{value}"</span>;
  if (typeof value === "number" || typeof value === "boolean")
    return <span className="text-sky-700 dark:text-sky-300">{String(value)}</span>;
  if (Array.isArray(value)) {
    if (value.length === 0) return <span className="text-muted-foreground">[]</span>;
    return (
      <div className="font-mono text-xs">
        <button
          type="button"
          className="inline-flex items-center hover:bg-accent rounded px-1"
          onClick={() => setOpen((o) => !o)}
        >
          {open ? (
            <ChevronDown className="h-3 w-3" />
          ) : (
            <ChevronRight className="h-3 w-3" />
          )}
          <span className="ml-1 text-muted-foreground">[{value.length}]</span>
        </button>
        {open ? (
          <div className="ml-4 border-l pl-2">
            {value.map((v, i) => (
              <div key={i}>
                <span className="text-muted-foreground">{i}: </span>
                <JsonTree value={v} depth={depth + 1} />
              </div>
            ))}
          </div>
        ) : null}
      </div>
    );
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0)
      return <span className="text-muted-foreground">{"{}"}</span>;
    return (
      <div className="font-mono text-xs">
        <button
          type="button"
          className="inline-flex items-center hover:bg-accent rounded px-1"
          onClick={() => setOpen((o) => !o)}
        >
          {open ? (
            <ChevronDown className="h-3 w-3" />
          ) : (
            <ChevronRight className="h-3 w-3" />
          )}
          <span className="ml-1 text-muted-foreground">
            {"{"}
            {entries.length}
            {"}"}
          </span>
        </button>
        {open ? (
          <div className="ml-4 border-l pl-2">
            {entries.map(([k, v]) => (
              <div key={k}>
                <span className="text-violet-700 dark:text-violet-300">
                  {k}
                </span>
                <span className="text-muted-foreground">: </span>
                <JsonTree value={v} depth={depth + 1} />
              </div>
            ))}
          </div>
        ) : null}
      </div>
    );
  }
  return <span>{String(value)}</span>;
}

function Breakdown({ title, entries }: { title: string; entries: [string, number][] }) {
  const total = entries.reduce((acc, [, v]) => acc + v, 0);
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">{title}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2">
        {entries.length === 0 ? (
          <div className="text-xs text-muted-foreground">—</div>
        ) : (
          entries
            .sort((a, b) => b[1] - a[1])
            .map(([k, v]) => {
              const pct = total > 0 ? Math.round((v / total) * 100) : 0;
              return (
                <div key={k} className="text-xs">
                  <div className="flex justify-between">
                    <span>{k}</span>
                    <span className="tabular-nums text-muted-foreground">
                      {v} ({pct}%)
                    </span>
                  </div>
                  <div className="h-1.5 mt-1 rounded-full bg-muted overflow-hidden">
                    <div
                      className="h-full bg-primary"
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                </div>
              );
            })
        )}
      </CardContent>
    </Card>
  );
}

export function ManifestView() {
  const { t } = useTranslation();
  const manifest = useManifest();
  const lock = useLock();
  const stats = useStats();

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          {t("manifest.title")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("manifest.subtitle")}
        </p>
      </div>
      <Tabs defaultValue="stats">
        <TabsList>
          <TabsTrigger value="manifest">{t("manifest.tabs.manifest")}</TabsTrigger>
          <TabsTrigger value="lock">{t("manifest.tabs.lock")}</TabsTrigger>
          <TabsTrigger value="stats">{t("manifest.tabs.stats")}</TabsTrigger>
        </TabsList>

        <TabsContent value="manifest" className="mt-4">
          {manifest.isLoading ? (
            <Skeleton className="h-40" />
          ) : manifest.data == null ? (
            <p className="text-sm text-muted-foreground">
              {t("manifest.not_found")}
            </p>
          ) : (
            <div className={cn("rounded-lg border p-3 bg-muted/30")}>
              <JsonTree value={manifest.data} />
            </div>
          )}
        </TabsContent>

        <TabsContent value="lock" className="mt-4">
          {lock.isLoading ? (
            <Skeleton className="h-40" />
          ) : lock.data == null ? (
            <p className="text-sm text-muted-foreground">
              {t("manifest.not_found")}
            </p>
          ) : (
            <div className="rounded-lg border p-3 bg-muted/30">
              <JsonTree value={lock.data} />
            </div>
          )}
        </TabsContent>

        <TabsContent value="stats" className="mt-4 space-y-4">
          {stats.isLoading || !stats.data ? (
            <div className="grid sm:grid-cols-3 gap-3">
              <Skeleton className="h-24" />
              <Skeleton className="h-24" />
              <Skeleton className="h-24" />
            </div>
          ) : (
            <>
              <div className="grid sm:grid-cols-3 gap-3">
                <Card>
                  <CardHeader>
                    <CardDescription>
                      {t("manifest.stats.page_count")}
                    </CardDescription>
                    <CardTitle className="text-3xl tabular-nums">
                      {stats.data.page_count}
                    </CardTitle>
                  </CardHeader>
                </Card>
                <Card>
                  <CardHeader>
                    <CardDescription>
                      {t("manifest.stats.avg_confidence")}
                    </CardDescription>
                    <CardTitle className="text-3xl tabular-nums">
                      {Math.round(stats.data.avg_confidence * 100)}%
                    </CardTitle>
                  </CardHeader>
                </Card>
                <Card>
                  <CardHeader>
                    <CardDescription>
                      {t("manifest.stats.total_backlinks")}
                    </CardDescription>
                    <CardTitle className="text-3xl tabular-nums">
                      {stats.data.total_backlinks}
                    </CardTitle>
                  </CardHeader>
                </Card>
              </div>
              <div className="grid md:grid-cols-2 gap-3">
                <Breakdown
                  title={t("manifest.stats.status_breakdown")}
                  entries={Object.entries(stats.data.status_breakdown)}
                />
                <Breakdown
                  title={t("manifest.stats.page_type_breakdown")}
                  entries={Object.entries(stats.data.page_type_breakdown)}
                />
              </div>
            </>
          )}
        </TabsContent>
      </Tabs>
    </div>
  );
}
