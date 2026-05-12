import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";
import en from "./en.json";
import es from "./es.json";
import { getConfig } from "@/lib/config";
import { useLocaleStore } from "@/stores/locale";

const fallback = getConfig().defaultLocale ?? "en";

void i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
      es: { translation: es },
    },
    fallbackLng: fallback,
    supportedLngs: ["en", "es"],
    interpolation: { escapeValue: false },
    detection: {
      order: ["localStorage", "navigator", "htmlTag"],
      lookupLocalStorage: "coral.locale",
      caches: ["localStorage"],
    },
  });

// NOTE(coral-ui frontend): zustand store overrides detection when user
// explicitly chooses a locale via the switcher.
const initial = useLocaleStore.getState().locale;
if (initial && initial !== i18n.language) {
  void i18n.changeLanguage(initial);
}
useLocaleStore.subscribe((s) => {
  if (s.locale && s.locale !== i18n.language) {
    void i18n.changeLanguage(s.locale);
  }
});

export default i18n;
