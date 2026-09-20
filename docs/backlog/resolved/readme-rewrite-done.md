---
title: "README rewrite: helpful, concise, aimed at the user — relocate, don't drop"
status: "resolved"
area: "meta"
priority: "medium"
blocked: null
resolved: "2026-09-20"
---

# README rewrite: audit + tightening — DONE 2026-09-20

Requested by the user, 2026-09-19 (third item of the directed queue, after
milestone 6 and the `scootctl` split, both now landed). PR #132
(`docs/readme-for-users`) already gave `README.md` its front-door shape (218
lines, linking out to `docs/`), so this was **an audit + tightening pass,
not a from-scratch rewrite** — verify the rewrite meets its own constraint,
fix what drifted since, and cut what serves reviewers instead of users.

## Verdict: the ticket's premise was mostly already satisfied

PR #132 had done the real work. `README.md` carries no defensive
constructions, no measurements, no resource bounds, no internal ticket/PR
links, and every enumerating surface is already a table; the credibility
signal, the why-over-niri, the screenshot and the "Not yet" list are all
where the `update-docs` skill wants them. What the item-by-item audit below
turned up was five doc corrections (two of them stale since a feature
landed), one precision restructure at the exact anchor the ticket suspected,
and one code bug — filed, not fixed, per the ticket's own rule. Nothing was
cut for serving reviewers, because nothing in `README.md` did; the changes
are all "present but wrong", not "present but noisy".

## Audit findings, per checklist item

- **Keys table vs actual defaults: already correct, no change.**
  All 18 bindings (12 rows) match `Keybindings::default()` in
  `crates/scoot/src/compositor/keybindings.rs:87-175` entry for entry
  (modifiers, keysyms, actions, `foot` as the `Super+Return` spawn), and
  the VT row matches `vt_switch_bindings()` (`keybindings.rs:233-253`,
  `Ctrl+Alt+F1`..`F12`, `--tty`-only). `focus-workspace-index` is
  correctly absent, as the ticket requires.
- **Configuring vs `Config::default()`: example valid, one wording fix.**
  The example's values are non-defaults (`gap = 8` vs `12` in
  `crates/scoot-core/src/config.rs:18`; four `column_widths` vs three),
  which is legitimate — an example, not a defaults listing — and every
  field it names exists with a valid value. The two startup exceptions the
  section names are real and the only two: `[tty] gpu` hard-errors
  (`config.rs` fail-closed resolve) and `[renderer] backend = "gles"` with
  no working EGL startup-errors (`GlesBackend::new`'s
  `could not build the GLES renderer on any EGL device`, propagated via
  `render.rs:245`). Fixed: "logs a warning" → "is logged" (`README.md`
  Configuring), because whole-file failures (malformed TOML, unknown
  field) log at error and discard the file
  (`docs/configuration.md` Failure semantics).
- **Running/Install vs both `--help` outputs: verified live, one code bug
  filed.** `scoot --help` and `scootctl --help` were run on the dev VM
  against Linux binaries built at `cc1c79e`; `scootctl --help` is
  byte-identical Mac-built, Linux-built, and `nix run .#scootctl`
  (`nix run .#scootctl -- --help` ran green on macOS 2026-09-20, proving
  the Install line). `scootctl`-primary / `scoot msg`-alias stated
  correctly; macOS story (Darwin default is `scootctl`, flake Darwin is
  Apple-Silicon-only) verified against `flake.nix:17-21,189-205` and
  `nix flake show`. **Filed, not fixed:
  `docs/backlog/config/cli-help-tty-missing-renderer.md`** — the `--tty`
  usage line omits `[--renderer pixman|gles]` though the parser accepts it
  on all backends and the docs advertise `--tty --renderer gles`.
- **What-works anchors vs `docs/protocols.md`: all resolve; the dmabuf
  suspicion was wrong, fixed by precision anyway.** All 16 internal links
  in `README.md` resolve (checked mechanically with the GitHub slug
  algorithm), and all 31 protocol name/version pairs match a live
  `wayland-info` dump from `--headless` at `cc1c79e` — the table had zero
  drift. The suspected dmabuf anchor landed in the section that describes
  the dmabuf advertisement, so it was never broken; it is now precise:
  the dmabuf block is its own `### GPU-rendering clients
  (zwp_linux_dmabuf_v1)` subsection with its own anchor, and the
  `README.md` row, the protocols version-table row, and the `tty.md`
  renderer link all target it. XWayland "nothing" row still true
  (protocols "Not implemented"); "Not yet" list still true (one output,
  no reload, narrow/unproven GPU scanout, no macOS adapter).
- **Developing vs CI + crate docs: three corrections.** Crate one-liners
  match all four `Cargo.toml` descriptions (verified line by line; the
  `scoot-core` "window management state and layout" wording is the
  sanctioned exception to the compositor-not-window-manager rule). The CI
  paragraph said `ldd` checks for `libgbm`/`libEGL` — the workflow greps
  `^lib(gbm|drm|EGL|GLESv2)` and its own comments say the EGL halves are
  belt-and-braces over `dlopen`ed libraries; said the macOS job checks
  "the `scootctl` client" — it is `cargo check --workspace
  --all-targets` (verified the `scoot` crate checks clean on macOS with
  its Linux halves cfg'd out); said "everything through `nix develop`" —
  the smoke step uses `nix shell --inputs-from .`. All three corrected in
  `README.md`.
- **Tighten: nothing to cut.** See the verdict above. The remaining edits
  in `docs/` are corrections, not cuts: `configuration.md`'s Failure
  semantics said "one exception" while its own `[renderer]` table
  documented the second startup error (stale since the renderer error
  landed); the `[renderer]` table said `"gles"` is "`--headless`/`--nested`
  only" (wrong in `gpu-scanout` builds, where `--tty` scans out); the
  flags table never stated the `--width`/`--height` default (`1600x1000`,
  `cli.rs:223-224`); and its `--width` refusal example used single quotes
  where the binary prints backticks (verified live).

## Out of scope, as filed

- The `--help`/`--tty`/`--renderer` mismatch above (filed as
  `docs/backlog/config/cli-help-tty-missing-renderer.md`, not fixed —
  docs-only PR).
- No code, no wire, no `PROTOCOL_VERSION` change. `cargo fmt` / `clippy`
  untouched — no code changed, so there was nothing for them to check.

## Original ticket (verbatim, for the record)

> PR #132 (`docs/readme-for-users`) already gave `README.md` its front-door
> shape (218 lines, linking out to `docs/`), so this is **an audit +
> tightening pass, not a from-scratch rewrite** — verify the rewrite meets
> its own constraint, fix what drifted since, and cut what serves reviewers
> instead of users. … **Relocate, not drop**: anything that leaves
> `README.md` must remain exactly one link away under `docs/`, and anything
> README *says* must match the code. … Out of scope: `--print-default-config`,
> config reload, workspace keybindings, or any other functional gap the audit
> walks past: file-or-link, don't build. No code, no wire, no
> `PROTOCOL_VERSION` change. Docs-only PR.
