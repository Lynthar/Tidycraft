//! Baseline: the accepted set of existing findings, so CI goes red only on
//! what a change introduces. Stored as readable JSON (`tidycraft.baseline.json`)
//! sorted by (rule, key), and meant to be committed alongside the project.

use crate::CliError;
use std::collections::HashMap;
use std::path::Path;
use tidycraft_core::analyzer::Issue;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Baseline {
    pub schema_version: u32,
    pub issues: Vec<Entry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub rule: String,
    pub key: String,
    /// `duplicate` only: member count of the content group. More members than
    /// the baseline accepted → the finding comes back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<usize>,
}

/// An issue's baseline identity, per rule family: content hash (+ member
/// count) for `duplicate`, referencing file + GUID for `missing_reference`,
/// directory + set for `pbr_set`, source + export for `dcc_source`, and the
/// relative path for everything else. Path-keyed rules re-fire on rename by
/// design — the rename is exactly the change under review.
pub fn issue_key(issue: &Issue, rel: &str) -> (String, Option<usize>) {
    let id = issue.rule_id.as_str();
    if id == "duplicate" {
        let members = issue.related_paths.as_ref().map(|p| p.len());
        return match issue.args.get("hash") {
            Some(h) => (format!("sha256:{h}"), members),
            None => (rel.to_string(), members),
        };
    }
    if id == "missing_reference" {
        if let Some(guid) = issue.args.get("guid") {
            return (format!("{rel}#{guid}"), None);
        }
    }
    if id.starts_with("pbr_set") {
        if let Some(set) = issue.args.get("set") {
            let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
            return (format!("{dir}#{set}"), None);
        }
    }
    if id.starts_with("dcc_source") {
        if let Some(export) = issue.args.get("export") {
            return (format!("{rel} -> {export}"), None);
        }
    }
    (rel.to_string(), None)
}

/// # Errors
/// A present but unreadable or unparseable baseline is a usage error (exit 2)
/// — silently ignoring it would re-red every accepted finding.
pub fn load(path: &Path) -> Result<Option<Baseline>, CliError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(CliError::Config(format!(
                "cannot read baseline {}: {e}",
                path.display()
            )));
        }
    };
    let baseline: Baseline = serde_json::from_str(&raw)
        .map_err(|e| CliError::Config(format!("invalid baseline {}: {e}", path.display())))?;
    Ok(Some(baseline))
}

pub fn write(path: &Path, mut entries: Vec<Entry>) -> Result<(), CliError> {
    entries.sort_by(|a, b| (&a.rule, &a.key).cmp(&(&b.rule, &b.key)));
    let baseline = Baseline {
        schema_version: 1,
        issues: entries,
    };
    let json = serde_json::to_string_pretty(&baseline)
        .map_err(|e| CliError::Runtime(format!("serialize baseline: {e}")))?;
    std::fs::write(path, json + "\n")
        .map_err(|e| CliError::Runtime(format!("cannot write {}: {e}", path.display())))
}

pub type Index<'a> = HashMap<(&'a str, &'a str), Option<usize>>;

pub fn index(baseline: &Baseline) -> Index<'_> {
    baseline
        .issues
        .iter()
        .map(|e| ((e.rule.as_str(), e.key.as_str()), e.members))
        .collect()
}

/// Whether a current finding is covered (suppressed) by the baseline.
pub fn covers(index: &Index, rule: &str, key: &str, members: Option<usize>) -> bool {
    match index.get(&(rule, key)) {
        None => false,
        Some(base_members) => match (base_members, members) {
            // duplicate: growth past the accepted member count re-fires; a
            // rename or a removed copy does not.
            (Some(base), Some(now)) => now <= *base,
            _ => true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as Map;
    use tidycraft_core::analyzer::Severity;

    fn issue(rule: &str, args: &[(&str, &str)], related: Option<usize>) -> Issue {
        Issue {
            rule_id: rule.to_string(),
            rule_name: String::new(),
            severity: Severity::Warning,
            message: String::new(),
            asset_path: String::new(),
            suggestion: None,
            auto_fixable: false,
            related_paths: related.map(|n| vec![String::new(); n]),
            args: args
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn keys_follow_each_familys_identity() {
        let (k, m) = issue_key(&issue("naming.case", &[], None), "Assets/a.png");
        assert_eq!((k.as_str(), m), ("Assets/a.png", None));

        let (k, m) = issue_key(
            &issue("duplicate", &[("hash", "ab12")], Some(3)),
            "Assets/b.png",
        );
        assert_eq!((k.as_str(), m), ("sha256:ab12", Some(3)));

        let (k, _) = issue_key(
            &issue("missing_reference", &[("guid", "deadbeef")], None),
            "Assets/x.prefab",
        );
        assert_eq!(k, "Assets/x.prefab#deadbeef");

        let (k, _) = issue_key(
            &issue("pbr_set.incomplete", &[("set", "T_Wood")], None),
            "Assets/wood/T_Wood_BaseColor.png",
        );
        assert_eq!(k, "Assets/wood#T_Wood");

        let (k, _) = issue_key(
            &issue(
                "dcc_source.outdated_export",
                &[("export", "hero.fbx")],
                None,
            ),
            "art/hero.blend",
        );
        assert_eq!(k, "art/hero.blend -> hero.fbx");
    }

    #[test]
    fn duplicate_growth_refires_and_shrink_stays_covered() {
        let mut idx: Map<(&str, &str), Option<usize>> = Map::new();
        idx.insert(("duplicate", "sha256:ab"), Some(3));
        idx.insert(("naming.case", "Assets/a.png"), None);

        assert!(covers(&idx, "duplicate", "sha256:ab", Some(3)));
        assert!(covers(&idx, "duplicate", "sha256:ab", Some(2)));
        assert!(!covers(&idx, "duplicate", "sha256:ab", Some(4)));
        assert!(covers(&idx, "naming.case", "Assets/a.png", None));
        assert!(!covers(&idx, "naming.case", "Assets/b.png", None));
    }
}
