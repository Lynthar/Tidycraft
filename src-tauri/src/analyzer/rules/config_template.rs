/// Default `tidycraft.toml` written into a project root the first time the
/// user opens the rules editor and no config exists yet. Every field is
/// listed with its default value so users can see the full surface; comments
/// flag the toggles most likely to need adjustment.
///
/// Keep this in sync with the field defaults declared in each rule's
/// `default_*` function — there's no compile-time check, so reviewers must
/// eyeball both. Drift here only affects the welcome template; runtime
/// defaults still come from `RuleConfig::default()` regardless.
// `r##"..."##` (two hashes) is needed because the template body contains
// the literal sequence `"#"` (e.g. inside the forbidden_chars example),
// which would otherwise close a single-hash raw string early.
pub const DEFAULT_CONFIG_TEMPLATE: &str = r##"# Tidycraft analysis rules.
# Edit and save — Run Analysis re-reads this file on each click; no rescan needed.
# Delete this file to fall back to built-in defaults.
# See docs/analyzer-rules.md for what each rule does and when to relax it.
#
# OUT-OF-BOX DEFAULTS ARE DELIBERATELY MINIMAL.
# Only naming.forbidden_chars + texture.color_space + duplicate (always-on) +
# missing_reference (Unity-only, always-on) fire by default. Every other
# section below ships with `enabled = false`; flip them to `true` to opt in.

# ─── Project metadata ─── (consumed by AI Learning)
# Optional. Tidycraft's AI Tagging feature reads `theme` and `goal` here so
# the model knows what kind of project this is when it suggests tags. Leave
# blank to skip the project-context block in the prompt — the AI still works,
# just with less project-specific framing.
[project]
# Free-form, e.g. "Cyberpunk top-down RPG" or "Photorealistic FPS military set".
theme = ""
# Free-form, e.g. "Asset library for player characters, vehicles, props".
goal = ""

# ─── Naming Convention ─── (applies to all assets)
# DEFAULT: enabled. The `forbidden_chars` check below catches shell-unsafe /
# Windows-illegal characters — that's a real bug, not a stylistic convention,
# so it stays on. The other sub-rules are loosened so default behavior
# produces almost no false positives.
[naming]
enabled = true
# Forbidden characters in filenames. Default catches shell-unsafe punctuation;
# add `<`, `>`, `:`, `"`, `|`, `?`, `*`, `/`, `\` if you also want every
# Windows-illegal character flagged.
# forbidden_chars = [' ', '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '+', '=']
# Set true to forbid CJK characters in filenames. Default false (many teams
# legitimately ship localized content).
forbid_chinese = false
# Generous default. Tighten to 64–96 for strict pipelines (UE, deep nesting).
max_length = 512
# Optional per-type prefix. Uncomment + set to e.g. "T_" to force textures
# to be named "T_*". Default: no prefix required.
# texture_prefix = "T_"
# model_prefix = "SM_"
# audio_prefix = "A_"
# Case style: "any" / "PascalCase" / "snake_case" / "camelCase".
case_style = "any"

# ─── Texture Standards ─── (applies to image assets)
# DEFAULT: disabled. PoT / max-size / file-size are pipeline-specific
# budgets — opt in by flipping `enabled` to true.
[texture]
enabled = false
# Power-of-two dimensions. UI / icon textures and HDRIs often need this off.
require_pot = true
# Maximum width or height in pixels. Hero assets / cinematic textures
# may justify raising to 8192.
max_size = 4096
# Minimum width or height. Below this triggers an info-severity issue.
min_size = 4
# Warn when not square. Most texture pipelines accept rectangular.
warn_non_square = false
# Maximum file size in bytes. 10 MB default; raise for cutscene / hero
# assets, lower for mobile-targeted projects.
max_file_size = 10485760

# ─── Texture Color Space ─── (applies to image assets)
# DEFAULT: enabled. Catches a real corruption bug — engine de-gammas
# sRGB-flagged data textures (normal / roughness / metallic / AO).
# Lives under its own section now so disabling the [texture] checks
# above doesn't also turn off this safety net.
[texture.color_space]
enabled = true

# ─── Model Standards ─── (applies to 3D model assets)
# DEFAULT: disabled. Vertex / face / material limits are per-project
# budgets — opt in by flipping `enabled` to true.
[model]
enabled = false
max_vertices = 100000
max_faces = 100000
max_materials = 10

# ─── Audio Standards ─── (applies to audio assets)
# DEFAULT: disabled. Sample rate / duration / mono limits are
# pipeline-specific — opt in by flipping `enabled` to true.
[audio]
enabled = false
allowed_sample_rates = [44100, 48000]
# Seconds; only enforced on files whose name suggests SFX
# (sfx / sound / effect / hit / click / ui). Music / VO are exempt.
max_sfx_duration = 30.0
max_file_size = 20971520         # 20 MB
prefer_mono_for_sfx = false

# ─── PBR Set Completeness ─── (cross-asset: groups textures by directory + base name)
# DEFAULT: disabled. Opinionated about which channels make a "complete"
# PBR material; off out-of-box because not every project uses PBR
# naming. Opt in by flipping `enabled` to true. The check fires only
# when the trigger channel is present in a directory's texture group,
# so directories of UI / particle textures stay quiet.
[pbr_set]
enabled = false
trigger = "basecolor"
required = ["basecolor", "normal"]

# Channel role → suffix list (case-insensitive, strict last-`_`-segment match).
[pbr_set.channels]
basecolor = ["BaseColor", "Albedo", "Diffuse", "Color"]
normal    = ["Normal", "Norm"]
roughness = ["Roughness", "Rough"]
metallic  = ["Metallic", "Metal"]
ao        = ["AO", "AmbientOcclusion"]
emissive  = ["Emissive", "Emission"]
height    = ["Height", "Disp"]

# Packed-channel suffixes that satisfy multiple roles at once. _ORM
# satisfies AO + Roughness + Metallic in one image.
[pbr_set.packed]
ORM = ["ao", "roughness", "metallic"]
MRA = ["metallic", "roughness", "ao"]
RMA = ["roughness", "metallic", "ao"]

# ─── Ignore Patterns ─── (skip matched assets entirely)
# Globs matched against asset paths RELATIVE to project root.
# Useful for vendored packages, legacy folders, or generated artifacts.
[ignore]
patterns = [
    # "ThirdParty/**",
    # "Plugins/**",
    # "Library/**",              # Unity generated artifacts
    # "Intermediate/**",         # Unreal build cache
    # "**/_legacy/**",
]
"##;
