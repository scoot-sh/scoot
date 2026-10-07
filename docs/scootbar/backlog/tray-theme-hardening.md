---
title: "Tray: themed-icon hardening follow-ups (N1-N4 from #497 review)"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# Tray: themed-icon hardening follow-ups (N1-N4 from #497 review)

Filed 2026-10-07 from the independent review of #497 (tray themed icon
names; review report in the coordinator's scratchpad). The review passed
#497 with four non-blocking findings; this entry files them together and
fixes them in one PR. Serves **daily-drive**: IconName-only items
(pasystray, Ayatana apps) are the common case, and these close the
remaining hostile-input and worst-case-memory gaps in their lookup path.

## The gap

All in `crates/scootbar/src/modules/tray/theme.rs` (paths from the
review):

- **N1 FIFO/TOCTOU in `read_if_inside`.** A same-user peer controlling
  its own `IconThemePath` dir can swap the path from a regular file
  (seen by the pre-open `metadata()` check) to a held-open FIFO before
  `File::open`, blocking the bus turn. Direct non-regular files are
  refused fast (proven in the review); only the rename race remains.
- **N2 no total cap on decoded themed bytes.** Per-icon cap 512 px/side
  (1 MiB RGBA) x 32 items = 32 MiB resident worst case, against the
  pixmap path's 64 px/side x 8 entries.
- **N3 size-dir order is first-match, not closest.** Same name in
  `22x22` and `48x48` resolves to the 22x22 entry regardless of drawn
  size; the module doc claims "nearest".
- **N4 name with an extension misses.** `foo.png` looks for
  `foo.png.png`, though many apps send a name with an extension.

## What to do

- N1: after `open`, `fstat` and require a regular file (open with
  `O_NONBLOCK`/`O_NOFOLLOW` via `rustix`, no new dependency), so a
  swapped-in FIFO/device can never block the bus turn; re-check the
  size cap on the opened fd's metadata and bound the read with `take`.
  Test that fails before: FIFO through the inner reader behind a
  watchdog.
- N2: cap the stored side at 64 px (the pixmap path's `MAX_STORED_SIDE`;
  larger decodes are downscaled on the bus turn, once per icon version)
  and add an explicit total byte budget across items
  (`MAX_ITEMS` x 64x64 RGBA = 512 KiB). Real theme icons ship 16-48 px
  (measured on the M2: hicolor app PNGs at 16/22/24/32/48/64, plus
  larger sizes for launchers); nothing real a tray draws exceeds 64.
  Test the per-icon cap and the total both.
- N3: order the fixed size dirs by the freedesktop closest-size rule
  (minimal distance to the drawn size among the sizes present; ties
  prefer the larger, which downscales sharper), `scalable` and the
  base-direct file last; fix the doc claim to the truth. Test with
  22x22 red vs 48x48 green fixtures at drawn sizes 24 and 48.
- N4: strip one trailing `.png`; return hidden (`None`) for `.svg`/`.xpm`.
  Test.

Hot path: all of this runs once per `GetAll` answer on the bus turn,
never per frame; keep it allocation-light (stack-sorted probe order,
no new dependency) and bounded. Publish release file/`.text` size and
idle RSS/wakeups before/after; the themed-icon size row was waived by
the maintainer on 2026-10-07 for the decoder only, so any further
growth is reported plainly and nothing is waived here.

## Not in this ticket

Review findings N5-N8 (attention-with-pixmap doc qualification, dead
`Indexed` arm comment, stale resolved-ticket measurement prose, XDG
relative entries): note-only or separate small docs edits, not this PR.
Full theme inheritance past `hicolor` (future work, said in the
icon-themes ticket).
