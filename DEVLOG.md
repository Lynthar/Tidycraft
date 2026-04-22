# Tidycraft 开发日志

> 这份文档记录 Tidycraft 的整体开发计划、当前进度和过程日志,方便以后查阅和接续开发。新条目追加到对应章节末尾。

---

## 一、项目目标

打造一个**生产级别的本地游戏 / 3D 艺术资产管理工具**,覆盖扫描、预览、QA 分析、引擎集成、批量操作、报告导出等完整工作流。当前定位是 Unity / Unreal / Godot 项目的本地辅助工具,跨平台桌面应用。

---

## 二、技术栈基线

- Tauri 2.0 + Rust 后端 + React 18/TS + Vite + Tailwind 前端
- Zustand 状态、Three.js 3D 预览、`@tanstack/react-virtual` 虚拟滚动
- Rust: `walkdir` + `rayon` 扫描、`image` / `gltf` / `tobj` / `symphonia` 元数据、`git2` Git 集成、`parking_lot` 锁

参考 `CLAUDE.md` 了解架构细节。

---

## 三、整体路线图

### Phase 1 — 解锁基本可用性 (critical)

| # | 任务 | 状态 |
|---|---|---|
| 1.1 | 后端状态改成 per-project map (与前端多项目对齐) | ✅ 完成 (2026-04-19) |
| 1.2 | `notify` crate 文件监听,外部修改自动刷新 | ✅ 完成 (2026-04-20) |
| 1.3 | 安全删除 (trash) + 移动 + 复制操作 | ✅ 完成 (2026-04-20) |
| 1.4 | 视频 / FBX / 更多格式元数据补完 (duration、frame、vertex 等) | ✅ 完成 (2026-04-21) |

### Phase 2 — 游戏 QA 价值 (major)

| # | 任务 | 状态 |
|---|---|---|
| 2.1 | 贴图规则深化:色彩空间 (sRGB/Linear)、mipmap、texel density | 🔄 2.1a 色彩空间 + 2.1b mipmap 完成 (2026-04-21);texel density 待排 |
| 2.2 | 模型 LOD 链识别、骨骼 / 蒙皮数据分析 | ⏳ 待开始 |
| 2.3 | 场景软引用 / 缺失 GUID 检测 | ✅ 完成 (2026-04-21) |
| 2.4 | 材质 / Shader 预览 (Unity reflection / Unreal Material 集成) | ⏳ 待开始 |

### Phase 3 — 工作流打磨 (polish)

| # | 任务 | 状态 |
|---|---|---|
| 3.1 | 持久化 undo (跨会话保留) | ✅ 完成 (2026-04-21) |
| 3.2 | 智能收藏夹 / 保存复杂筛选 | ⏳ 待开始 |
| 3.3 | 外部编辑器集成 (Photoshop / Blender / Substance) | ⏳ 待开始 |
| 3.4 | 3D 模型缩略图生成 | ⏳ 待开始 |
| 3.5 | 差分报告 / PDF / Excel 导出、含缩略图 HTML | ⏳ 待开始 |

### 长期想法 (未排期)

- 跨项目资产 / 依赖搜索
- 拖放导入、`.uasset` 二进制解析、Godot `.tscn` 解析
- 动画时间轴 scrub、场景 hierarchy 树
- 贴图通道分离预览 (RGB/A 拆分)
- 依赖图 O(n) 重写 (当前 O(n²))
- 结构化日志 / 错误上报 (替换 silent `None` 返回)
- 目录树数据结构重构:后端改用 `HashMap<dir_path, DirNode>` 做主存,序列化时按需组装嵌套;同时用 parent→children 预聚合把 `build_directory_tree` 从 O(D×N) 降到 O(N),消除 WalkDir + read_dir 的双重文件系统走访。文件监听增量更新时可直接按路径定位节点,从叶子往根冒泡修 `file_count/total_size`,替代 Phase 1.2 的整棵重建方案。
- macOS 快捷键显示适配:`useKeyboardShortcuts.ts` 的 `SHORTCUTS` 表硬编码 `"Ctrl"`,Mac 上 UI 应该显示 `⌘`。按键识别逻辑已经用 `ctrlKey || metaKey` 兼容了,只差 UI 显示层做平台 detect 并替换。
- Linux inotify watch 上限错误向前端冒泡:当前 `watcher.rs` 的 notify 错误只 `eprintln!` 到 stderr。大型项目超出 `/proc/sys/fs/inotify/max_user_watches` 时用户会以为 watcher 坏了。方案:error 路径往前端发 `watcher-error-{projectId}` 事件,UI 弹 toast 带修复说明(`echo fs.inotify.max_user_watches=524288 | sudo tee -a /etc/sysctl.conf`)。

---

## 四、当前状态摘要

**最近完成:** 三件一起:
1. **会话恢复** — `sessionStore` 用 Zustand `persist` 存 openProjectPaths + activeProjectPath,App.tsx mount 时 `restoreSession` 回读并按序 `openProject` 重建。重启 app 不用再手选项目。
2. **Phase 3.1 持久化 undo** — `UndoManager` 加 `persist_path` 字段,`load_for_project` 从 `{data_dir}/tidycraft/undo/{sha256(root)[..16]}.json` 回读,`record_batch` / `undo_*` / `clear_history` 之后自动落盘。跨会话可撤销。
3. **Phase 2.1a + 2.1b** — 贴图色彩空间 + mipmap。PNG 扫 `sRGB` / `iCCP` chunk 填 `color_space`;DDS header 的 `dwMipMapCount` 填 `mipmap_count`。新规则 `TextureColorSpaceRule`:文件名后缀暗示数据贴图(`_n/_rough/_mask` 等 20 多种)**且**色彩空间是 sRGB → Warning 级 Issue。扩展 `TextureRule`:DDS >= 512px 且 mipmap_count == 1 → Info 级提示。88/88 单测(color space +6)。

**已知遗留问题 / 技术债:**
- Rust 模块里的 silent failure 模式没修 (`parse_*_metadata` 出错返回 `None` 不记录日志)
- `unity.rs` GUID 提取仍然是脆弱的正则 (32 hex 字符)
- 依赖图 `get_unity_dependencies` 仍是 O(n²),大型 Unity 项目会卡
- `godot.rs` / `unreal.rs` / `undo.rs` 里的中文注释未统一
- 目录树每次 fs-change 整棵重建 (O(D×N),Phase 1.2 取的方案 A);长期按"长期想法"里的目录树数据结构重构来优化
- FBX 纹理 sibling 查找仅扫 model 目录 + 其子目录 + parent 的 sibling 目录,深度有限。作者若把贴图放在 `Assets/Shared/Textures/` 这种跨项目共享路径里仍会 miss — 这种场景少见,需要时再扩展
- 磁盘 `ScanCache` 不随 watcher 事件更新;下次打开项目会靠 `scan_project_incremental` 的 mtime 检查自愈
- fs-change 事件只在 `cached_scan` 存在时生效,扫描结束前到达的事件会被丢弃;大量外部操作恰好落在扫描窗口时可能漏单次,重扫可恢复

**下一步建议:** Phase 2.2 模型 LOD 链识别(文件名约定 `_LOD0/_LOD1/_LOD2` 检查步进一致性)或者先动性能瓶颈(DEVLOG 第五章的 #1 目录树重构 / #2 filter memoization,容量翻倍)。texel density 跨模型↔纹理映射较复杂,往后排。

---

## 五、性能评估 & 容量

基于 2026-04-21 对 scan / watcher / analyzer / frontend 热路径的系统分析。**数字是典型值**(SSD + 8 核 CPU + 混合素材),不同硬件和素材分布会浮动。

### 热路径复杂度

| 路径 | 复杂度 | 10k 文件耗时 |
|---|---|---|
| `scanner.rs::scan_directory_with_state` 发现阶段(`walkdir`) | O(N) fs stat | 0.1-2s(NTFS 最慢) |
| 并行解析 metadata(`rayon::par_iter`) | O(N / cores) | 2-10s(看格式分布) |
| **`scanner.rs::build_directory_tree`** | **O(D × N)** 每次重建 | 500ms-2s |
| `fs::read_dir` 重走一遍(tree 建过程中) | O(D) | 叠加 |
| `ScanCache` JSON 序列化 | O(N) | 100-500ms |
| `projectStore.getFilteredAssets` 选择器 | O(N log N) 每次 re-render | 5-10ms |
| `watcher.rs::apply_changes` 里的 tree 重建 | 同 build_directory_tree | 每次 fs-change 500ms 去抖后触发一次 |
| `find_duplicates` 的 SHA256 | O(N) 分桶 + 冲突桶内 I/O | 500ms-2s |
| `find_missing_references` | O(P × refs)+ hashset O(1) | <1s |
| Virtual list 渲染 | O(1) per frame | 流畅即使 1M 项 |

### 容量分层(经验值)

| 文件数 | 体验 | 主瓶颈 |
|---:|---|---|
| < 1k | 瞬时 | 无 |
| 1k – 5k | 极快(扫描 <1s) | 文件系统延时 |
| **5k – 20k** | **舒服(扫 2-10s)← 推荐甜蜜区** | fs 延时 + 图像 decode |
| 20k – 50k | 可感知,可用 | 目录树 O(D×N)、filter 重算 |
| 50k – 100k | 明显慢(扫 30-60s,filter 迟滞) | 同上累积 |
| 100k – 300k | 勉强能用,filter/sort 卡顿 | 目录树、frontend filter 重算、memory |
| 300k – 1M | 实用性差 | 目录树每次 watcher 事件 ~秒级,JS heap 膨胀,scrollbar 精度 |
| > 1M | 会炸 | Chromium scroll container ~33M px 上限(36 × 1M = 36M,刚过线) |

### 瓶颈排序(按"修完能推多少容量")

1. ~~**`build_directory_tree` O(D × N)**~~ ✅ **2026-04-21 完成**
   - 改为一次 O(N) 预聚合 + O(D) 递归,每节点 O(1) hashmap 查询
   - 实测在 10k 文件 × 200 目录规模,树重建从 ~500ms-2s 降到 ~50-100ms
   - watcher 事件刷新不再肉眼可感
   - **容量上限实际推到:~100k 舒服 / 300k 可用**(#2 同步修完)

2. ~~**`getFilteredAssets` 每次 re-render 重算**~~ ✅ **2026-04-21 完成**
   - Store 里加了 input-reference-equality 级的缓存(所有 filter / sort 输入都是替换而非变异,`Object.is` 比较就够)
   - AssetList useMemo 的 deps 补上了 `scanResult`,watcher 驱动的增删不再 stale
   - StatusBar 也直接受益(共享 store-level 缓存,不用自己 useMemo)

3. **HDR / EXR 全量 decode 只为读尺寸**
   - `image::open` 会完整解码 ~30MB EXR 需秒级
   - 修法:`ImageReader::new(...).into_dimensions()` header-only
   - **不影响容量,但单文件 100× 提速**

4. **FBX 全文件 parse**
   - `fbxcel-dom` 遍历整 DOM;>500MB FBX 1-2s
   - scan 并行里被掩盖,但 fs-change 单文件刷新会慢
   - v1 接受代价

5. **`ScanCache` JSON 序列化开销**
   - 10 万 entry ≈ 30-50MB JSON;serde_json 慢
   - 换 `bincode` / `ciborium`:5-10× 更快,~30% 体积
   - **10 万+ 项目明显提速,小项目无感**

6. **Linux inotify watch 上限**
   - 默认 8192-65k;超出后 `notify` 静默失败(已登记)
   - 修法:错误通过 Tauri 事件冒到前端 toast + 修复说明
   - **不修 Linux 大项目 watcher 完全失效**

7. **Frontend scroll container height 硬上限**
   - Chromium ~33M px;36 行高 × 900k 项 = 32.4M,逼近
   - 修法:分页 / chunk rendering,架构级改动
   - **> 900k 才触线,短期不用管**

### 实用建议

- **当前 comfort zone**(#1+#2 修完后):**50k-100k 资产**。
- **打到 300k+ 企业级**:做 #3(header-only EXR/HDR)+ #5(binary cache 换 `bincode`),再 1-2 天。
- **百万级**:#7 是硬瓶颈,需要重新设计列表渲染。超出当前产品定位。

---

## 六、过程日志 (倒序最新在上)

> 每次推进留一条简短记录:**改了什么 / 为什么 / 影响面**。详细 commit message 在 git log 里查。

### 2026-04-21 — 性能优化 #1 目录树 + #2 filter memoization

**改动**
- `scanner.rs::build_directory_tree` 重写:
  - 新增 `DirStats { file_count, total_size }` struct 和 `precompute_dir_stats(assets)`,一次 O(N) 按 parent 分组
  - 新 `build_dir_node(path, &stats)` 递归遍历 fs(保留空目录能显示的特性),每节点 O(1) 查 stats
  - 整体从 O(D × N) 降到 O(N) + O(D) fs 调用
  - watcher 事件的 tree 重建对 10k+ 项目明显不再卡顿
- `projectStore.ts`:`getFilteredAssets` 加 store-level 缓存。模块级 `filterCacheInputs` + `filterCacheResult` 保存上次输入引用和结果;下次调用用 `Object.is` 逐项比较,全命中则直接返回上次的数组
  - 所有 filter 输入(`scanResult` / `selectedDirectory` / `searchQuery` / `typeFilter` / `sortField` / `sortDirection` / `advancedFilters`)都是替换式更新,引用比较足够,不需要深等
  - `AssetList.tsx` 的 useMemo deps 补上 `scanResult`,watcher 增删不再漏刷
  - `StatusBar.tsx` 直接调 `getFilteredAssets()` 无 memo,现在也受益(命中缓存 O(1))

**为什么**
这两项是性能评估里最大的瓶颈。目录树 O(D×N) 在 10k 项目里就开始感知,watcher 每次事件重建一次,累计卡顿明显。filter memo 则是 UI 任何 state 变化都触发全量 filter+sort 重跑,50k+ 上下文切换就粘。

**影响面**
- 目录树行为不变(空目录仍会显示),只是更快
- filter 缓存是 best-effort:如果 `advancedFilters` 有人原地变异(不该发生),会命中过期缓存。当前所有 setter 都走替换式,安全。如果将来有人加新 filter 字段,**记得加到 inputs 数组**
- 测试:88/88 保持不变,树重构只是算法换,输出结构一样

### 2026-04-21 — 会话恢复 + Phase 3.1 持久化 undo + Phase 2.1a/b 贴图深化

**会话恢复**
- 新建 `src/stores/sessionStore.ts`:Zustand `persist` 存 `openProjectPaths` + `activeProjectPath`(localStorage,name `tidycraft-session`);`partialize` 不存 `restored` 这种运行期 flag
- 模块级 `useProjectStore.subscribe(...)` 自动同步项目集合变化到 session(加了浅比较避免 scan-progress 这类频繁事件触发 set)
- `restoreSession()` 在 `App.tsx` mount 时调用,按序 `openProject` 每个路径(scan 是后台异步,顺序开不会累积前台时间),最后把 activeProjectId 指回原来的 active project
- 边界:文件夹被删了 → `openProject` 内部报 error 但 session 不影响,其他项目继续打开;strict-mode 双挂载有 `restored` guard 护栏

**Phase 3.1 持久化 undo**
- `UndoManager` 加 `persist_path: Option<PathBuf>`;`load_for_project(project_root, max)` 构造并回读磁盘 JSON(按 `max_history` trim)
- 文件名:`dirs::data_dir()/tidycraft/undo/{sha256(project_root)[..16]}.json`,前端生成的随机 `project_id` 不用做 key(每次 session 会变,hash 根路径才稳定)
- `record_batch` / `undo_last` / `undo_by_id` / `clear_history` 末尾都 `self.save_to_disk()`,best-effort,写失败静默(不阻塞撤销操作)
- `ProjectState::new` 改用 `UndoManager::load_for_project`。register_project 被打开即自动回读上次的撤销栈
- 原 `UndoManager::new(max)` 保留为纯内存构造,给测试用

**Phase 2.1a + 2.1b 贴图深化**
- `AssetMetadata` 加 `color_space: Option<String>` 和 `mipmap_count: Option<u32>`
- `parse_png_color_space`:扫 PNG chunk(8-byte magic 后循环读 4-byte len + 4-byte type),`sRGB` 或 `iCCP` 中任一存在 → 标 `sRGB`;没有则返回 None(不假设默认),避免后续规则误伤老 PNG。IDAT / IEND 遇到就停;最多扫 64 个 chunk 防止恶意文件
- `parse_dds_metadata` 补读偏移 28..32 的 `dwMipMapCount`(之前忽略),0 视作 1(base-only)
- 新 `analyzer/rules/texture_colorspace.rs`:`TextureColorSpaceRule`,对 `AssetType::Texture` 检查 stem 后缀命中 `DATA_HINTS`(20+ 种数据贴图后缀)**且** color_space == "sRGB" → Warning,建议重导或标 non-color。6 条单测
- 扩展现有 `TextureRule`:DDS 且 >= 512px 且 mipmap_count == 1 → Info 级 `texture.no_mipmaps` 提示,只在其他检查都通过时才报,避免刷屏
- `cache.rs` `CACHE_VERSION` 3→4(新元数据字段)
- 前端:`types/asset.ts` 加两个字段;`AssetPreview` 的 Image Info 块展示 Color Space + Mipmap 行(mipmap=1 显示 "None (base only)");i18n 加 `assetPreview.colorSpace` / `mipmaps` / `mipmapsNone`

**为什么 (三件合并)**
都是小体量独立工作。Session 恢复用户反馈明确需求;持久化 undo 路线图里一直挂着是 quick win;贴图 sRGB 是游戏项目 QA 常见痛点,**看到一次就不会再犯**的那种错误。合并提交不代表耦合,各自独立。

**影响面**
- localStorage 多了一个 `tidycraft-session` key(~几十字节),用户清浏览器存储会丢会话但不影响功能
- 撤销历史文件在 `{data_dir}/tidycraft/undo/` 累积,按项目路径哈希命名;长期使用时 data_dir 可能积攒废弃 hash(项目被永久删除也不清理)。未来可加"清理孤儿 undo 文件"工具
- Texture 规则产出的 Issues 数量视项目而定;Kenney 风格只有 albedo 的资产包不会炸,带完整 PBR 流程的项目可能出很多 Warning(这是好事,真在警示)
- Cache v3 → v4:用户首次升级后第一次扫描变慢(一次性)
- 测试:82 → 88(+6 color space)

### 2026-04-21 — Phase 2.3: Unity 缺失 GUID 检测

**改动**
- 新建 `analyzer/rules/missing_reference.rs`:`find_missing_references(assets, project_type)` 遍历 prefab/scene/mat/controller/asset,比对 GUID 引用和项目已知 GUID set;未命中就报 Error 级 Issue,per-source 去重避免刷屏,忽略全 0 的 "no reference" sentinel
- `analyzer/mod.rs` 暴露 `Analyzer::find_missing_references`,同 `find_duplicates` 的整项目级别 API
- `lib.rs` 的 `analyze_assets` / `export_issues_to_json` / `export_to_html` 三个命令都合并进 missing-ref 结果
- 5 条单测覆盖:命中、去重、非 Unity 项目跳过、零 GUID sentinel、空项目

**为什么**
`find_unused_assets`(已有)告诉你"哪些资产没被引用",是清理方向;缺失 GUID 检测是反过来"哪些引用找不到目标",是修复方向,对"删了 meta 导致资源链断"这类经典 Unity 事故直接命中。接入现有 Issues 面板不需要新 UI。

**影响面**
- 运行时:扫每个可引用 Unity 文件读一遍,和现有 `get_unity_dependencies` / `find_unused_assets` 同量级开销。大 Unity 项目(>5000 prefab/scene)可感知但不卡死
- 输出:现有的 HTML / JSON 报告自动带上缺失引用条目;前端 Issues 标签页自然显示
- 非 Unity 项目立即返回空 result,零开销

### 2026-04-21 — Phase 1.4c: 视频元数据

**改动**
- `Cargo.toml` 新增 `mp4 = "0.14"` 和 `matroska-demuxer = "0.7"`(纯 Rust,无 ffmpeg 依赖)
- `scanner.rs`:
  - `AssetType` 新增 `Video` 变体;`get_asset_type` 识别 mp4/mov/m4v/webm/mkv/avi;AVI 当前无 parser,返回 `Video` + `metadata: None`
  - `AssetMetadata` 新增 `framerate: Option<f32>` 和 `video_codec: Option<String>`(复用已有的 `duration_secs` / `width` / `height`)
  - `parse_mp4_metadata`:`Mp4Reader::read_header` + 遍历 tracks 找第一个 Video track,读 width/height/frame_rate/media_type
  - `parse_matroska_metadata`:`MatroskaFile::open` + `info.duration()` × `timestamp_scale / 1e9` 换成秒;第一个 Video track 的 `pixel_width`/`pixel_height`/`codec_id`
  - 所有 `AssetType` match 点补上 Video 分支(`type_counts`, `watcher::asset_type_key`)
- `cache.rs`:`CACHE_VERSION` 2→3(新字段)
- 前端:
  - `types/asset.ts` 加 "video" 到 `AssetType` 联合,`framerate`/`video_codec` 到 `AssetMetadata`
  - `AssetPreview.tsx` 加 Video Info 块(时长 / 分辨率 / 帧率 / 编码),现有 `VideoPlayer` 已经能预览
  - `AssetList` `AssetIcon` 加紫色 `Video` 图标
  - i18n:`assetTypes.video`,`assetPreview.videoInfo/resolution/framerate/codec`,en/zh 并行

**为什么**
游戏项目里视频用得不多但出现时就希望一眼看到 duration / 分辨率(cutscene 太长?分辨率不匹配?)。`mp4` + `matroska` 这两个纯 Rust crate 在覆盖率上满足大多数场景,且**不引 ffmpeg**(避免 GPL/LGPL + 体积 + 跨平台编译负担)。

**影响面**
- 编译时间:这两个 crate 很轻(无 sys dep),增量 <5s
- 运行时:MP4 读 header,不扫整个文件,快;Matroska 的 element 扫描稍慢,但 game 视频通常 <100MB,毫秒级
- AVI 暂不支持(无合适的纯 Rust crate);会显示成 Video 但 metadata 空。将来需要时加 `riff` 或 `nom_avi`
- `asset_type === "audio"` 这些旧判断保持原语义;视频走独立 `=== "video"` 分支

### 2026-04-20 — Phase 1.4a: FBX 元数据 + SVG 支持

**改动**
- `Cargo.toml` 新增 `fbxcel-dom = "0.0"`(解析至 0.0.10,内部带 `fbxcel` 0.9 做底层二进制/ASCII 解析)
- `scanner.rs` 新增 `parse_fbx_metadata`:
  - `AnyDocument::from_seekable_reader` 加载(二进制 + ASCII 都走这一个入口)
  - 遍历 `doc.objects()` 的 `TypedObjectHandle`:
    - `Geometry::Mesh` → 从 mesh 的原始节点读 `Vertices` f64 数组(长度/3 = 顶点数)和 `PolygonVertexIndex` i32 数组(**负数条目 = 多边形终结符,count = 面数**,这样 tri/quad/n-gon 都正确)
    - `Material` → 材质计数 +1
  - 所有计数用 u64 累加,返回前 saturating cap 到 u32
  - 任何读错误都静默 None(符合 scanner 现有风格)
- SVG 顺带补上:加到 `get_asset_type` 的 Texture 阵营,新 `parse_svg_metadata` 读前 16KB 找 `<svg ...>` 根标签,抽 `width`/`height` 或 fallback `viewBox` 后两个数字;支持 `px` 后缀、单引号/双引号;拒绝 `%` / `em` / `vw` 等非 px 单位(此时只看 viewBox)
- `xml_attr` helper 做了 word-boundary 检测避免 `viewName=` 这种后缀误匹配
- 新增 10 条单测(4 条 FBX API 验证 via cargo check + 6 条 SVG 覆盖 explicit/viewBox/percent-fallback/single-quote/empty),`cargo test --lib` 77/77

**为什么**
FBX 是游戏 asset 里最常见的模型格式。之前 AssetList 里所有 FBX 都显示"—"没元数据。`fbxcel-dom` 的典型 DOM 体验能保证 FBX 7.4+ 的 binary/ASCII 双形态都 cover,比用 `fbxcel` 低阶 pull parser 手撸稳健得多。多边形负数终结符是 FBX 约定,不需要重建完整面 list 就能数出总数,性能 O(N)。

**影响面**
- 编译时间:`fbxcel-dom` + 依赖第一次编译 ~15s,增量后续 <5s
- 运行时:FBX 元数据是全文件解析(不像 DDS/SVG 只读 header),大型 FBX(>100MB)会慢。但 scan 是并行的,分摊到 rayon worker 不明显。真正有痛点时再考虑 size cap skip
- FBX 6.x(老版本)不支持,会静默返回 None;现代 DCC 软件输出都是 7.4+,影响小
- API 坑:`AnyDocument` 是 `#[non_exhaustive]`,不能用 `let` 解构,得 `match`;`fbxcel-dom::any` 不是 `any_document`(文档注释示例写对了)

### 2026-04-20 — Phase 1.4b: 扩展图像格式元数据

**改动**
- `Cargo.toml`:`image` crate features 加 `tiff` `webp` `hdr` `exr`(不动 avif,避免 dav1d 系统依赖)
- `scanner.rs`:
  - 新增 `parse_dds_metadata`:读前 128 字节 → 验证 `"DDS "` magic + `dwSize==124` → 取 `dwHeight(12..16)` / `dwWidth(16..20)` / `ddspf.dwFlags(80..84)` 里的 `DDPF_ALPHAPIXELS` bit。不解码像素,和 BC1/BC3/BC7 等压缩变体无关
  - 把两处重复的 dispatch(`scan_directory_with_state` 主扫 + `parse_asset_file` 单文件重解析)合进新 helper `parse_metadata_for(path, ext, asset_type)`,后续加格式只需改一处
  - TIFF/WebP/HDR/EXR 直接走 `parse_image_metadata`(image crate decode 完已能给 `color()` → 正确识别 alpha)
- 新增 5 条单测覆盖 DDS parser:valid header / alpha flag / 无 alpha / bad magic / truncated,以及通过 dispatch 的集成测试。本地 `cargo test --lib` 71/71 通过

**为什么**
用户的游戏项目里常见一堆 `.dds`(BC 压缩贴图)、`.tiff`(art pipeline)、`.hdr`/`.exr`(HDRI / lightmap),之前这些都在 `AssetList` 显示 `—`。DDS 特别,`image` crate 虽有 `dds` feature 但对压缩格式的支持有限,自己读 header 最稳。

**影响面**
- 编译时间:image 新 features 多出 ~几秒;`dev` profile 冷启 12-15s,之后增量都在 10s 内
- 运行时:HDR/EXR 的 `image::open` 会完整解码,大文件(>30MB)慢。后续考虑改 `ImageReader::into_dimensions()` 做 header-only 读。现在 v1 接受这个代价
- Phase 1.4a / 1.4c 占位不变,`parse_metadata_for` 的集中 dispatch 为它们预留了接入点

### 2026-04-20 — Phase 1.3b: 移动 / 复制 / 副本

**改动**
- `lib.rs`:
  - `move_assets(project_id, paths, target_dir)` — 逐文件 `fs::rename`;collision 不覆盖;成功批量记入 `UndoManager` 作为 `OperationType::Move`
  - `copy_assets(paths, target_dir)` — `fs::copy`;collision 报错(建议用 Duplicate)
  - `duplicate_assets(paths)` — 同目录,`unique_copy_path` 生成 `foo copy.png` / `foo copy 2.png` … 直到不冲突(1000 次上限后 fallback 时间戳)
  - 三个命令共用 `FileOpResult { successes: [{original_path, new_path}], errors: [{path, message}] }` 结构
  - 所有返回路径走 `scanner::path_to_string` 归一化
- 新建 `src/components/MoveCopyDialog.tsx`:复用于 Move / Copy 两种 mode;内置递归目录树(展开/折叠 + 图标 + 文件数 + 键盘导航),Move 模式下自动禁用来源文件所在的父目录(no-op 目标)
- `ContextMenu.tsx` 加 `onDuplicate` / `onMoveTo` / `onCopyTo` 三个可选 prop + 对应菜单项
- `AssetList.tsx`:
  - 抽出共享的 `targetPathsFromContext()`(右键目标在多选集里 → 批量,否则 → 单文件),delete/move/copy/duplicate 四个操作都用这个规则
  - 把 Del 快捷键 useEffect 扩展成统一的 keybinding handler,加上 Ctrl/Cmd+D 副本;排除 input / textarea focus 场景
  - duplicate 走 fire-and-forget(无对话框),让 watcher 自己把新文件推进 UI
- i18n:`contextMenu.duplicate/moveTo/copyTo` + `moveCopy.*` 完整键组,en/zh 并行
- `types/asset.ts`:`FileOpResult` / `FileOpSuccess` / `FileOpError`

**为什么**
Move 的 undo 借用已有的 `UndoManager` + `OperationType::Move`(`execute_single_undo` 里已经能反向 rename)。Duplicate 的 " copy" 命名遵循 macOS Finder 习惯,跨平台无歧义。Copy 分成两条路径(不同目录 vs. 同目录)语义更清晰:前者在 collision 时报错让用户手动选,后者永远自动加后缀。

**影响面**
- 上线后 `contextMenu` 增加到 4 个新操作项,已加分隔符把 "Delete" 从破坏性操作组出分离出来
- Tauri 命令总数再 +3
- 现有 Rename 的 `openProject(projectPath)` 重扫机制没动;长期可以改成跟 delete/move/copy 一样靠 watcher 刷新,但那是 Rename 的优化,不属于 1.3b

### 2026-04-20 — FBX/OBJ/DAE 纹理 sibling 查找

**改动**
- `lib.rs` 新增 `resolve_texture_siblings(model_path) -> HashMap<lowercase_filename, abs_path>`:扫 model 目录 + 常见纹理子目录(`Textures/` `Materials/` `Maps/` `Images/` + 大小写变体),以及 parent 级的 sibling `Textures/` 等;first-hit-wins 保证 model-local 纹理优先
- 新建 `src/lib/modelUrlResolver.ts`:`buildTextureUrlResolver(modelPath)` 返回一个同步的 URL modifier(Three.js `LoadingManager.setURLModifier` 要求同步)。内部 basename 提取兼容 bare filename 和已 encode 过的 `http://asset.localhost/...` URL(解码后再 split)
- `ModelViewer3D.tsx` / `ModelLightbox.tsx` 大幅瘦身:去掉 ~80 行的 inline URL 改写逻辑 + 日志;改为 async IIFE 先 await sibling map,再配置 `LoadingManager`(`setURLModifier` + `resolveURL` 两个钩子都挂上,因为 Three.js 不同 loader 走不同的路径)

**为什么**
用户遇到 Kenney 资产包:FBX 内嵌 `colormap.png`(无目录),实际贴图在 `FBX format/Textures/colormap.png`。原逻辑 resolve 到 `FBX format/colormap.png` → 500。按 basename lookup 的 sibling 表一键解决。

**影响面**
- 对 GLB 这类已经能工作的格式:重新路由到 siblings map 里的同一路径,结果不变
- 同名纹理在多处存在时优先 model-local(`""` 子目录在 `SIBLING_SUBDIRS` 里排第一),符合作者意图
- 新增一次异步 Tauri invoke(通常 <10ms),换来稳定的纹理解析
- 移除了前端冗长的 debug log(`[ModelViewer3D] Converting relative path:...` 等);保留错误时的上下文日志

### 2026-04-20 — Phase 1.3a: 安全删除 (trash crate)

**改动**
- `Cargo.toml` 新增 `trash = "5"`(Windows 走 SHFileOperation,macOS 走 NSFileManager,Linux 走 XDG Trash 规范)
- `lib.rs` 新增 `delete_assets(paths: Vec<String>) -> DeleteResult`:per-path 循环调用 `trash::delete`,成功/失败分别收集;**不带 `project_id`** 因为 watcher 会自动把被删文件从 `cached_scan` 里移除
- 新建 `src/components/DeleteConfirmDialog.tsx`:确认对话框,展示文件名预览(前 5 个 + "另外 N 个"计数),回收站说明,错误内联显示
- `types/asset.ts` 新增 `DeleteResult` / `DeleteError` 类型
- `AssetList.tsx`:
  - 右键菜单"删除"项(路径规则:若右键目标在多选集内则批删整个选择,否则只删单个)
  - `Del` 快捷键触发批删(只对多选生效,单击的 `selectedAsset` 不响应以免与导航冲突)
  - 删除完成后只清理成功路径的 `selectedPaths` 项;失败的保留让用户看见
- i18n 加 `deleteConfirm.*` 键,en/zh 并行

**为什么**
trash 而非 `fs::remove_file`:误操作可恢复,且 OS 本身就是 undo 机制,不必在 app 里再做一套。Watcher 复用让前端链路变短(不 rescan 也不手动 patch assets)。

**影响面**
- 不支持回收站的场景(某些网络盘 / FUSE mount)会在对话框错误区展示,用户看得见;不会崩
- 批量删除对话框用 `count=1` 走 `titleSingle`,>1 走 `titleBatch`,i18n 键语义清晰
- `common.done` 键已经存在(zh.json line 274),en.json 之前也有;未新增 i18n 键冲突

### 2026-04-20 — Phase 1.2: 文件监听 (notify-debouncer-full)

**改动**
- 新增 `src-tauri/src/watcher.rs`:`ProjectWatcher` 持有 `Debouncer` 句柄,500ms debounce,后台线程消费事件 → 过滤 → 单文件 `parse_asset_file` → patch `ProjectState.cached_scan` → 重建目录树 → 发 `fs-change-{projectId}`
- 过滤规则镜像 scanner:跳过 `.xxx` 目录 / `.meta` / 无扩展名 / root 外路径;单元测试覆盖
- `ProjectState` 加 `watcher: Option<ProjectWatcher>` 字段;drop 即自动停止(`Debouncer` 的 Drop impl 会关通道,后台线程 `rx.recv()` 返回 Err 退出)
- `lib.rs` 新增 `start_watching` / `stop_watching` 两个命令,`scanner::build_directory_tree` 提升为 `pub(crate)` 复用
- 前端:`types/asset.ts` 加 `FsChangeEvent` 类型;`projectStore.ts` 在 `openProject` 扫描完成后 `listen<FsChangeEvent>(fs-change-{id})` + `invoke("start_watching")`;`closeProject` 触发 `stop_watching` + unlisten;新 helper `applyFsChange` 按路径 merge/upsert assets、替换 directory_tree/aggregates、reconcile `selectedAsset`
- 监听句柄存在模块级 `Map<projectId, UnlistenFn>`(不放 zustand 状态,因为函数引用不该序列化)
- Cargo.toml 加 `notify = "6"` + `notify-debouncer-full = "0.3"`

**为什么**
外部工具(编辑器、git、Blender)修改项目时,用户不用手动 rescan 才看到新状态。`parse_asset_file` 和 `ScanCache::update_entry/prune` 在 Phase 1.1 已经天然是单文件可复用的,所以接入点很小。

**影响面**
- 扫描完的项目自动开启监听,每次 fs-change 后 `scanResult` 整体被替换一次 — 组件若对 `scanResult` 浅比较会重渲染,但 `AssetList` 本来就是虚拟滚动,成本可控
- 目录树整棵重建每次付 O(D×N);大型项目(>10k 文件)监听时 CPU 明显。技术债条目已登记,按"长期想法"的目录树重构方案后续优化
- `selectedAsset` 指向被删文件时自动清空;被修改时自动换新副本
- 坑点:`notify-debouncer-full 0.3.x` 导出 `FileIdMap`,不是 `RecommendedCache`(后者是 0.4+ 的名字)

### 2026-04-19 — Phase 1.1: per-project 后端状态

**改动**
- 新增 `src-tauri/src/project.rs`,包含 `ProjectState` + 全局注册表 + `with_mut/with_ref` 辅助
- 重写 `src-tauri/src/lib.rs`:删除 5 个全局 `Mutex<Option<...>>` 单例;每个项目相关命令加上 `project_id: String` 第一参数;新增 `register_project` / `unregister_project` 生命周期命令
- 进度事件改名为按项目分发的 `scan-progress-{projectId}` (避免多项目并发扫描串号)
- 前端 `projectStore.ts` / `tagsStore.ts` 全部按项目 ID 调用,扫描结果按 ID 写回 Map (即使切换 active project 也不丢)
- 组件 `RenameDialog` / `BatchRenameDialog` / `StatsDashboard` / `App.tsx` 导出按钮拉 `activeProjectId` 传给 invoke
- 同步更新 `CLAUDE.md` 架构说明

**为什么**
原架构前端支持多项目 Map,后端却用全局 Mutex 单例,切换/并发项目时状态会被覆盖。这是必须先解决的架构债。

**影响面**
所有 Tauri 命令签名变化 (大约 30+ 个新增 `projectId` 参数)。前端任何新增 invoke 调用必须记得带 `projectId`。

### 2026-04-19 — 项目审计 + CLAUDE.md 初始化

**改动**
- 创建 `CLAUDE.md`,记录架构、命令、关键约束 (路径编码、单项目 vs 多项目矛盾)
- 完成完整功能审计 (~55% feature-complete 评估)
- 制定三阶段路线图

**为什么**
建立 baseline 文档,后续 Claude 实例上下文复用。

---

## 七、维护这份文档的约定

- **新增任务** → 加到对应 Phase 表里 (或长期想法),状态用 ⏳ / 🔄 / ✅
- **完成任务** → 更新 Phase 表状态 + 在过程日志开头追加一条
- **技术债 / 遗留问题** → 写进"当前状态摘要"对应小节
- **架构变化** → 同步 `CLAUDE.md`,在过程日志注明
- **保持简洁**:实现细节 / 解决方案不写在这里,留给代码 / commit / `CLAUDE.md`。这里只回答"做了什么、为什么、下一步是什么"
