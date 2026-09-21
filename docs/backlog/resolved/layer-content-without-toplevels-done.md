---
title: "[nested] top-layer exclusive-zone content missing from frames with no toplevels mapped — CLOSED, could not reproduce"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# [nested] top-layer exclusive-zone content missing from frames with no toplevels mapped — CLOSED, could not reproduce

## What it said

Filed as gh issue #183 (live field report, same session as #182).
scoot `4f8707c` `--nested`, 430x744, noctalia-shell 4.7.7 bar with 30px
exclusive zone. Screenshot matrix (`scoot msg screenshot`): no toplevels
→ wallpaper only, bar absent; ghostty mapped → bar rendered on top;
ghostty closed → bar absent again. `scoot msg outputs` holds
`usable={y:30,h:714}` throughout — the zone-owning surface stays
registered while contributing no content.

Reporter's trace (verified, not assumed): `gather_elements` collects
`ABOVE_WINDOWS` layers unconditionally and `layer_elements` iterates
every mapped surface — no window-count gate in the render path at either
rev. The suspect was the layer-map/state side: the zone-owning surface
present for exclusion but skipped (or bufferless) in composition exactly
when the window stack is empty.

## Verdict

**Could not reproduce, on current `main` (54d0038) or on the reported rev
itself (4f8707c). No compositor change made — deliberately.** A real
Top-layer bar with a committed buffer draws with zero toplevels mapped at
both revs, live over `--nested` and in-harness. The field symptom —
zone reserved, wallpaper-only pixels — is exactly what a *bufferless*
bar looks like by design (pinned: a bar reserves its zone from its
initial commit, before its first buffer), so the compositor was almost
certainly rendering faithfully all along: quickshell committed no bar
buffer while the workspace was empty and started committing once a
toplevel existed. That last clause is inference, not proof — quickshell
is not installed on the dev VM, so the client half is stated, not
claimed. What is proven is the compositor half: any layer client that
commits buffers draws with no windows, at both revs.

## Evidence

Harness (dev VM, `cargo test -p scoot layer_shell`: 91 passed):
two new pixel pins, both in `layer_shell/tests/layout.rs` —
`a_mapped_bar_draws_with_no_windows_mapped` (mapped 30px Top bar, zero
windows: zone `{y:30}`, bar pixels in the strip, wallpaper below) and
`a_bufferless_bar_reserves_but_draws_nothing_with_no_windows_mapped`
(committed but bufferless, zero windows: zone held, background pixels,
no crash, no garbage). Sensitivity proven the honest way: with the
`ABOVE_WINDOWS` gather temporarily neutered the mapped-bar test goes
red, restored it goes green. Both ride the full workspace gate
(1306 passed, 6 skipped).

Live `--nested`, current `main` (binary built from `54d0038` + this
branch's test-only change): cage (headless, pixman) as host, inner
`scoot --nested --width 430 --height 744` (pixman; the host-follow
resizes it to 1280x720, so the matrix is size-relative, not
pixel-identical to the report), throwaway probe bar (Top layer,
full-width 30px, exclusive zone 30, opaque magenta shm buffer — the
noctalia-bar shape; built as a temporary example, not committed) plus
`foot` standing in for ghostty. `background_color = "#123456"`, magenta
census over the top 30 rows by ImageMagick:

- cell A (bar, no toplevels): `usable.y = 30`, magenta fraction **1**,
  below-bar all wallpaper.
- cell B (bar + foot): magenta fraction **1**, below-bar 35.5%
  non-wallpaper (foot content — the leg is meaningful).
- cell C (bar, foot closed, `windows: []`): magenta fraction **1**,
  below-bar all wallpaper; `cell-a.png` and `cell-c.png` are
  byte-identical (md5 `c0025751f56d10697ebf0bfd9b63d839`).

Positive control, reported rev `4f8707c` (built from a `/tmp` worktree
copy, private target dir, old `scoot msg` client grammar): the same
three cells give magenta **1/1/1** with identical byte sizes
(19562/22421/19562). So the path was not broken at the reported rev
either — "fixed by earlier work" would overclaim; nothing in this path
needed fixing. The phase-A–D/multi-output and layer-map/callback work
since `4f8707c` did not move this behavior.

Code-path audit, both revs: every surface commit ends in
`request_render()` unconditionally (`handlers.rs`); `gather_elements`
collects `ABOVE_WINDOWS` with no window-count gate (`elements.rs`);
every mapped layer surface gets frame callbacks every unlocked frame
(`headless.rs` render tail); `scoot msg screenshot` renders anything
outstanding synchronously (`capture_pixels_for` → `render()`).

## Companion issue #182 (shared root?)

Confirmed separate — and the mechanism is now complete on both sides.
#182's record already refuted a shared root for the *input* path (clicks
deliver at both revs, with and without toplevels) and noted the bar in
its probes painted with zero toplevels. This ticket closes the remaining
half: the render path has no window-count gate at either rev either.
What the two reports share is a client that was not in the state the
report assumed (nothing mapped to hit / nothing committed to draw),
not a compositor keying layer handling off window existence.

## What did NOT change, and why

- No compositor code: there is no defect to fix, and manufacturing one
  would risk the composition and focus behavior the existing suite pins.
- No benchmark: no hot path changed shape (frame scheduling and
  composition untouched — idle-CPU question does not arise).
- No README change: no user-facing surface changed; no documented
  behavior was wrong.
- Bug-bash edge cases from the ticket: bufferless-bar-with-zero-windows
  is a new pin (above); bar-unmapped-leaves-zone rides the existing
  destroy/disconnect tests; output-2-bar-with-no-windows rides the
  existing `a_bar_on_the_second_output_shrinks_only_the_second_output`
  pixel test; overlay-with-no-windows shares the exact `layer_elements`
  call the sensitivity probe neutered; zero-windows-plus-zero-layers and
  lock-covering-bar are clear-color / locked-list paths nothing here
  touches — all verified green in the full run, not re-argued.
- The throwaway probe bar (`crates/scoot/examples/throwaway_layerbar.rs`)
  was deleted before commit; only the two harness tests ship.

## Methodological notes (for the reviewer)

- Long-lived background processes do not survive an ssh session close on
  the dev VM (systemd session cleanup): cage/scoot/probe chains must run
  start-to-finish inside a single ssh invocation. `pkill -f cage` matches
  the invoking shell's own command line and kills it — use `pkill -x`.
- The magick census idiom reads inverted from how it looks: after
  `-opaque` maps the target color to white, `+opaque white` blacks out
  everything else, so `mean` is the *target* fraction, not the remainder.
- Smoke runs (`scripts/smoke-test.sh` headless + `MODE=--nested`, both
  exit 0) executed against binaries built from the same tree plus the
  since-deleted example target, which no shipped binary links — the
  compositor evidence is keyed to the final tree by the re-run full gate
  after the deletion.

Full gate (dev VM, final tree `54d0038` + test-only change): `cargo
nextest run --workspace` 1306 passed / 6 skipped; `cargo clippy
--workspace --all-targets -- -D warnings` clean; `cargo fmt --check
--all` clean.
