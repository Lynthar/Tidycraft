//! Project-level metadata read out of `tidycraft.toml`'s `[project]`
//! table. Consumed by AI Learning + AI Tagging to give the LLM a sense
//! of what kind of project this is.
//!
//! We deliberately do NOT extend `analyzer::rules::RuleConfig` with a
//! project field — analyzer concerns and AI-context concerns are
//! separate, and mixing them would force `RuleConfig::from_toml` to
//! validate fields it doesn't care about. Instead we re-parse the TOML
//! through `toml::Value` and pluck the `[project]` subtable.

use std::path::Path;

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

/// Persist `theme` + `goal` into `<root>/tidycraft.toml`'s `[project]`
/// section. Uses `toml_edit` so every comment, blank line, and other
/// section the user wrote is preserved on round-trip — the plain
/// `toml` crate would lose them.
///
/// Behavior matrix:
/// - File missing → creates it from `DEFAULT_CONFIG_TEMPLATE` (same
///   path `ensure_project_config` takes), then patches `[project]`
///   in place. Keeps the user's first-time experience consistent
///   regardless of which UI surface triggered the create.
/// - File present, no `[project]` table → appends a fresh `[project]`
///   table at the end of the document. TOML semantics are independent
///   of table order, so position doesn't matter.
/// - File present with `[project]` table → patches `theme` and `goal`
///   in place. Any extra keys the user added under `[project]` are
///   left untouched.
///
/// Empty strings are written as `theme = ""` / `goal = ""` rather
/// than dropping the keys, mirroring the template shape.
/// `ProjectMeta::from_toml` normalizes them back to `None` on read,
/// so the prompt builder still skips the project-context block when
/// both fields are empty.
pub fn write_back(project_root: &Path, theme: &str, goal: &str) -> Result<(), String> {
    let toml_path = project_root.join("tidycraft.toml");

    // Bootstrap from the analyzer-config template when the file is
    // absent — same path `ensure_project_config` takes, so users
    // creating their first tidycraft.toml via the LearnSetupModal
    // get the full annotated rule scaffold rather than a bare
    // [project] table.
    if !toml_path.exists() {
        std::fs::write(
            &toml_path,
            crate::analyzer::rules::config_template::DEFAULT_CONFIG_TEMPLATE,
        )
        .map_err(|e| format!("Failed to create tidycraft.toml: {e}"))?;
    }

    let content = std::fs::read_to_string(&toml_path)
        .map_err(|e| format!("Failed to read tidycraft.toml: {e}"))?;
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("Failed to parse tidycraft.toml: {e}"))?;

    if !doc.contains_key("project") {
        // Without `set_implicit(false)` toml_edit may render the table
        // implicitly (only as a header for nested keys); we want the
        // canonical `[project]` header line in the file.
        let mut t = toml_edit::Table::new();
        t.set_implicit(false);
        doc.insert("project", toml_edit::Item::Table(t));
    }

    let project_table = doc["project"]
        .as_table_mut()
        .ok_or_else(|| "[project] is not a table".to_string())?;
    project_table["theme"] = toml_edit::value(theme);
    project_table["goal"] = toml_edit::value(goal);

    std::fs::write(&toml_path, doc.to_string())
        .map_err(|e| format!("Failed to write tidycraft.toml: {e}"))?;
    Ok(())
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

    // ============ write_back round-trip tests ============

    use tempfile::tempdir;

    #[test]
    fn write_back_creates_file_from_template_when_absent() {
        let dir = tempdir().unwrap();
        write_back(dir.path(), "Cyberpunk RPG", "Asset library").unwrap();
        let path = dir.path().join("tidycraft.toml");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        // Bootstrap brought in the full analyzer-config template.
        assert!(content.contains("[naming]"));
        // [project] block patched with our values.
        let meta = ProjectMeta::from_toml(&content).unwrap();
        assert_eq!(meta.theme.as_deref(), Some("Cyberpunk RPG"));
        assert_eq!(meta.goal.as_deref(), Some("Asset library"));
    }

    #[test]
    fn write_back_preserves_comments_in_existing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tidycraft.toml");
        // User-authored config with inline comments — toml_edit must
        // round-trip them or we silently destroy user intent.
        let original = "# Top-level header comment\n\
                        # Another line.\n\
                        \n\
                        [project]\n\
                        theme = \"Old theme\"  # inline comment\n\
                        goal = \"Old goal\"\n\
                        \n\
                        [naming]\n\
                        # Loosened for prototype phase\n\
                        enabled = true\n\
                        max_length = 256\n";
        std::fs::write(&path, original).unwrap();
        write_back(dir.path(), "New theme", "New goal").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        // Header comments preserved.
        assert!(content.contains("# Top-level header comment"));
        assert!(content.contains("# Another line."));
        // [naming] section + its inline comment preserved.
        assert!(content.contains("# Loosened for prototype phase"));
        assert!(content.contains("max_length = 256"));
        // [project] values updated.
        let meta = ProjectMeta::from_toml(&content).unwrap();
        assert_eq!(meta.theme.as_deref(), Some("New theme"));
        assert_eq!(meta.goal.as_deref(), Some("New goal"));
    }

    #[test]
    fn write_back_appends_project_table_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tidycraft.toml");
        // Existing config without a [project] section — represents
        // pre-AI-Tagging projects upgraded to a build that has the
        // feature.
        let original = "# user comment\n\
                        \n\
                        [naming]\n\
                        enabled = true\n\
                        max_length = 96\n";
        std::fs::write(&path, original).unwrap();
        write_back(dir.path(), "New theme", "New goal").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        // Existing content untouched.
        assert!(content.contains("# user comment"));
        assert!(content.contains("[naming]"));
        assert!(content.contains("max_length = 96"));
        // New [project] table appended.
        let meta = ProjectMeta::from_toml(&content).unwrap();
        assert_eq!(meta.theme.as_deref(), Some("New theme"));
        assert_eq!(meta.goal.as_deref(), Some("New goal"));
    }

    #[test]
    fn write_back_preserves_extra_keys_under_project() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tidycraft.toml");
        let original = "[project]\n\
                        theme = \"Old\"\n\
                        goal = \"Old\"\n\
                        custom_field = \"user-added\"\n";
        std::fs::write(&path, original).unwrap();
        write_back(dir.path(), "New", "New").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        // Custom field survives — we only touch theme + goal.
        assert!(content.contains("custom_field = \"user-added\""));
    }

    #[test]
    fn write_back_empty_strings_normalize_to_none_on_read() {
        let dir = tempdir().unwrap();
        // User cleared both fields — keys still get written
        // (`theme = ""`) per template shape, but `from_toml`
        // normalizes empty back to None so the prompt builder
        // skips the project-context block.
        write_back(dir.path(), "", "").unwrap();
        let content =
            std::fs::read_to_string(dir.path().join("tidycraft.toml")).unwrap();
        let meta = ProjectMeta::from_toml(&content).unwrap();
        assert!(meta.is_empty());
    }
}
