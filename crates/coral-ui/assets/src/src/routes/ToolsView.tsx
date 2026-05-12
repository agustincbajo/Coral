import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Wrench, AlertTriangle, Play } from "lucide-react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { TokenDialog } from "@/components/TokenDialog";
import { useToast } from "@/components/ui/toaster";
import { useAuthStore } from "@/stores/auth";
import { getConfig } from "@/lib/config";
import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";
import {
  useDown,
  useRunTest,
  useUp,
  useVerify,
} from "@/features/tools/useTools";
import type { ToolRunResult } from "@/lib/types";

function parseCsv(value: string): string[] {
  return value
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

interface ResultBlockProps {
  result?: ToolRunResult;
  isError?: boolean;
  errorMessage?: string;
}

function ResultBlock({ result, isError, errorMessage }: ResultBlockProps) {
  const { t } = useTranslation();
  if (!result && !isError) return null;
  const ok = !isError && result?.status === "ok" && (result?.exit_code ?? 0) === 0;
  return (
    <div className="space-y-2 mt-4 border-t pt-4">
      <div className="flex items-center gap-3 text-xs flex-wrap">
        <Badge
          variant="outline"
          className={cn(
            "border-transparent uppercase",
            ok
              ? "bg-emerald-100 text-emerald-900 dark:bg-emerald-950/60 dark:text-emerald-200"
              : "bg-red-100 text-red-900 dark:bg-red-950/60 dark:text-red-200",
          )}
        >
          {ok ? t("tools.result.ok") : t("tools.result.error")}
        </Badge>
        {result ? (
          <>
            <div>
              <span className="text-muted-foreground">
                {t("tools.result.exit_code")}:{" "}
              </span>
              <span className="tabular-nums">{result.exit_code}</span>
            </div>
            <div>
              <span className="text-muted-foreground">
                {t("tools.result.duration")}:{" "}
              </span>
              <span className="tabular-nums">
                {t("tools.duration_ms", { ms: result.duration_ms })}
              </span>
            </div>
          </>
        ) : null}
      </div>
      {isError && errorMessage ? (
        <div className="text-xs text-destructive">{errorMessage}</div>
      ) : null}
      {result ? (
        <>
          <div>
            <div className="text-xs font-medium text-muted-foreground mb-1">
              {t("tools.result.stdout")}
            </div>
            <pre className="text-xs rounded border bg-muted/30 p-2 max-h-64 overflow-auto whitespace-pre-wrap break-words font-mono">
              {result.stdout_tail || t("tools.result.empty")}
            </pre>
          </div>
          <div>
            <div className="text-xs font-medium text-muted-foreground mb-1">
              {t("tools.result.stderr")}
            </div>
            <pre className="text-xs rounded border bg-muted/30 p-2 max-h-64 overflow-auto whitespace-pre-wrap break-words font-mono">
              {result.stderr_tail || t("tools.result.empty")}
            </pre>
          </div>
        </>
      ) : null}
    </div>
  );
}

interface PanelShellProps {
  title: string;
  description: string;
  disabled: boolean;
  children: ReactNode;
}

function PanelShell({ title, description, disabled, children }: PanelShellProps) {
  return (
    <Card className={cn(disabled && "opacity-60")}>
      <CardHeader>
        <CardTitle className="text-base">{title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}

function useAuthGate() {
  const cfg = getConfig();
  const token = useAuthStore((s) => s.token);
  const [tokenDialogOpen, setTokenDialogOpen] = useState(false);
  const needsToken = cfg.authRequired && !token;
  return { needsToken, tokenDialogOpen, setTokenDialogOpen };
}

function isUnauthorized(err: unknown): boolean {
  if (err instanceof ApiError && (err.status === 401 || err.status === 403)) {
    return true;
  }
  return false;
}

interface ConfirmDialogState {
  title: string;
  description: string;
  onConfirm: () => void;
}

function ConfirmDialog({
  state,
  onClose,
}: {
  state: ConfirmDialogState | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Dialog open={state !== null} onOpenChange={(v) => !v && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{state?.title}</DialogTitle>
          <DialogDescription>{state?.description}</DialogDescription>
        </DialogHeader>
        <DialogFooter className="gap-2">
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            variant="destructive"
            onClick={() => {
              state?.onConfirm();
              onClose();
            }}
          >
            {t("common.confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function VerifyPanel({
  disabled,
  onUnauthorized,
}: {
  disabled: boolean;
  onUnauthorized: () => void;
}) {
  const { t } = useTranslation();
  const toast = useToast();
  const m = useVerify();
  const [env, setEnv] = useState("");
  return (
    <PanelShell
      title={t("tools.verify.title")}
      description={t("tools.verify.description")}
      disabled={disabled}
    >
      <div className="space-y-3">
        <div className="space-y-1">
          <Label htmlFor="verify-env">{t("tools.fields.env")}</Label>
          <Input
            id="verify-env"
            value={env}
            onChange={(e) => setEnv(e.target.value)}
            placeholder={t("tools.fields.env_placeholder")}
            disabled={disabled}
          />
        </div>
        <Button
          onClick={() => {
            m.mutate(
              { env: env.trim() || undefined },
              {
                onError: (err) => {
                  if (isUnauthorized(err)) {
                    toast({
                      title: t("common.error"),
                      description: t("errors.unauthorized"),
                      variant: "error",
                    });
                    onUnauthorized();
                  } else {
                    toast({
                      title: t("common.error"),
                      description: err.message,
                      variant: "error",
                    });
                  }
                },
              },
            );
          }}
          disabled={disabled || m.isPending}
        >
          <Play className="h-4 w-4 mr-1" />
          {m.isPending ? t("common.running") : t("tools.verify.run")}
        </Button>
      </div>
      <ResultBlock
        result={m.data}
        isError={m.isError}
        errorMessage={(m.error as Error | null)?.message}
      />
    </PanelShell>
  );
}

function RunTestPanel({
  disabled,
  onUnauthorized,
}: {
  disabled: boolean;
  onUnauthorized: () => void;
}) {
  const { t } = useTranslation();
  const toast = useToast();
  const m = useRunTest();
  const [services, setServices] = useState("");
  const [kinds, setKinds] = useState("");
  const [tags, setTags] = useState("");
  const [env, setEnv] = useState("");
  return (
    <PanelShell
      title={t("tools.run_test.title")}
      description={t("tools.run_test.description")}
      disabled={disabled}
    >
      <div className="space-y-3">
        <div className="space-y-1">
          <Label htmlFor="rt-services">{t("tools.fields.services")}</Label>
          <Input
            id="rt-services"
            value={services}
            onChange={(e) => setServices(e.target.value)}
            placeholder={t("tools.fields.services_placeholder")}
            disabled={disabled}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="rt-kinds">{t("tools.fields.kinds")}</Label>
          <Input
            id="rt-kinds"
            value={kinds}
            onChange={(e) => setKinds(e.target.value)}
            placeholder={t("tools.fields.kinds_placeholder")}
            disabled={disabled}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="rt-tags">{t("tools.fields.tags")}</Label>
          <Input
            id="rt-tags"
            value={tags}
            onChange={(e) => setTags(e.target.value)}
            placeholder={t("tools.fields.tags_placeholder")}
            disabled={disabled}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="rt-env">{t("tools.fields.env")}</Label>
          <Input
            id="rt-env"
            value={env}
            onChange={(e) => setEnv(e.target.value)}
            placeholder={t("tools.fields.env_placeholder")}
            disabled={disabled}
          />
        </div>
        <Button
          onClick={() => {
            const body = {
              services: parseCsv(services),
              kinds: parseCsv(kinds),
              tags: parseCsv(tags),
              env: env.trim() || undefined,
            };
            // Drop empty arrays so the backend can apply defaults.
            const cleaned: Record<string, unknown> = { ...body };
            for (const key of ["services", "kinds", "tags"] as const) {
              if ((cleaned[key] as string[]).length === 0) {
                delete cleaned[key];
              }
            }
            m.mutate(cleaned as Parameters<typeof m.mutate>[0], {
              onError: (err) => {
                if (isUnauthorized(err)) {
                  toast({
                    title: t("common.error"),
                    description: t("errors.unauthorized"),
                    variant: "error",
                  });
                  onUnauthorized();
                } else {
                  toast({
                    title: t("common.error"),
                    description: err.message,
                    variant: "error",
                  });
                }
              },
            });
          }}
          disabled={disabled || m.isPending}
        >
          <Play className="h-4 w-4 mr-1" />
          {m.isPending ? t("common.running") : t("tools.run_test.run")}
        </Button>
      </div>
      <ResultBlock
        result={m.data}
        isError={m.isError}
        errorMessage={(m.error as Error | null)?.message}
      />
    </PanelShell>
  );
}

function UpPanel({
  disabled,
  onUnauthorized,
  requestConfirm,
}: {
  disabled: boolean;
  onUnauthorized: () => void;
  requestConfirm: (state: ConfirmDialogState) => void;
}) {
  const { t } = useTranslation();
  const toast = useToast();
  const m = useUp();
  const [env, setEnv] = useState("");
  const run = () => {
    m.mutate(
      { env: env.trim() || undefined },
      {
        onError: (err) => {
          if (isUnauthorized(err)) {
            toast({
              title: t("common.error"),
              description: t("errors.unauthorized"),
              variant: "error",
            });
            onUnauthorized();
          } else {
            toast({
              title: t("common.error"),
              description: err.message,
              variant: "error",
            });
          }
        },
      },
    );
  };
  return (
    <PanelShell
      title={t("tools.up.title")}
      description={t("tools.up.description")}
      disabled={disabled}
    >
      <div className="space-y-3">
        <div className="space-y-1">
          <Label htmlFor="up-env">{t("tools.fields.env")}</Label>
          <Input
            id="up-env"
            value={env}
            onChange={(e) => setEnv(e.target.value)}
            placeholder={t("tools.fields.env_placeholder")}
            disabled={disabled}
          />
        </div>
        <Button
          onClick={() =>
            requestConfirm({
              title: t("tools.confirm.title"),
              description: t("tools.confirm.description_up"),
              onConfirm: run,
            })
          }
          disabled={disabled || m.isPending}
        >
          <Play className="h-4 w-4 mr-1" />
          {m.isPending ? t("common.running") : t("tools.up.run")}
        </Button>
      </div>
      <ResultBlock
        result={m.data}
        isError={m.isError}
        errorMessage={(m.error as Error | null)?.message}
      />
    </PanelShell>
  );
}

function DownPanel({
  disabled,
  onUnauthorized,
  requestConfirm,
}: {
  disabled: boolean;
  onUnauthorized: () => void;
  requestConfirm: (state: ConfirmDialogState) => void;
}) {
  const { t } = useTranslation();
  const toast = useToast();
  const m = useDown();
  const [env, setEnv] = useState("");
  const [volumes, setVolumes] = useState(false);
  const run = () => {
    m.mutate(
      { env: env.trim() || undefined, volumes },
      {
        onError: (err) => {
          if (isUnauthorized(err)) {
            toast({
              title: t("common.error"),
              description: t("errors.unauthorized"),
              variant: "error",
            });
            onUnauthorized();
          } else {
            toast({
              title: t("common.error"),
              description: err.message,
              variant: "error",
            });
          }
        },
      },
    );
  };
  return (
    <PanelShell
      title={t("tools.down.title")}
      description={t("tools.down.description")}
      disabled={disabled}
    >
      <div className="space-y-3">
        <div className="space-y-1">
          <Label htmlFor="down-env">{t("tools.fields.env")}</Label>
          <Input
            id="down-env"
            value={env}
            onChange={(e) => setEnv(e.target.value)}
            placeholder={t("tools.fields.env_placeholder")}
            disabled={disabled}
          />
        </div>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={volumes}
            onChange={(e) => setVolumes(e.target.checked)}
            disabled={disabled}
          />
          {t("tools.fields.volumes")}
        </label>
        <Button
          variant={volumes ? "destructive" : "default"}
          onClick={() => {
            if (volumes) {
              requestConfirm({
                title: t("tools.confirm.title"),
                description: t("tools.confirm.description_down"),
                onConfirm: run,
              });
            } else {
              run();
            }
          }}
          disabled={disabled || m.isPending}
        >
          <Play className="h-4 w-4 mr-1" />
          {m.isPending ? t("common.running") : t("tools.down.run")}
        </Button>
      </div>
      <ResultBlock
        result={m.data}
        isError={m.isError}
        errorMessage={(m.error as Error | null)?.message}
      />
    </PanelShell>
  );
}

export function ToolsView() {
  const { t } = useTranslation();
  const cfg = getConfig();
  const writeEnabled = cfg.writeToolsEnabled;
  const { needsToken, tokenDialogOpen, setTokenDialogOpen } = useAuthGate();
  const [confirm, setConfirm] = useState<ConfirmDialogState | null>(null);

  const disabled = !writeEnabled;

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight flex items-center gap-2">
          <Wrench className="h-5 w-5" />
          {t("tools.title")}
        </h1>
        <p className="text-sm text-muted-foreground">{t("tools.subtitle")}</p>
      </div>

      {!writeEnabled ? (
        <div className="rounded-lg border bg-amber-50 dark:bg-amber-950/40 p-3 text-sm flex items-start gap-2">
          <AlertTriangle className="h-4 w-4 text-amber-700 dark:text-amber-300 shrink-0 mt-0.5" />
          <div>{t("tools.write_disabled_banner")}</div>
        </div>
      ) : needsToken ? (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm flex items-center justify-between gap-2">
          <span>{t("tools.needs_token")}</span>
          <Button size="sm" onClick={() => setTokenDialogOpen(true)}>
            {t("auth.token_dialog.open")}
          </Button>
        </div>
      ) : null}

      <TokenDialog
        open={tokenDialogOpen}
        onOpenChange={setTokenDialogOpen}
        showTrigger={false}
      />

      <div className="grid lg:grid-cols-2 gap-4">
        <VerifyPanel
          disabled={disabled}
          onUnauthorized={() => setTokenDialogOpen(true)}
        />
        <RunTestPanel
          disabled={disabled}
          onUnauthorized={() => setTokenDialogOpen(true)}
        />
        <UpPanel
          disabled={disabled}
          onUnauthorized={() => setTokenDialogOpen(true)}
          requestConfirm={setConfirm}
        />
        <DownPanel
          disabled={disabled}
          onUnauthorized={() => setTokenDialogOpen(true)}
          requestConfirm={setConfirm}
        />
      </div>

      <ConfirmDialog state={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}
