/// Platform detection for renderer-side code.
///
/// Tauri 2 has an async `os` plugin that requires importing and awaiting,
/// which is awkward for synchronous render-time checks (CSS class toggles,
/// shortcut display). Sniffing `navigator.userAgent` once and caching the
/// result is reliable enough — Tauri webviews keep the host platform's UA.
/// Server-side rendering / unit tests fall back to "linux" so callers
/// don't crash on `navigator` being undefined.

export type Platform = "macos" | "windows" | "linux";

let cached: Platform | null = null;

export function getPlatform(): Platform {
  if (cached) return cached;
  if (typeof navigator === "undefined") {
    cached = "linux";
    return cached;
  }
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("mac")) cached = "macos";
  else if (ua.includes("win")) cached = "windows";
  else cached = "linux";
  return cached;
}

export const isMacOS = (): boolean => getPlatform() === "macos";
export const isWindows = (): boolean => getPlatform() === "windows";
export const isLinux = (): boolean => getPlatform() === "linux";
