---
title: "A client that rounds its own corners (libadwaita dialogs) leaves a background sliver between its corner and scoot's ring"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Client-rounded corners vs the ring

Filed 2026-09-24 from the PR #240 review (gh #205 follow-up). Serves
**daily-drive** (the look of every GTK 4 / libadwaita dialog with
`corner_radius` set).

## What is wrong

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

## What floating windows changed (2026-09-25, floating windows PR 1)

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

## Options

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

## Done looks like

A self-rounding dialog at 1.0 and 1.5 shows either its own corners with a
matching ring, or scoot's corners with no background crescent, pinned by
a pixel check like `check_corners.py`, and with nothing regressed for
windows that do not round themselves.
