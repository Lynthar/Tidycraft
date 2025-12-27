# Tidycraft

**Game Asset Management & Analysis Tool**

A cross-platform desktop application for scanning, browsing, and analyzing game project assets. Built with Tauri 2.0 (Rust) and React.

---

## Features

### Asset Scanning
- **Fast async scanning** with real-time progress tracking and cancellation support
- **Project type detection** - Unity, Unreal, Godot, or generic projects
- **Directory tree visualization** with file counts and size statistics
- **Unity .meta file parsing** - extracts GUIDs for asset tracking

### Supported Formats

| Category | Formats |
|----------|---------|
| **Textures** | PNG, JPG/JPEG, TGA, BMP, GIF |
| **3D Models** | glTF/GLB, OBJ |
| **Audio** | WAV, MP3, OGG |
| **Other** | Scripts, Materials, Prefabs, Scenes, Data files |

### Asset Metadata Extraction

- **Images**: Resolution, alpha channel detection
- **Models**: Vertex count, face count, material count
- **Audio**: Duration, sample rate, channels, bit depth

### Asset Browser
- **Thumbnail preview** with disk caching (SHA256-based)
- **Virtual scrolling** for large asset lists (10,000+ files)
- **Search** by filename or path
- **Filter** by asset type
- **Asset preview panel** with detailed metadata

### Rule-Based Analysis

Configurable rules to detect common issues:

| Rule Category | Checks |
|---------------|--------|
| **Naming** | Forbidden characters, Chinese characters, prefix conventions, case style |
| **Textures** | Power-of-two dimensions, maximum size limits |
| **Models** | Vertex/face/material count limits |
| **Audio** | Sample rate standards, duration limits |
| **Duplicates** | SHA256 content-based detection |

- **TOML configuration** for custom rule settings
- **Issue severity levels**: Error, Warning, Info
- **Locate asset** - jump from issue to asset in browser

---

## Tech Stack

| Layer | Technology |
|-------|------------|
| **Framework** | Tauri 2.0 |
| **Backend** | Rust |
| **Frontend** | React 18 + TypeScript |
| **Styling** | Tailwind CSS |
| **State** | Zustand |
| **Virtualization** | @tanstack/react-virtual |

### Rust Crates
- `image` - Image parsing and thumbnail generation
- `gltf` - glTF/GLB model parsing
- `tobj` - OBJ model parsing
- `symphonia` - Audio metadata extraction
- `sha2` - File hashing for duplicates/caching
- `walkdir` - Directory traversal
- `toml` - Configuration parsing

---

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) 1.70+
- [Tauri CLI](https://tauri.app/v1/guides/getting-started/prerequisites)

### Installation

```bash
# Clone repository
git clone https://github.com/your-username/tidycraft.git
cd tidycraft

# Install dependencies
npm install

# Run in development mode
npm run tauri dev

# Build for production
npm run tauri build
```

---

## Usage

1. **Open Project** - Click "Open Project" and select your game project folder
2. **Browse Assets** - Navigate the directory tree, search, and filter assets
3. **Preview Assets** - Click any asset to view details and thumbnail
4. **Run Analysis** - Click "Run Analysis" to check for issues
5. **Review Issues** - Switch to Issues tab to see problems and suggestions

### Configuration

Create a `tidycraft.toml` in your project root to customize rules:

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

## Project Structure

```
tidycraft/
├── src/                    # React frontend
│   ├── components/         # UI components
│   ├── stores/             # Zustand state
│   ├── types/              # TypeScript types
│   └── lib/                # Utilities
├── src-tauri/              # Rust backend
│   └── src/
│       ├── scanner/        # Asset scanning
│       ├── analyzer/       # Rule engine
│       │   └── rules/      # Individual rules
│       ├── thumbnail.rs    # Thumbnail generation
│       └── lib.rs          # Tauri commands
└── docs/                   # Documentation
```

---

## Roadmap

- [ ] Dependency analysis & reference tracking
- [ ] Statistics dashboard & reports
- [ ] Git integration (change detection)
- [ ] Incremental scanning
- [ ] Batch operations
- [ ] Custom rule scripting
- [ ] Export reports (JSON, CSV, HTML)

---

## License

MIT
