# Changelog

All notable changes to Tidycraft will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it leaves alpha.

## [Unreleased]

## [0.8.1] - 2026-07-17

### Added
- **One-click naming fixes.** Auto-fixable naming issues — forbidden characters, missing type prefix, and case style — now carry a **Fix** button, and the issue toolbar a **Fix all naming** action. Both open a review dialog with the proposed compliant name for every file (editable before applying, with an intra-batch collision warning and the same Godot `res://` reference warning as Rename). Fixes run through the existing rename engine, so they carry Unity `.meta` sidecars, migrate tags, and land as a single undo. Suggestions come from the project's `tidycraft.toml` naming rules; a strict library with tens of thousands of fixable files renders a capped preview (the batch still applies to all of them). Overlong and non-ASCII names stay manual — auto-fixing them would be lossy.
- **Multi-select type filtering with an "Art assets" quick group.** Type pills keep their single-select click (re-clicking the sole active type clears back to All), and Ctrl/Cmd+click now composes a union — several types at once, or everything except code and data. An "Art assets" pill applies that most-wanted union in one click (textures / models / audio / video / animation / materials / prefabs / scenes) and only appears when the project actually has non-art files to hide. The advanced panel's type chips and the command palette's filter entries toggle membership in the same set.
- **Unity package references resolve by name.** With a local `Library/PackageCache` present, the dependency graph and the missing-reference rule now read the packages' own `.meta` files: a reference into a package renders as a neutral "package asset" node carrying its file and package name instead of an amber unresolved warning, and the missing-reference rule stops flagging it entirely. Without a local cache (fresh clone, CI) everything degrades to the previous unresolved treatment.

### Changed
- **The dependency graph walks direction-consistently.** The neighborhood expands "what it uses" and "what uses it" as two separate sweeps (two levels each) instead of one direction-mixed walk — the mixed walk's second hop turned any shared dependency into a parade of unrelated siblings (open a material, meet every other material on the same atlas). Same Uses / Used-by reading as the arrowheads.

### Fixed
- **Clearing an HTML-report limit field no longer persists "unlimited".** The number inputs in Settings → Export read an emptied box as 0 — the unlimited sentinel — and saved it; they now commit only real numbers, and leaving the field restores the last committed value.
- **The dependency graph no longer paints ordinary Unity projects red.** 0.8.0's broken-reference nodes treated every GUID outside the scan as "missing" — but Unity's built-in bundles and package assets (under gitignored `Library/PackageCache`, e.g. the shader every URP material points at) are outside the scan *by design*, so real projects lit up with false alarms, and those widely-shared GUIDs acted as phantom hubs that dragged unrelated assets into the 2-hop view. Now: null / built-in GUIDs are exempt (same doctrine as the missing-reference rule), other unknown Unity GUIDs render as an amber **unresolved** node (ambiguous by construction — package asset, ignored file, or genuine breakage), and Godot targets are checked against the disk — red **missing** only when the file truly isn't there, a dashed neutral **exists, not scanned** node when it's merely outside the scan. The subgraph walk treats all non-asset nodes as terminals (no more hub explosions), nodes keep their identity (`res://` path / short GUID, full string in the tooltip), and the footer gained a legend for whichever of these states are on screen.

## [0.8.0] - 2026-07-14

### Changed
- **Duplicate findings are one issue per content group** — every member is listed on the issue (original first) instead of one issue per extra copy. A real library with a 3,178-file identical group used to balloon the analysis payload past a gigabyte and freeze the app to a black screen; the same run now completes in seconds, and the issue list shows one collapsible group card with per-member Locate actions (rendered members capped at 200).
- **The preview panel is permanently mounted**, collapsing to zero width outside the assets view instead of mounting on selection. First selection used to land it at near-zero width (react-resizable-panels doesn't re-apply `defaultSize` to late-mounted panels) and every deselect→reselect reshuffled all three panels.
- **Issue rows show project-relative paths** (absolute path in the tooltip), and the aspirational fix-verb buttons ("Decimate", "Resize") are gone — the action was always locate, so that's what the button says. Rows advertise their expandable detail with a chevron.
- **Exports go through the native save dialog** and report the outcome as a toast (with a "Show in folder" action) instead of silently dropping files into Downloads and swallowing failures.
- **The HTML report's row caps are configurable** (Settings → Export; 0 = unlimited). The report used to hard-truncate at 100 issues / 500 assets with no recourse on large projects; the truncation footer now names the active limit and points at the setting.
- **Naming-prefix rules skip DCC source files.** A `.blend` isn't a runtime mesh and a `.psd` isn't a shipped texture: `model_prefix` / `texture_prefix` conventions (`SM_` / `T_`) no longer flag authoring sources, which strict configs used to flood with false "Missing Prefix" warnings. The other naming checks (forbidden characters, length, case) still apply to sources.
- **Command-palette shortcuts are platform-native.** They printed macOS glyphs (`⌘⇧R`) on every OS; Windows / Linux now read `Ctrl+Shift+R`, and `Cmd` / `Ctrl` + `,` opens Settings (it was labelled but bound to nothing).
- **Stat charts use the app's palette and theme.** The pie and bars pull the same asset-type colours as the rest of the UI instead of a private hex set, the tooltip follows the light / dark theme instead of a hardcoded dark box, and the Top Extensions chart no longer drops its first row's axis label.
- **Batch-select checkboxes are discoverable.** A selection checkbox now reveals on row / card hover (and stays while a selection is active) instead of only appearing after the first Ctrl-click, so multi-select isn't a hidden gesture — clicking a row still previews, the checkbox is a separate visible affordance.
- **The dependency graph shows direction and broken references.** Edges now carry arrowheads (an asset points at what it uses), and a reference whose target isn't in the project renders as a red "missing" node instead of being silently dropped — so a dangling GUID / `res://` link is visible in the graph, not only in the issues list.
- **Model rows surface their vertex count where dimensions go.** The list's Dimensions column showed a bare "—" for meshes (which have no pixel width/height); it now shows their vertex count there (`1,234 v`) while images keep their W×H, and the dedicated Vertices column stays available (off by default).
- **The type-filter pill row hints that it scrolls.** When the pills overflow a narrow window the right edge fades instead of hard-clipping the last one, so it's clear there are more to reach (the scrollbar is deliberately hidden).
- **The delete dialog notes that trashing isn't undoable in-app.** Moving files to the recycle bin was never part of the undo history; the confirmation now says to restore from the recycle bin rather than expecting Ctrl+Z.

### Added
- **Duplicate groups have a "Clean up" action.** Pick the copy to keep right on the group card — every other member goes to the system recycle bin (Unity `.meta` sidecars follow), the resolved card disappears, and the shared delete confirmation reports any per-file failures inline. Duplicate detection graduates from reporting to actually tidying.
- **Godot renames warn when references would break.** The rename dialogs (single and batch) check which scenes / resources / scripts — and `project.godot` itself, whose main-scene and autoload entries are the most breaking of all — reference the file's `res://` path, and say so before you commit. Warning only for now (reference rewriting is a future step); Unity projects don't need it, their GUID references survive renames.
- **Engine info card on the Stats dashboard.** Unity shows the editor version (from `ProjectSettings/ProjectVersion.txt`), Godot shows version / main scene / renderer / autoloads / features (from `project.godot`), Unreal shows the engine association / modules / plugins / target platforms (from the `.uproject`). The parsers have existed since early on — now they have a screen, and it's the first engine-aware feature Unreal projects get.
- **Source-tool badges on DCC files.** `.blend` / `.psd` / `.spp` and friends now say which tool authored them: a badge next to the name in the list view, on the card in the grid view, and a "Source tool" row in the preview panel.
- **Prefab / scene structure in the preview panel.** Selecting a Unity prefab or scene shows its component types (sorted chips) and GUID-reference count, parsed on demand.
- **A directory-scope bar** above the asset list whenever a folder scope is active (set by the tree or issue-list Locate): scoped folder, asset count, and one-click "Show entire project". Empty states now say why the list is empty — no search matches, no filter matches, or an empty folder — and offer the same escape hatch.
- **Escape, focus trapping, initial focus, and `aria-modal` on every blocking dialog** via a shared modal shell; focus returns to the opener on close, and the delete confirmation starts on Cancel instead of the destructive button.
- **A "Manage Tags" entry in the asset-preview tag picker**, so an empty tag library is no longer a dead end.
- **Recent projects in the switcher.** Projects you've opened are remembered per machine and offered under a "Recent" section, so reopening one doesn't mean re-navigating the folder picker; the first-launch copy no longer points at a "recent project" that doesn't exist yet.
- **A section nav in Settings.** The settings panel had grown into one long scroll — it now has a left-hand table of contents that jumps to each section.
- **First launch follows the OS language.** With no saved preference, a `zh-*` system starts in Chinese instead of always defaulting to English; an explicit choice still wins.

### Fixed
- **Selecting the project root in the tree truly means "entire project"** — the root path and "no scope" used to be two distinct states, so the tree highlight, the type-pill counts, and the visible list could disagree after a Locate jump.
- **Scenes aren't flagged as unused.** A `.unity` / `.tscn` scene is a graph root — loaded from build settings, by name at runtime, or the editor — so having no incoming reference no longer marks it "unused". Scenes still count as references to the assets *they* use.
- **A model's vertex count is consistent.** The preview panel and the 3D viewer disagreed for the same mesh (the viewer counted three.js's per-face-expanded vertices); the viewer now shows the same canonical count as everywhere else.
- **Short audio clips show a real duration.** Sub-10-second SFX rendered as `0:00` under floor-rounded `m:ss`; they now show one decimal (`0.4s`).
- **Near-synonym tag suggestions merge.** The heuristic suggester surfaced the same pile of files under two labels (a filename token and the directory it sits in); groups that overlap heavily and share a name prefix now collapse into one.
- **The manual git-refresh spinner tracks the actual refresh** instead of a fixed 600 ms, so it can't report "done" while a slow refresh is still running.
- **The learning cost card shows the cost, not a button label.** For local (Ollama) providers it read "Continue (local, free)" — the button's own text — where a cost belongs; it now says "Free — runs locally".

## [0.7.0] - 2026-07-05

### Changed
- **Learning results now commit on review**. Learned rules are staged in memory and written to `tidycraft.ai.toml` only when the review panel saves; closing without saving truly discards the run, and unreviewed rules never influence tag suggestions.
- **PNG color-space detection parses the embedded ICC profile** ("sRGB" / "Linear" / unknown) instead of treating any profile as sRGB. Deliberately linear data textures are no longer mis-warned, unreadable profiles keep the rule silent, and the advanced filter gains a Linear option.
- **AI batch requests go out in chunks** (≤20 assets each) so a single reply can't blow the model's output cap. Each completed chunk is cached before the next request — a mid-run failure only re-bills the remainder on retry.
- **The AI result panel has one Apply button**, and it honors deselected chips ("apply everything still selected"); the bypassing "Apply all" is gone.
- **Colorspace filename hints drop the collision-prone `_r` / `_m` single-letter suffixes** ("arrow_r", "icon_m" no longer warn); the `_n` normal-map shorthand still does.
- **Release builds no longer ship devtools** (debug builds keep the auto-opened inspector).
- **Unity missing-reference issues are warnings and skip built-in resources.** The editor-shipped GUIDs (`unity default resources` / `unity_builtin_extra` — referenced by any project using a built-in shader, material, or UI sprite) no longer flood ordinary projects with false reports, and remaining hits are Warning instead of Error, since the scan can't see into gitignored `Library/` or `Packages/`.

### Fixed
- **External directory deletions now update the list, tree, and selection.** Deleting a watched folder from outside the app (Finder, `rm -rf`) — which macOS reports as one event on the folder rather than per file — now removes its assets from the file list and directory tree, and if it was the selected folder the view falls back to the project root instead of stranding on a ghost path.
- **Case-only rename conflict guards compare file identity, not names.** `foo.PNG → foo.png` still works everywhere, and on case-sensitive filesystems a rename can no longer silently overwrite a coexisting file whose name differs only in case (undo path included).
- **LLM cache entries are keyed by which asset a suggestion answers**, not by response position — a model skipping one asset no longer files every later suggestion under the wrong asset's cache slot, and hallucinated paths never enter the cache.
- **Output-cap truncation is reported as its own error** (Claude `stop_reason: max_tokens`, OpenAI `finish_reason: length`) instead of an opaque parse failure, for both tagging and learning calls.
- **Stale analysis results are flagged** with a re-run banner when files change after the run, and the stats pass count intersects with live files — it can no longer go negative.
- **AI modals no longer re-open with a previous project's data** when a request resolves after a project switch, so their Save / Apply can't write into the newly active project.
- READMEs now disclose plaintext API-key storage and the real cache-invalidation behavior; several stale code comments and dead doc links corrected.
- **OBJ material counts are material counts.** A 30-group export sharing one material used to report 30 (the sub-mesh count) and trip the max-materials rule; the count now comes from the loaded MTL, and stays unknown when the MTL can't be read.
- **glTF face counts cover non-indexed and non-triangle primitives.** Non-indexed meshes counted 0 faces, triangle strips/fans were divided by 3 instead of using `n − 2`, and line/point primitives counted phantom faces.
- **Compressed DDS textures report their real alpha.** DXT3/DXT5 and (via the DX10 header) BC2/BC3/BC7 read as alpha-carrying, BC4/BC5 data maps stay opaque — previously every compressed DDS claimed "no alpha" because only the uncompressed-layout flag was consulted.
- **`allowed_sample_rates = []` no longer crashes analysis** — an empty allow-list now means "don't check sample rates".
- **Cancelled scans emit a terminal progress event** instead of leaving the phase stuck at "parsing" (the UI previously recovered only via its own stop flag).
- **Windows: engine info panels get forward-slash paths** — Unity / Unreal / Godot project info was the last place backslashes leaked to the frontend.
- **Version drift and prerelease versions fail the pipeline up-front.** CI runs `check-version` before the build step (which used to silently repair drift on the runner), and `sync-version` rejects prerelease suffixes with a clear message instead of letting a tagged release die inside the Windows MSI bundler.
- **Shift-click ranges survive filtering and sorting.** The range anchor is now the anchored file itself, not its former row number — narrowing a search between clicks no longer selects an unrelated range or silently breaks selection.
- **Git-status filtering updates with git refreshes** (the filtered view used to lag behind the always-fresh row badges).
- **The stats dashboard follows file changes** — watcher events and Ctrl+R rescans refresh totals, charts, and largest-files instead of freezing at first render.
- **Ctrl+R no longer wipes the multi-selection** — the rescan's brief "no scan result yet" window was being mistaken for "every file was deleted".
- **Deleting a tag (or switching projects) clears it from the active tag filters.** A dead filter id used to AND-hide every asset with no pill left to un-click.
- **Batch-rename failures are visible.** On partial failure the dialog stays open with a per-file error list (the asset list still refreshes), the failed files stay selected for a retry, and a closed dialog no longer leaks its previous find/replace state into the next open. Previously any success closed the dialog before the result could render.
- **AI apply can no longer mint duplicate "(AI)" tags** — every create path checks for an existing tag of the same name first, including tags created earlier in the same run.
- **Ollama "model not found" reads as what it is.** A 404 now names the model and suggests `ollama pull`, instead of masquerading as a network error.
- **Metadata panels no longer render "null" / "-bit" / "0.0 kHz"** rows for fields a format doesn't have — unset metadata is omitted from the wire instead of serialized as `null`.
- **Lightbox grid / background / language toggles no longer reload the model.** Pressing G or L (or switching the app language) used to tear down the whole scene and reload the file — seconds on a large FBX, plus a camera reset; the toggles now patch the live scene, and viewer error messages re-translate on a language switch.
- **A slow model load can no longer pollute the next preview.** Switching models mid-load let the previous file's late completion hijack the animation mixer, paint "Failed to load" over the successfully rendered next model, or display the wrong stats; stale loader callbacks are now discarded (preview panel and fullscreen lightbox both).
- **The preview panel's thumbnail always matches the selected asset** — a slow thumbnail response for a previously selected texture no longer overwrites the current one's image (the image lightbox fallback consumed the same state).
- **Closing the active project can't strand you on a blank asset view.** If the next project in line was a session-restored stub that had never been scanned, it now scans on promotion — previously only Ctrl+R could recover it.
- **Tag edits that resolve during a project switch stay out of the other project's view.** A tag create/assign completing just after a quick switch no longer shows up as a phantom in the newly active project's tag list (the write itself always landed in the correct project).
- **Unity `.meta` changes now invalidate what depends on them.** Rewritten, created, or deleted sidecars re-parse their host asset — both live (the watcher used to drop every `.meta` event) and across scans (the sidecar's mtime is now part of the incremental cache key) — so `unity_guid`, the dependency graph, and unused-asset detection stop going stale when Unity regenerates a GUID. Existing scan caches re-scan once after upgrading.
- **The sidebar tree no longer walks or shows gitignored directories.** A Unity project's `Library/` (50k+ entries the scan never looks at) was re-walked on every scan and every watcher batch, and appeared in the tree even though none of its files were scanned; with ".gitignore respected" on, ignored directories are pruned from both the walk and the display.
- **"Locate asset" always lands on a visible, scrolled-to row.** Jumping to an asset (from issues, the command palette, stats, or the dependency graph) now clears whichever filters would hide it — search, type, advanced, or tag filters — while preserving filters the target already matches, and both the list and gallery scroll the row into view instead of leaving it somewhere off-screen.
- **Fullscreen lightboxes block the shortcuts underneath them.** Pressing Delete inside an image/model lightbox used to open a delete-confirm dialog hidden *behind* it with the confirm button focused — Enter then trashed files sight unseen; Ctrl+R rotated the image *and* kicked off a full project rescan. Lightboxes now gate the global shortcuts (and their own letter shortcuts ignore modifier chords).
- **Externally edited images refresh in place.** A texture overwritten outside the app now updates its thumbnail in the preview panel and gallery cards that never left the viewport — asset entries carry the file's mtime and thumbnail views re-fetch when it changes, instead of only after scrolling away and back.
- **AI Learning's cost preview prices the real prompt.** The estimate is computed from the exact prompt a learning run would send (same sampling, same builder) plus a single-document output budget — replacing a `depth × 10` asset-equivalent guess through the per-asset tagging estimator that could be off by orders of magnitude in either direction on directory-heavy projects.
- **AI Learning no longer teaches itself dead regex rules.** The prompt's `filename_regex` example anchored to the start of the whole relative path, so models imitating it emitted rules that never matched files inside subdirectories; the example now anchors per path segment and the prompt spells out the pitfall.
- **`@tauri-apps/api` (npm) realigned with the `tauri` crate** (both 2.11.1), silencing the dev-startup version-mismatch warning.
- **Every persisted file now writes crash-safely.** Undo history, the LLM response cache, `tidycraft.ai.toml`, and cached thumbnails all go through the same atomic temp-file-and-rename discipline the tags file already used — a crash mid-write can no longer leave a torn file, and two threads generating the same thumbnail can no longer interleave into a corrupt PNG that stayed cached.
- **Git integration reads the branch's real upstream** (`branch.<name>.remote`/`.merge`) instead of assuming `origin/<branch>` — renamed remotes and fork workflows get ahead/behind counts again. Untracked files now show their own badge (staged adds keep "new"), count as "has changes", and each git refresh runs one full status pass instead of two.
- **Removed 12 dead backend commands** (legacy scan entry points, config validators, engine-info probes, unused undo/tag/model queries) — none had a caller; the leftover `scan_project` was also the only scan entry without in-flight protection.
- **Analyzer accuracy:** SFX detection matches whole name tokens ("guitar" no longer reads as UI audio, "white" as a hit sound); textures without parseable dimensions (PSD/PSB) still get the file-size check — previously the largest files were exactly the exempt ones; duplicate reports come out in stable path order; `kebab-case` naming convention actually works (documented but unimplemented); name-length limits count characters, not bytes (CJK names tripled); PBR sets group case-insensitively and their tests now really exercise the rule.
- **Ollama requests set `num_ctx`** sized to the actual prompt — the server default (often 4096) silently truncated long prompts from the front, dropping the system prompt and producing unparseable replies.
- **A model response can no longer be voided by one invented value.** Unknown tag categories fall back to "other" and unknown rule kinds are dropped with a log — previously a single out-of-schema value failed parsing of the entire (already paid) response.
- **AI apply flows are project-pinned.** Suggestions for paths that weren't part of the request are dropped, and every AI apply/save loop aborts if the active project changes mid-run instead of writing into the newly opened project.
- **Tagging cost previews include the system prompt** (~previously uncounted, dominating the error on small text-only batches).
- **Rename requests are validated at the IPC boundary** — a new name containing path separators (also reachable through batch find→replace text) is rejected instead of traversing out of the folder.
- **Rescan keeps your place.** Ctrl+R no longer resets the folder selection to the project root, and if the selected folder is deleted externally the view falls back to the root instead of filtering against a ghost directory. The command palette's "Rescan" now truly matches Ctrl+R (it used to skip the cache clear).
- **Media previews handle broken files.** Unsupported codecs show an error instead of a black box, a failed play() no longer leaves the pause icon lying, and seeking before metadata loads is ignored instead of jumping to NaN.
- **Batch rename with an empty "find" is a no-op** instead of inserting the replacement between every character; the AI analyze dialog no longer remembers thumbnail-upload consent from a previous session run; opening the same project twice concurrently can no longer create duplicate sidebar entries.
- **Smoother background scans:** the app shell, header, asset list, and directory tree subscribe to exactly the state they render, so a scan ticking in another project no longer re-renders the whole tree ten times a second.
- **Localization pass:** a dozen missing keys now exist in both languages (empty state, project switcher, clear-search, fullscreen hints, info-filter empty state…), and the image lightbox, model lightbox toolbar, move/copy dialog, batch rename counts, and AI chip tooltips are translatable instead of hardcoded English.
- **Build hygiene:** `tsc -b` now type-checks `vite.config.ts` too; the unused `@/*` path alias and two unused Rust dependencies (`walkdir`, `serde_yaml`) are gone.
- Locale files' duplicate top-level `common` blocks merged (a JSON-editing key-loss trap; verified key-for-key lossless).

### Planned
- VRAM budget estimates per texture / per directory.
- Cross-engine reverse-reference graph (Unreal / Godot beyond Unity).
- DCC source-file linking phase 2: 1→N pairing for Substance Painter `.spp` (per-channel PNG outputs); git-status-aware severity bump when the source is dirty in the working tree.

## [0.6.0] — 2026-05-11

### Added
- **AI Tagging — Learning mode** (recommended default). One LLM call samples the project, derives local heuristic rules, persists them to `<project>/tidycraft.ai.toml`. After that, `suggest_tags` matches locally with zero per-asset LLM cost. AITagPanel header gets a status badge with Run / Re-learn / Review controls.
- **AI Tagging — per-asset mode** (opt-in via Settings → AI Tagging). Multi-provider LLM tagging (Claude / OpenAI / Ollama) behind a shared `LLMProvider` trait, with a per-asset SHA256 disk cache so partial-batch re-runs stay free. Cost-preview modal gates every call. Thumbnail upload defaults off; filename + path carry most of the signal for game assets.
- **Project-aware prompt**. The LLM receives `[project]` theme / goal from `tidycraft.toml` plus the user's existing tag list (descriptions + up to 5 sample paths per tag), and marks each suggestion as `existing` or `new`.
- **`Tag.description`** optional field. Edited from TagManager, shipped to the LLM as semantic context.
- **Settings → AI Tagging** section. Provider radio, model dropdown, endpoint override, privacy reset, per-asset-mode toggle. Ollama lists installed models live via `/api/tags`. Maintenance gains an AI-tag cache row.
- **TagFilterPanel sidebar header**. Right-click opens TagManager; new gear icon for click access.
- **`.vox` model preview** (MagicaVoxel). Handles nTRN scene-graph (v200+) and chunk-only (v150).
- **Lazy session restore**. Only the previously-active project scans + watches + refreshes git on launch; others hydrate as stubs and run on first switch.
- **DCC source-file linking** (opt-in via `[dcc_source]`). Pairs authoring sources (`.blend` / `.ma` / `.psd` / `.spp` / `.ztl` / `.max` etc.) with same-stem runtime exports (`.fbx` / `.png` / etc.) and warns when the source is newer. Configurable per-tool mappings, sibling-dir lookup, and `mtime_tolerance_secs` to absorb git-checkout sync.
- **AI Learning review polish**. Rule confidence is now an editable 0.5–1.0 slider; invalid `filename_regex` patterns get a ⚠ red chip flagged via JS `RegExp` validation.
- **Theme/goal write-back from LearnSetupModal**. Inputs are editable; Continue writes to `tidycraft.toml [project]` via `toml_edit`, preserving user comments and other sections on round-trip.

### Fixed
- **`runAnalysis` race-safe across project switches**. The analyze call snapshots `projectId` at click time. Mid-flight switches (sidebar button, `Ctrl+Shift+R`, Command Palette) no longer pollute another project's analysis state. The `isAnalyzing` guard now lives in the store, gating every entry path.
- **ModelLightbox center math**. Was offsetting models by `(scale − 1) × center` on enlarge; voxel exports especially.
- **Vertex colors preserved** across material conversions in 3D viewers. Fixes voxel OBJ files rendering flat gray.
- **OBJ loader honors the actual `mtllib` filename**. Skips MTL fetch when no `mtllib` line is declared, ending spurious 500s in the console.
- **Thumbnail whitelist** now matches the `image` crate's enabled features. `tiff` / `webp` / `hdr` / `exr` were decoder-supported but blocked at the entry check.
- **Thumbnail failures log at `debug`**, not error. Cleaner console for PSD / DDS / SVG / deep EXR.
- **Header rescan button now clears the disk cache** before re-opening. Button label finally matches behavior; replaces a `CACHE_VERSION` bump path.

### Changed
- `AIResultPanel` batches `addTagToAssets` per `(label, category, source)` group. 50 assets × 3 tags is 3 IPC calls instead of 150.
- `tidycraft.toml` template gains a top-level `[project]` block (theme / goal). The analyzer ignores it; AI Tagging reads it.
- `PROMPT_VERSION = 2` (per-asset) and `LEARNING_PROMPT_VERSION = 1` (learning) are independently bumpable. Version sits in each cache key so prompt-meaning changes never serve stale results.
- `suggest_tags` auto-routes through `rule_suggest::load_or_fallback`. AI rules from `tidycraft.ai.toml` if present; heuristic suggester otherwise. Output shape is identical either way.
- `RuleSuggester` now executes `filename_regex` rules. The `regex` crate is a proper dep with linear-time matching. Patterns that fail to compile silent-skip with a stderr warning.
- Scanner respects `.gitignore` by default via `ignore::WalkBuilder`. Also honors `.ignore`, git globals, `.git/info/exclude`, and skips hidden dot directories. **Behavior change**: Unity / Unreal projects no longer scan `Library/` / `Intermediate/` / `Saved/` by default. Toggle off via Settings → Scanning to restore "scan everything".

---

## [0.0.1] — Alpha

This is the initial public alpha. Everything below has shipped and is considered stable for normal use; bugs are expected and welcome via [GitHub Issues](https://github.com/Lynthar/Tidycraft/issues).

### Scanning & state
- Parallel `walkdir + rayon` scan with per-asset metadata extraction (image dims, model vertex/face/material counts, audio sample rate / duration / channels, video codec / framerate).
- Incremental scan cache keyed by `(mtime, size)` on disk under `dirs::cache_dir()/tidycraft/scans/`.
- Multi-project workspace: open several projects simultaneously, switch with `ProjectSwitcher`, cross-session restore of open paths.
- Live filesystem watcher (`notify-debouncer-full`, 500ms debounce) — external file changes refresh the asset list automatically.
- Engine detection: Unity / Unreal / Godot / generic, with engine-specific parsers (`.meta` GUIDs, `.uproject`, `project.godot`).

### Analysis
- Rule engine with per-asset families: `naming`, `texture`, `texture.color_space`, `model`, `audio`.
- Cross-asset checks: `duplicate` (SHA256), `missing_reference` (Unity GUID lookup), `pbr_set` (per-folder texture group completeness, with packed channels — ORM / MRA / RMA).
- Out-of-box defaults are minimal — only real bug checks fire by default; stylistic conventions (PoT, file-size, vertex budgets) are opt-in via `tidycraft.toml`.
- Glob-based `[ignore]` patterns for vendored / generated paths.
- Per-project `tidycraft.toml`; in-app **Settings → Analysis Rules → Edit** opens the file in the OS editor.
- Issue list with severity filter, group-by-rule, virtualized rendering for 10k+ issues, JSON / CSV / HTML export.

### Tag system
- Per-project tags, persisted to `.tidycraft-tags.json` at project root.
- Multi-select tag filter; rename / move syncs tag bindings; deleted files reaped by watcher.
- Heuristic tag suggestions (filename token / dimension + PBR channel / path segment), surfaced in the Sidebar **AI Tag** button.

### Asset browsing
- List + Gallery (grid) views with `@tanstack/react-virtual` virtualization.
- Thumbnail preview with disk cache (PNG / JPG / GIF / BMP / TGA).
- 3D model preview via Three.js: glTF / GLB / FBX / OBJ (+MTL) / DAE / 3DS. `.blend` recognized but routed to a "please export to GLB" message.
- Video preview (MP4 / WebM / MOV / AVI / MKV / M4V).
- Audio preview with waveform.
- Command Palette (⌘K / Ctrl+K) — quick navigation, filters, actions.

### File operations
- Safe delete to OS trash, move, copy, duplicate; all watcher-driven (no rescan needed).
- Batch rename with persistent undo (50-entry stack).
- Tags follow file across rename / move; orphan tag bindings cleaned automatically when files vanish.

### External editors
- Settings → External Editors maps file extensions to specific binaries (Photoshop / Blender / Audacity / etc.).
- AssetPreview's open button uses the mapping; falls back to OS default app.
- Routed through `tauri-plugin-opener` so Windows codepage / quoting / `%`-character paths work correctly.

### Cross-platform
- Windows / macOS / Linux supported.
- macOS shortcut display uses `⌘ ⇧` glyphs; Windows / Linux show `Ctrl+Shift+`.
- Path normalization (forward-slash everywhere on the boundary); `lib/pathUtils.ts` for shared helpers.
- Show in File Manager (renamed from "Reveal in Finder") works correctly on all three platforms.

### Git integration
- Branch info, ahead/behind counts, per-file change status badges.
- Toggleable in Settings → Git.

### i18n
- English + Simplified Chinese.

### Known limitations (alpha)
- LLM-backed semantic tagging is **not** in this build; the AI Tag panel is heuristic only.
- macOS / Windows binaries are **not code-signed** in v0.0.1 — first-launch warnings are expected.
- Tested up to ~50k assets per project; larger projects should work but may be slow.
- 3D preview limitations on paths with spaces / non-ASCII characters (Tauri asset protocol quirk; not Tidycraft-fixable).
