// Build the CLI and stage it where the Tauri bundler expects a sidecar:
// `src-tauri/binaries/tidycraft-<host-triple>[.exe]`. The bundler strips the
// triple again when it packages, so the installed app gets a plain `tidycraft`.
//
// This runs before every `tauri dev` and `tauri build`, because declaring
// `externalBin` makes the staged file a hard build prerequisite — a fresh
// clone that skipped it fails the bundler with a missing-file error.
// Pass `--release` for the profile the bundler ships.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";

// The bundler looks the sidecar up by the triple it is building FOR, which is
// not the host when the release workflow cross-compiles (both macOS artifacts
// come off one arm64 runner). `TAURI_ENV_TARGET_TRIPLE` is set by the Tauri
// CLI; the release workflow also sets TIDYCRAFT_SIDECAR_TARGET for the case
// where this script runs on its own. Host is the fallback, via `rustc -vV` —
// deriving it from process.platform/arch guesses wrong on the splits that
// matter (musl, gnu vs msvc), and a wrong name here surfaces much later as
// "sidecar not found" at bundle time.
const hostTriple = () => {
  const line = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
    .split("\n")
    .find((l) => l.startsWith("host:"));
  if (!line) {
    console.error("[sidecar] could not read the host triple from `rustc -vV`");
    process.exit(1);
  }
  return line.slice("host:".length).trim();
};
const host = hostTriple();
const triple =
  process.env.TIDYCRAFT_SIDECAR_TARGET ||
  process.env.TAURI_ENV_TARGET_TRIPLE ||
  host;
const ext = triple.includes("windows") ? ".exe" : "";

const args = ["build", "-p", "tidycraft"];
if (release) args.push("--release");
// Cargo drops cross-compiled output under target/<triple>/, host builds don't.
const crossing = triple !== host;
if (crossing) args.push("--target", triple);
console.log(`[sidecar] cargo ${args.join(" ")}`);
execFileSync("cargo", args, { cwd: root, stdio: "inherit" });

const built = crossing
  ? join(root, "target", triple, profile, `tidycraft${ext}`)
  : join(root, "target", profile, `tidycraft${ext}`);
if (!existsSync(built)) {
  console.error(`[sidecar] cargo reported success but ${built} is missing`);
  process.exit(1);
}

const outDir = join(root, "src-tauri", "binaries");
mkdirSync(outDir, { recursive: true });
const staged = join(outDir, `tidycraft-${triple}${ext}`);
copyFileSync(built, staged);
console.log(`[sidecar] staged ${staged}`);
