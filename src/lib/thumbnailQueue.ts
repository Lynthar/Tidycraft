/// Bounded request queue for gallery thumbnails.
///
/// `get_thumbnail` decodes the image in full on the backend before resizing
/// it (a 4096² texture is 67 MB of pixels), and tokio's blocking pool grows a
/// thread per queued task up to 512 — so scrolling quickly through a texture
/// library used to put hundreds of full decodes in flight at once.
///
/// A concurrency cap alone would not fix what the user actually sees, though.
/// Stopping after a long scroll left the twenty visible cards queued behind
/// hundreds of requests for cards that had already gone past. So a request
/// still waiting when its card unmounts is dropped: scrolled-past work
/// evaporates, and what is left is what is on screen. FIFO plus dropping is
/// enough — no priority queue needed.
///
/// A request that has already been sent cannot be recalled: Tauri's `invoke`
/// has no cancellation and the backend will finish it either way. Cancelling
/// one is therefore a no-op, and the caller is expected to keep the result
/// (see `THUMB_CANCELLED`).

import { invoke } from "@tauri-apps/api/core";

/// Roughly three to four batches to fill a typical 20-card viewport, while
/// leaving cores free for a scan or an analysis running alongside.
const MAX_IN_FLIGHT = 6;

/// Rejection value for a request dropped before it was ever sent.
///
/// It has to be distinguishable from a real failure: the thumbnail cache
/// records failures as tombstones ("tried, don't retry"), so storing a
/// cancellation that way would leave whole swathes of a fast-scrolled gallery
/// showing the placeholder glyph permanently.
export const THUMB_CANCELLED = Symbol("thumbnail-request-cancelled");

interface PendingRequest {
  path: string;
  size: number;
  resolve: (value: string) => void;
  reject: (reason: unknown) => void;
  /// Dropped while queued — `pump` skips it rather than splicing it out, so
  /// cancelling stays O(1) during the burst of unmounts a fast scroll causes.
  cancelled: boolean;
  /// Already handed to `invoke`; no longer cancellable.
  sent: boolean;
}

const waiting: PendingRequest[] = [];
let inFlight = 0;

function pump(): void {
  while (inFlight < MAX_IN_FLIGHT) {
    const next = waiting.shift();
    if (next === undefined) return;
    if (next.cancelled) continue;

    next.sent = true;
    inFlight++;
    invoke<string>("get_thumbnail", { path: next.path, size: next.size })
      .then(next.resolve, next.reject)
      .finally(() => {
        inFlight--;
        pump();
      });
  }
}

/// Queue a thumbnail request.
///
/// Call `cancel` when the requester goes away. It drops the request if it is
/// still queued; if it has already been sent, it does nothing and the promise
/// settles normally — the caller should still cache that result, since the
/// backend has already paid for it.
export function requestThumbnail(
  path: string,
  size: number
): { promise: Promise<string>; cancel: () => void } {
  let entry!: PendingRequest;
  const promise = new Promise<string>((resolve, reject) => {
    entry = { path, size, resolve, reject, cancelled: false, sent: false };
    waiting.push(entry);
  });
  pump();

  return {
    promise,
    cancel: () => {
      if (entry.sent || entry.cancelled) return;
      entry.cancelled = true;
      entry.reject(THUMB_CANCELLED);
    },
  };
}
