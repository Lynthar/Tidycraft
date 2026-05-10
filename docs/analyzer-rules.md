# Analysis Rules

Tidycraft's **Run Analysis** is an asset-quality lint. It reads your scanned project, applies a set of opinionated rules, and produces an Issues list you can filter, group, and jump into. This document explains what each rule does, when it might bother you, and how to override it.

## How it works

Clicking **Run Analysis** (or `⌘⇧R`) runs five phases on the cached scan result:

1. **Per-asset rule checks** — five rule families (`naming`, `texture`, `texture.color_space`, `model`, `audio`) run against every asset. Each family is stateless and returns at most one issue per asset (the first sub-rule that fires).
2. **Duplicate detection** — files are grouped by size; same-size files are SHA256-hashed and any group with more than one match is reported (the first asset in a group is the "original", the rest are flagged).
3. **Missing-reference detection** (Unity only) — every `.prefab` / `.unity` / `.mat` / `.controller` / `.asset` is parsed for GUID references that don't resolve to any scanned `.meta`.
4. **PBR set completeness** — textures are grouped by directory + base stem (`T_Wood_BaseColor` + `T_Wood_Normal` are siblings); a set with the trigger channel but missing required channels is flagged.
5. **DCC source linking** — authoring source files (`.blend`, `.psd`, `.spp`, `.ma`, etc.) are paired with same-stem runtime exports (`.fbx`, `.png`, …); when the source's mtime is newer than the export's by more than the configured tolerance, an "outdated export" warning fires.

All five phases share the same `tidycraft.toml` configuration, read from your project root each time you click Run Analysis. **No rescan is needed after editing the file** — just save and re-run.

> **Note on other top-level sections.** `tidycraft.toml` may also contain a `[project]` table (`theme` / `goal`) consumed by AI Tagging. The analyzer ignores it; `llm::project_meta::ProjectMeta::from_toml` reads it via `toml::Value` so the two concerns don't interfere. Future AI-generated rule output will live in a separate `tidycraft.ai.toml` to keep program-written content out of the user-edited file.

## Out-of-box defaults

Most rule families ship `enabled = false` so a fresh project produces almost no false positives. **Default on**:

- `naming` — but only the `forbidden_chars` sub-rule meaningfully fires (shell-unsafe characters; thresholds elsewhere are loose)
- `texture.color_space` — its own section now; catches a real corruption bug, not a stylistic convention
- `duplicate` — always on, no config
- `missing_reference` — always on for Unity projects, no config

**Default off** (opt in via `tidycraft.toml`):

- `texture` (PoT / size / file-size)
- `model` (vertex / face / material limits)
- `audio` (sample-rate / duration / mono-for-SFX)
- `pbr_set` (per-folder texture group completeness)
- `dcc_source` (source-file ↔ export mtime pairing)

Out-of-box `Run Analysis` therefore flags only **real bugs** — illegal characters, duplicates, broken Unity references, sRGB-tagged data textures. Stricter conventions are opt-in.

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
| `dcc_source.outdated_export` | DCC source files (cross-asset) | warning |

---

## Naming Convention (`[naming]`)

| Sub-rule | Default | TOML key | When to relax |
|---|---|---|---|
| Max name length | 512 chars (loose) | `max_length = 64` | Strict pipelines (UE, deep nesting) |
| Forbidden characters | space, `! @ # $ % ^ & * ( ) + =` | `forbidden_chars` | Inheriting Unity Asset Store packages or third-party samples |
| Forbid Chinese characters | false | `forbid_chinese = true` | Strict ASCII-only pipelines |
| Required prefix per type | none | `texture_prefix = "T_"` / `model_prefix` / `audio_prefix` | Teams enforcing a naming convention |
| Case style | any | `case_style` ∈ `"any" \| "PascalCase" \| "snake_case" \| "camelCase"` | Mixed-case codebases |

> **First-issue mode**: a single asset that violates several naming sub-rules will only show the first match in the order above. Fix it, re-run, the next one surfaces.

---

## Texture Standards (`[texture]`) — *disabled by default*

| Sub-rule | Default | TOML key | When to relax |
|---|---|---|---|
| Power-of-two dimensions | true | `require_pot = false` | UI / icon textures, LUTs (256×16), HDRIs |
| Maximum size | 4096 px | `max_size = 8192` | High-detail hero assets, console-only projects |
| Minimum size | 4 px | `min_size` | Stamp / brush libraries with intentionally tiny tiles |
| Square only warning | off | `warn_non_square = true` | Pipeline that requires square atlases |
| Maximum file size | 10 MB | `max_file_size` (bytes) | Cinematic / cutscene textures |
| Missing mipmaps (DDS only, ≥ 512px) | always on | n/a | Disable the whole texture rule (`enabled = false`) — there's no per-sub-rule toggle |

---

## Texture Color Space (`[texture.color_space]`) — *enabled by default*

Detects PNGs flagged as **sRGB** whose filename suggests they're a data channel (normal map, roughness, metallic, AO, etc.). The engine would otherwise de-gamma those pixels at import and silently corrupt the data.

This used to share `[texture]`'s enabled flag, but lives in its own section now — disabling the size / PoT / file-size checks shouldn't also turn off this safety net. Default on because the underlying bug is real corruption, not a stylistic convention.

A warning fires only when **both** signals match:

1. The file's color profile chunk says `sRGB`.
2. The filename stem ends with one of: `_n`, `_normal`, `_norm`, `_nrm`, `_r`, `_rough`, `_roughness`, `_m`, `_metal`, `_metallic`, `_ao`, `_mask`, `_data`, `_lin`, `_linear`, `_height`, `_disp`, `_displacement`, `_orm`, `_mra`, `_rma`.

To suppress: rename the file (drop the suffix), re-export with Linear color space, or add the path to `[ignore].patterns`.

---

## Model Standards (`[model]`) — *disabled by default*

| Sub-rule | Default | TOML key | When to relax |
|---|---|---|---|
| Max vertices | 100,000 | `max_vertices = 500000` | Open-world chunk meshes, hero LOD0 |
| Max faces | 100,000 | `max_faces` | Same as above |
| Max materials | 10 | `max_materials` | Modular character with separate materials per part |

---

## Audio Standards (`[audio]`) — *disabled by default*

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

## PBR Set Completeness (`[pbr_set]`) — *disabled by default*

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

## DCC Source-File Linking (`[dcc_source]`) — *disabled by default*

Cross-asset check: pairs authoring source files (`.blend`, `.psd`, `.spp`, `.ma`, `.mb`, `.max`, `.ztl`, `.zpr`, `.lxo`, `.hip`, `.c4d`, `.zprj`, `.sbs`, `.psb`) with their runtime exports (`.fbx`, `.glb`, `.png`, etc.) by file-stem matching, then fires a warning when the source's mtime is newer than the export's by more than `mtime_tolerance_secs`.

Catches the **"edited locally, forgot to re-export"** loop reliably. Does NOT catch cross-commit stale pairs because `git checkout` synchronizes mtimes — a documented limitation.

```toml
[dcc_source]
enabled = true
# Tolerance in seconds. `git pull` touches every file's mtime; 60s
# avoids false positives in the minutes after a sync.
mtime_tolerance_secs = 60

# Per-tool mappings. Defaults cover Blender / Maya / Max / ZBrush /
# Modo / Houdini / Cinema4D / Marvelous / Substance Painter+Designer
# / Photoshop. Override `mappings` to customize — listing ANY mapping
# replaces the WHOLE default list, so include every entry you want active.
[[dcc_source.mappings]]
name = "blender"
sources = ["blend"]
exports = ["fbx", "glb", "gltf", "obj", "dae"]

# ... (8 more default mappings — see config_template for the full list)

[dcc_source.lookup]
same_dir = true
sibling_dirs = ["sources", "_source", "src"]
```

**Pairing strategy** (per source asset):

1. Look up which mapping owns the source's extension. Files whose extension isn't in any mapping skip the analysis entirely.
2. Build candidate export directories:
   - The source's own directory (if `lookup.same_dir = true`).
   - Walking up from the source's parent, any ancestor whose name matches an entry in `lookup.sibling_dirs` adds its own grandparent as a candidate. If the user lists multiple sibling names, the OTHER names also expand into sibling subdirs of the grandparent (handles `art/sources/x.blend ↔ art/exports/x.fbx` when `sibling_dirs = ["sources", "exports"]`).
3. In each candidate directory, look for files with the same lowercase stem and an extension in `mapping.exports`. Pick the **newest** across all candidates.
4. If `source.mtime > newest_export.mtime + tolerance_secs`, emit `dcc_source.outdated_export` with a humanized time delta and a suggestion to re-export from the named tool.

**1→N pairing (Substance Painter `.spp`)** is approximated as 1→newest-PNG in this phase. Future iteration will use PBR channel suffixes to identify the full set of expected per-channel outputs.

**Layout examples** (with default `sibling_dirs`):

| Source | Candidate export dirs |
|---|---|
| `models/character.blend` | `models/` |
| `models/sources/character.blend` | `models/sources/`, `models/`, `models/_source/`, `models/src/` |
| `Characters/Hero/sources/Hero.blend` | (the `sources/` parent), `Characters/Hero/`, `Characters/Hero/_source/`, `Characters/Hero/src/` |

**To suppress**:
- Add the source path to `[ignore].patterns` to drop it before analysis.
- Or set `enabled = false` to turn the rule off entirely.

The issue is anchored on the **source** file (clicking the issue jumps to the source, not the export) so you can act on it directly in your DCC tool.

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

- Rules live in `src-tauri/src/analyzer/rules/{naming,texture,texture_colorspace,model,audio,duplicate,missing_reference,pbr_set,dcc_source}.rs`.
- Each rule is `Send + Sync` and stateless; `Rule::check(&self, &AssetInfo) -> Option<Issue>` returns the first matching sub-rule's issue.
- `AnalysisResult` aggregates issues + counts by severity and by `rule_id`.
- Configuration: `RuleConfig` in `analyzer/rules/mod.rs`; serialized via `serde` + `toml`. The commented welcome template is `analyzer/rules/config_template::DEFAULT_CONFIG_TEMPLATE`.
- The frontend's `Settings → Analysis Rules → Edit` button calls `ensure_project_config` (creates the file from template if missing) then `open_with_default_app`. The toml is re-read on every `runAnalysis`.
