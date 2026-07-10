import { create } from "zustand";

/// Transient bottom-right notifications. Deliberately tiny: silent operations
/// (export, later copy/duplicate feedback) share this one channel instead of
/// each inventing an inline banner. Toasts auto-dismiss; `actionLabel` +
/// `onAction` render a single optional button (e.g. "Show in folder").

export interface Toast {
  id: number;
  kind: "success" | "error";
  message: string;
  actionLabel?: string;
  onAction?: () => void;
}

interface ToastState {
  toasts: Toast[];
  push: (toast: Omit<Toast, "id">, durationMs?: number) => void;
  dismiss: (id: number) => void;
}

let nextId = 1;

export const useToastStore = create<ToastState>((set, get) => ({
  toasts: [],
  push: (toast, durationMs) => {
    const id = nextId++;
    set({ toasts: [...get().toasts, { ...toast, id }] });
    // Errors linger longer — the user may want to read the reason.
    const ttl = durationMs ?? (toast.kind === "error" ? 8000 : 5000);
    setTimeout(() => get().dismiss(id), ttl);
  },
  dismiss: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
}));
