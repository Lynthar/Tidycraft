import type { TFunction } from "i18next";
import type { Issue } from "../types/asset";

export interface LocalizedIssue {
  title: string;
  message: string;
  suggestion?: string;
}

/// One issue's three user-facing strings in the active locale. Keys derive from
/// `rule_id`, and `defaultValue` is the analyzer's English prose, so a rule with
/// no locale entry renders in English rather than as a bare key.
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
    // suggestion for a rule that never makes one. `suggestion` arrives as `null`
    // rather than absent, and `== null` catches both null and undefined.
    suggestion:
      issue.suggestion == null
        ? undefined
        : t(`${base}.suggestion`, { ...vars, defaultValue: issue.suggestion }),
  };
}
