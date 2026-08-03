import type { TFunction } from "i18next";
import type { Issue } from "../types/asset";

export interface LocalizedIssue {
  title: string;
  message: string;
  suggestion?: string;
}

/// One issue's three user-facing strings in the active locale.
///
/// The keys are derived from `rule_id` — it is 1:1 with the rule's copy, so
/// there is no separate key field on the wire. Note that `issues.rules.*` and
/// `issues.duration.*` exist ONLY in the non-English locales: English is the
/// analyzer's own prose, which arrives on the issue itself. A new locale adds
/// its own section; copying the key set from en.json would miss this whole
/// subtree.
///
/// `defaultValue` is what makes partial translation safe — a rule with no
/// entry renders in English rather than as a bare key, so the 23 rules can be
/// translated in batches.
///
/// Careful with rule ids when writing those entries: i18next's default
/// `keySeparator` is `.`, and every rule id except `duplicate` and
/// `missing_reference` contains a dot (e.g. `texture.max_size`), so
/// `issues.rules.texture.max_size.title`
/// resolves as a five-level nest, not `rules` → a `"texture.max_size"` key.
/// A flat `"texture.max_size": { "title": ... }` under `rules` silently
/// misses and falls back to English with no error anywhere.
export function localizeIssue(issue: Issue, t: TFunction): LocalizedIssue {
  const base = `issues.rules.${issue.rule_id}`;
  const args = issue.args ?? {};
  // dcc_source's age unit is the one placeholder whose value is itself a
  // word. The bucket choice (s/m/h/d) stays in Rust; only the noun is looked
  // up here, so there is no second humanize implementation.
  const vars = args.age_unit
    ? { ...args, age_unit: t(`issues.duration.${args.age_unit}`, { defaultValue: args.age_unit }) }
    : args;
  return {
    title: t(`${base}.title`, { defaultValue: issue.rule_name }),
    message: t(`${base}.message`, { ...vars, defaultValue: issue.message }),
    // Guarded on the analyzer's own output: a locale entry must not conjure a
    // suggestion for a rule that never makes one. `suggestion` has no
    // `skip_serializing_if` on the Rust side (unlike `related_paths` / `args`),
    // so "no suggestion" arrives as `null`, not an absent key — `== null`
    // catches both `null` and `undefined`; `=== undefined` would let a real
    // `null` through and i18next's `isValidLookup` rejects a `null`
    // `defaultValue`, so `t()` would fall back to returning the bare key.
    suggestion:
      issue.suggestion == null
        ? undefined
        : t(`${base}.suggestion`, { ...vars, defaultValue: issue.suggestion }),
  };
}
