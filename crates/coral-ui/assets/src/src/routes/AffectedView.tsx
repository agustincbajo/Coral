import { useState } from "react";
import { useTranslation } from "react-i18next";
import { GitBranch, Play } from "lucide-react";
import { useAffected } from "@/features/affected/useAffected";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";

export function AffectedView() {
  const { t } = useTranslation();
  const [since, setSince] = useState("HEAD~1");
  const query = useAffected(since);

  function compute() {
    if (!since.trim()) return;
    void query.refetch();
  }

  return (
    <div className="space-y-4 max-w-3xl">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          {t("affected.title")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("affected.subtitle")}
        </p>
      </div>

      <div className="rounded-lg border p-4 space-y-3 bg-muted/30">
        <div className="space-y-1">
          <Label htmlFor="affected-since">
            {t("affected.since_label")}
          </Label>
          <div className="flex items-center gap-2">
            <Input
              id="affected-since"
              value={since}
              onChange={(e) => setSince(e.target.value)}
              placeholder={t("affected.since_placeholder")}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  compute();
                }
              }}
            />
            <Button
              onClick={compute}
              disabled={!since.trim() || query.isFetching}
            >
              <Play className="h-4 w-4 mr-1" />
              {t("affected.compute")}
            </Button>
          </div>
        </div>
      </div>

      {query.isError ? (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm">
          <div className="font-medium text-destructive">
            {t("common.error")}
          </div>
          <div className="text-xs text-destructive/80">
            {(query.error as Error)?.message ?? t("errors.unknown")}
          </div>
        </div>
      ) : null}

      {query.isFetching ? (
        <Skeleton className="h-32" />
      ) : query.data ? (
        query.data.data.length === 0 ? (
          <div className="rounded-lg border p-12 text-center text-muted-foreground">
            <GitBranch className="h-10 w-10 mx-auto mb-2 opacity-40" />
            {t("affected.empty")}
          </div>
        ) : (
          <div className="rounded-lg border">
            <div className="px-4 py-2 border-b text-sm font-medium bg-muted/50">
              {t("affected.result_title", { count: query.data.meta.total })}
            </div>
            <ul className="divide-y">
              {query.data.data.map((repo) => (
                <li
                  key={repo}
                  className="px-4 py-2 text-sm flex items-center gap-2"
                >
                  <GitBranch className="h-4 w-4 text-muted-foreground" />
                  <span className="font-mono">{repo}</span>
                </li>
              ))}
            </ul>
          </div>
        )
      ) : (
        <div className="text-xs text-muted-foreground">
          {t("affected.needs_ref")}
        </div>
      )}
    </div>
  );
}
