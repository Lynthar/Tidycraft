<div align="center">

# 🎮 Tidycraft

**游戏资源管理与分析工具**

[![Tauri](https://img.shields.io/badge/Tauri-2.0-blue?logo=tauri)](https://tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-1.75+-orange?logo=rust)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-18-61dafb?logo=react)](https://react.dev/)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue)](LICENSE)
[![CI](https://github.com/Lynthar/Tidycraft/actions/workflows/ci.yml/badge.svg)](https://github.com/Lynthar/Tidycraft/actions/workflows/ci.yml)

[English](README.md) | [简体中文](README.zh-CN.md)

*一款跨平台桌面应用，用于扫描、浏览和分析游戏项目资源。*

</div>

---

## 📸 截图

<div align="center">

**网格视图** — 虚拟化卡片 + 按需缩略图，右侧 3D 预览面板，自动分组的标签建议已应用到匹配资产上。

<img src="docs/screenshots/grid-view.png" alt="Tidycraft 网格视图" width="85%">

<br><br>

**列表视图** — 类型筛选 pill、排序 pill、可拖动调整的列宽 + 顶点数微型条形图，sticky 表头。

<img src="docs/screenshots/list-view.png" alt="Tidycraft 列表视图" width="85%">

</div>

---

## ✨ 为什么用 Tidycraft?

- **扫描快** — 万级资产秒级完成,基于并行遍历 + 增量缓存。
- **out-of-box 抓真 bug** — 重复文件(SHA256)、Unity GUID 缺失引用、被错标 sRGB 的数据贴图。Stylistic 约定(PoT、前缀、多边形预算)、PBR 材质组完整性、DCC 源 ↔ 导出关联都通过 `tidycraft.toml` 按需开启。
- **多引擎** — Unity / Unreal / Godot / 通用;引擎特定解析器(GUID 图、`.uproject`、`project.godot`)。
- **多项目工作区** — 同时打开多个项目,自由切换,跨会话恢复。
- **不打扰你** — 默认规则极简;扫描器默认遵守 `.gitignore`,自动跳过生成产物;文件系统 watcher 实时同步(无需手动重扫);通过 `Settings → Analysis Rules → Edit` 直接编辑规则。
- **本地优先** — 所有状态都在你硬盘上;无遥测、无网络调用。

> **状态:Alpha — 持续开发中。** 核心功能(扫描、分析、标签、3D 预览、Git、Watcher)已稳定。**基于 LLM 的 AI 标签**已 ship,提供两种模式:**学习模式**(推荐 — 一次 LLM 调用采样项目、推断本地启发式规则,复用你已有的标签系统;之后免费)和**高级单资产模式**(opt-in — 把缩略图发给 Claude / OpenAI / Ollama 直接打标)。两种模式都默认关闭,需要先在 Settings → AI Tagging 里配置 provider。详见下方 [功能特性 → AI 标签](#-ai-标签)。

---

## ⚠️ 路径与命名最佳实践

> **重要提示：** 为确保 3D 模型预览和资源加载的兼容性，请遵循以下指南。

### ✅ 推荐做法

- 文件和文件夹名称使用 **ASCII 字符**
- 使用 **连字符** `-` 或 **下划线** `_` 代替空格
- 保持路径 **简短**
- 将纹理文件放在与模型文件 **相同的目录** 中

**正确示例：**
```
/Projects/my-game/models/character_model.fbx
/Projects/my-game/textures/diffuse_map.png
```

### ❌ 应避免

| 问题 | 示例 | 影响 |
|------|------|------|
| 名称包含空格 | `floor color.png` | 可能加载失败 |
| 特殊字符 | `model[v2].fbx` | 路径解析错误 |
| 非 ASCII 路径 | `模型/character.fbx` | 编码问题 |
| 路径过长 | `>200 字符` | 系统限制 |

### 为什么有这些限制？

某些 3D 模型格式（FBX、OBJ、DAE）会在内部嵌入纹理路径。当这些路径包含特殊字符时，Tauri 资源协议可能无法正确解析。这是平台的已知限制。

---

## ✨ 功能特性

### 🔍 资源扫描
- **快速异步扫描**，支持实时进度显示和取消
- **项目类型检测** — Unity、Unreal、Godot 或通用项目
- **目录树可视化**，显示文件数量和大小统计
- **Unity .meta 文件解析** — 提取 GUID 用于资源追踪

### 🏷️ 标签系统
- 创建自定义 **彩色标签**,可选填描述(用作 AI 标签上下文)
- 支持单个或批量添加标签
- **按标签筛选资源**（单选或多选）
- 标签数据跨会话持久保存；重命名 / 移动会自动同步绑定，删除文件后自动清理孤儿
- **启发式标签推荐** — 按文件名 token / 尺寸+PBR 通道 / 路径段自动分组

### ✨ AI 标签

多 provider LLM 标签:**Claude**(Sonnet 4.6 / Haiku 4.5 / Opus 4.7)、**OpenAI**(GPT-5.4-mini / 4o-mini / 5.4 / 5.4-nano)、**Ollama**(本地 — qwen2.5-VL / Llama 3.2-Vision / LLaVA 等;已安装模型实时列出)。

**学习模式(推荐,默认路径)。** 一次 LLM 调用采样你项目的文件名 + 路径 + 已有标签系统,推断命名 / 目录约定,生成**本地启发式规则**(filename-token / path-prefix / path-segment 匹配)并持久化到 `tidycraft.ai.toml`。侧边栏的 "Suggest Tags" 面板之后跑这些规则在本地匹配 — 自此每条资产 LLM 成本为零。会自动创建模型认为你词汇里缺失的标签(可在审查面板撤回)。云 provider 每次学习运行约 ~$0.05;Ollama 免费。

**高级单资产模式**(通过 Settings → AI Tagging → "启用单资产 AI 标签" opt-in)。把单个资产元数据(文件名 + 路径;缩略图可选且默认关闭)发给 LLM 直接打标。会在多选工具栏加按钮 + 右键菜单加入口。比学习模式贵约 50× — 适合需要图像级分析、学习规则覆盖不到的场景。

两种模式共有:
- **每次云调用前预览成本**(verified pricing 算式;显示单资产美分数)。
- **每 provider 的缩略图上传同意流程** — 首次带缩略图调用时弹复选框;可在 Settings 撤回。
- **项目感知 prompt** — 从你项目的 `tidycraft.toml` 拉 `[theme]` / `[goal]` + 你已有标签系统(含描述 + 样例路径),让模型**优先复用你已有的标签**而不是发明同义词。
- **单资产磁盘缓存**,键为 `(缩略图字节, 文件名, 路径, provider, model, prompt 版本)` — 部分命中保持免费。
- **本地 + 私密** — Ollama 路径不上传任何东西离机;云路径上传文件名 + 标签上下文(opt-in 时才上传缩略图)。

### 📊 元数据提取

| 资源类型 | 提取信息 |
|----------|----------|
| **图片** | 分辨率、Alpha 通道、格式 |
| **3D 模型** | 顶点数、面数、材质数 |
| **音频** | 时长、采样率、声道数、位深度 |

### 🖼️ 资源浏览器
- **列表 + 网格双视图** + 虚拟滚动 — 流畅处理 10,000+ 文件
- **缩略图预览**，支持磁盘缓存
- **命令面板**（⌘K / Ctrl+K）快速导航、筛选与执行操作
- 按文件名或路径 **搜索**
- 按资源类型 **筛选**
- **3D 模型预览**，支持轨道控制（glTF / GLB / FBX / OBJ / DAE / 3DS / VOX）
- **外部编辑器映射** — 把扩展名映射到 Photoshop / Blender / Audacity 等

### 📋 规则分析

| 类别 | 检查项 |
|------|--------|
| **命名** | 禁用字符、中文字符、前缀、大小写风格 |
| **纹理** | 2 的幂次方、尺寸限制、文件大小、mipmap (DDS) |
| **纹理色彩空间** | 被标为 sRGB 的数据贴图（normal / roughness / metallic …）|
| **模型** | 顶点 / 面 / 材质数量限制 |
| **音频** | 采样率、时长、SFX 单声道、文件大小 |
| **重复文件** | 基于 SHA256 的内容比对 |
| **缺失引用**（Unity） | 在 `.meta` 文件中查找 GUID |
| **PBR Set 完整性** | 按目录分组的纹理集是否齐全（BaseColor / Normal / Roughness …）|
| **DCC 源文件关联** | 作者源文件（`.blend` / `.psd` / `.spp` 等）比同名导出新 → "需重新导出"提示 |
| **忽略规则** | 基于 glob 的路径排除（外部资源 / 生成产物）|

详见 [`docs/analyzer-rules.md`](docs/analyzer-rules.md)（每条规则的默认值与调参建议）。

---

## 📦 支持的格式

| 类别 | 格式 |
|------|------|
| **纹理** | PNG, JPG/JPEG, TGA, BMP, GIF, TIFF, WebP, HDR, EXR(解码);PSD / DDS / SVG(识别,无缩略图)|
| **3D 模型** | glTF, GLB, FBX, OBJ (+MTL), DAE, 3DS, **VOX**(MagicaVoxel),`.blend`（识别但不能直接渲染，请先在 Blender 中导出 GLB）|
| **音频** | WAV, MP3, OGG |
| **其他** | 脚本、材质、预制体、场景 |

---

## 🛠️ 技术栈

| 层级 | 技术 |
|------|------|
| **框架** | Tauri 2.0 |
| **后端** | Rust |
| **前端** | React 18 + TypeScript |
| **样式** | Tailwind CSS |
| **状态管理** | Zustand |
| **3D 渲染** | Three.js |
| **虚拟化** | @tanstack/react-virtual |

### Rust 依赖
`image` · `gltf` · `tobj` · `fbxcel-dom` · `symphonia` · `mp4` · `matroska-demuxer` · `sha2` · `ignore`(gitignore-aware walker)· `rayon` · `toml` + `toml_edit` · `globset` · `regex` · `git2` · `notify` · `trash` · `reqwest` + `async-trait`(LLM HTTP)· `tauri-plugin-opener`

---

## 🚀 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) 18+（CI 用 20）
- [pnpm](https://pnpm.io/) 8+（CI 用 9）
- [Rust](https://rustup.rs/) 1.75+

### 安装步骤

```bash
# 克隆仓库
git clone https://github.com/Lynthar/Tidycraft.git
cd Tidycraft

# 安装依赖
pnpm install

# 开发模式运行
pnpm tauri dev

# 构建生产版本
pnpm tauri build
```

---

## 📖 使用方法

1. **打开项目** — 点击"打开项目"（或 `⌘O` / `Ctrl+O`）并选择游戏项目文件夹
2. **浏览资源** — 导航目录树、列表 / 网格视图切换、搜索、筛选
3. **预览资源** — 点击任意资源查看详情、缩略图或 3D 视图
4. **标记资源** — 右键添加标签,或打开 **AI 标签面板** 查看分组建议(若已运行学习,跑 AI 派生的本地规则;否则启发式 fallback。面板顶部含 Run / Re-learn / Review 控件)
5. **运行分析** — `⌘⇧R` / `Ctrl+Shift+R`；通过 **Settings → Analysis Rules → Edit** 调整规则
6. **查看问题** — 切换到问题选项卡，按规则分组、按严重度筛选、跳转到文件
7. **外部编辑器** — 在 **Settings → External Editors** 把扩展名映射到 Photoshop / Blender 等，预览面板的 `⤴` 直接打开

---

## ⚙️ 配置说明

把 `tidycraft.toml` 放到项目根目录，下次 Run Analysis 会自动加载。侧边栏的 **运行分析** 按钮上会出现一个小圆点提示当前使用了自定义规则。

**Out-of-box 默认规则非常宽松**：仅 `naming.forbidden_chars`（shell-unsafe / Windows 非法字符）、`[texture.color_space]`、`duplicate`、`missing_reference`（Unity）默认开启。更严格的检查 —— `[texture]` 尺寸 / PoT、`[model]` 预算、`[audio]` 采样率、`[pbr_set]`、`[dcc_source]` —— 都是**按需启用**：把对应段的 `enabled = true` 即可。

可用的样例文件：[`examples/tidycraft.example.toml`](examples/tidycraft.example.toml) —— 复制到你的项目根目录、改名为 `tidycraft.toml`，按需调整即可。每条规则的含义和调参建议见 [`docs/analyzer-rules.md`](docs/analyzer-rules.md)。字段速查（下方值是实际内置默认）：

```toml
[naming]
enabled = true
forbidden_chars = [' ', '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '+', '=']
forbid_chinese = false
max_length = 512                       # 宽松；严格 pipeline 可调到 64-96
# texture_prefix = "T_"                # 取消注释强制纹理用此前缀
# model_prefix = "SM_"
# audio_prefix = "A_"
case_style = "any"                     # any | snake_case | kebab-case | PascalCase | camelCase

[texture]                              # 默认 disabled —— 需要严格图像规则时再开
enabled = false
require_pot = true
max_size = 4096
min_size = 4
warn_non_square = false
max_file_size = 10_485_760

[texture.color_space]                  # 默认 enabled；捕获 sRGB-标记数据贴图陷阱
enabled = true

[model]                                # 默认 disabled
enabled = false
max_vertices = 100_000
max_faces = 100_000
max_materials = 10

[audio]                                # 默认 disabled
enabled = false
allowed_sample_rates = [44_100, 48_000]
max_sfx_duration = 30.0
max_file_size = 20_971_520
prefer_mono_for_sfx = false

[pbr_set]                              # 默认 disabled；按目录的 PBR 完整性
enabled = false
trigger = "basecolor"
required = ["basecolor", "normal"]

[dcc_source]                           # 默认 disabled;DCC 源文件 ↔ 导出 mtime 配对
enabled = false
mtime_tolerance_secs = 60              # 吸收 git checkout 的 mtime 同步
# 默认 mappings 覆盖 Blender / Maya / Max / ZBrush / Modo / Houdini /
# Cinema 4D / Marvelous / Substance Painter+Designer / Photoshop。
# 完整 mapping 列表和 lookup 选项见 docs/analyzer-rules.md 或
# examples/tidycraft.example.toml。

# 注:扫描器默认已经遵守 .gitignore(Settings → Scanning),所以 Library/
# Intermediate/ 等通常在扫描阶段就跳过了。下面这些 [ignore].patterns
# 在分析阶段生效 — 用于静音那些**被扫到的**第三方/外部内容上的规则输出。
[ignore]
patterns = [
    # "ThirdParty/**",
    # "Plugins/**",
    # "**/_legacy/**",
]
```

任何字段都可省略，缺失的字段会回退到默认值。

---

## 📁 项目结构

```
tidycraft/
├── src/                    # React 前端
│   ├── components/         # UI 组件
│   ├── stores/             # Zustand 状态
│   ├── styles/             # 全局 CSS + Forge 设计 token
│   ├── types/              # TypeScript 类型
│   ├── hooks/              # React hooks
│   ├── i18n/locales/       # en.json + zh.json
│   └── lib/                # 工具函数（pathUtils、平台检测等）
├── src-tauri/              # Rust 后端
│   └── src/
│       ├── scanner.rs      # 资源扫描（gitignore-aware walker）
│       ├── watcher.rs      # 文件系统 watcher → fs-change 事件
│       ├── analyzer/       # 规则引擎（naming / texture / model / audio / duplicate / missing_reference / pbr_set / dcc_source）+ 标签推荐
│       ├── llm/            # 多 provider AI 标签（Claude / OpenAI / Ollama）+ 学习模式
│       ├── thumbnail.rs    # 缩略图生成
│       ├── tags.rs         # 标签管理
│       └── lib.rs          # Tauri 命令
├── docs/                   # 辅助文档
│   ├── analyzer-rules.md   # 各规则默认值与调优说明
│   ├── development.md      # 开发者指南（架构、贡献流程）
│   └── screenshots/        # README 截图
├── examples/               # `tidycraft.example.toml` 起始模板
└── README.md               # 用户文档（本文件）
```

---

## 🔒 隐私与数据

Tidycraft **本地优先**:

- **无遥测、无 analytics、无网络调用**(v0.x 构建)。
- **所有状态在你的磁盘上**:扫描缓存(`~/.cache/tidycraft/` 或平台等价目录)、标签绑定(每项目 `.tidycraft-tags.json`)、撤销历史、缩略图缓存、设置。
- **无账户、无登录**,打开就用。
- **AI 标签建议器**(已 ship)**完全 opt-in** —— 在你于 Settings → AI Tagging 配置 provider 之前没有任何调用。首次对云 provider 启用缩略图调用时弹出明确的同意对话框(可撤回)。本地 provider(Ollama)不上传任何东西离机;云路径(Claude / OpenAI)上传文件名 + 标签上下文,只在你 opt in 时才上传缩略图。完整上传 payload 边界见 [`SECURITY.md`](SECURITY.md)。

---

## 🗺️ 路线图

已发布：

- [x] 依赖分析与引用追踪（Unity GUID 图、未引用资源检测）
- [x] 统计仪表板与报告
- [x] Git 集成（分支信息、单文件变更状态）
- [x] 增量扫描（基于 mtime/size 缓存）
- [x] 批量重命名操作（持久化撤销）
- [x] 导出报告（JSON、CSV、HTML）
- [x] 文件系统实时 watcher（外部修改自动刷新）
- [x] 多项目工作区 + 跨会话恢复
- [x] 标签系统（支持多选筛选 + 启发式 AI 建议）
- [x] 安全删除 / 移动 / 复制 / 副本（系统回收站，自动同步标签）
- [x] Forge Dark 视觉重设计（全部完成）
- [x] 命令面板（⌘K）、列表 / 网格双视图
- [x] Settings → Analysis Rules 编辑器 + per-project `tidycraft.toml`
- [x] PBR set 完整性分析（按目录的纹理集检查）
- [x] 外部编辑器映射（Settings → External Editors，按扩展名配置）
- [x] 跨平台细节打磨（macOS ⌘ 字符、Windows 文件管理器 reveal 修复、path utils）
- [x] DCC 源文件关联（`.blend` / `.psd` / `.spp` / `.ma` / `.ztl` / `.max` / `.lxo` / `.hip` / `.c4d` / `.zprj` / `.sbs` / `.psb` → "源比导出新"警告，opt-in）
- [x] AI 标签 —— 学习模式（项目采样 → 本地规则持久化到 `tidycraft.ai.toml`;之后单资产 LLM 成本为零）+ 高级单资产模式（多 provider LLM,带成本预览 + per-provider 同意流程;opt-in）
- [x] 扫描器默认遵守 `.gitignore` / `.ignore`（通过 `ignore::WalkBuilder`;可在 Settings → Scanning 切换）

待办：

- [ ] VRAM 预算估算（每张纹理、按目录聚合）
- [ ] 跨引擎反向引用图（把 Unity GUID 图扩到 UE / Godot）

---

## 📄 许可证

[Apache 2.0](LICENSE)

---

<div align="center">

为游戏开发者用心打造 ❤️

</div>
