import { cn } from "../lib/utils";
import { dccSourceLabel } from "../lib/dccSource";

/// Small inline badge naming the authoring tool of a DCC source file (.blend →
/// "Blender"), rendered beside file names and over gallery thumbnails. `t` is
/// passed in, as for GitStatusBadge, because both callers virtualize rows.
export function DccSourceBadge({
  kind,
  t,
  className,
}: {
  kind: string;
  t: (key: string) => string;
  className?: string;
}) {
  const label = dccSourceLabel(kind);
  return (
    <span
      className={cn("tc-dcc-badge", className)}
      title={`${label} · ${t("assetList.dccSourceFile")}`}
    >
      {label}
    </span>
  );
}
