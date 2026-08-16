/// What a click or key press means for selection, as three orthogonal bits rather
/// than "which modifier was held": the keyboard has no `MouseEvent`, and
/// Shift+arrow is genuinely a move and an extension at once.
export type SelectIntent = {
  /** Make this asset the previewed one, and the range anchor. */
  select: boolean;
  /** Toggle this asset's membership in the checkbox selection, and anchor it. */
  toggle: boolean;
  /** Add anchor..this to the checkbox selection, leaving the anchor put. */
  extend: boolean;
};

/// Mouse gestures map to exactly one bit each. Shift+click does NOT set `select`:
/// it extends the checkbox selection without disturbing the preview. The
/// keyboard's Shift+arrow sets both, so the cursor stays visible.
export function intentFromMouse(e: {
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
}): SelectIntent {
  if (e.metaKey || e.ctrlKey) {
    return { select: false, toggle: true, extend: false };
  }
  if (e.shiftKey) {
    return { select: false, toggle: false, extend: true };
  }
  return { select: true, toggle: false, extend: false };
}
