# Tidycraft Developer Guide

Welcome. This guide is for anyone extending Tidycraft — whether you're fixing
a bug, adding a feature, or forking the project. It aims to give you enough
context to read the codebase productively within an hour.

For the user-facing introduction, see [README.md](README.md) /
[README.zh-CN.md](README.zh-CN.md). The README's "Roadmap" section tracks
shipped features and the backlog. For change history, use `git log`.

---

## 1. Design Philosophy

Tidycraft is a **local-first asset QA tool** for game developers. The design
commits to four things:

- **Offline, no telemetry.** All state (scan cache, tags, undo history) lives
  on the user's disk. No network calls.
- **Non-destructive.** Deletes go to the OS trash. Renames and moves are
  recorded in an undo stack. File operations always need explicit user intent.
- **Engine-agnostic.** Unity / Unreal / Godot get first-class parsers, but
  "Generic" projects are supported too. Engine detection is a hint.
- **Fast enough for real projects.** 10k+ asset directories scanned in seconds
  via parallel walking and incremental caching.

Goals: cross-platform (Windows / macOS / Linux), production-grade state
isolation (multiple projects open simultaneously), extensible rule engine,
watcher-driven UI that stays in sync with external tools (editors, `git`,
Blender saves).

---

## 2. Tech Stack

| Layer | Choice | Why |
|---|---|---|
| Shell | Tauri 2 | Small native binary, no browser runtime |
| Backend | Rust (edition 2021, MSRV 1.75) | Fast parallel scan, binary parsers |
| Frontend | React 18 + TypeScript + Vite 6 | Typed, fast dev, familiar |
| State | Zustand | Lighter than Redux, less ceremony than MobX |
| Styling | Tailwind CSS | Co-located styles, no naming bikeshed |
| 3D Preview | Three.js | Industry standard for web 3D |
| Virtualization | @tanstack/react-virtual | Handles 10k+ row lists smoothly |

Notable Rust crates: `rayon` (parallel scan), `walkdir`, `image`, `gltf`,
`tobj`, `fbxcel-dom` (FBX metadata), `symphonia` (audio), `mp4` +
`matroska-demuxer` (video), `git2`, `notify` + `notify-debouncer-full` (FS
events), `trash` (safe delete), `globset` (ignore patterns), `parking_lot`
(non-poisoning mutexes), `tauri-plugin-dialog` / `tauri-plugin-fs` /
`tauri-plugin-clipboard-manager` / `tauri-plugin-opener` (the last one
backs `open_with_default_app` and `open_in_editor`).

---

## 3. Build & Run

Prereqs:

- Rust 1.75+
- Node 18+
- pnpm 8+
- **Linux only:** `webkit2gtk-4.1-dev`, `libssl-dev`, GTK3 dev headers
- **macOS only:** Xcode Command Line Tools
- **Windows only:** MSVC build tools + WebView2 (usually preinstalled on W10+)

```bash
pnpm install              # first time
pnpm tauri dev            # full app; Vite HMR for frontend, cargo rebuild on Rust changes
pnpm build                # tsc + vite build (frontend typecheck + bundle)
cd src-tauri
cargo test --lib          # ~115 unit tests
cargo check               # fast Rust typecheck without binary
```

There is no project-wide formatter or linter configured. `tsc` and
`cargo check` are the only gates. Rust files follow `rustfmt` defaults; TS
files follow the style of nearby code (2-space indent, double quotes,
trailing commas).

---

## 4. Architecture

### 4.1 The Big Picture

```
┌───────────────────────────┐      invoke(command, args)      ┌─────────────────────────┐
│  React frontend (src/)    │ ──────────────────────────────► │  Rust backend (lib.rs)  │
│                           │                                  │                         │
│  - Zustand stores         │       emit(event, payload)       │  - ~57 tauri commands   │
│  - Components / UI        │ ◄────────────────────────────────┤  - Per-project state    │
│  - i18n (en / zh)         │                                  │  - Scanner + watcher    │
└───────────────────────────┘                                  └─────────────────────────┘
```

The frontend and backend communicate exclusively through two mechanisms:

- **Commands**: `invoke("command_name", { camelCaseArg })` returns a Promise.
  Every Tauri command is defined in `src-tauri/src/lib.rs` with
  `#[tauri::command]` and registered in the `invoke_handler!` macro.
- **Events**: the backend calls `app.emit(name, payload)`; the frontend
  subscribes with `listen<Payload>(name, handler)`. Used for progress updates
  and filesystem change notifications, both per-project (event names
  include `projectId`).

### 4.2 Backend modules (`src-tauri/src/`)

- **`lib.rs`** — Tauri command definitions. Keep command bodies thin; delegate
  real work to feature modules.
- **`project.rs`** — Per-project state. A global
  `OnceLock<Mutex<HashMap<projectId, Arc<Mutex<ProjectState>>>>>` registry
  holds one `ProjectState` per open project. **Every project-scoped command
  takes `project_id: String` as its first parameter.** Use
  `project::with_mut(&id, |s| { ... })` or `project::with_ref(&id, |s| { ... })`
  as the standard accessors — they grab the registry lock briefly, clone the
  per-project `Arc`, drop the registry lock, then lock the project itself.
  This pattern keeps registry contention minimal.
- **`scanner.rs`** — Directory walk (`walkdir` + `rayon` parallel filter_map),
  asset-type detection, metadata extraction dispatch. Single entry point for
  per-file parsing is `parse_metadata_for(path, ext, asset_type)` — add new
  format parsers there. Paths crossing to the frontend go through
  `path_to_string()` which normalizes to forward slashes.
- **`watcher.rs`** — `notify-debouncer-full` watcher. 500ms debounce, then
  re-parse affected files, patch `ProjectState.cached_scan`, emit
  `fs-change-{projectId}` with delta + rebuilt directory tree. The
  `ProjectWatcher` struct is held inside `ProjectState.watcher`; dropping it
  tears down the OS watch and the processing thread exits cleanly.
- **`analyzer/`** — Rule engine. `Rule` trait has four methods: `id`, `name`,
  `applies_to`, `check` — used by per-asset rules: `naming`, `texture`,
  `texture_colorspace`, `model`, `audio`. Three more checks are **cross-asset**
  and live outside the trait: `duplicate` (size-bucket + SHA256),
  `missing_reference` (Unity GUID lookup), and `pbr_set` (per-folder texture
  group completeness). `RuleConfig` is deserialized from `tidycraft.toml`;
  `Analyzer::with_config` wires the enabled per-asset rules, and
  `lib.rs::analyze_assets` runs the cross-asset phases sequentially after,
  merging into the same `AnalysisResult`.
- **`cache.rs`** — Disk-backed scan cache. File at
  `dirs::cache_dir()/tidycraft/scans/<sha256-prefix>.json`, keyed by
  (mtime, size). Incremental scans reuse cached entries for unchanged files.
- **`unity.rs` / `unreal.rs` / `godot.rs`** — Engine-specific parsers. Unity
  parses `.meta` / `.prefab` / `.unity` / `.mat` YAML via line-level string
  scanning (regex-lite, brittle — tracked as tech debt). Unreal reads
  `.uproject` JSON. Godot parses `project.godot` INI-style.
- **`tags.rs`** — Per-project tag system, persisted to
  `.tidycraft-tags.json` at the project root.
- **`undo.rs`** — 50-entry bounded in-memory undo stack for rename / move
  operations. Trash delete is intentionally not undoable (OS handles it).
  Copy / duplicate are not undoable (trivially reversible by deleting).
- **`git/mod.rs`** — `libgit2` wrapper. Discovers `.git`, reports branch +
  per-file status + ahead/behind counts.
- **`thumbnail.rs`** — On-demand base64 thumbnails for images, disk-cached by
  (path, mtime, size).
- **`llm/`** — Multi-provider AI tagging. `mod.rs` declares the `LLMProvider`
  async trait, the request/response/error schemas (`TagRequest` /
  `TagResponse` / `SuggestedTag` / `LLMError`), the `make_provider` factory,
  the shared 3-tier `parse_suggestions(text)` JSON parser, and the
  `suggest_with_cache(provider_id, request, fetcher)` helper that splits a
  batch into cache hits and misses, calls the fetcher only for the misses,
  persists fresh entries, and merges. Concrete providers (`claude.rs` /
  `openai.rs` / `ollama.rs`) own only their endpoint + auth + request/response
  shape + error mapping; everything else lives in shared modules. `cost.rs`
  holds verified per-million pricing in micro-USD (integer arithmetic) plus
  per-provider vision token rules. `cache.rs` is per-asset disk cache keyed by
  `SHA256(thumb_hash + filename + path + provider + model + prompt_version)`
  with `\x00` separators between fields. `prompts.rs` exports the system
  prompt + `build_user_prompt(assets, project_ctx, existing_tags,
  include_thumbnails)` — bumping `PROMPT_VERSION` invalidates every cached
  entry so prompt-meaning changes never serve stale results.
  `project_meta.rs` parses `tidycraft.toml [project]` via `toml::Value` so
  the analyzer's strict deserializer doesn't have to know about it.

### 4.3 Frontend modules (`src/`)

- **`stores/projectStore.ts`** — The hub. Holds a `Map<projectId, ProjectData>`
  plus **"mirror" fields** (`scanResult`, `selectedAsset`, `viewMode`, …) that
  shadow the active project's data for ergonomic component code. When mutating
  active-project state, use the `updateActiveProject()` helper. **For
  background work that targets a specific (possibly non-active) project** —
  e.g. a scan that completes after the user switched away, or an fs-change
  event — patch the project entry in the Map directly and only touch mirror
  fields when the target equals `activeProjectId`. See `openProject` and
  `applyFsChange` for the canonical examples.
- **`stores/tagsStore.ts`** — Follows the active project; re-subscribes and
  reloads tags whenever `activeProjectId` changes.
- **`stores/sessionStore.ts`** — Cross-session restore. Persists open
  project paths + active path; the full `ProjectData` (scanResult,
  analysisResult, UI state) is rebuilt by re-running `openProject` at boot.
  Internal `restored` guard prevents React strict-mode double-restore.
- **`stores/uiStore.ts`** — Transient overlay flags (`cmdkOpen`,
  `settingsOpen`, `tagManagerOpen`, `aiPanelOpen`). Global store, not
  App-level state, so `CommandPalette` can open Settings / TagManager /
  AITagPanel without prop drilling. Distinct from `settingsStore` (which
  holds persisted user preferences).
- **`stores/selectionStore.ts`** — Multi-selection (`selectedPaths`) lifted
  out of AssetList so non-list components (e.g. AITagPanel's Preview action)
  can drive it. Auto-clears on active-project change.
- **`stores/columnStore.ts`** — Persistent list-view column visibility +
  widths + `viewMode: 'list' | 'grid'`. Versioned (currently v4) with a
  migrate function so layout changes don't lose user customization.
- Other global stores: `settingsStore`, `themeStore`, `searchHistoryStore`.
- **`components/`** — Flat layout, one component per file, no barrel exports.
  `AssetList` is the parent shell that owns selection / dialogs and
  dispatches between `AssetListView` (virtualized list) and
  `AssetGalleryView` (virtualized card grid) based on `columnStore.viewMode`.
  `ContextMenu` + dialogs (`RenameDialog`, `BatchRenameDialog`,
  `DeleteConfirmDialog`, `MoveCopyDialog`) handle operations.
  `CommandPalette` is a hand-rolled four-section ⌘K (Suggestions / Navigate /
  Filter / Resources / Actions). `AITagPanel` is the heuristic tag-suggest
  overlay anchored top-left. `ModelViewer3D` / `ModelLightbox` do 3D
  preview via Three.js. Dialog pattern: the parent owns a nullable state
  object (`{ mode, paths }` or `paths | null`), renders the dialog
  conditionally, passes `onClose` and `onDone` callbacks.
- **`lib/modelUrlResolver.ts`** — Builds a Three.js `LoadingManager` URL
  modifier pre-seeded with a sibling-texture map fetched from the backend.
  See §4.5.
- **`lib/pathUtils.ts`** — Cross-platform path helpers (`basename`,
  `dirname`, `getExtension`, `basenameWithoutExt`, `getEditorDisplayName`).
  All accept both `/` and `\` defensively even though backend paths are
  forward-slash normalized — input can leak from file dialogs / FBX
  embedded URLs / user-supplied editor binaries.
- **`lib/platform.ts`** — `getPlatform()` / `isMacOS()` / `isWindows()` /
  `isLinux()`. Cached UA sniff; used by `formatShortcut` (renders `⌘⇧R`
  on macOS) and the `tc-platform-macos` body-class CSS hook.
- **`types/asset.ts`** — TS mirrors of the Rust `serde` structs. **Kept in
  sync manually; no codegen.** If you add a field to a Rust type that crosses
  the boundary, update this file.
- **`i18n/locales/`** — `en.json` + `zh.json`. Interpolation uses `{{name}}`.

### 4.4 Cross-cutting patterns

**Path normalization.** All paths that round-trip between backend and
frontend use forward slashes. The backend converts via
`scanner::path_to_string` on every exit point (`AssetInfo.path`,
`DirectoryNode.path`, `ScanResult.root_path`, fs-change events, move/copy
result paths). The frontend normalizes input from the Tauri file dialog
(`rawPath.replace(/\\/g, "/")`). **Don't slice a path by `"/"` alone in new
code without normalizing first.**

**Per-project progress / fs events.** Event names include the projectId so
multiple projects can scan or be watched concurrently without stepping on
each other. Subscribe with `listen<Payload>(\`scan-progress-\${id}\`, ...)`
or `listen<Payload>(\`fs-change-\${id}\`, ...)`. Always `unlisten()` on
cleanup — `projectStore.openProject` shows the lifecycle for both.

**Target-selection rule for bulk ops.** When the user right-clicks an asset
with the context menu open, we decide between "single-asset op" and "operate
on current multi-selection" by:

- If the right-clicked asset is part of the current selection → operate on
  the selection.
- Otherwise → operate on the single asset (even if other items happen to be
  checkbox-selected).

This is the `targetPathsFromContext()` helper in `AssetList`. Delete, move,
copy, and duplicate all follow it.

**Lazy session restore.** `sessionStore.restoreSession` does not run a full
`openProject` for every persisted path — that used to make cold starts with
many projects feel slow because non-active scans were thrown away anyway. It
now runs in two phases: parallel `registerProjectStub(path)` calls (cheap —
just register the project with the backend and add a stub `ProjectData` to
the Map) for non-active paths, then a single full `openProject(activePath)`.
`setActiveProject` detects a stub (`scanResult==null && !isScanning &&
!error`) and triggers `openProject(path, {force:true})` on first switch. The
`!error` guard prevents loops when a project's path is permanently broken;
the user can retry via the Header rescan button.

**LLM context flow.** `llm_suggest_tags` collects project framing inside the
project lock (clones `root_path`, all `Tag` objects with their description,
and up to 5 sample paths per tag via `TagsData::get_assets_with_tag`), drops
the lock, then reads `tidycraft.toml [project]` outside the lock. The
collected `ProjectMeta` and `Vec<ExistingTagContext>` ride into `TagRequest`
and the prompt builder emits per-block context only when non-empty (no token
waste on projects without theme/goal or without tags). When prompt semantics
change, bump `PROMPT_VERSION` in `prompts.rs` — it's part of the cache key,
so old entries naturally invalidate.

**Watcher-driven UI refresh.** File operations (delete, move, copy,
duplicate) don't explicitly tell the frontend what changed. They modify the
filesystem; the watcher emits `fs-change-{projectId}` with the delta;
`applyFsChange` in `projectStore` merges it into `scanResult`. This means
**no rescan is needed after a file op**, which matters because rescanning a
50k-file project takes seconds and blocks UI.

One exception: `rename_file` currently triggers an explicit rescan via
`openProject(projectPath)`. Tech debt — should migrate to watcher-driven
refresh.

**i18n.** Every user-facing string in components goes through `t("key")`.
Keep keys grouped by feature area (`deleteConfirm.title`, `moveCopy.moving`,
etc.). Keep `en.json` and `zh.json` in lockstep.

### 4.5 3D preview and texture resolution

FBX / OBJ / DAE files frequently reference textures by bare filename
(`colormap.png`) or a stale absolute path from the authoring machine. Without
help, the Tauri asset protocol would return 500 for the texture.

Solved in `buildTextureUrlResolver`:

1. Before loading a model, call the `resolve_texture_siblings` backend
   command. It walks the model's own directory plus common texture subdirs
   (`Textures/`, `Materials/`, `Maps/`, `Images/` and case variants) plus
   parent-level `Textures/`, and returns a `{ lowercase_filename:
   absolute_path }` map.
2. The Three.js `LoadingManager` URL modifier extracts the basename of
   any requested URL (works for both bare filenames and already-encoded
   `http://asset.localhost/...` URLs — decode first, then slash-split) and
   looks it up in the map.
3. On miss, falls back to the old behavior (resolve relative to modelDir).

---

## 5. How to Add Things

### A new Tauri command

1. Add `#[tauri::command] fn your_command(...)` to `lib.rs`. Prefer
   `Result<_, String>` for errors. If the command needs project state, take
   `project_id: String` as the first parameter and use `project::with_mut` or
   `project::with_ref`.
2. Register in the `invoke_handler!` macro near the bottom of `lib.rs`.
3. Call from the frontend:
   `invoke<ReturnType>("your_command", { projectId, someArg })`. Tauri
   converts `snake_case` arg names to `camelCase` automatically.

### A new per-asset analyzer rule

1. Create a new file under `src-tauri/src/analyzer/rules/`.
2. Define a `Config` struct with `#[serde(default)]` fields and a `Default`
   impl (so missing keys in `tidycraft.toml` use sensible defaults).
3. Implement the `Rule` trait (`id`, `name`, `applies_to`, `check`).
4. Add the config to `RuleConfig` in `analyzer/rules/mod.rs`.
5. Register in `Analyzer::with_config` (in `analyzer/mod.rs`).
6. Write unit tests in the same file.

### A new cross-asset analyzer pass

If your check needs to compare assets to each other (duplicates,
references, set completeness), it doesn't fit `Rule::check` — that takes
one `AssetInfo` at a time. Follow the `pbr_set` / `duplicate` shape:

1. Create the file under `analyzer/rules/` with a free function
   `pub fn find_<thing>_issues(assets: &[AssetInfo], config: &Config) -> AnalysisResult`.
2. Add config to `RuleConfig` if tunable.
3. Expose via a method on `Analyzer` for symmetry with `find_duplicates` /
   `find_missing_references`.
4. Call from `lib.rs::analyze_assets` after the per-asset phase and
   `result.merge(...)` the output.

### A new asset type or format parser

1. If it's a new top-level category, add a variant to `AssetType` in
   `scanner.rs` and handle it in `get_asset_type`.
2. If it's a new format within an existing category, add the extension to
   `get_asset_type` and add a branch to `parse_metadata_for`.
3. Implement the parser as a free function returning `Option<AssetMetadata>`.
4. If your parser yields metadata fields not already in `AssetMetadata`, add
   them both to the Rust struct (`scanner.rs`) and the TS interface
   (`types/asset.ts`).
5. Unit-test the parser. Use `tempfile::tempdir()` for filesystem fixtures.

### A new frontend component

One file per component in `src/components/`. Props-driven. If your component
triggers a modal, follow the `DeleteConfirmDialog` / `MoveCopyDialog`
pattern: parent owns a nullable state object, renders the dialog
conditionally, passes `onClose` and `onDone` callbacks. The dialog takes
transient state (`isLoading`, inline errors) internally and resets on
`isOpen` transitions.

### A new i18n string

Add to `src/i18n/locales/en.json` and `zh.json` in parallel. Group under an
existing feature key or create a new one. Interpolation syntax: `{{name}}`;
pass values via the second arg to `t()`:
`t("moveCopy.titleMove", { count: 5 })`.

---

## 6. Testing

**Rust side:** `cargo test --lib` from `src-tauri/`. Currently ~100 tests
across `scanner`, `analyzer` (incl. `tag_suggest`), `watcher`, `undo`,
`tags`, `unity`, `unreal`, `godot`, `cache`. Aim to add tests for new
parsers and pure functions. Skip integration tests that spawn the full
Tauri runtime — the payoff isn't worth the complexity.

**Frontend side:** no test runner currently. `pnpm build` runs `tsc`, which
is the only TS gate. If you introduce non-trivial component logic, consider
adding Vitest.

**Manual:** `pnpm tauri dev` and exercise the feature. If your change
touches paths, filesystems, or platform-specific APIs, verify on both
Windows and at least one POSIX target.

---

## 7. Cross-Platform Gotchas

- **Paths.** Windows uses backslashes natively; we normalize to forward
  slashes on every boundary. Don't write `path.lastIndexOf("/")` on a raw OS
  path without normalizing first — use `basename` / `dirname` /
  `getExtension` from `lib/pathUtils.ts`, which handle both separators
  defensively.
- **Tauri asset protocol.** File paths with spaces or non-ASCII characters
  can fail to resolve on certain OS / format combinations (e.g. FBX with
  embedded texture paths). Known platform limitation — don't try to fix
  by URL-escaping without testing 3D preview end-to-end.
- **Cross-device move.** `fs::rename` fails across filesystems on POSIX
  (EXDEV). We don't currently fall back to copy+remove. Rare in practice.
- **Linux inotify limits.** `/proc/sys/fs/inotify/max_user_watches` caps the
  watcher's capacity. Large projects can exhaust it; `notify` returns an
  error we only log to stderr.
- **macOS code signing.** Distribution requires Developer ID signature +
  notarization. Not set up yet.
- **Keyboard shortcuts display.** Detection in `useKeyboardShortcuts`
  honors both `ctrlKey` and `metaKey`, so shortcuts work on every OS.
  `formatShortcut` now reads `getPlatform()` from `lib/platform.ts` and
  renders `⌘⇧R` on macOS / `Ctrl+Shift+R` elsewhere. `CommandPalette`
  still hard-codes the glyphs in places — fine on macOS, slightly off on
  Windows; low-pri cleanup if/when those tooltips become a complaint.

---

## 8. Contribution Workflow

1. **Pick a task** from an open issue or the README's "Backlog" section.
   For larger work, open an issue first to discuss scope.
2. **Follow the existing style.** Rust: `rustfmt` defaults. TS: match nearby
   files. Prefer editing existing files to creating new abstractions.
3. **Commit in focused chunks.** One-line subject, blank line, a short body
   explaining **why** (the what is in the diff). Group related changes;
   don't bundle unrelated work.
4. **Verify locally** before pushing: `cargo test --lib`, `pnpm build`, and a
   quick `pnpm tauri dev` sanity check.
5. **Open the PR** with a summary that mirrors the commit body.

---

## 9. File Layout Reference

```
tidycraft/
├── src/                              # React frontend
│   ├── components/                   # UI (flat, one per file)
│   │   ├── AssetList.tsx             # Parent shell: dispatches list/grid + owns dialogs
│   │   ├── AssetListView.tsx         # Virtualized list view (column resize, sort, sticky header)
│   │   ├── AssetGalleryView.tsx      # Virtualized card grid view
│   │   ├── AssetPreview.tsx          # Right-pane preview (image/3D/audio/video)
│   │   ├── CommandPalette.tsx        # ⌘K — Suggestions/Navigate/Filter/Resources/Actions
│   │   ├── AITagPanel.tsx            # Heuristic tag-suggest overlay
│   │   ├── SettingsModal.tsx         # Appearance / Git / Analysis Rules / External Editors / Maintenance
│   │   └── …                         # Dialogs, Header, Sidebar, etc.
│   ├── stores/                       # Zustand state
│   │   ├── projectStore.ts           # Multi-project hub (mirror fields for active)
│   │   ├── tagsStore.ts              # Follows active project
│   │   ├── selectionStore.ts         # Multi-select paths (lifted from AssetList)
│   │   ├── sessionStore.ts           # Cross-session restore (paths only)
│   │   ├── uiStore.ts                # Transient modal flags (cmdkOpen, aiPanelOpen, …)
│   │   ├── columnStore.ts            # Persistent list cols + viewMode (versioned)
│   │   ├── settingsStore.ts          # Persistent user prefs (Git display toggles)
│   │   ├── themeStore.ts             # dark / light / system + matchMedia listener
│   │   └── searchHistoryStore.ts     # Recent search queries
│   ├── styles/                       # globals.css + redesign-tokens(/-v2) + redesign-components
│   ├── types/asset.ts                # TS mirrors of Rust structs
│   ├── lib/                          # Shared utilities (pathUtils, platform, modelUrlResolver, utils)
│   ├── hooks/                        # React hooks (useKeyboardShortcuts)
│   ├── i18n/locales/                 # en.json + zh.json
│   ├── main.tsx                      # React entry, imports global CSS
│   └── App.tsx
├── src-tauri/                        # Rust backend
│   ├── capabilities/default.json     # Tauri 2 capability declarations
│   ├── tauri.conf.json               # Window config, bundle id, asset protocol scope
│   └── src/
│       ├── lib.rs                    # All #[tauri::command] functions
│       ├── project.rs                # Per-project state registry
│       ├── scanner.rs                # Scan + metadata dispatch
│       ├── watcher.rs                # FS watcher + fs-change events
│       ├── cache.rs                  # Disk scan cache
│       ├── analyzer/
│       │   ├── mod.rs                # Analyzer / Issue / Severity
│       │   ├── tag_suggest.rs        # Heuristic tag suggester (filename / dim+channel / path)
│       │   └── rules/                # Rule implementations
│       │       ├── naming.rs / texture.rs / texture_colorspace.rs
│       │       ├── model.rs / audio.rs                       # Per-asset (Rule trait)
│       │       ├── duplicate.rs / missing_reference.rs       # Cross-asset
│       │       └── pbr_set.rs                                # Cross-asset, per-folder grouping
│       ├── unity.rs                  # Unity YAML parsers
│       ├── unreal.rs                 # .uproject parser (deep-integration stubs)
│       ├── godot.rs                  # project.godot parser (deep-integration stubs)
│       ├── tags.rs                   # Tag system
│       ├── undo.rs                   # Undo manager
│       ├── git/mod.rs                # libgit2 wrapper
│       └── thumbnail.rs              # Image thumbnail generation + cache
├── examples/                         # User-copyable starter configs
│   └── tidycraft.example.toml        # Annotated sample rule config
├── docs/                             # Auxiliary docs
│   ├── analyzer-rules.md             # Per-rule defaults and tuning advice
│   ├── development.md                # This file
│   └── screenshots/                  # README image assets
├── CLAUDE.md                         # Claude Code project instructions
└── README.md / README.zh-CN.md       # User-facing docs
```
