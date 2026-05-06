import { Plus, Pencil, Trash2, AlertCircle } from "lucide-react";
import { cn } from "../lib/utils";
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
  const configs: Record<
    GitFileStatus,
    { icon: React.ReactNode; color: string; bg: string } | null
  > = {
    new: { icon: <Plus size={10} />, color: "text-green-400", bg: "bg-green-400/20" },
    modified: { icon: <Pencil size={10} />, color: "text-yellow-400", bg: "bg-yellow-400/20" },
    deleted: { icon: <Trash2 size={10} />, color: "text-red-400", bg: "bg-red-400/20" },
    renamed: { icon: <Pencil size={10} />, color: "text-blue-400", bg: "bg-blue-400/20" },
    untracked: { icon: <Plus size={10} />, color: "text-gray-400", bg: "bg-gray-400/20" },
    conflicted: { icon: <AlertCircle size={10} />, color: "text-red-500", bg: "bg-red-500/20" },
    typechange: null,
    ignored: null,
    unchanged: null,
  };

  const config = configs[status];
  if (!config) return null;

  return (
    <span
      className={cn(
        "inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[10px] font-medium",
        config.color,
        config.bg
      )}
      title={t(`git.status.${status}`)}
    >
      {config.icon}
    </span>
  );
}
