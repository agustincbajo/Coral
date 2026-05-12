import { useTranslation } from "react-i18next";
import { Languages } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useLocaleStore, type Locale } from "@/stores/locale";

export function LocaleSwitcher() {
  const { i18n, t } = useTranslation();
  const setLocale = useLocaleStore((s) => s.setLocale);
  const current = (i18n.resolvedLanguage ?? i18n.language ?? "en").slice(
    0,
    2,
  ) as Locale;
  return (
    <div className="flex items-center gap-2">
      <Languages className="h-4 w-4 text-muted-foreground" aria-hidden />
      <Select
        value={current}
        onValueChange={(v) => setLocale(v as Locale)}
      >
        <SelectTrigger className="h-9 w-[120px]">
          <SelectValue placeholder={t("locale.label")} />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="en">{t("locale.en")}</SelectItem>
          <SelectItem value="es">{t("locale.es")}</SelectItem>
        </SelectContent>
      </Select>
    </div>
  );
}
