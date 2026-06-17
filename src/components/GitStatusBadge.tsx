import { Plus, Pencil, Trash2, AlertCircle } from "lucide-react";
import type { GitFileStatus } from "../types/asset";

/// Small inline badge rendered next to file names in AssetListView and
/// AssetGalleryView. Returns `null` for statuses that don't merit a visual
/// (typechange, ignored, unchanged) so callers can render unconditionally.
///
/// `t` is passed in rather than pulled via `useTranslation` because both
/// callers virtualize hundreds of rows and already have `t` in scope —
/// re-subscribing here per row would be wasted work.
export function GitStatusBadge({
  status,
  t,
}: {
  status: GitFileStatus;
  t: (key: string) => string;
}) {
  // Status colors come from the design tokens (git-* palette) rather than
  // hardcoded Tailwind classes, so badges track the active theme. `untracked`
  // has no dedicated token (folded into "new" upstream) → muted text;
  // `conflicted` borrows the generic error token.
  const configs: Record<
    GitFileStatus,
    { icon: React.ReactNode; color: string } | null
  > = {
    new: { icon: <Plus size={10} />, color: "var(--git-new)" },
    modified: { icon: <Pencil size={10} />, color: "var(--git-modified)" },
    deleted: { icon: <Trash2 size={10} />, color: "var(--git-deleted)" },
    renamed: { icon: <Pencil size={10} />, color: "var(--git-renamed)" },
    untracked: { icon: <Plus size={10} />, color: "var(--text-3)" },
    conflicted: { icon: <AlertCircle size={10} />, color: "var(--err)" },
    typechange: null,
    ignored: null,
    unchanged: null,
  };

  const config = configs[status];
  if (!config) return null;

  return (
    <span
      className="inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[10px] font-medium"
      style={{
        color: config.color,
        background: `color-mix(in oklch, ${config.color} 20%, transparent)`,
      }}
      title={t(`git.status.${status}`)}
    >
      {config.icon}
    </span>
  );
}
