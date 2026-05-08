import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { basename, dirname } from "./pathUtils";

/**
 * A synchronous URL modifier for Three.js LoadingManager, built asynchronously
 * from a pre-scanned sibling-texture map so that FBX/OBJ/DAE files referencing
 * textures by bare filename (or with a stale absolute path) can still find them.
 *
 * Strategy:
 * 1. Ask the backend to walk the model's dir + common texture subdirs
 *    (`Textures/`, `Materials/`, …) and build a filename → absolute-path map.
 * 2. In the returned modifier, extract the basename of whatever URL Three.js
 *    hands us (works for both bare filenames and already-encoded asset URLs)
 *    and look it up in the map.
 * 3. On miss, fall back to the old behavior (resolve relative to `modelDir`).
 */
export async function buildTextureUrlResolver(
  modelPath: string
): Promise<(url: string) => string> {
  let siblings: Record<string, string> = {};
  try {
    siblings = await invoke<Record<string, string>>("resolve_texture_siblings", {
      modelPath,
    });
  } catch (err) {
    console.warn("[modelUrlResolver] sibling scan failed:", err);
  }

  const dir = dirname(modelPath);
  const modelDir = dir ? `${dir}/` : "";

  const extractBasename = (url: string): string => {
    // Trim query/fragment if any
    let s = url.split("?")[0].split("#")[0];
    // Already-encoded asset.localhost URLs percent-encode slashes to %2F,
    // so decode first, then take the filename component.
    try {
      s = decodeURIComponent(s);
    } catch {
      // keep as-is if malformed
    }
    return basename(s);
  };

  return (url: string): string => {
    if (!url) return url;
    if (url.startsWith("data:") || url.startsWith("blob:")) return url;

    const name = extractBasename(url);
    const hit = siblings[name.toLowerCase()];
    if (hit) {
      return convertFileSrc(hit);
    }

    // Fallback: preserve legacy behavior for URLs we can't resolve.
    if (url.startsWith("asset://") || url.startsWith("http://") || url.startsWith("https://")) {
      return url;
    }
    if (url.startsWith("/")) {
      return convertFileSrc(url);
    }
    return convertFileSrc(modelDir + name);
  };
}
