import { X, CheckCircle2, AlertCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useToastStore } from "../stores/toastStore";

/// Fixed bottom-right toast stack. Mounted once at the App root; content
/// comes from toastStore. `aria-live` lets screen readers announce new
/// toasts without focus ever moving here.
export function Toasts() {
  const { t } = useTranslation();
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  if (toasts.length === 0) return null;

  return (
    <div className="tc-toasts" role="status" aria-live="polite">
      {toasts.map((toast) => (
        <div key={toast.id} className="tc-toast" data-kind={toast.kind}>
          <span className="tc-toast-icon">
            {toast.kind === "success" ? <CheckCircle2 size={14} /> : <AlertCircle size={14} />}
          </span>
          <span className="tc-toast-msg">{toast.message}</span>
          {toast.actionLabel && toast.onAction && (
            <button
              className="tc-toast-action"
              onClick={() => {
                toast.onAction?.();
                dismiss(toast.id);
              }}
            >
              {toast.actionLabel}
            </button>
          )}
          <button
            className="tc-toast-close"
            onClick={() => dismiss(toast.id)}
            aria-label={t("toast.dismiss")}
            title={t("toast.dismiss")}
          >
            <X size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}
