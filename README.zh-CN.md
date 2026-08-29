<div align="center">

<img src="docs/brand/hero.png" alt="Tidycraft — 游戏资产管理与分析" width="100%">

[![license](https://img.shields.io/github/license/Lynthar/Tidycraft)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Lynthar/Tidycraft/ci.yml?branch=main&label=CI)](https://github.com/Lynthar/Tidycraft/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/Lynthar/Tidycraft)](https://github.com/Lynthar/Tidycraft/releases)
[![crates.io](https://img.shields.io/crates/v/tidycraft)](https://crates.io/crates/tidycraft)

</div>

跨引擎的游戏资产 lint：扫描 Unity / Unreal / Godot 项目，桌面应用与 CI 共用同一套规则

[English](README.md) | 简体中文

给它一个游戏工程的目录，它会遍历资产树，识别出这是哪个引擎的项目，读出每个贴图、
模型、音频和视频文件的元数据，再按你配置的规则逐条检查。相当于资产版的 ESLint，
管的是那些不参与编译的文件。

一套引擎，两个前端：桌面应用用来浏览、打标签、批量改名和修问题，无头的 `tidycraft`
命令用在 CI 里。两边读同一份 `tidycraft.toml`，给出一样的结果——我想要的就是同一套
检查在这两处都能跑。

<img src="docs/screenshots/list-view.png" alt="Tidycraft 列表视图，右侧 3D 预览打开了一个模型" width="100%">

<sub>列表视图，扫的是一个资产包目录——84,974 个资产、895.9 MB。带类型筛选、标签，
以及选中模型的顶点数、面数与材质数。</sub>

<img src="docs/screenshots/grid-view.png" alt="Tidycraft 网格视图，浅色主题" width="100%">

<sub>同一个库的网格视图，浅色主题。</sub>

## 安装

**桌面应用**——从 [Releases](https://github.com/Lynthar/Tidycraft/releases) 取：

| 平台 | 安装包 |
|---|---|
| Windows | `Tidycraft_0.9.0_x64_en-US.msi`，或 `_x64-setup.exe` NSIS 安装器 |
| macOS | `Tidycraft_0.9.0_aarch64.dmg`（Apple Silicon），`_x64.dmg`（Intel） |
| Linux | `Tidycraft_0.9.0_amd64.deb`、`Tidycraft-0.9.0-1.x86_64.rpm`、`Tidycraft_0.9.0_amd64.AppImage` |

macOS 的包没有签名也没有公证，Gatekeeper 第一次会拦：

```bash
xattr -d com.apple.quarantine /Applications/Tidycraft.app
```

**命令行**——从 crates.io 装，或者直接拿独立二进制：

```bash
cargo install tidycraft
```

```bash
curl -L -o tidycraft https://github.com/Lynthar/Tidycraft/releases/latest/download/tidycraft-cli-linux-x86_64
chmod +x tidycraft
```

Windows 安装器和 Linux 的 `.deb` / `.rpm` 也会把 `tidycraft` 放进 PATH，`.dmg` 与
AppImage 不会，那两种情况下用上面的独立二进制。

从源码构建需要 Rust 1.88、Node 18+ 和 pnpm。

## 用法

```bash
tidycraft check .
```

```bash
tidycraft check . --fail-on warning     # error | warning | info
tidycraft check . --update-baseline     # 把今天的结果记成基线，把文件提交进去
tidycraft rules                         # 所有规则 id，以及本项目实际设成了什么
tidycraft explain naming.prefix         # 某条规则查什么、怎么调
tidycraft scan . --types texture,model  # 资产清单，JSON
```

`check` 还接 `--format human|json|sarif|github`、`--config`、`--baseline`、`--strict`、
`--max-issues`、`--summary-only`、`--group-by`。仓库根目录有一个 composite GitHub
Action，不想自己配 workflow 可以直接用它。

## 配置

规则写在工程根目录的 `tidycraft.toml` 里。`tidycraft rules` 会打印当前实际生效的配置，
`examples/tidycraft.example.toml` 是一份带注释的示例，可以照着改。

规则分几族：贴图（文件大小、二次幂、尺寸上下限、非正方形、mipmap、色彩空间）、命名
（长度、禁用字符、中文、前缀、大小写）、模型（顶点/面/材质）、音频（采样率、音效时长、
立体声音效、文件大小）、SHA256 精确查重、引用缺失、PBR 贴图组、DCC 源文件。配置段是
严格解析的——键名拼错会报错，不会被悄悄忽略。

工程目录里还会多出三个文件：`tidycraft.baseline.json`（你已接受的问题）、
`.tidycraft-tags.json`（你打的标签）、以及用了学习模式才有的 `tidycraft.ai.toml`。

## 能力边界

- **三个引擎的支持深度不一样，Unity 最深。** 引用缺失检测依赖 Unity GUID；Unreal 只有
  `.uproject` 识别和按扩展名分类，**没有 `.uasset` 依赖图**；Godot 的 `uid://` 引用是
  **有意不匹配**的。
- **命令行只有一个旗标会写盘。** `check`、`rules`、`explain`、`scan` 都不写文件，只有
  `check --update-baseline` 会写基线文件。所以可以放心把整个 `tidycraft` 前缀加进
  AI agent 的允许清单。
- **全绿不等于所有规则都检查过。** 读不到贴图尺寸时相关规则会静默跳过，所以一个 git-lfs
  指针没拉下来的检出会全绿。想让「读不到」也算失败，加 `--strict`。
- **它不做版本控制和团队协作。** git 集成只读取并展示状态，没有文件锁，也没有实时同步。
- **不生成 3D 缩略图。**「用外部编辑器打开」是交给系统默认程序处理，没有跟具体软件做集成。

## 文档

- [规则参考](docs/analyzer-rules.md) —— 每条规则查什么、什么时候触发、该设成多少。
- [贡献指南](CONTRIBUTING.md) —— 包括贡献以什么许可证接收。

## 安全

AI 打标是可选功能，默认关闭。打开之后，**你的 API key 以明文存放在 WebView 的
localStorage 里**：没有加密，也不走系统钥匙串。除 AI 打标之外，分析全部在本地进行；
只有开启这个功能时，才会有数据发往外部。

桌面端产物未做代码签名与公证。

## 许可证

GNU Affero 通用公共许可证 v3.0 only —— 见 [LICENSE](LICENSE)。Copyright (c) 2026 Lynthar。

贡献以 Apache License 2.0 接收，见 [CONTRIBUTING.md](CONTRIBUTING.md)。v0.8.5 及之前的
发布以 Apache 2.0 发出，并继续以该许可证提供。
