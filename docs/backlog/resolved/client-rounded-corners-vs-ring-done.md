---
title: "A client that rounds its own corners (libadwaita dialogs) leaves a background sliver between its corner and scoot's ring — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Client-rounded corners vs the ring — DONE

RESOLVED 2026-09-27 (PR #288). Serves **daily-drive** (the look of every
GTK 4 / libadwaita dialog with `corner_radius` set).

## Verdict

**"Let the client's corners show", as a ring-colored backdrop — no radius
knowledge anywhere.** Every window the CSD rule calls self-decorated gets a
solid rect in its own ring color drawn directly under its drawn rect. The
client's own alpha shapes the visible part, so the corners read as the ring
hugging the client's curve. scoot's clip and ring are untouched (a square
CSD window covers the backdrop completely, so nothing changes there, and a
self-rounding client with a *smaller* radius than configured keeps today's
clip rather than gaining a new cut).

The option that won, measured against the ticket's list:

- **Match the ring to the client — out.** Nothing on the wire reports the
  client's radius, confirmed once more: the `xdg_toplevel.configure` and
  `zxdg_toplevel_decoration_v1.configure` streams in the live captures
  carry sizes, states, bounds, capabilities and the decoration mode, never
  a corner radius. Per-commit alpha measurement and per-toolkit guessing
  stay what the ticket called them (costly+fragile, guessing).
- **Let the client's corners show — won, in backdrop form.** The ticket
  imagined skipping scoot's clip plus a client-suited ring radius; the
  radius half is the same unknown as above, so the fix keeps the clip and
  instead fills the crescent with ring color. No radius appears anywhere
  in the diff.
- **Tell libadwaita it has no rounded corners — out**, no protocol, as
  documented.
- **Floating settlement — out**, landed earlier and doesn't decide, as
  documented.

## The signal, verified with data

"Self-decorated" is exactly "never created a
`zxdg_toplevel_decoration_v1` object", recorded per window in
`State::decoration_bound` (`XdgDecorationHandler::new_decoration`,
removed in `remove_window`). Reading Smithay's `decoration_mode` field
alone does *not* work: scoot sets `ServerSide` in pending state for every
window under `prefer_no_csd`, bound or not — only the object's existence
says what the client heard. Under `prefer_no_csd = false` the mode last
sent breaks the tie (a bound window told `ClientSide` at its own request
backdrops like an unbound one).

Bind behavior measured from `WAYLAND_DEBUG` client logs on the dev VM
(`get_toplevel_decoration` present or not):

| client | binds | shape | rule says |
| --- | --- | --- | --- |
| foot | yes | square, fills slot | SSD |
| mpv (`--vo=wlshm`) | yes | square video | SSD |
| floaty (test client) | yes | square | SSD |
| zenity `--info/--question/--file-selection` (GTK 4.22/libadwaita) | no | self-rounded | CSD |
| gtk3-widget-factory | no | CSD headerbar | CSD |
| weston-terminal | no | square, draws past its slot | CSD (backdrop covered → invisible) |

No measured client is square, exactly slot-fitting, *and* unbound — the
one shape whose scoot-cut corners change from background to ring color —
so nothing regressed for windows that do not round themselves. foot is
additionally pinned byte-identical live (below).

## What the fix does per frame, per window

`render/elements.rs`'s `backdrop` closure returns the drawn rect, or
`None`: ring width 0, window unknown/gone, X window (no toplevel),
bound + forced `ServerSide`, bound + mode last sent `ServerSide`,
window-wide `wp_alpha_modifier_v1` translucency (an opaque backdrop would
tint every pixel — caught live by `alpha_modifier`'s own suite, whose
client never binds decorations), nothing committed yet (a fill under no
content would be a solid rect). `decorations.rs` pushes the rect through
the window's persistent backdrop buffer first in its ring run — counted
in the floating `spans`, so it travels with the window — in its ring
color, on both the square and painted paths. Steady state adds one
`HashSet` lookup, one map lookup, one toplevel-handle clone and three
uncontended state locks per placed window per frame, plus one opaque
solid fill per CSD window. A corner-squares-only fill was considered and
rejected: it reintroduces radius guessing for clients rounder than
`radius + width`.

## Evidence (dev VM, `~/evidence/csd/`)

Binaries: `scoot-before` (`a7067df8…`, `main` `ee1b424`) and
`scoot-after-final` (`4529b91a…`, the merged tree — the after screenshots
were re-taken with it and are byte-identical to the in-progress after
shots, 9,216,000 bytes). Session: headless 1920x1200, green background,
`corner_radius = 10`, `focus_ring_width = 4`, default ring colors, foot
tiled + `zenity --info` floating and focused (`run.sh`).

Crescent-band score (`csd_band.py`: per corner row, the solid band
between scoot's clip staircase and the measured content edge; exact ring
is *not* expected since the client's own shadow lies over the backdrop —
blue-dominant `b−r>40, b−g>40` is):

| run | score |
| --- | --- |
| before 1.5 | 0/58 rows blue (foot-dark `(34,34,34)`, shadow-green `(0,243,0)`) |
| after 1.5 | 58/58 |
| before 1.0 | 0/36 |
| after 1.0 | 36/36 |

Control (foot binds → SSD → no backdrop): the whole foot + ring region
minus the dialog overlap is byte-identical before/after — 0 differing px
of 1,031,436 at 1.5, 0 of 1,089,305 at 1.0. Screenshots viewed
(`before-s1.5.png`, `after-s1.5.png`): the dialog's corners read blue,
hugging its own curve, over both the terminal and the background.

Harness (fail-first: the two backdrop tests fail on `main`, the
SSD-twin and uncommitted tests pass): `rounded/tests/csd.rs` maps a
client drawing transparent corners past radius 12 — unbound (backdrop,
ring color in the crescent), bound (no backdrop, background in the
crescent, negotiation itself asserted), bound + `ClientSide` at its own
request under `prefer_no_csd = false` (backdrop), mapped-but-uncommitted
(hollow ring, no fill). `decorations` unit tests pin the push mechanics
(backdrop travels in the floating span, inactive color, width-0 /
fullscreen / invisible exclusion). The shared rounded-test client now
negotiates decorations for its plain windows (foot-shaped); only the
GTK-shaped step stays unbound. Full `cargo nextest run --workspace`:
2364/2365 (the one failure is `scootbg`'s backlog test running out of fds
under the shell's 1024 limit — untouched lane, passes at 65536, whole
`scootbg` package 302/302 there). `cargo test -p scoot`: 1778/0.
`clippy -D warnings`, `fmt --check`, `scripts/smoke-test.sh` (22 ok):
clean. Rounded/decorations/alpha suites also pass under
`SCOOT_TEST_RENDERER=gles`.

## Benchmarks (dev VM, dev profile, 3 runs per tree)

`rounded_corners_cost` (bench clients never bind → CSD on the new tree,
so the radius-12 tier prices the backdrop): paired square→rounded
premium, medians of 3 runs — single 11.9→13.8, tiled3 15.5→22.3,
overhang3 8.4→9.1, float1 11.0→17.0 (percentage points). I.e. about one
opaque solid fill per CSD window per redrawn frame (~tens of µs here),
0–7pp on the rounding premium of multi-CSD-window scenes; nothing at
idle (no render when clean) and nothing for SSD or windowless scenes
(`render_frame_cost`: 285.9/96.0µs BEST before vs 248.2/86.6µs after —
noise). Raw files: `bench-{before,after}-{rounded,frame}*.txt`.

## Filed context (the open ticket, verbatim)

Filed 2026-09-24 from the PR #240 review (gh #205 follow-up). Serves
**daily-drive** (the look of every GTK 4 / libadwaita dialog with
`corner_radius` set).

### What is wrong

PR #240 clips and rings what a client actually draws, so a short dialog is
ringed where it ends, not around its whole slot. That is a big improvement
over `main`, where the ring circled the empty slot. But libadwaita draws its
own rounded corners even when tiled, with a larger radius than scoot's, and
anti-aliases them. scoot's staircase clip cuts nothing that the client has
not already made transparent. So between the client's own curve and scoot's
tighter ring there is a crescent of background at every corner.

Measured on the dev VM (`zenity --info`, GTK 4.22 / libadwaita, a 300x223
window geometry; `corner_radius = 10`, `focus_ring_width = 4`, scoot
`340f3c7`; `check_corners.py` from the gh #205 evidence):

- at 1.5, all four corners fail, with 53–59 background pixels inside the
  clip per corner and 0/15 staircase rows hugging the content;
- at 1.0, all four fail with 19 each;
- the outer arc passes (21/21, 14/14).

The client's own shadow also darkens a few ring pixels ("content outside
clip" 12–16 at 1.5). The montage is in the dev VM's
`~/evidence/r205/review1/zenity-corners-s1.5.png`, and the raw frames are
alongside it.

### What floating windows changed (2026-09-25, floating windows PR 1)

libadwaita dialogs now float: GTK 4 attaches `xdg_dialog_v1` to them, and
scoot floats any window that does (`zenity --info`, `--question` and
`--file-selection`, GTK 4.22, all float on the dev VM). A floating window is
sent no `tiled_*` state and chooses its own size, so the ring now surrounds
a dialog drawn at its natural size (300x223 for `zenity --info`) rather than
a short client parked in a full-height column.

It does **not** fix the corners. The crescent is still there: a crop of the
top-left corner of `zenity --question` floating over `foot` at scale 1.5
(scoot `b54cef7`, `corner_radius = 10`, `focus_ring_width = 4`) shows the
terminal's dark background between the dialog's own anti-aliased curve and
scoot's tighter ring (dev VM
`~/evidence/float/live/s15/corner-tl.png`, 8x zoom of the 60x60 crop at
(955, 577) of `user-dialog-centred-over-terminal.png`). No shadow was seen
outside the ring in that crop. The options below stand; floating removed
the "settle it inside floating windows" option, which turned out not to
decide the radius.

### Options

- ~~**Floating windows will cover most of it.**~~ They landed (see above):
  the dialogs float and keep their own corners, and a floating window gets
  the same ring as any other, so the mismatch is unchanged.
- **Match the ring to the client.** Take the client's radius as the ring's
  inner radius. Nothing on the wire says what that radius is, so it would
  have to be measured from the buffer's alpha (per commit, which is costly
  and fragile) or guessed per toolkit.
- **Let the client's corners show.** Skip scoot's clip, and ring the
  client's geometry with a radius that suits the client, for windows that
  draw client-side decorations (no `zxdg_toplevel_decoration_v1`
  `ServerSide`). This needs a signal for "this client rounds itself". The
  CSD decoration mode is the obvious candidate.
- **Tell libadwaita it has no rounded corners.** There is no protocol for
  that. Tiled states are the closest, and libadwaita dialogs ignore them for
  corners.

### Done looks like

A self-rounding dialog at 1.0 and 1.5 shows either its own corners with a
matching ring, or scoot's corners with no background crescent, pinned by
a pixel check like `check_corners.py`, and with nothing regressed for
windows that do not round themselves.
