# Changelog

All notable changes to Tidycraft will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it leaves alpha.

## [Unreleased]

### Changed
- **Learning results now commit on review**. Learned rules are staged in memory and written to `tidycraft.ai.toml` only when the review panel saves; closing without saving truly discards the run, and unreviewed rules never influence tag suggestions.
- **PNG color-space detection parses the embedded ICC profile** ("sRGB" / "Linear" / unknown) instead of treating any profile as sRGB. Deliberately linear data textures are no longer mis-warned, unreadable profiles keep the rule silent, and the advanced filter gains a Linear option.
- **AI batch requests go out in chunks** (≤20 assets each) so a single reply can't blow the model's output cap. Each completed chunk is cached before the next request — a mid-run failure only re-bills the remainder on retry.
- **The AI result panel has one Apply button**, and it honors deselected chips ("apply everything still selected"); the bypassing "Apply all" is gone.
- **Colorspace filename hints drop the collision-prone `_r` / `_m` single-letter suffixes** ("arrow_r", "icon_m" no longer warn); the `_n` normal-map shorthand still does.
- **Release builds no longer ship devtools** (debug builds keep the auto-opened inspector).
- **Unity missing-reference issues are warnings and skip built-in resources.** The editor-shipped GUIDs (`unity default resources` / `unity_builtin_extra` — referenced by any project using a built-in shader, material, or UI sprite) no longer flood ordinary projects with false reports, and remaining hits are Warning instead of Error, since the scan can't see into gitignored `Library/` or `Packages/`.

### Fixed
- **Case-only rename conflict guards compare file identity, not names.** `foo.PNG → foo.png` still works everywhere, and on case-sensitive filesystems a rename can no longer silently overwrite a coexisting file whose name differs only in case (undo path included).
- **LLM cache entries are keyed by which asset a suggestion answers**, not by response position — a model skipping one asset no longer files every later suggestion under the wrong asset's cache slot, and hallucinated paths never enter the cache.
- **Output-cap truncation is reported as its own error** (Claude `stop_reason: max_tokens`, OpenAI `finish_reason: length`) instead of an opaque parse failure, for both tagging and learning calls.
- **Stale analysis results are flagged** with a re-run banner when files change after the run, and the stats pass count intersects with live files — it can no longer go negative.
- **AI modals no longer re-open with a previous project's data** when a request resolves after a project switch, so their Save / Apply can't write into the newly active project.
- READMEs now disclose plaintext API-key storage and the real cache-invalidation behavior; several stale code comments and dead doc links corrected.
- **OBJ material counts are material counts.** A 30-group export sharing one material used to report 30 (the sub-mesh count) and trip the max-materials rule; the count now comes from the loaded MTL, and stays unknown when the MTL can't be read.
- **glTF face counts cover non-indexed and non-triangle primitives.** Non-indexed meshes counted 0 faces, triangle strips/fans were divided by 3 instead of using `n − 2`, and line/point primitives counted phantom faces.
- **Compressed DDS textures report their real alpha.** DXT3/DXT5 and (via the DX10 header) BC2/BC3/BC7 read as alpha-carrying, BC4/BC5 data maps stay opaque — previously every compressed DDS claimed "no alpha" because only the uncompressed-layout flag was consulted.
- **`allowed_sample_rates = []` no longer crashes analysis** — an empty allow-list now means "don't check sample rates".
- **Cancelled scans emit a terminal progress event** instead of leaving the phase stuck at "parsing" (the UI previously recovered only via its own stop flag).
- **Windows: engine info panels get forward-slash paths** — Unity / Unreal / Godot project info was the last place backslashes leaked to the frontend.
- **Version drift and prerelease versions fail the pipeline up-front.** CI runs `check-version` before the build step (which used to silently repair drift on the runner), and `sync-version` rejects prerelease suffixes with a clear message instead of letting a tagged release die inside the Windows MSI bundler.
- **Shift-click ranges survive filtering and sorting.** The range anchor is now the anchored file itself, not its former row number — narrowing a search between clicks no longer selects an unrelated range or silently breaks selection.
- **Git-status filtering updates with git refreshes** (the filtered view used to lag behind the always-fresh row badges).
- **The stats dashboard follows file changes** — watcher events and Ctrl+R rescans refresh totals, charts, and largest-files instead of freezing at first render.
- **Ctrl+R no longer wipes the multi-selection** — the rescan's brief "no scan result yet" window was being mistaken for "every file was deleted".
- **Deleting a tag (or switching projects) clears it from the active tag filters.** A dead filter id used to AND-hide every asset with no pill left to un-click.
- **Batch-rename failures are visible.** On partial failure the dialog stays open with a per-file error list (the asset list still refreshes), the failed files stay selected for a retry, and a closed dialog no longer leaks its previous find/replace state into the next open. Previously any success closed the dialog before the result could render.
- **AI apply can no longer mint duplicate "(AI)" tags** — every create path checks for an existing tag of the same name first, including tags created earlier in the same run.
- **Ollama "model not found" reads as what it is.** A 404 now names the model and suggests `ollama pull`, instead of masquerading as a network error.
- **Metadata panels no longer render "null" / "-bit" / "0.0 kHz"** rows for fields a format doesn't have — unset metadata is omitted from the wire instead of serialized as `null`.
- Locale files' duplicate top-level `common` blocks merged (a JSON-editing key-loss trap; verified key-for-key lossless).

### Planned
- VRAM budget estimates per texture / per directory.
- Cross-engine reverse-reference graph (Unreal / Godot beyond Unity).
- DCC source-file linking phase 2: 1→N pairing for Substance Painter `.spp` (per-channel PNG outputs); git-status-aware severity bump when the source is dirty in the working tree.

## [0.6.0] — 2026-05-11

### Added
- **AI Tagging — Learning mode** (recommended default). One LLM call samples the project, derives local heuristic rules, persists them to `<project>/tidycraft.ai.toml`. After that, `suggest_tags` matches locally with zero per-asset LLM cost. AITagPanel header gets a status badge with Run / Re-learn / Review controls.
- **AI Tagging — per-asset mode** (opt-in via Settings → AI Tagging). Multi-provider LLM tagging (Claude / OpenAI / Ollama) behind a shared `LLMProvider` trait, with a per-asset SHA256 disk cache so partial-batch re-runs stay free. Cost-preview modal gates every call. Thumbnail upload defaults off; filename + path carry most of the signal for game assets.
- **Project-aware prompt**. The LLM receives `[project]` theme / goal from `tidycraft.toml` plus the user's existing tag list (descriptions + up to 5 sample paths per tag), and marks each suggestion as `existing` or `new`.
- **`Tag.description`** optional field. Edited from TagManager, shipped to the LLM as semantic context.
- **Settings → AI Tagging** section. Provider radio, model dropdown, endpoint override, privacy reset, per-asset-mode toggle. Ollama lists installed models live via `/api/tags`. Maintenance gains an AI-tag cache row.
- **TagFilterPanel sidebar header**. Right-click opens TagManager; new gear icon for click access.
- **`.vox` model preview** (MagicaVoxel). Handles nTRN scene-graph (v200+) and chunk-only (v150).
- **Lazy session restore**. Only the previously-active project scans + watches + refreshes git on launch; others hydrate as stubs and run on first switch.
- **DCC source-file linking** (opt-in via `[dcc_source]`). Pairs authoring sources (`.blend` / `.ma` / `.psd` / `.spp` / `.ztl` / `.max` etc.) with same-stem runtime exports (`.fbx` / `.png` / etc.) and warns when the source is newer. Configurable per-tool mappings, sibling-dir lookup, and `mtime_tolerance_secs` to absorb git-checkout sync.
- **AI Learning review polish**. Rule confidence is now an editable 0.5–1.0 slider; invalid `filename_regex` patterns get a ⚠ red chip flagged via JS `RegExp` validation.
- **Theme/goal write-back from LearnSetupModal**. Inputs are editable; Continue writes to `tidycraft.toml [project]` via `toml_edit`, preserving user comments and other sections on round-trip.

### Fixed
- **`runAnalysis` race-safe across project switches**. The analyze call snapshots `projectId` at click time. Mid-flight switches (sidebar button, `Ctrl+Shift+R`, Command Palette) no longer pollute another project's analysis state. The `isAnalyzing` guard now lives in the store, gating every entry path.
- **ModelLightbox center math**. Was offsetting models by `(scale − 1) × center` on enlarge; voxel exports especially.
- **Vertex colors preserved** across material conversions in 3D viewers. Fixes voxel OBJ files rendering flat gray.
- **OBJ loader honors the actual `mtllib` filename**. Skips MTL fetch when no `mtllib` line is declared, ending spurious 500s in the console.
- **Thumbnail whitelist** now matches the `image` crate's enabled features. `tiff` / `webp` / `hdr` / `exr` were decoder-supported but blocked at the entry check.
- **Thumbnail failures log at `debug`**, not error. Cleaner console for PSD / DDS / SVG / deep EXR.
- **Header rescan button now clears the disk cache** before re-opening. Button label finally matches behavior; replaces a `CACHE_VERSION` bump path.

### Changed
- `AIResultPanel` batches `addTagToAssets` per `(label, category, source)` group. 50 assets × 3 tags is 3 IPC calls instead of 150.
- `tidycraft.toml` template gains a top-level `[project]` block (theme / goal). The analyzer ignores it; AI Tagging reads it.
- `PROMPT_VERSION = 2` (per-asset) and `LEARNING_PROMPT_VERSION = 1` (learning) are independently bumpable. Version sits in each cache key so prompt-meaning changes never serve stale results.
- `suggest_tags` auto-routes through `rule_suggest::load_or_fallback`. AI rules from `tidycraft.ai.toml` if present; heuristic suggester otherwise. Output shape is identical either way.
- `RuleSuggester` now executes `filename_regex` rules. The `regex` crate is a proper dep with linear-time matching. Patterns that fail to compile silent-skip with a stderr warning.
- Scanner respects `.gitignore` by default via `ignore::WalkBuilder`. Also honors `.ignore`, git globals, `.git/info/exclude`, and skips hidden dot directories. **Behavior change**: Unity / Unreal projects no longer scan `Library/` / `Intermediate/` / `Saved/` by default. Toggle off via Settings → Scanning to restore "scan everything".

---

## [0.0.1] — Alpha

This is the initial public alpha. Everything below has shipped and is considered stable for normal use; bugs are expected and welcome via [GitHub Issues](https://github.com/Lynthar/Tidycraft/issues).

### Scanning & state
- Parallel `walkdir + rayon` scan with per-asset metadata extraction (image dims, model vertex/face/material counts, audio sample rate / duration / channels, video codec / framerate).
- Incremental scan cache keyed by `(mtime, size)` on disk under `dirs::cache_dir()/tidycraft/scans/`.
- Multi-project workspace: open several projects simultaneously, switch with `ProjectSwitcher`, cross-session restore of open paths.
- Live filesystem watcher (`notify-debouncer-full`, 500ms debounce) — external file changes refresh the asset list automatically.
- Engine detection: Unity / Unreal / Godot / generic, with engine-specific parsers (`.meta` GUIDs, `.uproject`, `project.godot`).

### Analysis
- Rule engine with per-asset families: `naming`, `texture`, `texture.color_space`, `model`, `audio`.
- Cross-asset checks: `duplicate` (SHA256), `missing_reference` (Unity GUID lookup), `pbr_set` (per-folder texture group completeness, with packed channels — ORM / MRA / RMA).
- Out-of-box defaults are minimal — only real bug checks fire by default; stylistic conventions (PoT, file-size, vertex budgets) are opt-in via `tidycraft.toml`.
- Glob-based `[ignore]` patterns for vendored / generated paths.
- Per-project `tidycraft.toml`; in-app **Settings → Analysis Rules → Edit** opens the file in the OS editor.
- Issue list with severity filter, group-by-rule, virtualized rendering for 10k+ issues, JSON / CSV / HTML export.

### Tag system
- Per-project tags, persisted to `.tidycraft-tags.json` at project root.
- Multi-select tag filter; rename / move syncs tag bindings; deleted files reaped by watcher.
- Heuristic tag suggestions (filename token / dimension + PBR channel / path segment), surfaced in the Sidebar **AI Tag** button.

### Asset browsing
- List + Gallery (grid) views with `@tanstack/react-virtual` virtualization.
- Thumbnail preview with disk cache (PNG / JPG / GIF / BMP / TGA).
- 3D model preview via Three.js: glTF / GLB / FBX / OBJ (+MTL) / DAE / 3DS. `.blend` recognized but routed to a "please export to GLB" message.
- Video preview (MP4 / WebM / MOV / AVI / MKV / M4V).
- Audio preview with waveform.
- Command Palette (⌘K / Ctrl+K) — quick navigation, filters, actions.

### File operations
- Safe delete to OS trash, move, copy, duplicate; all watcher-driven (no rescan needed).
- Batch rename with persistent undo (50-entry stack).
- Tags follow file across rename / move; orphan tag bindings cleaned automatically when files vanish.

### External editors
- Settings → External Editors maps file extensions to specific binaries (Photoshop / Blender / Audacity / etc.).
- AssetPreview's open button uses the mapping; falls back to OS default app.
- Routed through `tauri-plugin-opener` so Windows codepage / quoting / `%`-character paths work correctly.

### Cross-platform
- Windows / macOS / Linux supported.
- macOS shortcut display uses `⌘ ⇧` glyphs; Windows / Linux show `Ctrl+Shift+`.
- Path normalization (forward-slash everywhere on the boundary); `lib/pathUtils.ts` for shared helpers.
- Show in File Manager (renamed from "Reveal in Finder") works correctly on all three platforms.

### Git integration
- Branch info, ahead/behind counts, per-file change status badges.
- Toggleable in Settings → Git.

### i18n
- English + Simplified Chinese.

### Known limitations (alpha)
- LLM-backed semantic tagging is **not** in this build; the AI Tag panel is heuristic only.
- macOS / Windows binaries are **not code-signed** in v0.0.1 — first-launch warnings are expected.
- Tested up to ~50k assets per project; larger projects should work but may be slow.
- 3D preview limitations on paths with spaces / non-ASCII characters (Tauri asset protocol quirk; not Tidycraft-fixable).
