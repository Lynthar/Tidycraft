//! Project-level metadata read out of `tidycraft.toml`'s `[project]`
//! table. Consumed by AI Learning + AI Tagging to give the LLM a sense
//! of what kind of project this is.
//!
//! We deliberately do NOT extend `analyzer::rules::RuleConfig` with a
//! project field — analyzer concerns and AI-context concerns are
//! separate, and mixing them would force `RuleConfig::from_toml` to
//! validate fields it doesn't care about. Instead we re-parse the TOML
//! through `toml::Value` and pluck the `[project]` subtable.

use serde::{Deserialize, Serialize};

/// Free-form project context the user (optionally) writes into
/// `tidycraft.toml`. All fields are `Option` because empty / missing
/// is the common case — most projects start without any.
///
/// Empty-string fields are normalized to `None` at read time so the
/// prompt builder doesn't emit `Theme: ""` placeholders.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProjectMeta {
    pub theme: Option<String>,
    pub goal: Option<String>,
}

impl ProjectMeta {
    /// Returns true when there's nothing useful to feed the LLM.
    /// Callers skip emitting the project-context block entirely in
    /// that case to save tokens.
    pub fn is_empty(&self) -> bool {
        self.theme.as_deref().map_or(true, str::is_empty)
            && self.goal.as_deref().map_or(true, str::is_empty)
    }

    /// Parse from a TOML document. Looks for a top-level `[project]`
    /// table; absent → returns default. Empty-string values are
    /// normalized to `None`.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        let val: toml::Value = toml::from_str(content)?;
        let meta = val
            .get("project")
            .cloned()
            .map(|v| v.try_into::<ProjectMeta>())
            .transpose()?
            .unwrap_or_default();
        Ok(meta.normalize())
    }

    fn normalize(mut self) -> Self {
        if self.theme.as_deref().map_or(false, str::is_empty) {
            self.theme = None;
        }
        if self.goal.as_deref().map_or(false, str::is_empty) {
            self.goal = None;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filled_project_table() {
        let toml_str = r#"
[project]
theme = "Cyberpunk RPG"
goal = "Asset library for characters and props"

[naming]
enabled = true
"#;
        let m = ProjectMeta::from_toml(toml_str).unwrap();
        assert_eq!(m.theme.as_deref(), Some("Cyberpunk RPG"));
        assert_eq!(m.goal.as_deref(), Some("Asset library for characters and props"));
        assert!(!m.is_empty());
    }

    #[test]
    fn missing_project_table_yields_default() {
        let toml_str = r#"
[naming]
enabled = true
"#;
        let m = ProjectMeta::from_toml(toml_str).unwrap();
        assert!(m.is_empty());
    }

    #[test]
    fn empty_strings_normalize_to_none() {
        // The default config template ships with empty placeholder
        // strings; we should treat those as "not set".
        let toml_str = r#"
[project]
theme = ""
goal = ""
"#;
        let m = ProjectMeta::from_toml(toml_str).unwrap();
        assert!(m.is_empty());
        assert!(m.theme.is_none());
        assert!(m.goal.is_none());
    }

    #[test]
    fn partial_fields_work() {
        let toml_str = r#"
[project]
theme = "Sci-fi platformer"
"#;
        let m = ProjectMeta::from_toml(toml_str).unwrap();
        assert_eq!(m.theme.as_deref(), Some("Sci-fi platformer"));
        assert!(m.goal.is_none());
        assert!(!m.is_empty());
    }

    #[test]
    fn other_unknown_fields_dont_break_parse() {
        let toml_str = r#"
[project]
theme = "X"
random_extra = "should-not-fail"
"#;
        // Unknown field tolerated due to no #[serde(deny_unknown_fields)].
        let m = ProjectMeta::from_toml(toml_str).unwrap();
        assert_eq!(m.theme.as_deref(), Some("X"));
    }
}
