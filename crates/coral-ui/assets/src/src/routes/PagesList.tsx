import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight, FileText } from "lucide-react";
import { usePages } from "@/features/pages/usePages";
import { FiltersSidebar } from "@/features/pages/FiltersSidebar";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { PageTypeBadge } from "@/components/PageTypeBadge";
import { StatusBadge } from "@/components/StatusBadge";
import { ConfidenceBar } from "@/components/ConfidenceBar";
import { formatRelative } from "@/lib/utils";
import { useFiltersStore } from "@/stores/filters";
import { useCurrentRepo } from "@/lib/repo";

export function PagesList() {
  const { t, i18n } = useTranslation();
  const { data, isLoading, isError, error, refetch } = usePages();
  const { page, pageSize, setPage } = useFiltersStore();
  const repo = useCurrentRepo();

  const total = data?.meta.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  const from = total === 0 ? 0 : page * pageSize + 1;
  const to = Math.min(total, (page + 1) * pageSize);

  return (
    <div className="flex gap-6">
      <FiltersSidebar />
      <div className="flex-1 min-w-0 space-y-4">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">
            {t("pages.list.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("pages.list.subtitle")}
          </p>
        </div>

        {isError ? (
          <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm">
            <div className="font-medium text-destructive">
              {t("common.error")}
            </div>
            <div className="text-xs text-destructive/80">
              {(error as Error)?.message ?? t("errors.unknown")}
            </div>
            <Button
              variant="outline"
              size="sm"
              className="mt-2"
              onClick={() => refetch()}
            >
              {t("common.retry")}
            </Button>
          </div>
        ) : null}

        <div className="rounded-lg border overflow-hidden">
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="bg-muted/50 text-xs uppercase text-muted-foreground">
                <tr>
                  <th className="text-left px-3 py-2">
                    {t("pages.list.columns.slug")}
                  </th>
                  <th className="text-left px-3 py-2">
                    {t("pages.list.columns.page_type")}
                  </th>
                  <th className="text-left px-3 py-2">
                    {t("pages.list.columns.status")}
                  </th>
                  <th className="text-left px-3 py-2">
                    {t("pages.list.columns.confidence")}
                  </th>
                  <th className="text-left px-3 py-2">
                    {t("pages.list.columns.generated_at")}
                  </th>
                  <th className="text-right px-3 py-2">
                    {t("pages.list.columns.backlinks")}
                  </th>
                </tr>
              </thead>
              <tbody>
                {isLoading && !data
                  ? Array.from({ length: 6 }).map((_, i) => (
                      <tr key={i} className="border-t">
                        <td className="px-3 py-2">
                          <Skeleton className="h-4 w-40" />
                        </td>
                        <td className="px-3 py-2">
                          <Skeleton className="h-5 w-16" />
                        </td>
                        <td className="px-3 py-2">
                          <Skeleton className="h-5 w-16" />
                        </td>
                        <td className="px-3 py-2">
                          <Skeleton className="h-3 w-24" />
                        </td>
                        <td className="px-3 py-2">
                          <Skeleton className="h-4 w-20" />
                        </td>
                        <td className="px-3 py-2 text-right">
                          <Skeleton className="h-4 w-8 inline-block" />
                        </td>
                      </tr>
                    ))
                  : data?.data.length === 0
                    ? (
                      <tr>
                        <td
                          colSpan={6}
                          className="px-3 py-12 text-center text-muted-foreground"
                        >
                          <FileText className="h-10 w-10 mx-auto mb-2 opacity-40" />
                          {t("pages.list.empty_state")}
                        </td>
                      </tr>
                    )
                    : data?.data.map((p) => (
                        <tr key={p.slug} className="border-t hover:bg-muted/50">
                          <td className="px-3 py-2">
                            <Link
                              to={`/pages/${encodeURIComponent(repo)}/${encodeURIComponent(p.slug)}`}
                              className="font-medium text-primary hover:underline"
                            >
                              {p.slug}
                            </Link>
                          </td>
                          <td className="px-3 py-2">
                            <PageTypeBadge type={p.page_type} />
                          </td>
                          <td className="px-3 py-2">
                            <StatusBadge status={p.status} />
                          </td>
                          <td className="px-3 py-2">
                            <ConfidenceBar value={p.confidence} />
                          </td>
                          <td className="px-3 py-2 text-xs text-muted-foreground">
                            {formatRelative(p.generated_at, i18n.language)}
                          </td>
                          <td className="px-3 py-2 text-right tabular-nums">
                            {p.backlinks_count}
                          </td>
                        </tr>
                      ))}
              </tbody>
            </table>
          </div>
        </div>

        <div className="flex items-center justify-between text-xs text-muted-foreground">
          <div>
            {t("pages.list.showing", { from, to, total })}
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setPage(Math.max(0, page - 1))}
              disabled={page === 0}
            >
              <ChevronLeft className="h-4 w-4" />
              {t("common.previous")}
            </Button>
            <span className="tabular-nums">
              {page + 1} {t("common.of")} {totalPages}
            </span>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setPage(Math.min(totalPages - 1, page + 1))}
              disabled={page >= totalPages - 1}
            >
              {t("common.next")}
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
