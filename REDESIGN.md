# Tidycraft Redesign — 进度与决策记录

跟踪从交接包 (`design_handoff_tidycraft_redesign/`) 到生产代码的迁移进度。每完成一个阶段在此追加日志，标注决策与注意事项。

---

## 总体决策（2026-04-28 锁定）

1. **去掉左侧 ProjectList 面板** —— 项目切换/添加/关闭统一进入 header 的 ProjectSwitcher dropdown，主区域多出 ~12% 宽度。
2. **CSS 策略：Tailwind + 全局 CSS 混合**
   - **Token 层**：进 `tailwind.config.js`（颜色、圆角、字体、阴影），驱动 utility class
   - **组件层**：设计稿的 `.tc-*` CSS 直接迁入 `src/styles/`，不硬翻成 Tailwind utility（避免 `color-mix` 改造成本）
   - 双轨并行：旧组件继续用现有 `bg-card-bg` 等 legacy class，迁移后切换到新 token
3. **AI Tag Suggest：先 MVP，再接外部模型**
   - MVP 启发式：filename token + 维度 + 路径段，纯本地计算
   - 后端预留 trait/接口位，方便后续接外部多模态 LLM
   - 前端先把 Apply/Skip/Preview 流程跑通
4. **迁移节奏：渐进**，每个 Phase 独立可发布、不破坏现有功能。

---

## 阶段总览

| Phase | 范围 | 状态 |
|-------|------|------|
| 0 | 基础设施：字体引入、token 系统、tailwind 扩展 | ✅ Done (2026-04-28) |
| 1 | 视觉换肤：StatusBar / Header / Sidebar / AssetList / AssetPreview / IssueList / DirectoryTree | ✅ Done (2026-04-29) |
| 2 | ProjectSwitcher dropdown + EmptyState（删除 ProjectList 面板） | ✅ Done (2026-04-29) |
| 3 | Command Palette (⌘K) | ✅ Done (2026-04-29) |
| 4 | Gallery / grid 视图 + list/grid toggle | ⏳ Pending |
| 5 | AI Tag Suggest（Rust 后端 trait + UI panel） | ⏳ Pending |
| 6 | 收尾：watcher pulse 暴露、refresh/globe 接线、视觉微调 | ⏳ Pending |

---

## Phase 0 · 基础设施（2026-04-28 完成）

### 完成项

- 安装 fontsource 离线字体包：`@fontsource/inter-tight` + `@fontsource/jetbrains-mono`（仅 400/500/600 三档，约 250 KB 总量）
- 创建 `src/styles/redesign-tokens.css`，含全部 Forge 设计 tokens（OKLCH）
  - 表面色：`--bg`, `--bg-soft`, `--panel`, `--panel-2`, `--panel-hover`, `--panel-active`
  - 边线：`--line`, `--line-soft`, `--line-strong`
  - 文字 4 级：`--text` ~ `--text-4`
  - 主色：`--primary` 及 `-strong/-soft/-tint`，`--on-primary`
  - 资产类型 11 种：`--c-texture/--c-model/--c-audio/--c-video/--c-animation/--c-material/--c-prefab/--c-scene/--c-script/--c-data/--c-other`
  - Git 状态 4 种：`--git-new/-modified/-deleted/-renamed`
  - 语义状态：`--ok/--warn/--err/--info`
  - 阴影：`--shadow-card`, `--shadow-pop`
  - 工具类：`.mono` (JetBrains Mono + ss02/zero), `.tnum` (tabular-nums)
- 扩展 `tailwind.config.js`：新 token 全部暴露为 utility，沿用清晰命名空间避免与 legacy 冲突
  - 表面：`bg-base`, `bg-panel`, `bg-panel-2`, `bg-panel-hover`, `bg-panel-active`
  - 文字：`text-ink`, `text-ink-2/-3/-4`
  - 主色：`bg-accent`, `bg-accent-strong/-soft/-tint`, `text-on-accent`
  - 边线：`border-line`, `border-line-soft/-strong`
  - 资产色：`text-c-texture`, `bg-c-texture` 等 11 套
  - Git 色：`text-git-new` 等 4 套
  - 状态：`text-ok/warn/err`
  - 字体：`font-display` (Inter Tight)，`font-mono` 覆盖默认指向 JetBrains Mono
  - 阴影：`shadow-card`, `shadow-pop`
- `src/main.tsx` 增加 token 与字体 CSS import
- 创建本进度文档

### 关键决策

- **新旧 token 并存**：保留 `--color-*` legacy 变量与对应 Tailwind 工具类（`text-primary`、`bg-card-bg` 等），现有组件不受影响。Phase 1 起逐组件切换到新 token，迁完再清理 legacy。
- **Token 作用域**：用 `[data-theme="dark"|"light"]`（沿用 themeStore 现有约定），不要求设计稿的 `.tc-app.theme-dark` 包裹。这样不需要改 React 树就能让 token 生效。
- **字体仅注册不全局应用**：fontsource CSS 只声明 `@font-face`，不修改 `body { font-family }`。Phase 1 组件按需用 `font-display` / `font-mono` / `.mono` 类切换。
- **Tailwind 命名映射**：CSS 变量保持 `--text/--panel/--primary` 与设计稿一致（确保 Phase 1 移植 .tc-* CSS 时直接生效），Tailwind utility 用清晰命名（`text-ink/bg-panel/bg-accent`）避免 `text-text` 这种怪名字。

### 注意事项与风险

- **OKLCH 浏览器兼容**：Tauri 用 WebView2 (Chromium ≥ 111 已稳定支持 `oklch()` + `color-mix(in oklch)`)。当前 Tauri 2.x 默认 webview 已满足。Phase 1 跑 `pnpm tauri dev` 时需肉眼验证渲染。
- **字体包 esbuild build script 警告**：pnpm 提示 `Ignored build scripts: esbuild@0.25.12`。这是 pnpm 10 默认行为，无需处理；运行 `pnpm approve-builds` 可单独允许。当前不影响构建。
- **设计稿 1400×900 固定尺寸**：实际应用窗口可变。Phase 1 重写布局时需测试 13" 笔记本 (1280×800) 下 sidebar 248px + preview 304px 是否压缩主区域过多。
- **`info` 状态色冲突**：设计稿用 `--info` (oklch)，legacy 用 `--color-info` (#3b82f6 hex)。Tailwind config 里 `info` 仍指向 legacy，新代码若需新 info 色暂用 `text-[var(--info)]` 或在 Phase 1 一并替换。
- **`font-mono` 已覆盖**：tailwind config 现在让 `font-mono` 指向 JetBrains Mono（之前是默认 ui-monospace 栈）。已检查现有代码，`font-mono` 主要用于 AssetList/AssetPreview 元数据展示，视觉差异极小（都是等宽），可作为 Phase 0 的轻微视觉变化接受。
- **删除左侧 ProjectList 后用户引导**：Phase 2 必须保证 dropdown 入口足够明显（设计稿要求空状态自动展开 dropdown 作为引导）。

### 验证

- `pnpm build` 通过（`tsc` + `vite build` 均无报错）
- 现有视觉无显著变化（除 `font-mono` 字体微调）

---

## Phase 1 · 视觉换肤（2026-04-29 完成）

按 StatusBar → Header/Sidebar shell → AssetList/AssetPreview/IssueList → DirectoryTree 的顺序拆四次提交，每次都同步在 `src/styles/redesign-components.css` 累积 `.tc-*` 组件层 CSS。

### 完成项

- **`5c21a1f`** start: v2 tokens + StatusBar migration
  - 新增 `src/styles/redesign-tokens-v2.css`（236 行）—— 在 Phase 0 OKLCH tokens 之上补齐组件层需要的辅助变量
  - StatusBar 完全切换到 `.tc-*` 类（158 行 ↔ 重写）
  - AssetList 顺手吃 v2 tokens（轻量 10 行差异）
  - `tailwind.config.js` 扩展 + `main.tsx` 引入 v2 tokens CSS
- **`b4f7f85`** Header / Sidebar shell + v2 tokens
  - `src/styles/redesign-components.css` +410 行（Header/Sidebar 整套 `.tc-*` 类）
  - `Header.tsx` 273 行重写、`Sidebar.tsx` 78 行重写
- **`5ee8c1d`** full main-frame migration（AssetList / AssetPreview / IssueList）
  - `redesign-components.css` +521 行
  - 重头戏是 `AssetPreview.tsx`（739 行），把元数据展示、3D viewer、图片 lightbox 入口全切到新 token
- **`2bb6f04`** full container migration（DirectoryTree）
  - `redesign-components.css` +118 行；`DirectoryTree.tsx` 由 71 行 → 简化收尾

文档侧顺手在 `ff58241` 把 `CONTRIBUTING.md` 内 6 处 DEVLOG 引用全部替换为 REDESIGN，并删掉文件树里的 `DEVLOG.md` 条目。

### 关键决策

- **CSS 层只走全局类，不进 Tailwind utility**：所有 `.tc-*` 类都集中在 `redesign-components.css` 单文件累积（共 ~1495 行），没有按组件拆分。当前体量可接受；后续若超过 2000 行考虑拆分。
- **Legacy token 仍然保留**：`themeStore` 的 `--color-*` 变量没动，旧组件（settings / dialogs 等）继续工作；新组件统一用 `--bg/--panel/--text/--primary`。
- **OKLCH 渲染验证通过**：本地 `pnpm tauri dev`（Tauri 2.x WebView2 = Chromium 130+）下 `oklch()` + `color-mix(in oklch, ...)` 全部正确渲染，与 offline_bundle HTML 视觉一致。

### 注意事项

- AssetPreview 重写体量最大（739 行），如发现回归首先怀疑这一块（图片/3D/视频/音频四种 preview 路径）。
- DirectoryTree 中虚拟滚动逻辑没动，只换了 row 类名；如果有点击/展开异常仍属于 logic 问题，不在迁移范围。

---

## Phase 2 · ProjectSwitcher + EmptyState（2026-04-29 完成）

### 完成项

- **`1364db7`** ProjectSwitcher dropdown + EmptyState; remove left panel
  - 新建 `src/components/ProjectSwitcher.tsx`（211 行）—— header 内嵌 dropdown，列出已开项目 + Open / Recent / Close
  - 新建 `src/components/EmptyState.tsx`（47 行）—— 无项目时主区域引导
  - `App.tsx` 由四列（含 ProjectList 面板）改为三列（Sidebar + Main + Preview），`isEmpty` 时主区域渲染 `<EmptyState />`
  - `Header.tsx` 简化 -122 行（项目切换逻辑全部迁出到 ProjectSwitcher）
  - `redesign-components.css` +299 行（dropdown / menu item / kbd hints / empty hero）
- **`f4a2789`** Fix nested button in ProjectSwitcher menu items
  - dropdown menu item 之前嵌套了 `<button>` 内含子 `<button>`（关闭项目按钮），React 会警告且 a11y 不正确；改为外层 `<div role="button">` + 内层真 `<button>`

### 关键决策

- **`ProjectList.tsx` 暂留**：文件还在 `src/components/`，但已经从 `App.tsx` 移除引用。用户提的"先保留作为参考"逻辑生效，待 Phase 3-4 稳定后再彻底删。
- **dropdown 不用 portal**：直接 absolute 定位在 trigger 按钮 wrapper 内，遵循设计稿"portal 会让坐标偏移"的明确告警。

---

## Phase 3 · Command Palette（2026-04-29 完成）

### 完成项

**CSS 层**（`src/styles/redesign-components.css` 末尾 +165 行）：
- `.tc-overlay` / `.tc-cmdk`（580px 宽 modal，设计稿原 640 收紧）/ `.tc-cmdk-input` / `-list` / `-section` / `-item`（`[data-active]` 选中态）/ `-item-icon|-label|-sub` / `-empty` / `-foot`（`-keys`）
- 来源：`design_handoff_tidycraft_redesign/app/styles.css:1279-1353` + 自写的 `.tc-overlay` 容器与两段 keyframes（fade + slide）

**新建文件：**
- `src/stores/uiStore.ts` — 全局 modal 标志（`cmdkOpen` / `settingsOpen` / `tagManagerOpen`）。**职责单一**：只管 transient overlay state，跟 `settingsStore`（持久化设置内容）正交。
- `src/components/CommandPalette.tsx` — 完整四段式 cmdk：
  - **Suggestions**：Run Analysis（⌘⇧A）/ Cancel Scan（扫描中显示）/ Rescan Project（⌘R）
  - **Navigate**：Go to Assets/Issues/Stats（1/2/3）+ Switch to {project name}（开 ≥2 个项目时）
  - **Resources**：资产 quick-jump（query 非空时，filename + path 双字段 `.includes`，cap 50 项 → `ASSET_RESULT_CAP` 常量）
  - **Actions**：Manage Tags / Toggle Theme / Toggle Language / Export JSON·CSV·HTML / Close Active Project / Settings(⌘,)
  - 键盘导航：↑↓/Home/End/Enter/Esc + mouse hover 同步 active；input 自动聚焦
  - 性能：`useDeferredValue` 包 query；`useMemo` items 按 store 字段精细订阅；scrollIntoView 用 `querySelector` 不用 ref array

**接线改动：**
- `src/hooks/useKeyboardShortcuts.ts` — `⌘K` 触发 `useUiStore.toggleCmdk()`，前置于 input-blur guard（input focus 也能开）；`cmdkOpen` 时其它分支统一 return（避免 Esc 同时关 cmdk + cancel scan）；`SHORTCUTS.commandPalette` 加进显示表
- `src/App.tsx` — 顶层渲染 `<CommandPalette />` + `<SettingsModal />` + `<TagManager />`，`dispatchExport(format)` 派发到现有三个 export handler
- `src/components/Header.tsx` — 删 local `showSettings`，改用 `useUiStore.setSettingsOpen`；`<SettingsModal>` 渲染从此处移除
- `src/components/AssetList.tsx` — 删 local `showTagManager`，改用 `useUiStore.setTagManagerOpen`；`<TagManager>` 渲染从此处移除
- `src/i18n/locales/en.json` + `zh.json` — `commandPalette.*` 翻译键（placeholder / empty / 4 个 section / footer / 18 条 item label）

### 关键决策

- **modal 状态升 `useUiStore`，不走 prop drilling**：CommandPalette 要触发 SettingsModal/TagManager，但这俩之前是组件内 local state。两个候选：(a) 提到 App.tsx 顶层 + props 透传给 Header/AssetList/CmdK，(b) 全局 store。选了 (b)——新组件接入只要 `useUiStore` 一行，三层 prop drilling 太重。
- **`scrollIntoView` 用 `listRef.current.querySelectorAll(".tc-cmdk-item")[i]` 而非 ref array**：稳定 key（`it.id`）下 React 不重新调用 callback ref，原本 `itemRefs.current = []` 写法会让 ref 永远空。DOM 顺序 = filteredItems 顺序，querySelector 是天然正确的索引。
- **不引入 `cmdk` npm 包，手写**：现成 165 行 `.tc-cmdk-*` CSS 是 className-based，cmdk 包是 `[cmdk-input]` data-attribute 选择器，迁移成本 > 手写键盘逻辑（~50 行）；未来命令面板复杂度若到 Slack/Linear 级再迁。
- **资产搜索量 MVP 不上 fuzzy**：`.filter().slice(0, 50)` + `useDeferredValue`。`ASSET_RESULT_CAP` 注释提到了 `fuse.js` / Web Worker 作为升级位。
- **MVP 不做 Filter 段**（设计稿 mock 有 "Filter: Models only" 等）：跟 AdvancedFilters / `getFilteredAssets` 耦合大，留到 Phase 6 收尾时与 watcher pulse / refresh-globe 一起加。
- **Settings/TagManager 渲染都升到 App.tsx 顶层**：之前各自在 Header/AssetList 内部，改后语义更接近"全局 modal"。

### 注意事项

- `Manage Tags` 命令依赖 `<TagManager>` 已升到 App.tsx 顶层（之前在 AssetList 内部，CommandPalette 进不去）。任何后续动 TagManager 渲染位置的改动要保留这个不变量。
- `cmdkOpen` 时 `useKeyboardShortcuts` 主动让位，**整个 hook 几乎全 disable**（除了 `⌘K` toggle 本身），CommandPalette 自己监听键盘。Phase 6 加新全局快捷键时不要忘记同样让位。
- ProjectSwitcher dropdown 仍是项目切换的"看全列表 + 添加/关闭"主入口；CommandPalette 是 power user 的 type-to-jump 互补入口（仅当开 ≥2 个项目时显示 Switch）。
- Run Analysis shortcut 标 ⌘⇧A（按代码现状），跟设计稿 mock 的 ⌘⇧R 不一致 — 改快捷键不在 Phase 3 范围。
- 验证已通过：`pnpm build` 通过；`pnpm tauri dev` 用户肉眼测试 OK（⌘K 开关 / 键盘导航 / 资产 quick-jump / 切项目 / 全部操作动词 / EmptyState 兜底 / cmdkOpen 时 Esc 不再 cancel scan）。

---

## Phase 4 · Gallery（⏳ 待开始）

`AssetList.tsx` 增 `viewMode: 'list' | 'grid'`，建议存进 `columnStore` 或新 `viewStore`。Grid 用 `@tanstack/react-virtual` grid 模式 + `getVirtualItems` 投影。缩略图复用现有 `get_thumbnail` Tauri command（已支持 base64 + 磁盘缓存）。

---

## Phase 5 · AI Tag Suggest（⏳ 待开始）

- Rust：`src-tauri/src/analyzer/tag_suggest.rs`，定义 `trait TagSuggester` —— MVP 实现 `HeuristicSuggester`（filename token + dimension bucket + path segment）。预留 `ExternalLLMSuggester` trait impl 占位
- Tauri command: `suggest_tags(project_id) -> Vec<TagGroup>`，类型 `{ name, color, file_paths, confidence, hint }`
- 前端：`AITagPanel.tsx` 浮在 sidebar 之上，sidebar 加 `+✨` 入口

---

## Phase 6 · 收尾（⏳ 待开始）

- StatusBar 加 watcher pulse 指示
- Header refresh / globe 按钮接现有 rescan / i18n 切换
- Command Palette 补 Filter 段（按资产类型一键过滤，跟 AdvancedFilters 联动）
- 11 视图截图清单 vs 设计稿全量对照微调

---

## 文件位置

- 设计参考：`design_handoff_tidycraft_redesign/`（不进 build）
  - `README.md` — 设计师交付说明
  - `app/Tidycraft.jsx` — 全部 React mock
  - `app/styles.css` — 完整 token + 组件 CSS（2184 行）
- 离线 HTML 预览：`offline_bundle/Tidycraft-Redesign-Offline.html`
- Token 实现：`src/styles/redesign-tokens.css`
- 进度文档：`REDESIGN.md`（本文件）
