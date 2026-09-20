---
title: "`scoot --help` hides `--renderer` under `--tty` — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `scoot --help` hides `--renderer` under `--tty` — DONE

## The entry as filed

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

## Resolution

RESOLVED 2026-09-20, fixed as filed plus the review note the coordinator
folded in (PR #159 N3): the `--tty` usage line now reads `scoot --tty
[--gpu PATH] [--mode WxH] [--renderer pixman|gles] [--socket PATH]
[--config PATH] [-- COMMAND...]` — no behavior change, the flag already
parsed there. `RendererKind::Gles`'s rustdoc no longer says gles is
"`--headless`/`--nested` only … `--tty` warns and keeps pixman"
unconditionally: `--headless`/`--nested` honour it in every build (the
offscreen pipeline), while under `--tty` it needs the `gpu-scanout`
feature (GPU scanout tier, with `tty::init`'s warn-and-dumb-tier fallback
when the device cannot drive it) and warns-and-keeps-pixman without it —
the exact truth `compositor::render::resolve`'s docs state. The same stale
unconditional claim in two adjacent comments (`compositor::mod`'s
"no GPU scanout path yet", `config.rs`'s module doc) was corrected in the
same one-line class. Pinned by
`cli::tests::usage_lists_renderer_on_every_backend_that_parses_it`, which
asserts each of the three usage lines names `[--renderer pixman|gles]`
and that `--renderer pixman|gles` parses on all three backends (fail-first:
failed pre-fix on the `--tty` line only).

Bug-bash audit, all three usage lines vs parser acceptance: `compositor()`
accepts every flag on every backend (no gating), so the usage lines show
each backend's meaningful subset — `--outputs` only on `--headless`,
`--gpu`/`--mode` only on `--tty` (each warned-and-ignored or silently
dropped elsewhere per `CompositorOptions`'s docs). `--renderer` was the
only divergence: parseable everywhere, documented everywhere, shown in
`--help` in two of three places. No README change (it already documented
the flag correctly — that was the bug).

Evidence (all Mac-side, `main` at `9badc8b`, branch
`backlog/cli-help-tty-renderer`): `cargo build -p scoot` then
`./target/debug/scoot --help` before shows the `--tty` line without
`--renderer`, after shows it with — `diff` of the two outputs moves only
that line. Live parse proof: `--tty --renderer gles` passes parsing
(reaches the macOS "compositor only runs on Linux" refusal),
`--tty --renderer glse` is refused as `invalid --renderer`. Full cheap
set green: `cargo nextest run --workspace` (137 passed),
`cargo clippy -p scoot --all-targets -- -D warnings`, `cargo fmt --check
-p scoot`. Benchmark n/a (one string literal, two comments, one unit
test — nothing on a hot path).
