---
title: "`scoot --help` hides `--renderer` under `--tty`"
status: "open"
area: "config"
priority: "low"
blocked: null
---

# `scoot --help` hides `--renderer` under `--tty`

Found 2026-09-20 during the README audit
(`docs/backlog/resolved/readme-rewrite-done.md`): the `USAGE` string in
`crates/scoot/src/cli.rs` lists `[--renderer pixman|gles]` on the
`--headless` and `--nested` lines but not on the `--tty` line — while the
parser in the same file (`compositor()`, one function for all three
backends) accepts `--renderer` everywhere, `README.md`'s Running section
shows `scoot --tty --renderer gles -- foot`, and `docs/configuration.md` /
`docs/tty.md` both document it there.

Verified live at `cc1c79e`: `/var/cargo-target/debug/scoot --help` prints
`scoot --tty [--gpu PATH] [--mode WxH] [--socket PATH] [--config PATH]
[-- COMMAND...]` with no `--renderer`, and `render::resolve`
(`crates/scoot/src/compositor/render.rs`) is what decides it per backend —
warn-and-pixman without the `gpu-scanout` feature, GPU scanout with it.

## Fix

One-line docs-adjacent code change (which is why it was filed, not fixed,
in the docs-only audit PR): add `[--renderer pixman|gles]` to the `--tty`
usage line so `--help` agrees with the parser and the docs. No behavior
change — the flag already parses there. A unit test pinning each usage line
against the flags its backend's parse accepts would keep it from drifting
again.
