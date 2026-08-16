/// Path utilities for the renderer. Backend paths are always forward-slashed, but
/// both separators are accepted defensively: file dialogs, embedded texture URLs
/// from model loaders, and OS-picker editor paths can all carry either.

/// Last index of either `/` or `\` in `path`, or -1 if neither appears.
function lastSeparatorIndex(path: string): number {
  return Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
}

/// Filename portion of a path — everything after the last separator.
/// Returns `path` unchanged when there is no separator.
export function basename(path: string): string {
  const i = lastSeparatorIndex(path);
  return i >= 0 ? path.slice(i + 1) : path;
}

/// Directory portion of a path — everything up to (but excluding) the
/// last separator. Returns "" for inputs with no separator.
export function dirname(path: string): string {
  const i = lastSeparatorIndex(path);
  return i >= 0 ? path.slice(0, i) : "";
}

/// Lowercase extension with a leading dot (e.g. ".png"). Returns "" for
/// dotfiles (`.gitignore`), paths without an extension, and edge cases
/// where the only dot is the path separator's neighbor.
export function getExtension(path: string): string {
  const lastDot = path.lastIndexOf(".");
  const lastSep = lastSeparatorIndex(path);
  if (lastDot <= lastSep) return "";
  return path.slice(lastDot).toLowerCase();
}

/// Project-relative form of `path` when it lives under `root`, else the input
/// unchanged. Accepts either separator and compares the prefix case-insensitively.
/// Used for user-facing path display, where the absolute prefix is noise.
export function relativeToRoot(path: string, root: string | null | undefined): string {
  if (!root) return path;
  const p = path.replace(/\\/g, "/");
  const r = root.replace(/\\/g, "/").replace(/\/+$/, "");
  if (p.toLowerCase() === r.toLowerCase()) return basename(p) || p;
  if (p.toLowerCase().startsWith(r.toLowerCase() + "/")) return p.slice(r.length + 1);
  return path;
}

/// Pretty display name for an editor binary path: strips the directory and the
/// `.exe` / `.app` suffix, so the menu reads "Open in Photoshop".
export function getEditorDisplayName(editorPath: string): string {
  const name = basename(editorPath);
  const lower = name.toLowerCase();
  if (lower.endsWith(".exe")) return name.slice(0, -4);
  if (lower.endsWith(".app")) return name.slice(0, -4);
  return name;
}
