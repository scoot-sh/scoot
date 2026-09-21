---
title: "README + docs consistency re-audit (post-#159 surfaces) — DONE"
status: "resolved"
area: "meta"
priority: "medium"
blocked: null
resolved: "2026-09-21"
---

# README + docs re-audit — DONE 2026-09-21

Coordinator-directed (no gh issue — the PR body carries no Fixes keyword).
PR #159 audited everything up to `9badc8b`; this re-audits every
user-facing surface that landed since, against `README.md` and `docs/`,
under the same constraint the original rewrite established:
relocate-don't-drop, every stated value matches code. Docs-only: no code,
no wire, no `PROTOCOL_VERSION` change. `cargo fmt` / `clippy` untouched —
no code changed, so there was nothing for them to check.

## Verdict: most surfaces already consistent; six fixes

The per-PR README updates held — twelve of the thirteen surfaces below
audited clean, verified by running rather than reading. What drifted is
all "present but wrong", and five of the six fixes share one cause:
milestone 19 phases A–D landed multi-output under `--headless` while
three sentences in `docs/` still said "one output". The sixth is the
`docs/nix.md` live-defaults paste going stale two ways at once. Nothing
was cut; one stale header was retitled, the rest are wording-level
corrections.

## Per-surface findings

- **`scootctl` split + `scoot msg` alias + Darwin story + flake outputs
  (`scoot`/`scootctl`/`scoot-gpu`) + modules + `session.command` +
  `sessionScript` pairing: already right.** `scoot --help` and
  `scootctl --help` run live on Mac binaries show `scoot msg REQUEST`
  as the kept alias with identical request/action grammars; `flake.nix`
  (`default` = compositor on Linux, client on Darwin; `scoot-gpu`
  `overrideAttrs` with `cargoBuildFeatures`; `apps` mirroring all three;
  `homeModules` alias; Darwin HM default `null`) matches README Install
  and `docs/nix.md` line for line; `nix/modules/home.nix`
  (`sessionScript` at `<dirOf configFile>/session.sh`) and
  `nix/modules/nixos.nix` (`session.command`, null = bare `--tty`)
  match the pairing both docs describe.
- **`scootctl reload` + PROTOCOL_VERSION 3 + applied/refused semantics:
  already right.** `crates/scoot-ipc/src/lib.rs:47` is `3`; both live
  `--version` lines print `scoot 0.1.0 (ipc protocol 3)`; the applied
  set (gap, ring width/colors, background, `corner_radius`,
  `prefer_no_csd`, binds) and refused set (column widths, cursor
  fields, scale, gpu, renderer backend, autostart) in
  `crates/scoot/src/compositor/reload.rs:69-85,126-233` match
  `docs/configuration.md#reloading-the-config` and
  `docs/ipc.md#what-the-replies-carry` field for field, including the
  two-empty-lists and failed-reload-keeps-running rules.
- **`focus-workspace-index` binds + `move-window-to-workspace-index`:
  already right.** 36 binds in `Keybindings::default()`
  (`keybindings.rs:105-199` base plus the `DIGITS` loop, pinned by
  `numbered_workspaces_are_bound_by_default`), 36 in the README keys
  table and the configuration.md defaults table, the same grammar in
  `docs/ipc.md#actions`, both live `--help` outputs, the live
  `--print-default-config` emission (18 base + 18 numbered), and the
  `scoot-ipc` `Action` enum (`action.rs:24-74`, additive, no bump).
- **Session env vars + portals + `scoot-portals.conf`: already right.**
  `session_env.rs` (unconditional `XDG_CURRENT_DESKTOP=scoot`,
  fill-where-unset `XDG_SESSION_TYPE`/`XDG_SESSION_DESKTOP`) matches
  the configuration.md environment section; `resources/scoot-portals.conf`
  (`default=gtk`, ScreenCast/Screenshot to `wlr`) matches the portals
  section including the 0.8.0 floor, the `grim` requirement and the
  activation-environment bus half.
- **`[autostart]` + Starting-a-session: already right.**
  `compositor/mod.rs:288-303` runs autostart entries first in file
  order through `act`, then the `--` command through `spawn`;
  fail-open-per-entry, no supervision, never-re-run-on-reload all match
  `docs/configuration.md#autostart` and `#starting-a-session`.
- **`--headless --outputs N` + `screenshot --output` + per-output
  bars/lock/groups: FIXED (README header, ipc.md name row).**
  README's "Not yet" lead bullet was still headlined "One output is
  composited" with "plug in a second monitor and it stays dark" as the
  first sentence — false for `--headless` since phases A–D (every
  output has its own render target, `screenshot_refusal` refuses only
  unknown ids per `outputs/tests.rs:474-494`, `None` defaults to the
  primary per `screenshot.rs:316`). Retitled "Multi-output is partial",
  keeping every true sub-claim and the `--tty` second-monitor truth.
  `docs/ipc.md`'s `outputs` table said `(scoot has exactly one)` and
  named `headless` as the only non-`--tty` name — now one entry per
  output, `headless`, `headless-2`, … (configuration.md and the phase D
  record already said this). The configuration.md "More than one
  output" section itself audited clean.
- **`[appearance] corner_radius`: already right in configuration.md,
  FIXED in nix.md (see below).** Default `0`, negative-clamps-to-`0`,
  per-window half-min-dimension clamp, popups square, live-reloadable —
  `decorations.rs:200-253,340-345` matches the table; the emission
  carries `# corner_radius = 0`.
- **`--version` (both binaries) + `--print-default-config: already
  right.** Both version lines run live (above); the emission captured
  live from a dev-VM Linux build of this tree's code (see Evidence)
  matches configuration.md's account (commented keys, stdout-only,
  macOS honest refusal) byte for byte in structure.
- **`--tty` usage `--renderer` line: already right.** Fixed in #160;
  live `scoot --help` shows `[--renderer pixman|gles]` on all three
  usage lines and configuration.md's flags table matches.
- **CI picture: FIXED (one word).** The README Developing paragraph
  matched `ci.yml` (fmt/clippy/nextest + `cargo test`, `--headless`
  smoke, `nix fmt`, both-jobs `flake check -L`, macOS
  `cargo check --workspace --all-targets`) and `nix-build.yml`
  (main-only `.#scoot .#scootctl`) — except the ldd parenthetical
  named only `libgbm` as the live assertion while the workflow greps
  `^lib(gbm|drm|EGL|GLESv2)`. Now `libgbm`/`libdrm`.
- **CHANGELOG session-identity + repo-move entries: already right, no
  contradictions.** README Install (`github:scoot-sh/scoot`),
  configuration.md env section, nix.md migration section all agree
  with the CHANGELOG entries added in #199.
- **`docs/protocols.md`: three stale one-output sentences FIXED.**
  `output_leave` ("scoot has one output" — now "nothing moves a window
  across outputs yet", the phase-D reason); output-management
  ("exactly one output today" — now "one head per output,
  still read-only"); output scaling ("Startup only, and one output" —
  now "session-wide", the operative claim "no per-output setting" was
  already true). Workspaces, layer-shell, session-lock and dmabuf
  sections had already been updated by their phase PRs and audited
  clean.
- **`docs/nix.md` live-defaults reference: FIXED (stale two ways).**
  Pasted at `82371df`, before `corner_radius` landed — the
  `corner_radius = 0` line was missing; added, matching the live
  emission. Worse, the paste had uncommented two keys the emission
  keeps commented because they are *unset placeholders, not defaults*:
  `cursor_theme = "Adwaita"` (would override `$XCURSOR_THEME`
  following) and `tty.gpu = "/dev/dri/card0"` (would fail-close onto
  one device instead of the automatic search). Both stay commented
  exactly as emitted, with the reason stated; the "uncommented" claim
  and the mechanical-differences note corrected to match. Every other
  value re-verified against the live emission (below) — no further
  drift.

## Evidence (recorded, not narrated)

- Base: `main` at `6fb8773`, clean; branch `backlog/readme-rereview`
  (docs-only — code identical to `6fb8773` throughout).
- Mac, repo checkout: `./target/debug/scoot --help` (three usage
  lines all carrying `[--renderer pixman|gles]`, `scoot msg REQUEST`,
  full requests/actions); `./target/debug/scootctl --help` (same
  grammar); `./target/debug/scoot --version` and
  `./target/debug/scootctl --version` both print
  `scoot 0.1.0 (ipc protocol 3)`; `./target/debug/scoot
  --print-default-config` prints the macOS honest refusal
  (`needs the compositor's defaults, which only exist on Linux`),
  matching configuration.md's macOS note.
- Dev VM (`ssh -p 2222 dev@localhost`, both VMs checked up via
  `nc -z localhost 31022/2222` first, neither restarted):
  `CARGO_TARGET_DIR=/tmp/scoot-rereview-target cargo build
  --manifest-path /mnt/scoot/Cargo.toml -p scoot` green
  (1m 50s); `/tmp/scoot-rereview-target/debug/scoot
  --print-default-config` captured in full — 18 commented base binds
  + 18 numbered (`super+shift+ctrl+j` spelling included), `# gap =
  12`, widths `[0.3333333333333333, 0.5, 0.6666666666666666]`,
  `# background_color = "#14141a"` (matches `hex()`'s round-half-up
  at `config.rs:688-701`; configuration.md's `#141419` is the
  pixman pixel-sampled render, documented as such — not a conflict),
  `# corner_radius = 0`, `# cursor_theme` / `# gpu` commented
  placeholders. Raw output kept in the PR description's working
  notes; the nix.md paste now matches it value for value.
- Anchor check for every touched link: mechanical (GitHub slug
  algorithm over the edited files) — all resolve; the only new links
  are the backlog-index line and the ROADMAP entry below, both to
  this record.
- `cargo fmt` / `clippy`: not run — no code changed (docs-only);
  stated, not skipped silently.

## Out of scope, as filed

- No functional gap was built. Nothing walked past needed filing:
  the audit's one candidate (protocols.md already covers per-output
  lock/capture/groups) was already documented by its phase PR.
- Reviewer-serving prose elsewhere (e.g. tty.md's measurement-heavy
  renderer section) left alone — out of scope, and it serves
  integrators choosing a renderer, not reviewers.
