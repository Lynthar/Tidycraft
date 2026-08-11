/// What a click or key press means for selection, as three orthogonal bits
/// rather than "which modifier was held". The keyboard has no `MouseEvent` to
/// read, and Shift+arrow is genuinely two things at once — a move and an
/// extension — which a "which modifier" enum cannot express without a flag
/// hanging off one of its variants.
export type SelectIntent = {
  /** Make this asset the previewed one, and the range anchor. */
  select: boolean;
  /** Toggle this asset's membership in the checkbox selection, and anchor it. */
  toggle: boolean;
  /** Add anchor..this to the checkbox selection, leaving the anchor put. */
  extend: boolean;
};

/// Mouse gestures map to exactly one bit each. Shift+click deliberately does
/// NOT set `select`: it extends the checkbox selection without disturbing what
/// is being previewed. The keyboard's Shift+arrow sets both, because there the
/// user is walking the cursor and must be able to see where it went.
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
