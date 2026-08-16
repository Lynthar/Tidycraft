/// Bounded request queue for gallery thumbnails: `get_thumbnail` decodes the image
/// in full before resizing, so a fast scroll would otherwise put hundreds of
/// decodes in flight. A request still queued when its card unmounts is dropped.

import { invoke } from "@tauri-apps/api/core";

/// Roughly three to four batches to fill a typical 20-card viewport, while
/// leaving cores free for a scan or an analysis running alongside.
const MAX_IN_FLIGHT = 6;

/// Rejection value for a request dropped before it was ever sent. It must be
/// distinguishable from a real failure: the thumbnail cache records failures as
/// tombstones, which would leave fast-scrolled cards permanently blank.
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

/// Queue a thumbnail request. Call `cancel` when the requester goes away: it drops
/// a still-queued request, and does nothing for one already sent — the caller
/// should still cache that result, since the backend has already paid for it.
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
