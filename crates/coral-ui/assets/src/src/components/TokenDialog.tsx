import { useState } from "react";
import { useTranslation } from "react-i18next";
import { KeyRound } from "lucide-react";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAuthStore } from "@/stores/auth";

interface TokenDialogProps {
  /** Optional controlled mode — if `open` is set, no internal trigger is shown. */
  open?: boolean;
  onOpenChange?: (v: boolean) => void;
  /** Whether to render a default trigger button. Defaults to true. */
  showTrigger?: boolean;
}

export function TokenDialog({
  open,
  onOpenChange,
  showTrigger = true,
}: TokenDialogProps) {
  const { t } = useTranslation();
  const stored = useAuthStore((s) => s.token);
  const setToken = useAuthStore((s) => s.setToken);
  const clear = useAuthStore((s) => s.clear);
  const [draft, setDraft] = useState(stored ?? "");

  return (
    <Dialog
      open={open}
      onOpenChange={(v) => {
        if (v) setDraft(stored ?? "");
        onOpenChange?.(v);
      }}
    >
      {showTrigger && open === undefined ? (
        <DialogTrigger asChild>
          <Button variant="outline" size="sm">
            <KeyRound className="mr-2 h-4 w-4" />
            {t("auth.token_dialog.open")}
          </Button>
        </DialogTrigger>
      ) : null}
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("auth.token_dialog.title")}</DialogTitle>
          <DialogDescription>
            {t("auth.token_dialog.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <Label htmlFor="coral-token">
            {t("auth.token_dialog.label")}
          </Label>
          <Input
            id="coral-token"
            type="password"
            placeholder={t("auth.token_dialog.placeholder")}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            autoComplete="off"
          />
          <p className="text-xs text-muted-foreground">
            {t("auth.token_dialog.stored_hint")}
          </p>
        </div>
        <DialogFooter className="gap-2">
          <DialogClose asChild>
            <Button
              variant="ghost"
              onClick={() => {
                clear();
                setDraft("");
              }}
            >
              {t("auth.token_dialog.clear")}
            </Button>
          </DialogClose>
          <DialogClose asChild>
            <Button
              onClick={() => setToken(draft.trim() || null)}
              disabled={!draft.trim()}
            >
              {t("auth.token_dialog.save")}
            </Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
