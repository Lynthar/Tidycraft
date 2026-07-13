import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import zh from "./locales/zh.json";

/// Resolve the startup language: an explicit user choice (persisted on the
/// language toggle) always wins; otherwise sniff the OS/browser locale so a
/// zh-* system boots into Chinese instead of always defaulting to English.
/// Only en/zh ship, so every other locale falls back to en.
function detectLanguage(): "en" | "zh" {
  const saved = localStorage.getItem("language");
  if (saved === "en" || saved === "zh") return saved;
  const candidates = [navigator.language, ...(navigator.languages ?? [])];
  if (candidates.some((l) => l?.toLowerCase().startsWith("zh"))) return "zh";
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
