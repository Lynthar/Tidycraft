import i18n from "../i18n";

/// The active locale's issue templates, flattened for the Rust side:
/// `issues.rules.texture.max_size.message` → `"texture.max_size.message"`.
/// Returns undefined for English, where the report uses the analyzer's prose.
export function issueTemplatesForExport(): Record<string, string> | undefined {
  if (i18n.language === "en") return undefined;
  const bundle = i18n.getResourceBundle(i18n.language, "translation");
  const issues = bundle?.issues;
  if (!issues) return undefined;

  const flat: Record<string, string> = {};
  const walk = (node: unknown, prefix: string) => {
    if (typeof node === "string") {
      flat[prefix] = node;
      return;
    }
    if (node && typeof node === "object") {
      for (const [k, v] of Object.entries(node)) {
        walk(v, prefix ? `${prefix}.${k}` : k);
      }
    }
  };
  walk(issues.rules, "");
  walk(issues.duration, "duration");
  return Object.keys(flat).length > 0 ? flat : undefined;
}
