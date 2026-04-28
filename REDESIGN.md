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
| 1 | 视觉换肤：Header/Sidebar/AssetList/Preview/StatusBar/IssueList 重写样式 | ⏳ Pending |
| 2 | ProjectSwitcher dropdown + EmptyState（删除 ProjectList 面板） | ⏳ Pending |
| 3 | Command Palette (⌘K) | ⏳ Pending |
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

## 后续阶段（待开始）

### Phase 1 · 视觉换肤
组件级重写顺序建议：StatusBar → Header → Sidebar → AssetList row → AssetPreview → IssueList → modals。每个组件单独 PR，便于回滚。`.tc-*` CSS 建议拆 `src/styles/redesign-components.css` 或按组件分文件。

### Phase 2 · ProjectSwitcher + EmptyState
删除 `src/components/ProjectList.tsx`（先保留文件作为参考，确认稳定后再删）。`App.tsx` 改回三列布局。新 `ProjectSwitcher.tsx` 必须用 absolute 嵌入按钮 wrapper，**不要 portal**（设计稿明确警告 portal 会偏移）。

### Phase 3 · Command Palette
新 `CommandPalette.tsx`（fixed inset 0 + 居中 modal），数据源 = `scanResult.assets` + `projects` Map + 视图切换 + 操作动词。挂到 `useKeyboardShortcuts` 的 `⌘K`。

### Phase 4 · Gallery
`AssetList.tsx` 增 `viewMode: 'list' | 'grid'`，建议存进 `columnStore` 或新 `viewStore`。Grid 用 `@tanstack/react-virtual` grid 模式 + `getVirtualItems` 投影。缩略图复用现有 `get_thumbnail` Tauri command（已支持 base64 + 磁盘缓存）。

### Phase 5 · AI Tag Suggest
- Rust：`src-tauri/src/analyzer/tag_suggest.rs`，定义 `trait TagSuggester` —— MVP 实现 `HeuristicSuggester`（filename token + dimension bucket + path segment）。预留 `ExternalLLMSuggester` trait impl 占位
- Tauri command: `suggest_tags(project_id) -> Vec<TagGroup>`，类型 `{ name, color, file_paths, confidence, hint }`
- 前端：`AITagPanel.tsx` 浮在 sidebar 之上，sidebar 加 `+✨` 入口

### Phase 6 · 收尾
- StatusBar 加 watcher pulse 指示
- Header refresh / globe 按钮接现有 rescan / i18n 切换
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
