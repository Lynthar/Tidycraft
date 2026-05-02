import { create } from "zustand";

export type ThemePreference = "dark" | "light" | "system";
export type AppliedTheme = "dark" | "light";

const STORAGE_KEY = "theme";

const getInitialPreference = (): ThemePreference => {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved === "light" || saved === "dark" || saved === "system") return saved;
  // Forge Dark is the default per redesign spec — first launch hits brand
  // visual rather than auto-following system.
  return "dark";
};

const systemPrefersLight = (): boolean =>
  typeof window !== "undefined" &&
  typeof window.matchMedia === "function" &&
  window.matchMedia("(prefers-color-scheme: light)").matches;

const resolveTheme = (pref: ThemePreference): AppliedTheme => {
  if (pref === "system") return systemPrefersLight() ? "light" : "dark";
  return pref;
};

const applyTheme = (theme: AppliedTheme) => {
  document.documentElement.setAttribute("data-theme", theme);
};

interface ThemeState {
  /// User's stored choice. "system" follows OS prefers-color-scheme live.
  preference: ThemePreference;
  /// Concretely-applied theme that components read for icon/label decisions.
  /// Equals `preference` when preference is dark/light; resolved from system
  /// when preference is "system".
  theme: AppliedTheme;
  setPreference: (pref: ThemePreference) => void;
  /// Convenience cycle for the toolbar Sun/Moon button: toggles between
  /// explicit dark and light. "system" mode is only reachable via Settings.
  toggleTheme: () => void;
}

export const useThemeStore = create<ThemeState>((set, get) => {
  const preference = getInitialPreference();
  const theme = resolveTheme(preference);
  if (typeof document !== "undefined") applyTheme(theme);

  // When preference is "system", track OS theme changes live. The listener
  // is attached once at store creation and never removed — store lives for
  // the whole app session, so cleanup isn't worth the complexity.
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const onSystemChange = () => {
      if (get().preference !== "system") return;
      const next = resolveTheme("system");
      applyTheme(next);
      set({ theme: next });
    };
    mq.addEventListener("change", onSystemChange);
  }

  return {
    preference,
    theme,
    setPreference: (pref) => {
      localStorage.setItem(STORAGE_KEY, pref);
      const next = resolveTheme(pref);
      applyTheme(next);
      set({ preference: pref, theme: next });
    },
    toggleTheme: () => {
      const next = get().theme === "dark" ? "light" : "dark";
      get().setPreference(next);
    },
  };
});
