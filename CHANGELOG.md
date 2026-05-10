# Changelog

All notable changes to Tidycraft will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it leaves alpha.

## [Unreleased]

### Added
- **AI Tagging — Learning mode** (default recommended path). The user runs a one-time learning pass: backend samples files per directory by-asset-type-ratio (round-robin so a 100-PNG/1-FBX dir still surfaces the FBX), feeds samples + the user's existing tag system + project `[theme]`/`[goal]` to the LLM as text-only context, parses inferred conventions + tag gaps + heuristic rules, and persists rules to `<project>/tidycraft.ai.toml`. Once learned, `suggest_tags` runs the local `RuleSuggester` (filename-token / path-prefix / path-segment matching) instead of the heuristic suggester — zero per-asset LLM cost from then on. AITagPanel header shows "🧠 AI · 5d ago · N rules" status badge with inline Run / Re-learn / Review controls; first-time users see a prominent CTA banner above demoted heuristic groups.
- **AI Tagging — per-asset mode** (advanced, opt-in via Settings → AI Tagging → "Enable per-asset AI tagging"). Multi-provider LLM tagging via Claude / OpenAI / Ollama, with shared `LLMProvider` trait, per-asset disk cache (SHA256-keyed; partial-batch hits stay free), and a verified-pricing cost estimator. Cost-preview modal with per-provider consent, an API-key plaintext-localStorage warning, and a thumbnail-upload checkbox **defaulting to OFF** (filename + path usually carry the signal for game assets; vision adds 60-80% to cost). Result panel groups suggestions by `(label, category, source)` and routes `existing` chips to the user's already-defined tag (no duplicate creation), `new` chips to a fresh tag with an `(AI)` suffix.
- **Project-aware prompt**. The LLM receives `[theme]` / `[goal]` from `tidycraft.toml`, the user's existing tag system (with optional descriptions and up to 5 sample paths per tag), and is instructed to mark each suggestion as `existing` or `new`.
- **Tag.description** optional field, edited from TagManager (collapsed by default) and shipped to the LLM as semantic context.
- **Settings → AI Tagging** section: provider radio + model dropdown (cloud lists are static; Ollama lists installed models live via `/api/tags`) + endpoint override + privacy reset + advanced "Enable per-asset AI tagging" toggle. Maintenance section gains an "AI tag cache" row with size + clear button.
- **TagFilterPanel** sidebar header: right-click opens TagManager directly + new gear icon for click access (previously only reachable via per-file right-click).
- **`.vox` model preview** (MagicaVoxel) with both nTRN scene-graph (v200+) and chunk-only (v150) handling.
- **Session restore is now lazy**: only the previously-active project gets a full scan + watcher + git refresh on launch; non-active projects register as stubs and hydrate on first switch. Cold start with many projects is significantly faster.
- **DCC source-file linking** (cross-asset analyzer, opt-in via `[dcc_source]` in `tidycraft.toml`). Pairs authoring source files (`.blend` / `.ma` / `.mb` / `.max` / `.ztl` / `.zpr` / `.lxo` / `.hip` / `.c4d` / `.zprj` / `.psd` / `.psb` / `.spp` / `.sbs`) with their runtime exports (`.fbx` / `.glb` / `.png` / etc.) by stem matching, and warns when the source's mtime is newer than its export's — catching the "tweaked the model but forgot to re-export" class of mistake. Configurable per-tool mappings + sibling-dir lookup (handles `art/sources/x.blend ↔ art/x.fbx` layouts) + `mtime_tolerance_secs` for git-checkout robustness. `AssetMetadata.dcc_source_kind` field labels recognized authoring formats so future UI can show source/runtime distinctions inline.

### Fixed
- **ModelLightbox center math** (was offsetting models by `(scale − 1) × center` on enlarge — voxel exports especially).
- **Vertex colors preserved** across material conversions in ModelViewer3D / ModelLightbox (`MeshPhongMaterial → MeshStandardMaterial` now carries `vertexColors`); fixes voxel OBJ files rendering flat gray.
- **OBJ loader honors the actual `mtllib` filename** instead of guessing `<basename>.mtl`; skips MTL fetch entirely when no `mtllib` line is declared (no more spurious 500s in the console).
- **Thumbnail whitelist** now matches the `image` crate's enabled features — `tiff` / `tif` / `webp` / `hdr` / `exr` were decoder-supported but blocked at the entry check.
- **Thumbnail failures log at debug**, not error (clean console for PSD / DDS / SVG / deep EXR; fallback box-icon UX unchanged).
- **Header rescan button now actually clears the disk cache** before re-opening (button name finally matches behavior; replaces a `CACHE_VERSION` bump path).

### Changed
- `AIResultPanel` now share a `(label, category, source)` apply path that batches `addTagToAssets` per group rather than per-asset (50 assets × 3 tags is now 3 IPC calls, not 150).
- `tidycraft.toml` template gains a top-level `[project]` block (theme / goal). Analyzer happily ignores it; AI Tagging reads it.
- `PROMPT_VERSION = 2` (per-asset prompt) and `LEARNING_PROMPT_VERSION = 1` (learning prompt) are independently bumpable — version sits in each cache key so prompt-meaning changes never serve stale results.
- `suggest_tags` command auto-routes through `rule_suggest::load_or_fallback`: AI rules from `tidycraft.ai.toml` if present, heuristic suggester otherwise. Both produce the same `TagGroup[]` shape.
- `RuleSuggester` regex rule kind currently silent-skips (the `regex` crate isn't a project dep yet) — `filename_token` / `path_prefix` / `path_segment` cover the common patterns the LLM is steered toward. Regex rules will activate in v2 without prompt changes.

### Planned
- VRAM budget estimates per texture / per directory.
- Cross-engine reverse-reference graph (Unreal / Godot beyond Unity).
- DCC source-file linking phase 2: 1→N pairing for Substance Painter `.spp` (per-channel PNG outputs); git-status-aware severity bump when the source is dirty in the working tree.
- AI Tagging polish: regex rule kind activation; `tidycraft.toml [project]` write-back from LearnSetupModal (currently read-only — user edits toml directly to avoid clobbering comments); confidence-slider editing in LearnReviewPanel.

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
