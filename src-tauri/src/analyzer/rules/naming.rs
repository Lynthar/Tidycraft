use crate::analyzer::{Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};
use serde::{Deserialize, Serialize};

use super::Rule;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Forbidden characters in file names
    #[serde(default = "default_forbidden_chars")]
    pub forbidden_chars: Vec<char>,

    /// Whether to forbid Chinese characters
    #[serde(default = "default_forbid_chinese")]
    pub forbid_chinese: bool,

    /// Maximum file name length
    #[serde(default = "default_max_length")]
    pub max_length: usize,

    /// Required prefix for textures
    #[serde(default)]
    pub texture_prefix: Option<String>,

    /// Required prefix for models
    #[serde(default)]
    pub model_prefix: Option<String>,

    /// Required prefix for audio
    #[serde(default)]
    pub audio_prefix: Option<String>,

    /// Naming case style: "PascalCase", "snake_case", "camelCase", or "any"
    #[serde(default = "default_case_style")]
    pub case_style: String,
}

fn default_enabled() -> bool {
    true
}

fn default_forbidden_chars() -> Vec<char> {
    vec![' ', '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '+', '=']
}

fn default_forbid_chinese() -> bool {
    // Out-of-box: don't flag Chinese characters. Many teams legitimately
    // ship non-ASCII names (localized content, learning material). Users
    // who want strict ASCII can set this to true.
    false
}

fn default_max_length() -> usize {
    // Generous out-of-box; 512 means only pathologically long names trip
    // it. Strict pipelines (UE asset references, deep nesting) can lower
    // to 64 / 96 in tidycraft.toml.
    512
}

fn default_case_style() -> String {
    "any".to_string()
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            // Stays enabled — `forbidden_chars` catches genuinely
            // shell-unsafe / Windows-illegal characters and is a real
            // bug check, not a stylistic convention. Other sub-rules
            // are loosened so default behavior produces almost no
            // issues unless a real problem exists.
            enabled: true,
            forbidden_chars: default_forbidden_chars(),
            forbid_chinese: false,
            max_length: 512,
            texture_prefix: None, // strict pipelines re-enable e.g. "T_"
            model_prefix: None,
            audio_prefix: None,
            case_style: "any".to_string(),
        }
    }
}

pub struct NamingRule {
    config: NamingConfig,
}

impl NamingRule {
    pub fn new(config: NamingConfig) -> Self {
        Self { config }
    }

    fn check_forbidden_chars(&self, name: &str) -> Option<char> {
        for c in name.chars() {
            if self.config.forbidden_chars.contains(&c) {
                return Some(c);
            }
        }
        None
    }

    fn check_chinese(&self, name: &str) -> bool {
        if !self.config.forbid_chinese {
            return false;
        }
        name.chars().any(|c| {
            let code = c as u32;
            // CJK Unified Ideographs range
            (0x4E00..=0x9FFF).contains(&code)
                || (0x3400..=0x4DBF).contains(&code)
                || (0x20000..=0x2A6DF).contains(&code)
        })
    }

    fn check_prefix(&self, name: &str, asset_type: &AssetType) -> Option<String> {
        let required_prefix = match asset_type {
            AssetType::Texture => self.config.texture_prefix.as_ref(),
            AssetType::Model => self.config.model_prefix.as_ref(),
            AssetType::Audio => self.config.audio_prefix.as_ref(),
            _ => None,
        };

        if let Some(prefix) = required_prefix {
            if !name.starts_with(prefix) {
                return Some(prefix.clone());
            }
        }
        None
    }

    fn check_case_style(&self, name: &str) -> bool {
        match self.config.case_style.as_str() {
            "PascalCase" => is_pascal_case(name),
            "snake_case" => is_snake_case(name),
            "camelCase" => is_camel_case(name),
            // Documented in the sample config since day one, but the branch
            // was missing — a `case_style = "kebab-case"` silently behaved
            // like "any".
            "kebab-case" => is_kebab_case(name),
            _ => true, // "any" or unknown
        }
    }

    /// Generate a compliant filename for the single naming violation this rule
    /// WOULD report for `asset`, following `check`'s exact priority order but
    /// acting only on the three auto-fixable kinds: forbidden characters,
    /// missing prefix, and case style. Returns `None` when the violation that
    /// fires isn't auto-fixable (length / Chinese), when the asset is already
    /// compliant, or when no safe fix exists.
    ///
    /// Invariant: any returned name is re-checked against the very predicate
    /// that flagged the asset, so Fix-it never proposes a rename that leaves
    /// the same issue standing (which would just re-fire on the next scan).
    /// That is what makes odd inputs fall through to `None` instead of a bogus
    /// rename — a stem of only forbidden characters, a name with `.` / `~` that
    /// no case style accepts, or a config that forbids the separator itself.
    pub fn suggest_compliant_name(&self, asset: &AssetInfo) -> Option<String> {
        let name = &asset.name;
        // Same stem/extension split the case check uses (rsplit_once on '.'),
        // so a dotless name keeps its whole self as the stem.
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s, Some(e)),
            None => (name.as_str(), None),
        };

        // Length is the highest-priority violation and is NOT auto-fixable:
        // when the name is over the limit that's what `check` reports, so there
        // is no rename to offer.
        if name.chars().count() > self.config.max_length {
            return None;
        }

        // Forbidden characters (applies to DCC sources too, like `check`).
        if self.check_forbidden_chars(name).is_some() {
            let fixed_stem = self.fix_forbidden_chars(stem)?;
            let candidate = reattach_ext(&fixed_stem, ext);
            return (candidate != *name && self.check_forbidden_chars(&candidate).is_none())
                .then_some(candidate);
        }

        // Chinese characters — reported but not auto-fixable.
        if self.check_chinese(name) {
            return None;
        }

        // Missing prefix — DCC authoring sources are exempt (same as `check`).
        let is_dcc_source = asset
            .metadata
            .as_ref()
            .and_then(|m| m.dcc_source_kind.as_ref())
            .is_some();
        if !is_dcc_source {
            if let Some(prefix) = self.check_prefix(name, &asset.asset_type) {
                let candidate = format!("{}{}", prefix, name);
                return (candidate != *name && candidate.starts_with(&prefix)).then_some(candidate);
            }
        }

        // Case style.
        if !self.check_case_style(stem) {
            let fixed_stem = to_case_style(stem, &self.config.case_style)?;
            let candidate = reattach_ext(&fixed_stem, ext);
            return (candidate != *name && self.check_case_style(&fixed_stem)).then_some(candidate);
        }

        None
    }

    /// Rewrite `stem` so it carries no forbidden character: replace each with a
    /// separator, collapse runs of that separator, and trim it from the ends.
    /// The separator is `_`, unless the config forbids `_` (then `-`, then — if
    /// that is forbidden too — outright removal), so the fix can never
    /// reintroduce a forbidden character. Returns `None` if nothing survives
    /// (e.g. a stem of only forbidden characters).
    fn fix_forbidden_chars(&self, stem: &str) -> Option<String> {
        let sep = if !self.config.forbidden_chars.contains(&'_') {
            Some('_')
        } else if !self.config.forbidden_chars.contains(&'-') {
            Some('-')
        } else {
            None
        };

        let mut replaced = String::with_capacity(stem.len());
        for c in stem.chars() {
            if self.config.forbidden_chars.contains(&c) {
                if let Some(s) = sep {
                    replaced.push(s);
                }
            } else {
                replaced.push(c);
            }
        }

        let cleaned = match sep {
            Some(s) => collapse_and_trim(&replaced, s),
            None => replaced,
        };
        (!cleaned.is_empty()).then_some(cleaned)
    }
}

impl Rule for NamingRule {
    fn id(&self) -> &str {
        "naming"
    }

    fn name(&self) -> &str {
        "Naming Convention"
    }

    fn applies_to(&self, _asset: &AssetInfo) -> bool {
        true // Applies to all assets
    }

    fn check(&self, asset: &AssetInfo) -> Option<Issue> {
        let name = &asset.name;
        let name_without_ext = name.rsplit_once('.').map(|(n, _)| n).unwrap_or(name);

        // Check length in CHARACTERS — `len()` counts bytes, which triples
        // the tally for CJK names (a 40-character Chinese filename read as
        // 120 and false-tripped the limit).
        let char_count = name.chars().count();
        if char_count > self.config.max_length {
            return Some(Issue {
                rule_id: "naming.length".to_string(),
                rule_name: "Name Too Long".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "File name is {} characters, max allowed is {}",
                    char_count,
                    self.config.max_length
                ),
                asset_path: asset.path.clone(),
                suggestion: Some(format!("Shorten the file name to {} characters", self.config.max_length)),
                auto_fixable: false,
            related_paths: None,
            });
        }

        // Check forbidden characters
        if let Some(c) = self.check_forbidden_chars(name) {
            return Some(Issue {
                rule_id: "naming.forbidden_char".to_string(),
                rule_name: "Forbidden Character".to_string(),
                severity: Severity::Warning,
                message: format!("File name contains forbidden character: '{}'", c),
                asset_path: asset.path.clone(),
                suggestion: Some(format!("Remove '{}' from the file name", c)),
                auto_fixable: true,
            related_paths: None,
            });
        }

        // Check Chinese characters
        if self.check_chinese(name) {
            return Some(Issue {
                rule_id: "naming.chinese".to_string(),
                rule_name: "Chinese Characters".to_string(),
                severity: Severity::Warning,
                message: "File name contains Chinese characters".to_string(),
                asset_path: asset.path.clone(),
                suggestion: Some("Use English characters for file names".to_string()),
                auto_fixable: false,
            related_paths: None,
            });
        }

        // Check prefix. DCC authoring sources (`.blend` / `.psd` / `.spp` /
        // ...) are exempt: type-prefix conventions (`SM_` / `T_`) target the
        // engine-runtime exports, while source files follow the DCC's own
        // naming habits — flagging every `.blend` for a missing `SM_` is
        // noise, not a finding (UX audit P2-9). The other naming checks
        // (forbidden chars / length / case) still apply to sources.
        let is_dcc_source = asset
            .metadata
            .as_ref()
            .and_then(|m| m.dcc_source_kind.as_ref())
            .is_some();
        if !is_dcc_source {
            if let Some(prefix) = self.check_prefix(name, &asset.asset_type) {
                return Some(Issue {
                    rule_id: "naming.prefix".to_string(),
                    rule_name: "Missing Prefix".to_string(),
                    severity: Severity::Warning,
                    message: format!("File name should start with '{}'", prefix),
                    asset_path: asset.path.clone(),
                    suggestion: Some(format!("Rename to {}{}", prefix, name)),
                    auto_fixable: true,
                    related_paths: None,
                });
            }
        }

        // Check case style
        if !self.check_case_style(name_without_ext) {
            return Some(Issue {
                rule_id: "naming.case".to_string(),
                rule_name: "Naming Case".to_string(),
                severity: Severity::Info,
                message: format!(
                    "File name does not follow {} convention",
                    self.config.case_style
                ),
                asset_path: asset.path.clone(),
                suggestion: Some(format!("Use {} for file names", self.config.case_style)),
                auto_fixable: true,
            related_paths: None,
            });
        }

        None
    }
}

fn is_pascal_case(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let first = s.chars().next().unwrap();
    first.is_uppercase() && !s.contains('_') && !s.chars().all(|c| c.is_uppercase())
}

fn is_snake_case(s: &str) -> bool {
    s.chars().all(|c| c.is_lowercase() || c.is_numeric() || c == '_')
}

fn is_camel_case(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let first = s.chars().next().unwrap();
    first.is_lowercase() && !s.contains('_')
}

fn is_kebab_case(s: &str) -> bool {
    // Same leniency level as is_snake_case, with `-` as the separator.
    s.chars().all(|c| c.is_lowercase() || c.is_numeric() || c == '-')
}

/// Reattach an extension (without its dot) to a rewritten stem. Mirrors the
/// `rsplit_once('.')` split used to produce the stem.
fn reattach_ext(stem: &str, ext: Option<&str>) -> String {
    match ext {
        Some(e) => format!("{}.{}", stem, e),
        None => stem.to_string(),
    }
}

/// Collapse consecutive runs of `sep` into one and trim it from both ends.
fn collapse_and_trim(s: &str, sep: char) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == sep && out.chars().last() == Some(sep) {
            continue;
        }
        out.push(c);
    }
    out.trim_matches(sep).to_string()
}

/// Split a stem into words for case conversion: break on `_`, `-`, space, and
/// camelCase / PascalCase humps (a lowercase or digit followed by an uppercase
/// letter). Empty fragments are dropped. Deliberately does NOT split on `.` —
/// a dotted stem (`archive.tar`) has no clean case form, and the caller's
/// re-check turns that into `None` rather than a bogus rename. Acronym
/// boundaries (`XMLParser`) are best-effort; the goal is a name that passes the
/// case predicate, not perfect word segmentation.
fn tokenize_words(stem: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev: Option<char> = None;
    for c in stem.chars() {
        if c == '_' || c == '-' || c == ' ' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            if c.is_uppercase() && (p.is_lowercase() || p.is_numeric()) && !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c);
        prev = Some(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// First character uppercase, the rest lowercase — lowercasing the tail keeps
/// an all-caps input (`ID`) from producing an all-uppercase word, which
/// `is_pascal_case` rejects.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
    }
}

/// Rebuild `stem` in the requested case style. `None` for `any` / unknown
/// styles (no canonical form) or when the stem tokenizes to nothing. The result
/// is only a proposal — the caller re-checks it against the rule's own case
/// predicate and discards anything that still doesn't pass.
fn to_case_style(stem: &str, style: &str) -> Option<String> {
    let words = tokenize_words(stem);
    if words.is_empty() {
        return None;
    }
    let out = match style {
        "snake_case" => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("_"),
        "kebab-case" => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("-"),
        "PascalCase" => words.iter().map(|w| capitalize(w)).collect::<String>(),
        "camelCase" => words
            .iter()
            .enumerate()
            .map(|(i, w)| if i == 0 { w.to_lowercase() } else { capitalize(w) })
            .collect::<String>(),
        _ => return None, // "any" or unknown — no canonical form
    };
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::AssetMetadata;

    fn asset(name: &str, ext: &str, asset_type: AssetType, dcc_kind: Option<&str>) -> AssetInfo {
        AssetInfo {
            path: format!("/proj/{}", name),
            name: name.to_string(),
            extension: ext.to_string(),
            asset_type,
            size: 1,
            modified: 0,
            metadata: dcc_kind.map(|kind| AssetMetadata {
                dcc_source_kind: Some(kind.to_string()),
                ..Default::default()
            }),
            unity_guid: None,
        }
    }

    fn prefix_rule() -> NamingRule {
        NamingRule::new(NamingConfig {
            model_prefix: Some("SM_".to_string()),
            texture_prefix: Some("T_".to_string()),
            ..Default::default()
        })
    }

    #[test]
    fn prefix_check_fires_for_runtime_assets() {
        let rule = prefix_rule();
        let issue = rule
            .check(&asset("rock.fbx", "fbx", AssetType::Model, None))
            .expect("unprefixed runtime model should be flagged");
        assert_eq!(issue.rule_id, "naming.prefix");
    }

    #[test]
    fn prefix_check_skips_dcc_sources() {
        let rule = prefix_rule();
        // .blend is AssetType::Model and .psd is AssetType::Texture, but both
        // carry dcc_source_kind — the prefix convention must not apply.
        assert!(rule
            .check(&asset("rock.blend", "blend", AssetType::Model, Some("blender")))
            .is_none());
        assert!(rule
            .check(&asset("rock.psd", "psd", AssetType::Texture, Some("photoshop")))
            .is_none());
    }

    #[test]
    fn other_naming_checks_still_apply_to_dcc_sources() {
        // The exemption is prefix-only: a source file with a forbidden
        // character is still a real finding.
        let rule = prefix_rule();
        let issue = rule
            .check(&asset(
                "my rock.blend",
                "blend",
                AssetType::Model,
                Some("blender"),
            ))
            .expect("forbidden space should still be flagged on a source file");
        assert_eq!(issue.rule_id, "naming.forbidden_char");
    }

    // ---- suggest_compliant_name (Fix-it name generator) ----

    fn default_rule() -> NamingRule {
        NamingRule::new(NamingConfig::default())
    }

    fn cased_rule(style: &str) -> NamingRule {
        NamingRule::new(NamingConfig {
            case_style: style.to_string(),
            ..Default::default()
        })
    }

    #[test]
    fn fix_forbidden_replaces_spaces_with_underscores() {
        assert_eq!(
            default_rule().suggest_compliant_name(&asset("my file.png", "png", AssetType::Texture, None)),
            Some("my_file.png".to_string())
        );
    }

    #[test]
    fn fix_forbidden_collapses_runs_and_trims_ends() {
        let rule = default_rule();
        // Two spaces collapse to a single separator.
        assert_eq!(
            rule.suggest_compliant_name(&asset("a  b.png", "png", AssetType::Texture, None)),
            Some("a_b.png".to_string())
        );
        // Space + '&' + space collapse to one; "Rock & Roll" -> "Rock_Roll".
        assert_eq!(
            rule.suggest_compliant_name(&asset("Rock & Roll.png", "png", AssetType::Texture, None)),
            Some("Rock_Roll.png".to_string())
        );
        // Leading / trailing forbidden characters are trimmed off.
        assert_eq!(
            rule.suggest_compliant_name(&asset("+foo+.png", "png", AssetType::Texture, None)),
            Some("foo.png".to_string())
        );
    }

    #[test]
    fn fix_forbidden_applies_to_dcc_sources_too() {
        // Prefix is the only source-exempt check; a forbidden space in a .psd
        // is still fixed.
        assert_eq!(
            prefix_rule().suggest_compliant_name(&asset(
                "my rock.psd",
                "psd",
                AssetType::Texture,
                Some("photoshop")
            )),
            Some("my_rock.psd".to_string())
        );
    }

    #[test]
    fn fix_forbidden_returns_none_for_degenerate_stem() {
        // "***" is all forbidden characters -> nothing survives -> no fix.
        assert_eq!(
            default_rule().suggest_compliant_name(&asset("***.png", "png", AssetType::Texture, None)),
            None
        );
    }

    #[test]
    fn fix_forbidden_falls_back_when_separator_is_forbidden() {
        // '_' forbidden -> use '-' instead.
        let mut chars = default_forbidden_chars();
        chars.push('_');
        let dash = NamingRule::new(NamingConfig {
            forbidden_chars: chars,
            ..Default::default()
        });
        assert_eq!(
            dash.suggest_compliant_name(&asset("my file.png", "png", AssetType::Texture, None)),
            Some("my-file.png".to_string())
        );
        // Both '_' and '-' forbidden -> remove the offending characters.
        let mut chars = default_forbidden_chars();
        chars.push('_');
        chars.push('-');
        let strip = NamingRule::new(NamingConfig {
            forbidden_chars: chars,
            ..Default::default()
        });
        assert_eq!(
            strip.suggest_compliant_name(&asset("my file.png", "png", AssetType::Texture, None)),
            Some("myfile.png".to_string())
        );
    }

    #[test]
    fn fix_prefix_prepends_for_runtime_assets() {
        let rule = prefix_rule();
        assert_eq!(
            rule.suggest_compliant_name(&asset("rock.fbx", "fbx", AssetType::Model, None)),
            Some("SM_rock.fbx".to_string())
        );
        assert_eq!(
            rule.suggest_compliant_name(&asset("albedo.png", "png", AssetType::Texture, None)),
            Some("T_albedo.png".to_string())
        );
    }

    #[test]
    fn fix_prefix_skips_dcc_sources() {
        assert_eq!(
            prefix_rule().suggest_compliant_name(&asset(
                "rock.blend",
                "blend",
                AssetType::Model,
                Some("blender")
            )),
            None
        );
    }

    #[test]
    fn fix_case_converts_to_each_style() {
        assert_eq!(
            cased_rule("snake_case").suggest_compliant_name(&asset(
                "MyTexture.png",
                "png",
                AssetType::Texture,
                None
            )),
            Some("my_texture.png".to_string())
        );
        assert_eq!(
            cased_rule("PascalCase").suggest_compliant_name(&asset(
                "my_texture.png",
                "png",
                AssetType::Texture,
                None
            )),
            Some("MyTexture.png".to_string())
        );
        assert_eq!(
            cased_rule("camelCase").suggest_compliant_name(&asset(
                "My_Texture.png",
                "png",
                AssetType::Texture,
                None
            )),
            Some("myTexture.png".to_string())
        );
        assert_eq!(
            cased_rule("kebab-case").suggest_compliant_name(&asset(
                "MyTexture.png",
                "png",
                AssetType::Texture,
                None
            )),
            Some("my-texture.png".to_string())
        );
    }

    #[test]
    fn fix_case_returns_none_when_uncleanable() {
        // '~' survives lowercasing and no case style accepts it: proposing a
        // rename that still fails the case check would loop forever.
        assert_eq!(
            cased_rule("snake_case").suggest_compliant_name(&asset(
                "foo~bar.png",
                "png",
                AssetType::Texture,
                None
            )),
            None
        );
    }

    #[test]
    fn fix_targets_only_the_reported_violation() {
        // Both a forbidden space and a case problem are present; the rule
        // reports forbidden first, so the fix addresses only that. The leftover
        // case issue re-surfaces on the next scan (by design).
        let rule = NamingRule::new(NamingConfig {
            case_style: "snake_case".to_string(),
            ..Default::default()
        });
        assert_eq!(
            rule.suggest_compliant_name(&asset("my Texture.png", "png", AssetType::Texture, None)),
            Some("my_Texture.png".to_string())
        );
    }

    #[test]
    fn no_fix_for_length_chinese_or_already_compliant() {
        // Over-length -> reported as naming.length, not auto-fixable.
        let short = NamingRule::new(NamingConfig {
            max_length: 4,
            ..Default::default()
        });
        assert_eq!(
            short.suggest_compliant_name(&asset("toolong.png", "png", AssetType::Texture, None)),
            None
        );
        // Chinese -> not auto-fixable when forbid_chinese is on.
        let cn = NamingRule::new(NamingConfig {
            forbid_chinese: true,
            ..Default::default()
        });
        assert_eq!(
            cn.suggest_compliant_name(&asset("贴图.png", "png", AssetType::Texture, None)),
            None
        );
        // Already compliant -> nothing to propose.
        assert_eq!(
            default_rule().suggest_compliant_name(&asset("clean_name.png", "png", AssetType::Texture, None)),
            None
        );
    }
}
