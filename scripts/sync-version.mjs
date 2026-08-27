// Single-source the app version: `package.json > version` is the source of
// truth, propagated into the workspace Cargo.toml (which every crate inherits)
// and tauri.conf.json. `--check` verifies only and exits 1 on drift.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const checkOnly = process.argv.includes("--check");

const pkgPath = join(root, "package.json");
const cargoPath = join(root, "Cargo.toml");
const confPath = join(root, "src-tauri", "tauri.conf.json");
const cliCargoPath = join(root, "crates", "tidycraft-cli", "Cargo.toml");

const version = JSON.parse(readFileSync(pkgPath, "utf8")).version;
// Fully anchored: a prerelease or build suffix (`0.7.0-beta.1`) would propagate
// into tauri.conf.json and only explode much later, inside the tagged release
// build — the Windows MSI bundler cannot encode non-numeric versions.
if (typeof version !== "string" || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error(`[sync-version] package.json version must be plain MAJOR.MINOR.PATCH, got: ${JSON.stringify(version)}`);
  console.error("[sync-version] prerelease/build suffixes are rejected because the Windows MSI bundler cannot encode them (a v-tag release would fail at the bundling step).");
  process.exit(1);
}

// Each target: a label, its path, and a regex whose capture groups are
// (prefix)(current version)(suffix). The version sits in group 2.
const targets = [
  {
    label: "Cargo.toml",
    path: cargoPath,
    // `\n[ \t]*version` pins the match to a line-leading `version` key inside
    // the [workspace.package] table — never `rust-version` or a dependency's.
    re: /(\[workspace\.package\][\s\S]*?\n[ \t]*version[ \t]*=[ \t]*")([^"]*)(")/,
  },
  {
    label: "tauri.conf.json",
    path: confPath,
    re: /("version"[ \t]*:[ \t]*")([^"]*)(")/,
  },
  {
    // The one dependency version that cannot inherit from the workspace:
    // crates.io rejects a bare path dependency, so tidycraft-cli spells out
    // tidycraft-core's version. Left out of this list it silently ages, and
    // the first symptom is a published cli pulling a stale core.
    label: "tidycraft-cli/Cargo.toml (tidycraft-core dep)",
    path: cliCargoPath,
    re: /(tidycraft-core[ \t]*=[ \t]*\{[^}]*?version[ \t]*=[ \t]*")([^"]*)(")/,
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
  console.log(`[sync-version] all files already at ${version}`);
}
