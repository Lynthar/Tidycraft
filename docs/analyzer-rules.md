# Analysis Rules

Tidycraft's **Run Analysis** is an asset-quality lint. It reads your scanned project, applies a set of opinionated rules, and produces an Issues list you can filter, group, and jump into. This document explains what each rule does, when it might bother you, and how to override it.

## How it works

Clicking **Run Analysis** (or `⌘⇧R`) runs four phases on the cached scan result:

1. **Per-asset rule checks** — five rule families (`naming`, `texture`, `texture.color_space`, `model`, `audio`) run against every asset. Each family is stateless and returns at most one issue per asset (the first sub-rule that fires).
2. **Duplicate detection** — files are grouped by size; same-size files are SHA256-hashed and any group with more than one match is reported (the first asset in a group is the "original", the rest are flagged).
3. **Missing-reference detection** (Unity only) — every `.prefab` / `.unity` / `.mat` / `.controller` / `.asset` is parsed for GUID references that don't resolve to any scanned `.meta`.
4. **PBR set completeness** — textures are grouped by directory + base stem (`T_Wood_BaseColor` + `T_Wood_Normal` are siblings); a set with the trigger channel but missing required channels is flagged.

All four phases share the same `tidycraft.toml` configuration, read from your project root each time you click Run Analysis. **No rescan is needed after editing the file** — just save and re-run.

## Rules at a glance

| Rule family | Applies to | Default severity range |
|---|---|---|
| `naming.*` | All assets | warning / info |
| `texture.*` | Image assets | warning / info |
| `texture.color_space` | Image assets | warning |
| `model.*` | 3D models | warning |
| `audio.*` | Audio files | warning / info |
| `duplicate` | All assets | warning |
| `missing_reference` | Unity prefabs / scenes / materials | error |
| `pbr_set.incomplete` | Texture groups (cross-asset) | warning |

---

## Naming Convention (`[naming]`)

| Sub-rule | Default | TOML key | When to relax |
|---|---|---|---|
| Max name length | 64 chars | `max_length` | Long descriptive names in research / academic projects |
| Forbidden characters | space, `! @ # $ % ^ & * ( ) + =` | `forbidden_chars` | Inheriting Unity Asset Store packages or third-party samples |
| Forbid Chinese characters | true | `forbid_chinese = false` | Teams that ship Chinese asset names (localized content, learning material) |
| Required prefix per type | textures `T_`; model/audio off | `texture_prefix` / `model_prefix` / `audio_prefix` | Existing project where you can't bulk-rename |
| Case style | any | `case_style` ∈ `"any" \| "PascalCase" \| "snake_case" \| "camelCase"` | Mixed-case codebases |

> **First-issue mode**: a single asset that violates several naming sub-rules will only show the first match in the order above. Fix it, re-run, the next one surfaces.

---

## Texture Standards (`[texture]`)

| Sub-rule | Default | TOML key | When to relax |
|---|---|---|---|
| Power-of-two dimensions | true | `require_pot = false` | UI / icon textures, LUTs (256×16), HDRIs |
| Maximum size | 4096 px | `max_size = 8192` | High-detail hero assets, console-only projects |
| Minimum size | 4 px | `min_size` | Stamp / brush libraries with intentionally tiny tiles |
| Square only warning | off | `warn_non_square = true` | Pipeline that requires square atlases |
| Maximum file size | 10 MB | `max_file_size` (bytes) | Cinematic / cutscene textures |
| Missing mipmaps (DDS only, ≥ 512px) | always on | n/a | Disable the whole texture rule (`enabled = false`) — there's no per-sub-rule toggle |

---

## Texture Color Space (`[texture]` — same enabled flag)

Detects PNGs flagged as **sRGB** whose filename suggests they're a data channel (normal map, roughness, metallic, AO, etc.). The engine would otherwise de-gamma those pixels at import and silently corrupt the data.

A warning fires only when **both** signals match:

1. The file's color profile chunk says `sRGB`.
2. The filename stem ends with one of: `_n`, `_normal`, `_norm`, `_nrm`, `_r`, `_rough`, `_roughness`, `_m`, `_metal`, `_metallic`, `_ao`, `_mask`, `_data`, `_lin`, `_linear`, `_height`, `_disp`, `_displacement`, `_orm`, `_mra`, `_rma`.

To suppress: rename the file (drop the suffix), re-export with Linear color space, or add the path to `[ignore].patterns`.

---

## Model Standards (`[model]`)

| Sub-rule | Default | TOML key | When to relax |
|---|---|---|---|
| Max vertices | 100,000 | `max_vertices = 500000` | Open-world chunk meshes, hero LOD0 |
| Max faces | 100,000 | `max_faces` | Same as above |
| Max materials | 10 | `max_materials` | Modular character with separate materials per part |

---

## Audio Standards (`[audio]`)

| Sub-rule | Default | TOML key | When to relax |
|---|---|---|---|
| Allowed sample rates | 44100 Hz, 48000 Hz | `allowed_sample_rates = [22050, 44100]` | Retro / chiptune projects, voice-over batches |
| SFX duration | ≤ 30s | `max_sfx_duration` | Long reverb tails, stingers |
| Force mono for SFX | off | `prefer_mono_for_sfx = true` | 3D-spatialized audio pipelines |
| Maximum file size | 20 MB | `max_file_size` (bytes) | Music / ambient tracks |

> **SFX detection is heuristic**: the duration / mono rules only fire when the filename contains `sfx`, `sound`, `effect`, `hit`, `click`, or `ui`. Music or VO files are exempt regardless of length.

---

## Duplicate Detection

No configuration. Files are grouped by size first (cheap), then SHA256-hashed within groups of 2+ to confirm true content equality. The first asset in each duplicate group is the "original"; the rest get a `duplicate` warning.

**Cannot be configured because** the check is binary (same content = duplicate). To suppress, add deliberate copies to `[ignore].patterns` or accept the warnings.

---

## Missing References (Unity only)

For Unity projects, every referenceable file (`.prefab`, `.unity`, `.mat`, `.controller`, `.asset`) is YAML-parsed for `guid:` references. Any GUID not present in the project's `.meta` files becomes a `missing_reference` error.

Skips:
- The all-zero GUID (Unity's "no reference" sentinel).
- Duplicate references to the same missing GUID within one file (reported once).

**Cannot be tuned** — the check is binary. To suppress, fix the broken reference (re-link the asset in Unity) or add the source file to `[ignore].patterns`.

---

## PBR Set Completeness (`[pbr_set]`)

Cross-asset check: textures sharing the same directory and base stem are grouped into a "set", and a set is flagged when its expected channels aren't all present. A set forms only when the **trigger channel** (default `basecolor`) is in the group, so directories of UI / particle / non-PBR textures don't produce spurious warnings.

```toml
[pbr_set]
enabled = true
trigger = "basecolor"
required = ["basecolor", "normal"]

[pbr_set.channels]
basecolor = ["BaseColor", "Albedo", "Diffuse", "Color"]
normal    = ["Normal", "Norm"]
roughness = ["Roughness", "Rough"]
metallic  = ["Metallic", "Metal"]
ao        = ["AO", "AmbientOcclusion"]
emissive  = ["Emissive", "Emission"]
height    = ["Height", "Disp"]

[pbr_set.packed]
ORM = ["ao", "roughness", "metallic"]
MRA = ["metallic", "roughness", "ao"]
RMA = ["roughness", "metallic", "ao"]
```

**Suffix matching is strict** — the suffix is the substring after the **last** `_` in the file stem, and must equal-match (case-insensitive) one of the configured suffixes. `T_brand_new.png` finds suffix `new`, doesn't match anything default, is silently ignored. This avoids treating innocuous names as misnamed maps.

**Packed channels** (e.g. `_ORM` carrying AO + Roughness + Metallic in one image) satisfy all the roles listed under `[pbr_set.packed]`. So a set with `_BaseColor` + `_Normal` + `_ORM` is considered complete even when `required` lists `roughness` and `metallic` separately.

**To relax:**
- Drop `normal` from `required` if your project ships flat-shaded materials.
- Add aliases to `[pbr_set.channels]` if your team uses a custom suffix (e.g. `basecolor = ["BaseColor", "BC"]`).
- Set `enabled = false` to turn the rule off entirely.
- Add the directory to `[ignore].patterns` to skip a known-incomplete folder.

The issue is anchored on the **trigger** texture (the BaseColor file), so clicking the issue takes you to the most recognizable member of the broken set.

---

## Ignore Patterns (`[ignore]`)

The most powerful escape hatch. Glob patterns matched against asset paths **relative to the project root**; any matching asset is dropped before any rule runs (per-asset, duplicate, and missing-reference all respect it).

```toml
[ignore]
patterns = [
    "ThirdParty/**",       # vendored code/assets you can't change
    "Plugins/**",
    "Library/**",          # Unity generated artifacts
    "Intermediate/**",     # Unreal build cache
    "**/_legacy/**",       # archive folder for old assets
    "Assets/Scenes/Test*", # files prefixed Test in any Scenes folder
]
```

Glob syntax (via the [`globset`](https://docs.rs/globset/) crate):
- `*` matches any sequence within a path component
- `**` matches across path separators (full subtree)
- `?` matches a single character
- `[abc]` matches one character from a set
- Patterns are **case-sensitive** on case-sensitive filesystems

If a pattern is malformed, Run Analysis fails fast with a "Invalid ignore pattern" error instead of silently producing garbage results.

---

## Editing your config

1. Open Tidycraft → **Settings** → **Analysis Rules** → **Edit**
2. Your system editor opens `<project_root>/tidycraft.toml`. If the file didn't exist, a commented default template is created first so every option is visible.
3. Save the file.
4. Click **Run Analysis** (or `⌘⇧R`). The toml is re-read on every click — no rescan needed.

If parsing fails (typo, broken array, missing quote), Run Analysis surfaces the error from the toml parser; fix the line and re-run.

To go back to defaults, delete `tidycraft.toml`. The next analysis run uses built-in values.

---

## Common scenarios

### "I'm getting flooded with warnings on third-party content"
Add the third-party root to `[ignore].patterns`. This is the right answer 90% of the time.

### "Our team uses Chinese asset names — every file is flagged"
```toml
[naming]
forbid_chinese = false
```

### "UI textures are all flagged as non-POT"
```toml
[texture]
require_pot = false
```
…or add `Assets/UI/**` to `[ignore].patterns` if you only want to skip a subdirectory.

### "Open-world meshes trip the vertex limit"
```toml
[model]
max_vertices = 500000
max_faces = 1000000
```

### "I deliberately keep backup copies (`weapon.fbx` + `weapon_backup.fbx`)"
Either rename one outside the project or add the backup pattern:
```toml
[ignore]
patterns = ["**/*_backup.*", "**/*_old.*"]
```

### "An asset triggers a rule but I want to keep it as-is"
Per-asset suppression isn't built in yet — current options are:
- Add the specific path to `[ignore].patterns` (loses **all** rules on that file)
- Lower the rule's strictness globally
- Live with the issue and ignore it in the Issues view

If you hit this enough that pattern-level suppression isn't enough, file an issue.

---

## Architecture notes (for contributors)

- Rules live in `src-tauri/src/analyzer/rules/{naming,texture,texture_colorspace,model,audio,duplicate,missing_reference}.rs`.
- Each rule is `Send + Sync` and stateless; `Rule::check(&self, &AssetInfo) -> Option<Issue>` returns the first matching sub-rule's issue.
- `AnalysisResult` aggregates issues + counts by severity and by `rule_id`.
- Configuration: `RuleConfig` in `analyzer/rules/mod.rs`; serialized via `serde` + `toml`. The commented welcome template is `analyzer/rules/config_template::DEFAULT_CONFIG_TEMPLATE`.
- The frontend's `Settings → Analysis Rules → Edit` button calls `ensure_project_config` (creates the file from template if missing) then `open_with_default_app`. The toml is re-read on every `runAnalysis`.
