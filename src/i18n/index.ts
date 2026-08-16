import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import zh from "./locales/zh.json";

/// Resolve the startup language: an explicit user choice always wins, otherwise
/// the OS or browser locale decides. Only en and zh ship, so every other locale
/// falls back to en.
function detectLanguage(): "en" | "zh" {
  const saved = localStorage.getItem("language");
  if (saved === "en" || saved === "zh") return saved;
  // `navigator.languages` is ordered by preference, so the FIRST supported entry
  // wins. Unsupported locales are skipped rather than ending the search, so
  // ["fr", "zh-CN"] still reaches Chinese.
  const candidates = [navigator.language, ...(navigator.languages ?? [])];
  for (const candidate of candidates) {
    const tag = candidate?.toLowerCase();
    if (!tag) continue;
    if (tag.startsWith("zh")) return "zh";
    if (tag.startsWith("en")) return "en";
  }
  return "en";
}

const savedLanguage = detectLanguage();

i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    zh: { translation: zh },
  },
  lng: savedLanguage,
  fallbackLng: "en",
  interpolation: {
    escapeValue: false,
  },
});

export default i18n;
