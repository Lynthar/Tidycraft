<div align="center">

# 🎮 Tidycraft

**Game Asset Management & Analysis Tool**

[![Tauri](https://img.shields.io/badge/Tauri-2.0-blue?logo=tauri)](https://tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-1.75+-orange?logo=rust)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-18-61dafb?logo=react)](https://react.dev/)
[![License](https://img.shields.io/badge/License-MIT-green)](LICENSE)

[English](README.md) | [简体中文](README.zh-CN.md)

*A cross-platform desktop application for scanning, browsing, and analyzing game project assets.*

</div>

---

## 📸 Screenshots

<!-- TODO: Add screenshots -->
<div align="center">
<i>Screenshots coming soon...</i>
</div>

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
- Create custom **color-coded tags**
- Tag single or multiple assets at once
- **Filter assets by tags** (single or multi-select)
- Tags persist across sessions

### 📊 Metadata Extraction

| Asset Type | Extracted Info |
|------------|----------------|
| **Images** | Resolution, alpha channel, format |
| **3D Models** | Vertices, faces, materials |
| **Audio** | Duration, sample rate, channels, bit depth |

### 🖼️ Asset Browser
- **Thumbnail preview** with disk caching
- **Virtual scrolling** — handles 10,000+ files smoothly
- **Search** by filename or path
- **Filter** by asset type
- **3D model preview** with orbit controls

### 📋 Rule-Based Analysis

| Category | Checks |
|----------|--------|
| **Naming** | Forbidden chars, Chinese chars, prefix, case style |
| **Textures** | Power-of-two, max size |
| **Models** | Vertex/face/material limits |
| **Audio** | Sample rate, duration |
| **Duplicates** | SHA256-based detection |

---

## 📦 Supported Formats

| Category | Formats |
|----------|---------|
| **Textures** | PNG, JPG/JPEG, TGA, BMP, GIF |
| **3D Models** | glTF, GLB, FBX, OBJ (+MTL), DAE |
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
`image` · `gltf` · `tobj` · `symphonia` · `sha2` · `walkdir` · `toml` · `git2` · `rayon`

---

## 🚀 Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [pnpm](https://pnpm.io/) 8+
- [Rust](https://rustup.rs/) 1.75+

### Installation

```bash
# Clone repository
git clone https://github.com/AquaStarfish/Tidycraft.git
cd tidycraft

# Install dependencies
pnpm install

# Run in development mode
pnpm tauri dev

# Build for production
pnpm tauri build
```

---

## 📖 Usage

1. **Open Project** — Click "Open Project" and select your game project folder
2. **Browse Assets** — Navigate the directory tree, search, and filter
3. **Preview Assets** — Click any asset to view details and preview
4. **Tag Assets** — Right-click to add tags for organization
5. **Run Analysis** — Click "Run Analysis" to check for issues
6. **Review Issues** — Switch to Issues tab to see problems

---

## ⚙️ Configuration

Create a `tidycraft.toml` in your project root:

```toml
[naming]
check_forbidden_chars = true
forbidden_chars = ['<', '>', ':', '"', '|', '?', '*']
check_chinese = true
check_prefix = false
required_prefix = "tex_"

[texture]
check_pot = true
check_max_size = true
max_width = 4096
max_height = 4096

[model]
check_vertex_count = true
max_vertices = 100000
check_face_count = true
max_faces = 50000

[audio]
check_sample_rate = true
allowed_sample_rates = [44100, 48000]
check_duration = true
max_duration_secs = 300.0

[duplicate]
enabled = true
```

---

## 📁 Project Structure

```
tidycraft/
├── src/                    # React frontend
│   ├── components/         # UI components
│   ├── stores/             # Zustand state
│   ├── types/              # TypeScript types
│   └── lib/                # Utilities
├── src-tauri/              # Rust backend
│   └── src/
│       ├── scanner.rs      # Asset scanning
│       ├── analyzer.rs     # Rule engine
│       ├── thumbnail.rs    # Thumbnail generation
│       ├── tags.rs         # Tag management
│       └── lib.rs          # Tauri commands
└── docs/                   # Documentation
```

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
- [x] Tag system with multi-select filtering
- [x] Safe delete / move / copy / duplicate (OS trash)

In progress:

- [ ] Visual redesign — Forge Dark theme migration (see `REDESIGN.md`)
- [ ] Command Palette (⌘K)
- [ ] Gallery / grid view
- [ ] AI tag suggestions

Backlog:

- [ ] Custom rule scripting (`tidycraft.toml` is parsed but not yet wired through the UI)

---

## 📄 License

[MIT](LICENSE)

---

<div align="center">

Made with ❤️ for game developers

</div>
