# Contributing to Tidycraft

Welcome. This guide is for anyone extending Tidycraft — whether you're fixing
a bug, adding a feature, or forking the project. It aims to give you enough
context to read the codebase productively within an hour.

For the user-facing introduction, see [README.md](README.md) /
[README.zh-CN.md](README.zh-CN.md). For the active visual-redesign roadmap,
see [REDESIGN.md](REDESIGN.md). For pre-redesign history, use `git log`.

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
`tobj`, `symphonia` (audio), `git2`, `notify` + `notify-debouncer-full` (FS
events), `trash` (safe delete), `parking_lot` (non-poisoning mutexes),
`tauri-plugin-dialog` / `tauri-plugin-fs` / `tauri-plugin-clipboard-manager`.

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
cargo test --lib          # 70+ unit tests
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
│  - Zustand stores         │       emit(event, payload)       │  - ~50 tauri commands   │
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
  `applies_to`, `check`. Existing rules cover naming, texture, model, audio
  standards plus duplicate detection. `RuleConfig` is deserialized from
  `tidycraft.toml`; `Analyzer::with_config` wires enabled rules.
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
  `settingsOpen`, `tagManagerOpen`). Global store, not App-level state, so
  `CommandPalette` can open Settings / TagManager without prop drilling.
  Distinct from `settingsStore` (which holds persisted user preferences).
- Other global stores: `settingsStore`, `themeStore`, `columnStore`,
  `searchHistoryStore`.
- **`components/`** — Flat layout, one component per file, no barrel exports.
  `AssetList` is the virtualized file list (the central UI). `ContextMenu` +
  dialogs (`RenameDialog`, `BatchRenameDialog`, `DeleteConfirmDialog`,
  `MoveCopyDialog`) handle operations. `ModelViewer3D` / `ModelLightbox` do
  3D preview via Three.js. Dialog pattern: the parent owns a nullable state
  object (`{ mode, paths }` or `paths | null`), renders the dialog
  conditionally, passes `onClose` and `onDone` callbacks.
- **`lib/modelUrlResolver.ts`** — Builds a Three.js `LoadingManager` URL
  modifier pre-seeded with a sibling-texture map fetched from the backend.
  See §4.5.
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

### A new analyzer rule

1. Create a new file under `src-tauri/src/analyzer/rules/`.
2. Define a `Config` struct with `#[serde(default)]` fields and a `Default`
   impl (so missing keys in `tidycraft.toml` use sensible defaults).
3. Implement the `Rule` trait (`id`, `name`, `applies_to`, `check`).
4. Add the config to `RuleConfig` in `analyzer/rules/mod.rs`.
5. Register in `Analyzer::with_config` (in `analyzer/mod.rs`).
6. Write unit tests in the same file.

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

**Rust side:** `cargo test --lib` from `src-tauri/`. Currently 70+ tests
across `scanner`, `analyzer`, `watcher`, `undo`, `tags`, `unity`, `unreal`,
`godot`, `cache`. Aim to add tests for new parsers and pure functions. Skip
integration tests that spawn the full Tauri runtime — the payoff isn't worth
the complexity.

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
  path without normalizing first.
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
- **Keyboard shortcuts display.** `useKeyboardShortcuts` handles both
  `ctrlKey` and `metaKey` for detection, but the `SHORTCUTS` display table
  hardcodes `"Ctrl"`. macOS users see incorrect labels. Low priority.

---

## 8. Contribution Workflow

1. **Read `REDESIGN.md`** before starting. It tracks the active visual-
   redesign phases and the locked design decisions; the phase table tells
   you what's in flight and what's on deck.
2. **Pick a task** from the phase table, an open issue, or the README's
   "Backlog" section. For larger work, open an issue first to discuss scope.
3. **Follow the existing style.** Rust: `rustfmt` defaults. TS: match nearby
   files. Prefer editing existing files to creating new abstractions.
4. **Commit in focused chunks.** One-line subject, blank line, a short body
   explaining **why** (the what is in the diff). Group related changes;
   don't bundle unrelated work.
5. **Update `REDESIGN.md`** when you finish or reprioritize a phase:
   - Update the phase-status table.
   - Add a dated note under the relevant phase section recording *改动
     (what changed)*, *为什么 (why)*, and *影响面 (blast radius / caveats)*.
6. **Verify locally** before pushing: `cargo test --lib`, `pnpm build`, and a
   quick `pnpm tauri dev` sanity check.
7. **Open the PR** with a summary that mirrors the commit body.

---

## 9. File Layout Reference

```
tidycraft/
├── src/                              # React frontend
│   ├── components/                   # UI (flat, one per file)
│   ├── stores/                       # Zustand state
│   │   ├── projectStore.ts           # Multi-project hub
│   │   ├── tagsStore.ts              # Follows active project
│   │   ├── sessionStore.ts           # Cross-session restore (paths only)
│   │   └── uiStore.ts                # Transient modal flags (cmdkOpen, etc.)
│   ├── styles/                       # globals.css + redesign-tokens(/-v2) + redesign-components
│   ├── types/asset.ts                # TS mirrors of Rust structs
│   ├── lib/                          # Shared utilities
│   ├── hooks/                        # React hooks
│   ├── i18n/locales/                 # en.json + zh.json
│   ├── main.tsx                      # React entry, imports global CSS
│   └── App.tsx
├── src-tauri/                        # Rust backend
│   └── src/
│       ├── lib.rs                    # All #[tauri::command] functions
│       ├── project.rs                # Per-project state registry
│       ├── scanner.rs                # Scan + metadata dispatch
│       ├── watcher.rs                # FS watcher + fs-change events
│       ├── cache.rs                  # Disk scan cache
│       ├── analyzer/
│       │   ├── mod.rs                # Analyzer / Issue / Severity
│       │   └── rules/                # Rule implementations
│       ├── unity.rs                  # Unity YAML parsers
│       ├── unreal.rs                 # .uproject parser
│       ├── godot.rs                  # project.godot parser
│       ├── tags.rs                   # Tag system
│       ├── undo.rs                   # Undo manager
│       ├── git/mod.rs                # libgit2 wrapper
│       └── thumbnail.rs              # Image thumbnail generation
├── REDESIGN.md                       # Visual redesign phase tracker
├── CONTRIBUTING.md                   # This file
├── README.md / README.zh-CN.md       # User-facing docs
└── tidycraft.toml                    # Optional user config (see README)
```
