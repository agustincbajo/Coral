import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ShieldCheck, Play } from "lucide-react";
import { useGuarantee } from "@/features/guarantee/useGuarantee";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { GuaranteeCheck, GuaranteeVerdict } from "@/lib/types";

const VERDICT_TONES: Record<GuaranteeVerdict, string> = {
  GREEN: "bg-emerald-500 text-white",
  YELLOW: "bg-amber-500 text-amber-950",
  RED: "bg-red-600 text-white",
};

function verdictKey(v: string | undefined): keyof typeof VERDICT_TONES | "unknown" {
  if (v === "GREEN" || v === "YELLOW" || v === "RED") return v;
  return "unknown";
}

function Semaphore({ verdict }: { verdict: string | undefined }) {
  const { t } = useTranslation();
  const k = verdictKey(verdict);
  const isKnown = k !== "unknown";
  return (
    <div
      className={cn(
        "rounded-lg p-6 flex items-center gap-4",
        isKnown
          ? VERDICT_TONES[k as GuaranteeVerdict]
          : "bg-muted text-muted-foreground",
      )}
    >
      <div
        className={cn(
          "h-16 w-16 rounded-full grid place-items-center shrink-0 border-4",
          isKnown ? "border-white/40" : "border-muted-foreground/30",
        )}
      >
        <ShieldCheck className="h-8 w-8" />
      </div>
      <div>
        <div className="text-xs uppercase opacity-80 tracking-wide">
          {verdict ?? "—"}
        </div>
        <div className="text-xl font-semibold">
          {isKnown
            ? t(`guarantee.verdict.${k.toLowerCase()}` as const)
            : t("guarantee.verdict.unknown")}
        </div>
      </div>
    </div>
  );
}

function CheckRow({ check }: { check: GuaranteeCheck }) {
  const total = check.passed + check.warnings + check.failures;
  const pPct = total > 0 ? (check.passed / total) * 100 : 0;
  const wPct = total > 0 ? (check.warnings / total) * 100 : 0;
  const fPct = total > 0 ? (check.failures / total) * 100 : 0;
  return (
    <tr className="border-t">
      <td className="px-3 py-2 align-top">
        <div className="font-medium text-sm">{check.name}</div>
        {check.detail ? (
          <div className="text-xs text-muted-foreground mt-1">{check.detail}</div>
        ) : null}
      </td>
      <td className="px-3 py-2 align-top w-1/3">
        <div className="flex h-2 rounded-full overflow-hidden bg-muted">
          <div
            className="bg-emerald-500"
            style={{ width: `${pPct}%` }}
            title={`passed: ${check.passed}`}
          />
          <div
            className="bg-amber-500"
            style={{ width: `${wPct}%` }}
            title={`warnings: ${check.warnings}`}
          />
          <div
            className="bg-red-500"
            style={{ width: `${fPct}%` }}
            title={`failures: ${check.failures}`}
          />
        </div>
      </td>
      <td className="px-3 py-2 text-right tabular-nums text-xs text-emerald-700 dark:text-emerald-300">
        {check.passed}
      </td>
      <td className="px-3 py-2 text-right tabular-nums text-xs text-amber-700 dark:text-amber-300">
        {check.warnings}
      </td>
      <td className="px-3 py-2 text-right tabular-nums text-xs text-red-700 dark:text-red-300">
        {check.failures}
      </td>
    </tr>
  );
}

export function GuaranteeView() {
  const { t } = useTranslation();
  const [env, setEnv] = useState("");
  const [strict, setStrict] = useState(false);
  const query = useGuarantee({ env, strict });

  function check() {
    void query.refetch();
  }

  return (
    <div className="space-y-4 max-w-4xl">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          {t("guarantee.title")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("guarantee.subtitle")}
        </p>
      </div>

      <div className="rounded-lg border p-4 space-y-3 bg-muted/30">
        <div className="grid sm:grid-cols-[1fr_auto_auto] items-end gap-3">
          <div className="space-y-1">
            <Label htmlFor="guarantee-env">{t("guarantee.env")}</Label>
            <Input
              id="guarantee-env"
              value={env}
              onChange={(e) => setEnv(e.target.value)}
              placeholder={t("guarantee.env_placeholder")}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  check();
                }
              }}
            />
          </div>
          <label className="flex items-center gap-2 text-sm h-10">
            <input
              type="checkbox"
              checked={strict}
              onChange={(e) => setStrict(e.target.checked)}
            />
            {t("guarantee.strict")}
          </label>
          <Button onClick={check} disabled={query.isFetching}>
            <Play className="h-4 w-4 mr-1" />
            {t("guarantee.check")}
          </Button>
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
        <Skeleton className="h-48" />
      ) : query.data ? (
        <>
          <Semaphore verdict={query.data.data.verdict} />
          <div className="text-xs text-muted-foreground">
            {t("guarantee.exit_code")}:{" "}
            <span className="tabular-nums">{query.data.meta.exit_code}</span>
          </div>
          <div className="rounded-lg border overflow-hidden">
            <div className="px-4 py-2 border-b bg-muted/50 text-sm font-medium">
              {t("guarantee.checks.title")}
            </div>
            <table className="w-full text-sm">
              <thead className="text-xs uppercase text-muted-foreground bg-muted/30">
                <tr>
                  <th className="text-left px-3 py-2">
                    {t("guarantee.checks.name")}
                  </th>
                  <th className="text-left px-3 py-2">
                    {t("guarantee.checks.detail")}
                  </th>
                  <th className="text-right px-3 py-2">
                    {t("guarantee.checks.passed")}
                  </th>
                  <th className="text-right px-3 py-2">
                    {t("guarantee.checks.warnings")}
                  </th>
                  <th className="text-right px-3 py-2">
                    {t("guarantee.checks.failures")}
                  </th>
                </tr>
              </thead>
              <tbody>
                {query.data.data.checks.map((c, i) => (
                  <CheckRow key={`${c.name}-${i}`} check={c} />
                ))}
              </tbody>
            </table>
          </div>
        </>
      ) : (
        <div className="text-xs text-muted-foreground">
          {t("guarantee.checks.empty")}
        </div>
      )}
    </div>
  );
}
