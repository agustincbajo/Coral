import { useTranslation } from "react-i18next";
import { AlertTriangle } from "lucide-react";
import { useDrift } from "@/features/drift/useDrift";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { DriftFinding, DriftReport, DriftSeverity } from "@/lib/types";

const SEVERITY_TONES: Record<DriftSeverity, string> = {
  critical: "bg-red-100 text-red-900 dark:bg-red-950/60 dark:text-red-200",
  high: "bg-orange-100 text-orange-900 dark:bg-orange-950/60 dark:text-orange-200",
  medium:
    "bg-amber-100 text-amber-900 dark:bg-amber-950/60 dark:text-amber-200",
  low: "bg-blue-100 text-blue-900 dark:bg-blue-950/60 dark:text-blue-200",
  info: "bg-gray-100 text-gray-900 dark:bg-gray-800 dark:text-gray-200",
};

function normalizeSeverity(s: unknown): DriftSeverity {
  const v = typeof s === "string" ? s.toLowerCase() : "";
  if (v === "critical" || v === "high" || v === "medium" || v === "low" || v === "info") {
    return v;
  }
  return "info";
}

function FindingItem({ finding }: { finding: DriftFinding }) {
  const { t } = useTranslation();
  const sev = normalizeSeverity(finding.severity);
  const message =
    typeof finding.message === "string" ? finding.message : JSON.stringify(finding);
  return (
    <li className="flex items-start gap-2 text-sm">
      <Badge
        variant="outline"
        className={cn("border-transparent shrink-0 uppercase text-[10px]", SEVERITY_TONES[sev])}
      >
        {t(`drift.severity.${sev}`)}
      </Badge>
      <span className="text-muted-foreground break-words">{message}</span>
    </li>
  );
}

function ReportCard({ report }: { report: DriftReport }) {
  const { t } = useTranslation();
  const findings = Array.isArray(report.findings) ? report.findings : [];
  const title = report.slug ?? report.repo ?? "(unnamed)";
  const subtitle = report.repo && report.slug ? report.repo : undefined;
  // Pick the highest severity to tint the card border.
  const order: DriftSeverity[] = ["critical", "high", "medium", "low", "info"];
  const peak = findings
    .map((f) => normalizeSeverity(f.severity))
    .sort((a, b) => order.indexOf(a) - order.indexOf(b))[0];
  return (
    <Card
      className={cn(
        peak === "critical" && "border-red-500/40",
        peak === "high" && "border-orange-500/40",
        peak === "medium" && "border-amber-500/40",
      )}
    >
      <CardHeader>
        <CardTitle className="text-base">{title}</CardTitle>
        {subtitle ? <CardDescription>{subtitle}</CardDescription> : null}
      </CardHeader>
      <CardContent>
        {findings.length === 0 ? (
          <div className="text-xs text-muted-foreground">
            {t("drift.no_findings")}
          </div>
        ) : (
          <ul className="space-y-2">
            {findings.map((f, i) => (
              <FindingItem key={i} finding={f} />
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

export function DriftView() {
  const { t } = useTranslation();
  const { data, isLoading, isError, error, refetch } = useDrift();

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          {t("drift.title")}
        </h1>
        <p className="text-sm text-muted-foreground">{t("drift.subtitle")}</p>
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

      {isLoading && !data ? (
        <div className="grid md:grid-cols-2 gap-3">
          <Skeleton className="h-40" />
          <Skeleton className="h-40" />
        </div>
      ) : !data || data.length === 0 ? (
        <div className="rounded-lg border p-12 text-center text-muted-foreground">
          <AlertTriangle className="h-10 w-10 mx-auto mb-2 opacity-40" />
          {t("drift.no_reports")}
        </div>
      ) : (
        <div className="grid md:grid-cols-2 gap-3">
          {data.map((r, i) => (
            <ReportCard key={r.slug ?? r.repo ?? i} report={r} />
          ))}
        </div>
      )}
    </div>
  );
}
