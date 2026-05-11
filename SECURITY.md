# Security Policy

## Reporting an issue

If you find a security-relevant problem in Tidycraft — for example a path traversal, unsafe binary parsing, leaked API keys, or anything that could compromise user data — **please don't open a public GitHub Issue**.

Instead:

- **Preferred**: open a private [GitHub Security Advisory](https://github.com/Lynthar/Tidycraft/security/advisories/new). The maintainer is notified privately and the report stays embargoed until a fix ships.
- **Alternative**: DM the maintainer via their GitHub profile ([@Lynthar](https://github.com/Lynthar)) requesting a private channel.

## In scope

- Path traversal: any code path that lets a maliciously crafted asset / config read or write files outside the project root.
- Binary-parser crashes or unsafe behavior: FBX (`fbxcel-dom`), images (`image`), audio (`symphonia`), video (`mp4` / `matroska-demuxer`), models (`gltf` / `tobj`).
- TOML config parsing: anything that lets `tidycraft.toml` cause unintended behavior.
- **AI Tagging API key handling**: stored plaintext in `localStorage["tidycraft-settings"]` under `aiProviders[id].apiKey`. A first-save warning toast informs the user. OS-keyring storage (`tauri-plugin-stronghold` or similar) is on the roadmap. Reports of concrete leak paths beyond "localStorage is plaintext" are in scope.
- **AI Tagging upload semantics**: `llm_suggest_tags` ships the following payload to the configured cloud provider:
  - thumbnails (256×256 PNG, base64) — only when the user opts in (off by default);
  - filenames + project-relative paths;
  - the user's existing tag list (names, descriptions, up to 5 sample paths per tag);
  - project `[theme]` / `[goal]` from `tidycraft.toml`.

  First call per provider requires explicit consent (revocable in Settings → AI Tagging → Reset upload consent). The Ollama path is local — no upload. Reports of code paths that upload more than the documented payload, or upload without consent, are in scope.
- Code execution from a maliciously crafted asset file or config.
- Tauri command auth gaps (commands that should require a registered project but don't).

## Out of scope

- Bugs in third-party crates / npm packages — please report those upstream. `cargo audit` / `npm audit` are good first steps.
- Issues that require physical access to the user's machine.
- The app being slow on adversarial input (e.g. opening a project with billions of zero-byte files). Tidycraft is intended for non-adversarial input; a "DoS" report on degenerate inputs alone isn't actionable.
- Anything that depends on the user explicitly running malicious shell commands themselves.

## Response time

This is a single-maintainer alpha project. Expect days to a week for the first response. Actively-exploited issues will be triaged faster.

## Disclosure timeline

After a fix ships in a release, the security advisory is made public with credit to the reporter (unless they prefer to stay anonymous).

## Past advisories

None yet.
