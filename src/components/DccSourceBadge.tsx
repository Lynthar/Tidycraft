import { cn } from "../lib/utils";
import { dccSourceLabel } from "../lib/dccSource";

/// Small inline badge naming the authoring tool of a DCC source file
/// (.blend → "Blender"), rendered next to file names in AssetListView and
/// over the thumbnail in AssetGalleryView. Same conventions as
/// GitStatusBadge: `t` is passed in because both callers virtualize
/// hundreds of rows and already have it in scope.
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
