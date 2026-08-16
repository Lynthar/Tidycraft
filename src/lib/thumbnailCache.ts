/// Shared LRU cache for gallery thumbnails (base64 PNG). Its own module so both
/// AssetGalleryView and projectStore's watcher eviction can reach it. Value `null`
/// = tried and failed; `peekThumb` returning `undefined` = not cached yet.

const CAP = 600;
const cache = new Map<string, string | null>();

/** Read without touching eviction order — safe to call during render. */
export function peekThumb(path: string): string | null | undefined {
  return cache.get(path);
}

export function hasThumb(path: string): boolean {
  return cache.has(path);
}

/** Insert/refresh an entry, evicting the oldest entries once past CAP. */
export function putThumb(path: string, value: string | null): void {
  if (cache.has(path)) cache.delete(path); // re-set → move to newest position
  cache.set(path, value);
  while (cache.size > CAP) {
    const oldest = cache.keys().next();
    if (oldest.done) break;
    cache.delete(oldest.value);
  }
}

/** Drop specific paths (files modified/removed via an fs-change event). */
export function evictThumbs(paths: Iterable<string>): void {
  for (const p of paths) cache.delete(p);
}
