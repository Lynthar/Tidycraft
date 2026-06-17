/// Shared LRU cache for gallery thumbnails (base64 PNG strings).
///
/// Lives in its own module — rather than inside AssetGalleryView — so two
/// callers can reach it without a store→component dependency:
///   - AssetGalleryView reads/writes it as cards mount.
///   - projectStore.applyFsChange evicts entries when the watcher reports a
///     file changed, so an external edit shows a fresh image instead of the
///     stale cached one (the backend disk cache is mtime-keyed and already
///     regenerates; this keeps the frontend in step).
///
/// Bounded to CAP entries by insertion-order eviction (oldest written entry
/// drops first). A re-fetch after eviction is a cheap disk-cache read on the
/// backend, not a re-decode, so capping costs almost nothing.
///
/// Value `null` = "tried, failed — don't retry". `peekThumb` returning
/// `undefined` = "not cached yet".

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
