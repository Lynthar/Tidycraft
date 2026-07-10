import { useEffect, useRef } from "react";

/// Shared behavior wrapper for every blocking modal: renders the backdrop
/// layer and adds the things each modal previously lacked —
///
///   - Escape closes (global shortcuts deliberately bail while a blocking
///     overlay is open, so without this Esc was dead in every modal)
///   - Tab focus trap (focus cycles inside; it can no longer walk into the
///     asset list behind the backdrop)
///   - `role="dialog"` + `aria-modal`
///   - initial focus (respects `autoFocus` children / `initialFocusRef`)
///   - focus restore to the triggering element on close
///
/// Visuals stay with each modal: children are the untouched content card,
/// and `className` overrides the default backdrop look when needed.
///
/// Callers keep their `if (!open) return null` gate — mounting the shell IS
/// opening it; the lifecycle hooks below key off mount/unmount.

/// Stack of live shells so stacked dialogs (e.g. a confirm over a lightbox)
/// only close the TOP one per Escape press.
const modalStack: symbol[] = [];

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

interface ModalShellProps {
  onClose: () => void;
  /** Accessible dialog name (screen readers announce it on focus entry). */
  ariaLabel: string;
  /** Backdrop classes; defaults to the app's standard dim-and-center layer. */
  className?: string;
  /** Element to focus on open instead of the first focusable child. */
  initialFocusRef?: React.RefObject<HTMLElement | null>;
  /** Click on the backdrop closes (default true). */
  closeOnBackdrop?: boolean;
  /** True while an operation is in flight — Esc/backdrop won't close. */
  disabled?: boolean;
  children: React.ReactNode;
}

export function ModalShell({
  onClose,
  ariaLabel,
  className = "fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4",
  initialFocusRef,
  closeOnBackdrop = true,
  disabled = false,
  children,
}: ModalShellProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  // Refs so the single mount-effect below always sees current values
  // without re-running (re-running would re-steal focus).
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const disabledRef = useRef(disabled);
  disabledRef.current = disabled;
  const initialFocusRefRef = useRef(initialFocusRef);
  initialFocusRefRef.current = initialFocusRef;

  useEffect(() => {
    const token = Symbol("modal");
    modalStack.push(token);
    const container = containerRef.current;
    const opener = document.activeElement as HTMLElement | null;

    const focusables = (): HTMLElement[] =>
      container
        ? Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
            (el) => el.offsetParent !== null || el === document.activeElement
          )
        : [];

    // Initial focus. Deferred a frame so children (and their autoFocus)
    // mount first; if something inside already took focus, leave it alone.
    const raf = requestAnimationFrame(() => {
      if (!container || container.contains(document.activeElement)) return;
      const target = initialFocusRefRef.current?.current ?? focusables()[0] ?? container;
      target.focus();
    });

    const onKeyDown = (event: KeyboardEvent) => {
      // Only the top-most shell reacts; lower ones wait their turn.
      if (modalStack[modalStack.length - 1] !== token) return;

      if (event.key === "Escape") {
        if (disabledRef.current) return;
        // Capture-phase stop: the window-level shortcut handler and any
        // overlay behind this one must not also react to this press.
        event.preventDefault();
        event.stopPropagation();
        onCloseRef.current();
        return;
      }

      if (event.key !== "Tab" || !container) return;
      const items = focusables();
      if (items.length === 0) {
        event.preventDefault();
        container.focus();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement;
      const inside = container.contains(active);
      if (event.shiftKey) {
        if (!inside || active === first) {
          event.preventDefault();
          last.focus();
        }
      } else {
        if (!inside || active === last) {
          event.preventDefault();
          first.focus();
        }
      }
    };

    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("keydown", onKeyDown, true);
      const i = modalStack.indexOf(token);
      if (i >= 0) modalStack.splice(i, 1);
      // Hand focus back to whatever opened the dialog, if it still exists.
      if (opener && document.contains(opener)) opener.focus();
    };
  }, []);

  return (
    <div
      ref={containerRef}
      className={className}
      role="dialog"
      aria-modal="true"
      aria-label={ariaLabel}
      tabIndex={-1}
      onClick={(event) => {
        if (!closeOnBackdrop || disabled) return;
        if (event.target === event.currentTarget) onClose();
      }}
    >
      {children}
    </div>
  );
}
