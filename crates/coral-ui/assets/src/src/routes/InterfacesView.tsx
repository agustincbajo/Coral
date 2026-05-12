import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { Plug } from "lucide-react";
import { useInterfaces } from "@/features/interfaces/useInterfaces";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { StatusBadge } from "@/components/StatusBadge";
import { ConfidenceBar } from "@/components/ConfidenceBar";
import { formatDate } from "@/lib/utils";

export function InterfacesView() {
  const { t } = useTranslation();
  const { data, isLoading, isError, error, refetch } = useInterfaces();

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          {t("interfaces.title")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("interfaces.subtitle")}
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
                  {t("interfaces.columns.slug")}
                </th>
                <th className="text-left px-3 py-2">
                  {t("interfaces.columns.repo")}
                </th>
                <th className="text-left px-3 py-2">
                  {t("interfaces.columns.status")}
                </th>
                <th className="text-left px-3 py-2">
                  {t("interfaces.columns.confidence")}
                </th>
                <th className="text-left px-3 py-2">
                  {t("interfaces.columns.sources")}
                </th>
                <th className="text-left px-3 py-2">
                  {t("interfaces.columns.valid_from")}
                </th>
                <th className="text-left px-3 py-2">
                  {t("interfaces.columns.valid_to")}
                </th>
                <th className="text-right px-3 py-2">
                  {t("interfaces.columns.backlinks")}
                </th>
              </tr>
            </thead>
            <tbody>
              {isLoading && !data
                ? Array.from({ length: 5 }).map((_, i) => (
                    <tr key={i} className="border-t">
                      <td className="px-3 py-2">
                        <Skeleton className="h-4 w-40" />
                      </td>
                      <td className="px-3 py-2">
                        <Skeleton className="h-4 w-24" />
                      </td>
                      <td className="px-3 py-2">
                        <Skeleton className="h-5 w-16" />
                      </td>
                      <td className="px-3 py-2">
                        <Skeleton className="h-3 w-24" />
                      </td>
                      <td className="px-3 py-2">
                        <Skeleton className="h-4 w-12" />
                      </td>
                      <td className="px-3 py-2">
                        <Skeleton className="h-4 w-20" />
                      </td>
                      <td className="px-3 py-2">
                        <Skeleton className="h-4 w-20" />
                      </td>
                      <td className="px-3 py-2 text-right">
                        <Skeleton className="h-4 w-8 inline-block" />
                      </td>
                    </tr>
                  ))
                : !data || data.length === 0
                  ? (
                    <tr>
                      <td
                        colSpan={8}
                        className="px-3 py-12 text-center text-muted-foreground"
                      >
                        <Plug className="h-10 w-10 mx-auto mb-2 opacity-40" />
                        {t("interfaces.empty")}
                      </td>
                    </tr>
                  )
                  : data.map((iface) => (
                      <tr
                        key={`${iface.repo}/${iface.slug}`}
                        className="border-t hover:bg-muted/50"
                      >
                        <td className="px-3 py-2">
                          <Link
                            to={`/pages/${encodeURIComponent(iface.repo)}/${encodeURIComponent(iface.slug)}`}
                            className="font-medium text-primary hover:underline"
                          >
                            {iface.slug}
                          </Link>
                        </td>
                        <td className="px-3 py-2 text-xs text-muted-foreground">
                          {iface.repo}
                        </td>
                        <td className="px-3 py-2">
                          <StatusBadge status={iface.status} />
                        </td>
                        <td className="px-3 py-2">
                          <ConfidenceBar value={iface.confidence} />
                        </td>
                        <td className="px-3 py-2 text-xs text-muted-foreground tabular-nums">
                          {iface.sources?.length ?? 0}
                        </td>
                        <td className="px-3 py-2 text-xs text-muted-foreground">
                          {iface.valid_from
                            ? formatDate(iface.valid_from)
                            : "—"}
                        </td>
                        <td className="px-3 py-2 text-xs text-muted-foreground">
                          {iface.valid_to ? formatDate(iface.valid_to) : "—"}
                        </td>
                        <td className="px-3 py-2 text-right tabular-nums">
                          {iface.backlinks_count}
                        </td>
                      </tr>
                    ))}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}
