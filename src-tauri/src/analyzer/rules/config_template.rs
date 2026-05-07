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

# ─── Naming Convention ─── (applies to all assets)
[naming]
enabled = true
forbid_chinese = true            # set false if your team writes Chinese asset names
max_length = 64
case_style = "any"               # "any" | "PascalCase" | "snake_case" | "camelCase"
texture_prefix = "T_"            # required prefix for textures; remove this line to disable
# model_prefix = "SM_"           # uncomment to require a prefix on models
# audio_prefix = "SFX_"          # uncomment to require a prefix on audio
# Default forbidden_chars covers shell-unsafe punctuation. Override if needed:
# forbidden_chars = [" ", "!", "@", "#", "$", "%", "^", "&", "*", "(", ")", "+", "="]

# ─── Texture Standards ─── (applies to image assets)
[texture]
enabled = true
require_pot = true               # power-of-two dimensions; disable for UI/LUT textures
max_size = 4096                  # px (warn if exceeded)
min_size = 4
warn_non_square = false
max_file_size = 10485760         # 10 MB

# ─── Model Standards ─── (applies to 3D model assets)
[model]
enabled = true
max_vertices = 100000
max_faces = 100000
max_materials = 10

# ─── Audio Standards ─── (applies to audio assets)
[audio]
enabled = true
allowed_sample_rates = [44100, 48000]
max_sfx_duration = 30.0          # seconds; only enforced on files whose name suggests SFX
max_file_size = 20971520         # 20 MB
prefer_mono_for_sfx = false

# ─── PBR Set Completeness ─── (cross-asset: groups textures by base name)
# A "set" is the group of textures sharing the same base stem in the same
# directory, e.g. T_Wood_BaseColor.png + T_Wood_Normal.png. The check fires
# only when the trigger channel (default: basecolor) is present, so projects
# without PBR-style naming produce no warnings even with this rule on.
[pbr_set]
enabled = true
trigger = "basecolor"            # set must contain this channel to be checked
required = ["basecolor", "normal"]   # any role missing here → Warning

# Channel role → suffix list (case-insensitive). Suffix matches the part
# after the LAST `_` in the file stem; T_brand_new isn't a "Normal" map.
[pbr_set.channels]
basecolor = ["BaseColor", "Albedo", "Diffuse", "Color"]
normal    = ["Normal", "Norm"]
roughness = ["Roughness", "Rough"]
metallic  = ["Metallic", "Metal"]
ao        = ["AO", "AmbientOcclusion"]
emissive  = ["Emissive", "Emission"]
height    = ["Height", "Disp"]

# Packed-channel suffixes that satisfy multiple roles at once. A file
# ending in `_ORM` counts as if AO + Roughness + Metallic were all present.
[pbr_set.packed]
ORM = ["ao", "roughness", "metallic"]
MRA = ["metallic", "roughness", "ao"]
RMA = ["roughness", "metallic", "ao"]

# ─── Ignore Patterns ─── (skip matched assets entirely)
[ignore]
# Globs matched against asset paths RELATIVE to project root.
# Useful for vendored packages, legacy folders, or generated artifacts.
patterns = [
    # "ThirdParty/**",
    # "Plugins/**",
    # "Library/**",              # Unity generated artifacts
    # "Intermediate/**",         # Unreal build cache
    # "**/_legacy/**",
]
"##;
