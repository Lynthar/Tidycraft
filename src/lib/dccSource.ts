/** Display names for `AssetMetadata.dcc_source_kind`. Must stay in sync with the
 *  canonical list in Rust `scanner::dcc_source_kind_for` — there is no codegen.
 *  Tool names are proper nouns and are not localized. */
const DCC_LABELS: Record<string, string> = {
  blender: "Blender",
  maya_ascii: "Maya",
  maya_binary: "Maya",
  max: "3ds Max",
  zbrush: "ZBrush",
  substance_painter: "Substance Painter",
  substance_designer: "Substance Designer",
  marvelous: "Marvelous Designer",
  photoshop: "Photoshop",
  modo: "Modo",
  houdini: "Houdini",
  cinema4d: "Cinema 4D",
};

/** Unknown kinds (a newer backend teaching new tools before this map is
 *  synced) fall back to the raw identifier so the badge never renders empty. */
export function dccSourceLabel(kind: string): string {
  return DCC_LABELS[kind] ?? kind;
}
