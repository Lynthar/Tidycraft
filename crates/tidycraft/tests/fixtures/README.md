# CLI fixtures

`unity-project/` is a real Unity 6 project tree, small enough to read in one
sitting: 28 assets, each built to trip exactly one rule at the thresholds set in
its `tidycraft.toml`, plus the clean files that prove the rules stay quiet.
`tests/cli.rs` runs the built `tidycraft` binary over a private copy of it and
compares the report with what `docs/analyzer-rules.md` says must be there.

The asset bytes come from `make_unity_project.py` (deterministic PNG / OBJ /
WAV writers), the `.meta` sidecars and the two `.mat` files from a Unity 6000.3
editor importing that tree; `M_Broken.mat` then had its texture guid edited to
one no `.meta` carries, which is the whole point of it.

To change the fixture: edit and re-run the script, open `unity-project/` in a
Unity 6 editor once so new files get their `.meta`, update `EXPECTED` in
`tests/cli.rs` from the rule documentation, and never from the tool's output.
Keep `Library/`, `Temp/` and `Logs/` out.
