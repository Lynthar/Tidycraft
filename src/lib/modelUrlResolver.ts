import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { basename, dirname } from "./pathUtils";

/** A synchronous URL modifier for three.js's LoadingManager, built from a
 *  pre-scanned sibling-texture map so FBX/OBJ/DAE files referencing textures by
 *  bare filename can find them. On a miss it resolves relative to `modelDir`. */
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
