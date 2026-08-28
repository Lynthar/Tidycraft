# Contributing to Tidycraft

Thanks for taking the time. Bug reports, reproductions and patches are all welcome.

## Licensing of contributions

Tidycraft is distributed under the **GNU AGPL v3.0** (`AGPL-3.0-only`). Contributions are accepted under the **Apache License 2.0** instead.

Apache 2.0 is one-way compatible with the AGPL, so your patch can ship inside the AGPL-licensed project unchanged. Keeping incoming contributions under the more permissive licence also leaves the project able to offer the combined work under other terms later — a commercial licence for a company whose policy forbids the AGPL, or a distribution channel that requires different terms. Without this, a single AGPL-only patch would foreclose that for everyone.

By submitting a pull request you agree that your contribution is licensed under Apache 2.0 and that the project may distribute it under the AGPL or other terms.

## Developer Certificate of Origin

Every commit must be signed off. Add `-s` to `git commit` and it appends the line for you:

```
Signed-off-by: Your Name <your.email@example.com>
```

The sign-off is your statement that you wrote the patch or otherwise have the right to submit it under Apache 2.0 — the full text is the [Developer Certificate of Origin 1.1](https://developercertificate.org/). There is no separate agreement to sign.

## Before opening a pull request

Every gate below has to be green. They run on all three platforms in CI, so a failure here is a failure there.

```bash
pnpm install
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --bins
cargo check -p tidycraft-core --no-default-features
pnpm check-version
pnpm build
```

Read the output, not just the exit code: cargo reports some problems — output filename collisions among them — as warnings that no `-D warnings` flag can turn into a failure.

Run the lint gates on the toolchain CI uses, which is `stable`. A local toolchain even a few days old makes `cargo clippy` a false green. The MSRV is 1.88, and a new lint's suggested fix can raise it silently — check with `rustc +1.88.0` before taking one.

The frontend has no test runner, linter or formatter; `pnpm build` runs `tsc` and is its only gate. Match the style of the code around you.

## Notes on the codebase

- The Rust side is a cargo workspace: `crates/tidycraft-core` (scanning, analysis, engine parsers — no Tauri types), `crates/tidycraft` (the headless `tidycraft` command), and `src-tauri` (the Tauri command layer and session state). Run cargo from the repository root.
- The desktop app and the CLI share one analysis pipeline. A rule change lands in `tidycraft-core` and both get it; nothing in either front end should re-implement analysis.
- Every CLI verb is read-only, and that is a contract rather than a coincidence: it lets people allow the whole `tidycraft` command prefix in an agent or CI sandbox. A verb that writes has to be a new verb, never a flag on an existing one.
- `docs/analyzer-rules.md` documents each rule's defaults and how to tune it. A new rule needs an entry there.
