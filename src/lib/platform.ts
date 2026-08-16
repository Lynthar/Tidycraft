/// Platform detection for renderer-side code. Tauri 2's `os` plugin is async,
/// which is awkward for synchronous render-time checks, and Tauri webviews keep
/// the host platform's UA. Falls back to "linux" when `navigator` is undefined.

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
