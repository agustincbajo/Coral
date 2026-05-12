import { Link, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ChevronLeft } from "lucide-react";
import { usePageDetail } from "@/features/pages/usePageDetail";
import { MarkdownRenderer } from "@/features/pages/MarkdownRenderer";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import { PageTypeBadge } from "@/components/PageTypeBadge";
import { StatusBadge } from "@/components/StatusBadge";
import { ConfidenceBar } from "@/components/ConfidenceBar";
import type { PageType, Status } from "@/lib/types";
import { formatDate, formatRelative } from "@/lib/utils";

export function PageDetail() {
  const { t, i18n } = useTranslation();
  const { repo, slug } = useParams<{ repo: string; slug: string }>();
  const { data, isLoading, isError, error } = usePageDetail(repo, slug);

  const fm = data?.frontmatter;

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Button asChild variant="ghost" size="sm">
          <Link to="/pages">
            <ChevronLeft className="h-4 w-4 mr-1" />
            {t("common.back")}
          </Link>
        </Button>
        <h1 className="text-xl font-semibold truncate">{slug}</h1>
      </div>

      {isLoading ? (
        <div className="grid lg:grid-cols-[1fr_300px] gap-6">
          <div className="space-y-3">
            <Skeleton className="h-6 w-2/3" />
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-4 w-5/6" />
          </div>
          <Skeleton className="h-64" />
        </div>
      ) : isError ? (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm">
          <div className="font-medium text-destructive">
            {t("pages.detail.not_found")}
          </div>
          <div className="text-xs text-destructive/80">
            {(error as Error)?.message}
          </div>
        </div>
      ) : data ? (
        <div className="grid lg:grid-cols-[1fr_300px] gap-6">
          <article className="min-w-0">
            <MarkdownRenderer source={data.body} />
          </article>

          <aside className="space-y-5 lg:sticky lg:top-20 lg:self-start">
            <div className="space-y-2">
              <div className="flex items-center gap-2 flex-wrap">
                {fm?.status ? (
                  <StatusBadge status={fm.status as Status} large />
                ) : null}
                {fm?.page_type ? (
                  <PageTypeBadge type={fm.page_type as PageType} />
                ) : null}
              </div>
              {typeof fm?.confidence === "number" ? (
                <div>
                  <div className="text-xs text-muted-foreground mb-1">
                    {t("pages.detail.confidence")}
                  </div>
                  <ConfidenceBar value={fm.confidence} />
                </div>
              ) : null}
            </div>

            <Separator />

            <div className="space-y-1 text-xs">
              {fm?.generated_at ? (
                <div className="flex justify-between gap-2">
                  <span className="text-muted-foreground">
                    {t("pages.detail.generated_at")}
                  </span>
                  <span title={String(fm.generated_at)}>
                    {formatRelative(String(fm.generated_at), i18n.language)}
                  </span>
                </div>
              ) : null}
              {fm?.valid_from ? (
                <div className="flex justify-between gap-2">
                  <span className="text-muted-foreground">
                    {t("pages.detail.valid_from")}
                  </span>
                  <span>{formatDate(String(fm.valid_from))}</span>
                </div>
              ) : null}
              {fm?.valid_to ? (
                <div className="flex justify-between gap-2">
                  <span className="text-muted-foreground">
                    {t("pages.detail.valid_to")}
                  </span>
                  <span>{formatDate(String(fm.valid_to))}</span>
                </div>
              ) : null}
            </div>

            <div>
              <div className="text-xs uppercase text-muted-foreground mb-1">
                {t("pages.detail.backlinks")}
              </div>
              {data.backlinks.length === 0 ? (
                <div className="text-xs text-muted-foreground">
                  {t("pages.detail.no_backlinks")}
                </div>
              ) : (
                <ul className="space-y-1 text-sm">
                  {data.backlinks.map((s) => (
                    <li key={s}>
                      <Link
                        to={`/pages/default/${encodeURIComponent(s)}`}
                        className="text-primary hover:underline"
                      >
                        {s}
                      </Link>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            {Array.isArray(fm?.sources) && fm.sources.length > 0 ? (
              <div>
                <div className="text-xs uppercase text-muted-foreground mb-1">
                  {t("pages.detail.sources")}
                </div>
                <ul className="space-y-1 text-xs break-all">
                  {(fm.sources as string[]).map((s) => (
                    <li key={s}>{s}</li>
                  ))}
                </ul>
              </div>
            ) : null}

            <details>
              <summary className="text-xs uppercase text-muted-foreground cursor-pointer">
                {t("pages.detail.frontmatter")}
              </summary>
              <pre className="mt-2 text-[11px] whitespace-pre-wrap break-all bg-muted/50 rounded p-2">
                {JSON.stringify(fm ?? {}, null, 2)}
              </pre>
            </details>
          </aside>
        </div>
      ) : null}
    </div>
  );
}
