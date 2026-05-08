# Changelog

All notable changes to Tidycraft will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it leaves alpha.

## [Unreleased]

### Planned
- LLM-backed semantic tag clustering (multi-provider: Claude / OpenAI / Ollama). See [`docs/ai-tagging-plan.md`](docs/ai-tagging-plan.md).
- VRAM budget estimates per texture / per directory.
- DCC source-file linking (`.blend` / `.spp` → exported `.fbx` / `.glb`).
- Cross-engine reverse-reference graph (Unreal / Godot beyond Unity).

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
- LLM-backed semantic tagging is **not** in this build; the AI Tag panel is heuristic only. See the [design doc](docs/ai-tagging-plan.md).
- macOS / Windows binaries are **not code-signed** in v0.0.1 — first-launch warnings are expected.
- Tested up to ~50k assets per project; larger projects should work but may be slow.
- 3D preview limitations on paths with spaces / non-ASCII characters (Tauri asset protocol quirk; not Tidycraft-fixable).
