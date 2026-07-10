/// Path utilities for the renderer.
///
/// All paths emitted by the backend are normalized to forward slashes by
/// `scanner::path_to_string`, so a `lastIndexOf("/")` would suffice for
/// well-behaved input. We accept both separators here defensively because:
///   - File dialogs occasionally surface raw OS paths.
///   - FBX/OBJ/DAE loaders emit embedded texture URLs with mixed separators.
///   - User-supplied editor paths in Settings come straight from the OS picker.

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

/// Filename minus its extension — for `"a/b/wood.png"` returns `"wood"`.
/// Dotfiles (no extension) keep their full name.
export function basenameWithoutExt(path: string): string {
  const name = basename(path);
  const lastDot = name.lastIndexOf(".");
  return lastDot > 0 ? name.slice(0, lastDot) : name;
}

/// Project-relative form of `path` when it lives under `root`; otherwise the
/// input unchanged. Accepts either separator on both sides and compares the
/// prefix case-insensitively (Windows drive-letter / user-folder casing can
/// differ between the dialog, the backend, and persisted state). Used for
/// user-facing path display — issue rows, the directory-scope bar — where the
/// absolute prefix is pure noise.
export function relativeToRoot(path: string, root: string | null | undefined): string {
  if (!root) return path;
  const p = path.replace(/\\/g, "/");
  const r = root.replace(/\\/g, "/").replace(/\/+$/, "");
  if (p.toLowerCase() === r.toLowerCase()) return basename(p) || p;
  if (p.toLowerCase().startsWith(r.toLowerCase() + "/")) return p.slice(r.length + 1);
  return path;
}

/// Pretty display name for an editor binary path: strips directory and
/// the `.exe` / `.app` suffix common on Windows / macOS. Used in
/// ContextMenu and AssetPreview to render "Open in Photoshop" rather
/// than "Open in Photoshop.exe".
export function getEditorDisplayName(editorPath: string): string {
  const name = basename(editorPath);
  const lower = name.toLowerCase();
  if (lower.endsWith(".exe")) return name.slice(0, -4);
  if (lower.endsWith(".app")) return name.slice(0, -4);
  return name;
}
