// Single-source the app version. `package.json > version` is the source of
// truth (the titlebar imports it directly, and `pnpm version` edits it); this
// script propagates that value into the two files Tauri reads at build time:
//   - src-tauri/Cargo.toml  ([package] version)
//   - src-tauri/tauri.conf.json  (top-level version)
//
// Wired into `package.json`'s "build" script, so `pnpm build` and
// `pnpm tauri build` (whose beforeBuildCommand is `pnpm build`) keep all three
// in lockstep automatically — no manual three-file edit per release.
//
// Usage:
//   node scripts/sync-version.mjs           # rewrite the two derived files
//   node scripts/sync-version.mjs --check    # verify only; exit 1 on drift (CI)
//
// Why regex instead of a TOML/JSON round-trip:
//   - Cargo.toml carries hand-written comments a parse+reserialize would drop.
//     We anchor on the [package] table's own `version = "..."` line (matched
//     via a preceding newline, so the `rust-version` key and every dependency
//     `version` are left untouched), leaving all other bytes identical.
//   - We only write when the value actually changes, so an up-to-date Cargo.toml
//     keeps its mtime and doesn't trigger a needless Rust rebuild.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const checkOnly = process.argv.includes("--check");

const pkgPath = join(root, "package.json");
const cargoPath = join(root, "src-tauri", "Cargo.toml");
const confPath = join(root, "src-tauri", "tauri.conf.json");

const version = JSON.parse(readFileSync(pkgPath, "utf8")).version;
if (typeof version !== "string" || !/^\d+\.\d+\.\d+/.test(version)) {
  console.error(`[sync-version] package.json version is not semver: ${JSON.stringify(version)}`);
  process.exit(1);
}

// Each target: a label, its path, and a regex whose capture groups are
// (prefix)(current version)(suffix). The version sits in group 2.
const targets = [
  {
    label: "Cargo.toml",
    path: cargoPath,
    // `\n[ \t]*version` pins the match to a line-leading `version` key inside
    // the [package] table — never `rust-version` or a dependency's version.
    re: /(\[package\][\s\S]*?\n[ \t]*version[ \t]*=[ \t]*")([^"]*)(")/,
  },
  {
    label: "tauri.conf.json",
    path: confPath,
    re: /("version"[ \t]*:[ \t]*")([^"]*)(")/,
  },
];

let drift = false;
for (const { label, path, re } of targets) {
  const src = readFileSync(path, "utf8");
  const m = src.match(re);
  if (!m) {
    console.error(`[sync-version] could not locate the version field in ${label}`);
    process.exit(1);
  }
  if (m[2] === version) continue;

  drift = true;
  if (checkOnly) {
    console.error(`[sync-version] drift: ${label} is ${m[2]}, expected ${version}`);
  } else {
    writeFileSync(path, src.replace(re, `$1${version}$3`));
    console.log(`[sync-version] ${label}: ${m[2]} -> ${version}`);
  }
}

if (checkOnly && drift) {
  console.error("[sync-version] versions out of sync — run `node scripts/sync-version.mjs`");
  process.exit(1);
}
if (!drift) {
  console.log(`[sync-version] all three files already at ${version}`);
}
