# Contributing to Tidycraft

Thanks for taking a look. This file is the short version; the full developer guide (architecture, file layout, how to add a Tauri command / analyzer rule / asset parser) lives in [`docs/development.md`](docs/development.md).

## TL;DR

- **Bug reports & feature requests** → open a [GitHub Issue](https://github.com/Lynthar/Tidycraft/issues) using one of the templates.
- **Open-ended questions** → [GitHub Discussions](https://github.com/Lynthar/Tidycraft/discussions).
- **Code contributions** → fork, branch, PR. For non-trivial changes, open an issue first to confirm scope.

## Project status

Tidycraft is **alpha-stage** and **single-maintainer**. PRs are welcome but expect:

- **Slow review.** Days to weeks, not hours.
- **Scope discipline.** PRs that change the architecture, add new dependencies, or rewrite existing modules will likely be asked to first surface as an issue / discussion.
- **No CLA.** Your contribution is licensed under Apache 2.0 by submitting it (same as the rest of the project).

## Setup

```bash
pnpm install          # first time
pnpm tauri dev        # run the full app
pnpm build            # frontend type-check + bundle
cd src-tauri
cargo test --lib      # backend unit tests
cargo check           # fast Rust typecheck
```

System prerequisites:

- Rust 1.75+
- Node 18+ (20 recommended)
- pnpm 8+ (9 used in CI)
- **Linux only:** `webkit2gtk-4.1-dev`, GTK3 dev headers, `librsvg2-dev`, `libssl-dev`, `patchelf`
- **macOS only:** Xcode Command Line Tools
- **Windows only:** MSVC build tools + WebView2 (preinstalled on Windows 10+)

## What changes are easy to land

- **Bug fixes** with a clear repro and minimal diff.
- **New analyzer rules** (per-asset or cross-asset) — see `docs/development.md §5`.
- **Format parsers** for asset types we don't yet handle (e.g. AVI video metadata, FLAC audio metadata, USD meshes).
- **i18n** — add a new locale by copying `src/i18n/locales/en.json` and translating.

## What changes need a discussion first

- **New top-level UI features** (new panels, new viewMode, new tab).
- **Changes to the rule engine architecture** (e.g. async rules, rule chaining).
- **Adding a new dependency**, especially a heavy one (LLM clients, vision models, image-processing crates).
- **Cross-cutting refactors** (renaming public modules, changing the Tauri command schema).

Open a GitHub Discussion or Issue first — saves you from writing 500 lines we'll have to ask you to revert.

## Code style

- **Rust:** `rustfmt` defaults; no formatter is enforced in CI but try to match nearby code.
- **TypeScript:** match nearby code (2-space indent, double quotes, trailing commas). No ESLint config — `tsc` (via `pnpm build`) is the only gate.
- **Commits:** focused, single-purpose. Subject line + blank line + brief body explaining the **why**. Don't bundle unrelated changes.

## Tests

- **Rust:** `cargo test --lib` runs the backend's unit tests (across `scanner`, `analyzer`, `watcher`, `undo`, `tags`, `llm`, etc.). Add tests for any new pure-function logic.
- **Frontend:** no test runner configured. `pnpm build` runs `tsc`, which is the only TS gate.
- **Manual:** if your change touches paths, filesystems, or platform-specific APIs, verify on at least Windows AND a POSIX target before submitting.

## Localization (i18n)

Every user-facing string in components goes through `t("key")`. When adding a string:

1. Add it to **both** `src/i18n/locales/en.json` and `zh.json` (lockstep).
2. Group under an existing feature key, or add a new section if the area is new.
3. Interpolation: `{{name}}`; pass values via the second arg to `t()`.

## Reporting security issues

For security-sensitive issues (LLM API key handling, path traversal, anything that could leak user data), see [`SECURITY.md`](SECURITY.md) for the disclosure path. **Don't open a public GitHub Issue** for these.

## License

By contributing, you agree your contribution will be licensed under the [Apache License 2.0](LICENSE), the same as the rest of the project.
