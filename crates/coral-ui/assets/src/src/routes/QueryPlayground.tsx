import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { Send, AlertTriangle, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { TokenDialog } from "@/components/TokenDialog";
import { useQueryStream } from "@/features/query/useQueryStream";
import { useQueryHistory, type QueryMode } from "@/stores/query";
import { useAuthStore } from "@/stores/auth";
import { getConfig } from "@/lib/config";
import { cn } from "@/lib/utils";

const MODES: QueryMode[] = ["local", "global", "hybrid"];

export function QueryPlayground() {
  const { t } = useTranslation();
  const cfg = getConfig();
  const token = useAuthStore((s) => s.token);
  const turns = useQueryHistory((s) => s.turns);
  const clear = useQueryHistory((s) => s.clear);
  const { send, streaming } = useQueryStream();

  const [draft, setDraft] = useState("");
  const [mode, setMode] = useState<QueryMode>("hybrid");
  const [tokenDialogOpen, setTokenDialogOpen] = useState(false);
  const bottomRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [turns]);

  const needsToken = cfg.authRequired && !token;

  function submit() {
    if (!draft.trim()) return;
    if (needsToken) {
      setTokenDialogOpen(true);
      return;
    }
    void send(draft, mode);
    setDraft("");
  }

  return (
    <div className="space-y-4 max-w-4xl mx-auto">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          {t("query.title")}
        </h1>
        <p className="text-sm text-muted-foreground">{t("query.subtitle")}</p>
      </div>

      <div className="rounded-lg border bg-amber-50 dark:bg-amber-950/40 p-3 text-xs flex items-start gap-2">
        <AlertTriangle className="h-4 w-4 text-amber-700 dark:text-amber-300 shrink-0 mt-0.5" />
        <div>{t("query.token_warning")}</div>
      </div>

      {needsToken ? (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm flex items-center justify-between gap-2">
          <span>{t("query.needs_token")}</span>
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

      <div className="rounded-lg border min-h-[300px] p-4 space-y-4">
        {turns.length === 0 ? (
          <div className="text-sm text-muted-foreground text-center py-12">
            {t("query.empty")}
          </div>
        ) : (
          turns.map((turn) => (
            <div key={turn.id} className="space-y-2">
              <div className="flex justify-end">
                <div className="bg-primary text-primary-foreground rounded-2xl rounded-tr-sm px-3 py-2 max-w-[80%] text-sm whitespace-pre-wrap">
                  <div className="text-[10px] uppercase opacity-70">
                    {t("query.user_label")}
                  </div>
                  {turn.question}
                </div>
              </div>
              <div className="flex justify-start">
                <div className="bg-muted rounded-2xl rounded-tl-sm px-3 py-2 max-w-[80%] text-sm whitespace-pre-wrap">
                  <div className="text-[10px] uppercase opacity-70 flex items-center gap-1">
                    {t("query.assistant_label")}
                    {turn.status === "streaming" ? (
                      <span className="ml-1 text-muted-foreground">
                        {t("query.streaming")}
                      </span>
                    ) : null}
                  </div>
                  {turn.answer || (turn.status === "pending" ? "…" : "")}
                  {turn.status === "error" ? (
                    <div className="mt-1 text-xs text-destructive">
                      {turn.errorMessage || t("query.stream_error")}
                    </div>
                  ) : null}
                  {turn.sources.length > 0 ? (
                    <div className="mt-2 pt-2 border-t">
                      <div className="text-[10px] uppercase opacity-70 mb-1">
                        {t("query.sources")}
                      </div>
                      <ul className="text-xs space-y-1">
                        {turn.sources.map((s) => (
                          <li
                            key={s}
                            className="flex items-center justify-between gap-2"
                          >
                            <span className="truncate">{s}</span>
                            <Link
                              to={`/pages/default/${encodeURIComponent(s)}`}
                              className="text-primary hover:underline shrink-0"
                            >
                              {t("query.open_in_pages")}
                            </Link>
                          </li>
                        ))}
                      </ul>
                    </div>
                  ) : null}
                </div>
              </div>
            </div>
          ))
        )}
        <div ref={bottomRef} />
      </div>

      <div className="space-y-2">
        <div className="flex items-center gap-3 flex-wrap">
          <Label className="text-xs uppercase text-muted-foreground">
            {t("query.mode")}
          </Label>
          {MODES.map((m) => (
            <label
              key={m}
              className={cn(
                "flex items-center gap-1 text-xs cursor-pointer rounded-md px-2 py-1 border",
                mode === m
                  ? "border-primary bg-primary/10"
                  : "border-transparent hover:bg-accent",
              )}
              title={t(`query.mode_help.${m}`)}
            >
              <input
                type="radio"
                name="query-mode"
                value={m}
                checked={mode === m}
                onChange={() => setMode(m)}
              />
              {t(`query.modes.${m}`)}
            </label>
          ))}
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto"
            onClick={clear}
            disabled={turns.length === 0}
          >
            <Trash2 className="h-4 w-4 mr-1" />
            {t("query.clear_history")}
          </Button>
        </div>
        <Textarea
          rows={3}
          placeholder={t("query.input_placeholder")}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              submit();
            }
          }}
        />
        <div className="flex justify-end">
          <Button onClick={submit} disabled={!draft.trim() || streaming}>
            <Send className="h-4 w-4 mr-1" />
            {t("query.send")}
          </Button>
        </div>
      </div>
    </div>
  );
}
