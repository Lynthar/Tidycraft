import { Plus, Pencil, Trash2, AlertCircle } from "lucide-react";
import type { GitFileStatus } from "../types/asset";

/// Small inline badge rendered next to file names. Returns `null` for statuses
/// that merit no visual, so callers can render it unconditionally. `t` is passed
/// in because both callers virtualize hundreds of rows and already have it.
export function GitStatusBadge({
  status,
  t,
}: {
  status: GitFileStatus;
  t: (key: string) => string;
}) {
  // Status colours come from the design tokens (git-* palette) so badges track the
  // active theme. `untracked` has no dedicated token — folded into "new" upstream —
  // and `conflicted` borrows the generic error token.
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
