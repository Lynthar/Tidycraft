<div align="center">

# 🎮 Tidycraft

**Game Asset Management & Analysis Tool**

[![Tauri](https://img.shields.io/badge/Tauri-2.0-blue?logo=tauri)](https://tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-1.75+-orange?logo=rust)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-18-61dafb?logo=react)](https://react.dev/)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue)](LICENSE)
[![CI](https://github.com/Lynthar/Tidycraft/actions/workflows/ci.yml/badge.svg)](https://github.com/Lynthar/Tidycraft/actions/workflows/ci.yml)

[English](README.md) | [简体中文](README.zh-CN.md)

*A cross-platform desktop application for scanning, browsing, and analyzing game project assets.*

</div>

---

## 📸 Screenshots

<div align="center">

**Grid view** — virtualized cards with on-demand thumbnails, 3D preview pane, and auto-grouped tag suggestions applied to matching assets.

<img src="docs/screenshots/grid-view.png" alt="Tidycraft grid view" width="85%">

<br><br>

**List view** — type-filter pills, sort pill, resizable columns with a vertex-count mini bar, sticky header.

<img src="docs/screenshots/list-view.png" alt="Tidycraft list view" width="85%">

</div>

---

## ✨ Why Tidycraft?

- **Fast scanning** — 10k+ assets in seconds via parallel walk + incremental cache.
- **Real bug detection out of the box** — duplicates (SHA256), broken Unity GUID references, sRGB-flagged data textures. Stylistic conventions (PoT, prefix, vertex budgets), PBR set completeness, and DCC source ↔ export pairing are opt-in via `tidycraft.toml`.
- **Multi-engine** — Unity / Unreal / Godot / generic; engine-specific parsers for GUID graphs, `.uproject`, `project.godot`.
- **Multi-project workspace** — open several projects simultaneously, switch between them, cross-session restore.
- **Stays out of your way** — minimal default rules; scanner respects `.gitignore` so generated artefacts stay out of view; live filesystem watcher (no manual rescan); editable rules via `Settings → Analysis Rules → Edit`.
- **Local-first** — all state on your disk; no telemetry, no network calls.

> **Status: Alpha — actively developed.** Core features (scanning, analysis, tags, 3D preview, Git, watcher) are stable. **LLM-backed AI tagging** ships in two modes: a **Learning mode** (recommended — one-time LLM call samples the project, derives local heuristic rules, reuses your existing tag system; free thereafter) and an **advanced per-asset mode** (opt-in — sends thumbnails to Claude / OpenAI / Ollama for direct tagging). Both are off until you configure a provider. See [Features → AI Tagging](#-ai-tagging) below.

---

## ⚠️ Path & Naming Best Practices

> **Important:** For optimal compatibility with 3D model preview and asset loading, please follow these guidelines.

### ✅ Recommended

- Use **ASCII characters** for file and folder names
- Use **hyphens** `-` or **underscores** `_` instead of spaces
- Keep paths **short and simple**
- Place texture files in the **same directory** as the model file

**Good Examples:**
```
/Projects/my-game/models/character_model.fbx
/Projects/my-game/textures/diffuse_map.png
```

### ❌ Avoid

| Issue | Example | Problem |
|-------|---------|---------|
| Spaces in names | `floor color.png` | May fail to load |
| Special characters | `model[v2].fbx` | Breaks path parsing |
| Non-ASCII paths | `模型/character.fbx` | Encoding issues |
| Very long paths | `>200 characters` | System limitations |

### Why These Limitations?

Some 3D model formats (FBX, OBJ, DAE) embed texture paths internally. When these paths contain special characters, the Tauri asset protocol may not resolve them correctly. This is a known platform limitation.

---

## ✨ Features

### 🔍 Asset Scanning
- **Fast async scanning** with real-time progress and cancellation
- **Project type detection** — Unity, Unreal, Godot, or generic
- **Directory tree visualization** with file counts and sizes
- **Unity .meta file parsing** — extracts GUIDs for asset tracking

### 🏷️ Tag System
- Create custom **color-coded tags** with optional descriptions (used as AI tagging context)
- Tag single or multiple assets at once
- **Filter assets by tags** (single or multi-select)
- Tags persist across sessions; rename / move syncs bindings; deleted files are reaped automatically
- **Heuristic tag suggestions** — auto-grouped by filename token / dimension + PBR channel / path segment

### ✨ AI Tagging

Multi-provider LLM tagging via **Claude** (Sonnet 4.6 / Haiku 4.5 / Opus 4.7), **OpenAI** (GPT-5.4-mini / 4o-mini / 5.4 / 5.4-nano), and **Ollama** (local — qwen2.5-VL / Llama 3.2-Vision / LLaVA / etc.; installed models listed live).

**Learning mode (default, recommended).** One-time LLM call samples your project's filenames + paths + existing tag system, infers naming/directory conventions, and emits **local heuristic rules** (filename-token / path-prefix / path-segment matching) persisted to `tidycraft.ai.toml`. The Sidebar's "Suggest Tags" panel then runs those rules locally — zero per-asset LLM cost from then on. Auto-creates tags the model thinks your vocabulary is missing (revocable from the review panel). ~$0.05 per learning run on cloud providers; free on Ollama.

**Advanced per-asset mode** (opt-in via Settings → AI Tagging → "Enable per-asset AI tagging"). Sends individual asset metadata (filename + path; thumbnails optional and OFF by default) to the LLM for direct tagging. Adds a multi-select toolbar button and right-click entry. Costs ~50× more than Learning mode — use when you need image-level analysis Learning rules can't capture.

Common to both:
- **Cost preview** before every cloud call (verified pricing math; per-asset cents shown).
- **Per-provider consent** for thumbnail uploads — first call with thumbnails on shows a checkbox; revocable in Settings.
- **Project-aware prompt** — pulls `[theme]` / `[goal]` from your project's `tidycraft.toml` and your existing tag system (with descriptions + sample paths) so the model **prefers your existing labels** instead of inventing synonyms.
- **Per-asset disk cache** keyed on `(thumbnail bytes, filename, path, provider, model, prompt version)` — partial-batch hits stay free.
- **Local + private** — Ollama path uploads nothing off-machine; cloud paths upload filenames + tag context (and thumbnails only if you opt in).

### 📊 Metadata Extraction

| Asset Type | Extracted Info |
|------------|----------------|
| **Images** | Resolution, alpha channel, format |
| **3D Models** | Vertices, faces, materials |
| **Audio** | Duration, sample rate, channels, bit depth |

### 🖼️ Asset Browser
- **List + Grid views** with virtual scrolling — handles 10,000+ files smoothly
- **Thumbnail preview** with disk caching
- **Command Palette** (⌘K / Ctrl+K) for quick navigation, filters, and actions
- **Search** by filename or path
- **Filter** by asset type
- **3D model preview** with orbit controls (glTF / GLB / FBX / OBJ / DAE / 3DS / VOX)
- **External editor mappings** — map extensions to Photoshop / Blender / Audacity / etc.

### 📋 Rule-Based Analysis

| Category | Checks |
|----------|--------|
| **Naming** | Forbidden chars, Chinese chars, prefix, case style |
| **Textures** | Power-of-two, size limits, file size, mipmap (DDS) |
| **Texture Color Space** | sRGB-tagged data textures (normal / roughness / metallic / …) |
| **Models** | Vertex / face / material limits |
| **Audio** | Sample rate, duration, mono-for-SFX, file size |
| **Duplicates** | SHA256-based content detection |
| **Missing References** (Unity) | GUID lookups against `.meta` files |
| **PBR Set Completeness** | Per-folder texture group missing channels (BaseColor / Normal / Roughness …) |
| **DCC Source Linking** | Authoring file (`.blend` / `.psd` / `.spp` / etc.) newer than its same-stem export → "needs re-export" warning |
| **Ignore Patterns** | Glob-based exclusion of vendored / generated paths |

See [`docs/analyzer-rules.md`](docs/analyzer-rules.md) for per-rule defaults and tuning advice.

---

## 📦 Supported Formats

| Category | Formats |
|----------|---------|
| **Textures** | PNG, JPG/JPEG, TGA, BMP, GIF, TIFF, WebP, HDR, EXR (decode); PSD/DDS/SVG (recognized, no thumbnail) |
| **3D Models** | glTF, GLB, FBX, OBJ (+MTL), DAE, 3DS, **VOX** (MagicaVoxel), `.blend` (detected; export to GLB to preview) |
| **Audio** | WAV, MP3, OGG |
| **Other** | Scripts, Materials, Prefabs, Scenes |

---

## 🛠️ Tech Stack

| Layer | Technology |
|-------|------------|
| **Framework** | Tauri 2.0 |
| **Backend** | Rust |
| **Frontend** | React 18 + TypeScript |
| **Styling** | Tailwind CSS |
| **State** | Zustand |
| **3D Rendering** | Three.js |
| **Virtualization** | @tanstack/react-virtual |

### Rust Crates
`image` · `gltf` · `tobj` · `fbxcel-dom` · `symphonia` · `mp4` · `matroska-demuxer` · `sha2` · `ignore` (gitignore-aware walker) · `rayon` · `toml` + `toml_edit` · `globset` · `regex` · `git2` · `notify` · `trash` · `reqwest` + `async-trait` (LLM HTTP) · `tauri-plugin-opener`

---

## 📥 Install

### From a release (recommended)

Grab the latest binary from [Releases](https://github.com/Lynthar/Tidycraft/releases) and follow your platform's first-launch step.

**Linux**

- `.deb`: `sudo dpkg -i tidycraft_*.deb`
- `.AppImage`: `chmod +x tidycraft_*.AppImage && ./tidycraft_*.AppImage`

No further steps — the app runs directly.

**Windows**

1. Run the `.msi` installer.
2. On first launch, Windows SmartScreen may flag the app as "unrecognized" — the binary isn't code-signed yet (planned, see [`SECURITY.md`](SECURITY.md)). Click **More info** → **Run anyway**.

**macOS**

1. Mount the matching `.dmg`:
   - Apple Silicon (M1 / M2 / M3 / M4): `Tidycraft_*_aarch64.dmg`
   - Intel: `Tidycraft_*_x64.dmg`
2. Drag `Tidycraft.app` into `/Applications`.
3. The app isn't notarized yet, so Gatekeeper will block it on first launch. Open **Terminal** and run:
   ```bash
   xattr -d com.apple.quarantine /Applications/Tidycraft.app
   ```
   Then open the app normally. (Alternative: right-click the app → **Open** → confirm in the prompt; or System Settings → **Privacy & Security** → **Open Anyway**.)

These first-launch hoops will go away once code signing lands (post-alpha; see [`SECURITY.md`](SECURITY.md)).

### From source (for development)

Prerequisites:

- [Node.js](https://nodejs.org/) 18+ (CI uses 20)
- [pnpm](https://pnpm.io/) 8+ (CI uses 9)
- [Rust](https://rustup.rs/) 1.75+

```bash
# Clone repository
git clone https://github.com/Lynthar/Tidycraft.git
cd Tidycraft

# Install dependencies
pnpm install

# Run in development mode
pnpm tauri dev

# Build for production
pnpm tauri build
```

---

## 📖 Usage

1. **Open Project** — Click "Open Project" (or `⌘O` / `Ctrl+O`) and select your game project folder
2. **Browse Assets** — Navigate the directory tree, switch list ↔ grid view, search, and filter
3. **Preview Assets** — Click any asset to view details, thumbnail, or 3D viewer
4. **Tag Assets** — Right-click to tag manually, or open the **AI Tag panel** for grouped suggestions (AI-derived rules if you've run Learning, heuristic fallback otherwise; Run / Re-learn / Review controls in the panel header)
5. **Run Analysis** — `⌘⇧R` / `Ctrl+Shift+R`; tweak rules via **Settings → Analysis Rules → Edit**
6. **Review Issues** — Switch to Issues tab; group by rule, filter by severity, jump to file
7. **External Editors** — Map extensions to Photoshop / Blender / etc. in **Settings → External Editors**, then the `⤴` button opens directly

---

## ⚙️ Configuration

Drop a `tidycraft.toml` in your project root and the next analysis will pick it up automatically. The Sidebar's **Run Analysis** button shows a small dot when custom rules are loaded.

**Out-of-box defaults are minimal**: only `naming.forbidden_chars` (shell-unsafe / Windows-illegal chars), `[texture.color_space]`, `duplicate`, and `missing_reference` (Unity) fire by default. Stricter checks — `[texture]` size / PoT, `[model]` budgets, `[audio]` rates, `[pbr_set]`, `[dcc_source]` — are **opt-in**: flip `enabled = true` in the relevant section.

A working starter config is at [`examples/tidycraft.example.toml`](examples/tidycraft.example.toml) — copy to your project root, rename to `tidycraft.toml`, and tweak. For per-rule explanations and tuning advice see [`docs/analyzer-rules.md`](docs/analyzer-rules.md). Quick reference (values below are the actual built-in defaults):

```toml
[naming]
enabled = true
forbidden_chars = [' ', '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '+', '=']
forbid_chinese = false
max_length = 512                       # loose; tighten to 64-96 for strict pipelines
# texture_prefix = "T_"                # uncomment to require a per-type prefix
# model_prefix = "SM_"
# audio_prefix = "A_"
case_style = "any"                     # any | snake_case | kebab-case | PascalCase | camelCase

[texture]                              # disabled by default — enable for stricter image rules
enabled = false
require_pot = true
max_size = 4096
min_size = 4
warn_non_square = false
max_file_size = 10_485_760

[texture.color_space]                  # enabled by default; catches sRGB-tagged data textures
enabled = true

[model]                                # disabled by default
enabled = false
max_vertices = 100_000
max_faces = 100_000
max_materials = 10

[audio]                                # disabled by default
enabled = false
allowed_sample_rates = [44_100, 48_000]
max_sfx_duration = 30.0
max_file_size = 20_971_520
prefer_mono_for_sfx = false

[pbr_set]                              # disabled by default; per-folder PBR completeness
enabled = false
trigger = "basecolor"
required = ["basecolor", "normal"]

[dcc_source]                           # disabled by default; pairs DCC sources with exports
enabled = false
mtime_tolerance_secs = 60              # absorbs git-checkout mtime sync
# Default mappings cover Blender / Maya / Max / ZBrush / Modo / Houdini /
# Cinema 4D / Marvelous / Substance Painter+Designer / Photoshop.
# See docs/analyzer-rules.md or examples/tidycraft.example.toml for the
# full mapping list and lookup options.

# Scanner already respects .gitignore by default (Settings → Scanning),
# so Library/ / Intermediate/ etc. are typically skipped at scan time.
# These [ignore] patterns apply at analyze time — useful for muting rule
# output on vendored / third-party content that IS scanned.
[ignore]
patterns = [
    # "ThirdParty/**",
    # "Plugins/**",
    # "**/_legacy/**",
]
```

Any field can be omitted — missing fields fall back to defaults.

---

## 📁 Project Structure

```
tidycraft/
├── src/                    # React frontend
│   ├── components/         # UI components
│   ├── stores/             # Zustand state
│   ├── styles/             # Global CSS + Forge design tokens
│   ├── types/              # TypeScript types
│   ├── hooks/              # React hooks
│   ├── i18n/locales/       # en.json + zh.json
│   └── lib/                # Utilities (pathUtils, platform detect, …)
├── src-tauri/              # Rust backend
│   └── src/
│       ├── scanner.rs      # Asset scanning (gitignore-aware walker)
│       ├── watcher.rs      # FS watcher → fs-change events
│       ├── analyzer/       # Rule engine (naming / texture / model / audio / duplicate / missing_reference / pbr_set / dcc_source) + tag suggesters
│       ├── llm/            # Multi-provider AI tagging (Claude / OpenAI / Ollama) + Learning mode
│       ├── thumbnail.rs    # Thumbnail generation
│       ├── tags.rs         # Tag management
│       └── lib.rs          # Tauri commands
├── docs/                   # Auxiliary docs
│   ├── analyzer-rules.md   # Per-rule defaults and tuning advice
│   ├── development.md      # Developer guide (architecture, contributing)
│   └── screenshots/        # README image assets
├── examples/               # Starter `tidycraft.example.toml`
└── README.md               # User-facing docs (this file)
```

---

## 🔒 Privacy & Data

Tidycraft is **local-first by design**:

- **No telemetry, no analytics, no network calls** in the current build (v0.x).
- **All state lives on your disk**: scan cache (`~/.cache/tidycraft/` or the platform equivalent), tag bindings (`.tidycraft-tags.json` per project), undo history, thumbnail cache, settings.
- **No account, no sign-in.** Open the app and use it.
- **AI tag suggestions** are **opt-in only** — no calls happen until you configure a provider in Settings → AI Tagging. First call per cloud provider with thumbnails on shows an explicit consent dialog (revocable). Local provider (Ollama) uploads nothing off-machine; cloud paths (Claude / OpenAI) upload filenames + tag context, and thumbnails only if you opt in. See [`SECURITY.md`](SECURITY.md) for the full upload-payload disclosure.

---

## 🗺️ Roadmap

Shipped:

- [x] Dependency analysis & reference tracking (Unity GUID graph, unused-asset detection)
- [x] Statistics dashboard & reports
- [x] Git integration (branch info, per-file change status)
- [x] Incremental scanning (mtime/size cache)
- [x] Batch rename operations (with persistent undo)
- [x] Export reports (JSON, CSV, HTML)
- [x] Live filesystem watcher (auto-refresh on file changes)
- [x] Multi-project workspace + cross-session restore
- [x] Tag system with multi-select filtering + heuristic AI suggestions
- [x] Safe delete / move / copy / duplicate (OS trash) with automatic tag sync
- [x] Forge Dark visual redesign (full migration)
- [x] Command Palette (⌘K), List / Gallery views
- [x] Settings → Analysis Rules editor + per-project `tidycraft.toml`
- [x] PBR set completeness analyzer (per-folder texture group)
- [x] External editor mappings (Settings → External Editors, per-extension)
- [x] Cross-platform polish (macOS ⌘ glyphs, Windows file-manager reveal fix, path utils)
- [x] DCC source-file linking (`.blend` / `.psd` / `.spp` / `.ma` / `.ztl` / `.max` / `.lxo` / `.hip` / `.c4d` / `.zprj` / `.sbs` / `.psb` → "source newer than export" warnings, opt-in)
- [x] AI Tagging — Learning mode (project sampling → local rules persisted to `tidycraft.ai.toml`; zero per-asset LLM cost after the one-time run) + advanced per-asset mode (multi-provider LLM with cost preview + per-provider consent; opt-in)
- [x] Scanner respects `.gitignore` / `.ignore` by default via `ignore::WalkBuilder` (toggleable in Settings → Scanning)

Backlog:

- [ ] VRAM budget estimates (per texture, per directory)
- [ ] Cross-engine reverse-reference graph (extend Unity GUID graph to UE / Godot)

---

## 📄 License

[Apache 2.0](LICENSE)

---

<div align="center">

Made with ❤️ for game developers

</div>
