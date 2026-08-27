//! Per-family rule documentation for headless consumers (`tidycraft explain`).
//! Prose distills docs/analyzer-rules.md — the section named on each entry is
//! the full reference, with every TOML key and worked examples.

/// One rule family's documentation.
pub struct RuleDoc {
    /// Family prefix (`naming`) or exact rule id (`texture.color_space`).
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    /// Section heading inside docs/analyzer-rules.md.
    pub section: &'static str,
}

pub const RULE_DOCS: &[RuleDoc] = &[
    RuleDoc {
        id: "naming",
        title: "Naming Convention",
        summary: "Checks file names against the project's conventions. Out of the box only \
                  the forbidden-characters sub-rule meaningfully fires (shell-unsafe and \
                  Windows-illegal characters); max length, forbid-Chinese, per-type prefixes \
                  and case style are opt-in via [naming] in tidycraft.toml. Prefix rules skip \
                  DCC authoring sources, case is checked past the prefix, and one asset \
                  reports only its first matching sub-rule per run.",
        section: "Naming Convention",
    },
    RuleDoc {
        id: "texture",
        title: "Texture Standards",
        summary: "Disabled by default. Checks image dimensions and weight against [texture] \
                  budgets: power-of-two, max/min size, an optional non-square warning, a max \
                  file size, and a DDS-only missing-mipmaps check for textures 512px and up. \
                  Enable when the project has agreed texture budgets.",
        section: "Texture Standards",
    },
    RuleDoc {
        id: "texture.color_space",
        title: "Texture Color Space",
        summary: "Enabled by default — it catches real corruption, not a style preference. \
                  Fires only when both signals agree: the PNG's color profile says sRGB, and \
                  the filename stem ends with a data-channel suffix (_normal, _rough, _metal, \
                  _ao, _orm, ...). An engine would de-gamma such a texture at import and \
                  silently corrupt the data. Fix by re-exporting as Linear or renaming.",
        section: "Texture Color Space",
    },
    RuleDoc {
        id: "model",
        title: "Model Standards",
        summary: "Disabled by default. Checks 3D models against [model] budgets: max \
                  vertices, faces and materials. Vertex counts follow tobj's single-index \
                  semantics — unique (position, uv, normal) triples, not raw `v` lines — so \
                  they can exceed what a DCC tool displays.",
        section: "Model Standards",
    },
    RuleDoc {
        id: "audio",
        title: "Audio Standards",
        summary: "Disabled by default. Checks audio files against [audio] budgets: allowed \
                  sample rates, SFX duration, mono-for-SFX, and a max file size. SFX \
                  detection is heuristic — the duration and mono checks only fire when the \
                  filename contains sfx, sound, effect, hit, click or ui; music and \
                  voice-over are exempt regardless of length.",
        section: "Audio Standards",
    },
    RuleDoc {
        id: "duplicate",
        title: "Duplicate Detection",
        summary: "Always on, no configuration. Files are grouped by size, then by a hash of \
                  their first 8 KB, and only survivors are fully SHA256-hashed — so a large \
                  library stays cheap. Each content group produces one warning listing every \
                  member via related_paths, anchored on the first redundant copy. Suppress \
                  deliberate copies with [ignore].patterns.",
        section: "Duplicate Detection",
    },
    RuleDoc {
        id: "missing_reference",
        title: "Missing References",
        summary: "Always on for Unity projects. Every .prefab, .unity, .mat, .controller and \
                  .asset is parsed for guid: references that resolve to no scanned .meta \
                  file. The all-zero GUID and Unity's built-in bundles are exempt. Reported \
                  as a warning, not an error: gitignored Library/ or Packages/ may hold GUIDs \
                  the scan cannot see, so a miss is strong signal rather than proof.",
        section: "Missing References",
    },
    RuleDoc {
        id: "pbr_set",
        title: "PBR Set Completeness",
        summary: "Disabled by default. Textures sharing a directory and base stem form a \
                  set; a set is flagged when the [pbr_set].required channels are not all \
                  present. A set only forms when the trigger channel (default basecolor) is \
                  there, so non-PBR folders stay quiet. Packed maps (_ORM, _MRA, _RMA) \
                  satisfy all their listed roles. Note: supplying [pbr_set.channels] replaces \
                  the entire default table — TOML tables do not merge.",
        section: "PBR Set Completeness",
    },
    RuleDoc {
        id: "dcc_source",
        title: "DCC Source-File Linking",
        summary: "Disabled by default. Pairs authoring sources (.blend, .psd, .spp, ...) \
                  with same-stem runtime exports — in the same directory or in sibling-named \
                  directories, searching upward only — and warns when the source's mtime is \
                  newer than the export's by more than the configured tolerance: the \
                  \"edited locally, forgot to re-export\" loop. Cross-commit staleness is \
                  invisible by design, because git checkout synchronizes mtimes.",
        section: "DCC Source-File Linking",
    },
];

/// Resolve a rule id to its documentation: exact id first, then the longest
/// family prefix followed by a dot (`texture.max_size` → `texture`, while
/// `texture.color_space` keeps its own entry).
pub fn rule_doc(rule_id: &str) -> Option<&'static RuleDoc> {
    if let Some(doc) = RULE_DOCS.iter().find(|d| d.id == rule_id) {
        return Some(doc);
    }
    RULE_DOCS
        .iter()
        .filter(|d| {
            rule_id
                .strip_prefix(d.id)
                .is_some_and(|rest| rest.starts_with('.'))
        })
        .max_by_key(|d| d.id.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every rule id the analyzer can emit must resolve to a doc — `explain`
    /// answering "unknown rule" for a real finding is a broken contract.
    #[test]
    fn every_declared_rule_id_resolves_to_a_doc() {
        for (id, _) in crate::analyzer::RULE_ARGS {
            assert!(rule_doc(id).is_some(), "no rule doc covers `{id}`");
        }
    }

    #[test]
    fn exact_entry_wins_over_family_prefix() {
        assert_eq!(
            rule_doc("texture.color_space").unwrap().id,
            "texture.color_space"
        );
        assert_eq!(rule_doc("texture.max_size").unwrap().id, "texture");
    }

    #[test]
    fn unknown_and_lookalike_ids_resolve_to_none() {
        assert!(rule_doc("nonsense").is_none());
        // Prefix must be followed by a dot: `texturefoo` is not `texture.*`.
        assert!(rule_doc("texturefoo").is_none());
    }
}
