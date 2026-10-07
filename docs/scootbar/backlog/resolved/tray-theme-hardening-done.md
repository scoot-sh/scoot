---
title: "Tray: themed-icon hardening follow-ups (N1-N4 from #497 review)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
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

## Status (2026-10-07): landed in #502

All four fixed in `crates/scootbar/src/modules/tray/theme.rs`, each
with a test that fails before it (proven by revert-run-restore on the
M2: seam commit `958ceac07` fails all five new tests, fix commit
`022ff36fd` passes them):

- N1: `read_file` opens `O_NONBLOCK`/`O_NOFOLLOW` through `rustix`
  (no new dependency) and requires a regular file under the cap on the
  opened fd's own metadata; the read stays `take`-bounded. Test:
  `a_fifo_is_refused_without_blocking` (FIFO through the inner reader
  behind a 10 s watchdog; blocked 10 s before).
- N2: stored side capped at 64 px (`MAX_STORED_SIDE`, the pixmap bound;
  larger decodes downscale once on the bus turn through the shared
  resampler) and `MAX_THEME_TOTAL_BYTES` (32 x 16 KiB = 512 KiB),
  pinned to its per-icon share by a `debug_assert` in `decode`. Tests:
  `a_large_icon_is_stored_at_the_bound` (128 px stores at 64) and
  `thirty_two_large_icons_fit_the_total_budget` (2 MiB before).
- N3: `closest_order` sorts the fixed dirs by distance to the drawn
  size (ties prefer the larger), `scalable` and the base file last;
  `load` keeps a documented default (`LOOKUP_SIDE` 24, the middle of
  the 14-48 tray range) and `load_for_side` takes an explicit size.
  Test: `the_closest_size_wins` (22 red vs 48 green at 24 and 48).
- N4: one trailing `.png` stripped, `.svg`/`.xpm` hidden. Test:
  `a_name_with_an_extension_resolves_like_the_bare_name`.

Real icon sizes (Asahi M2, `/run/current-system/sw/share/icons`):
hicolor app PNGs at 16/22/24/32/48/64/72/96/128/256/512, every
standard dir populated, so 64 keeps every real tray-size icon while
bounding stored bytes to 16 KiB each.

Verification: fmt clean; clippy matrix clean (none, default,
`--all-features`, tray alone, tray with/without `icon-image`, each
module alone, popup+tray); nextest 1385 passed / 4 skipped (no sway
on the box; the one sway-gated clock test fails environmentally with
`SCOOTBAR_REQUIRE_SWAY=1`, same as the #497 review); `cargo test`
clean; `cargo deny check` ok; `scripts/backlog check` shows only the
3 pre-existing problems; `nix build .#docs-site` green. Live recheck
with pasystray 0.8.2 (IconName-only) on the M2: `shown:true`,
tooltip `pasystray`, icon ink in a 1600x1000 screenshot.

Size (release, stripped, aarch64, base `ee211c67d` vs head):
file 2,364,128 B both (+0); `.text` 1,791,944 vs 1,793,448 (+1,504,
+0.08%); `.rodata` +64 B. Idle tray-only bars on headless scoot
(20 s): RSS 4400 kB both, 0 wakeups both, 8 fds, 1 thread. The
2026-10-07 size waiver covered the decoder only: this growth is
reported plainly and nothing is waived here.

## Not in this ticket

Review findings N5-N8 (attention-with-pixmap doc qualification, dead
`Indexed` arm comment, stale resolved-ticket measurement prose, XDG
relative entries): note-only or separate small docs edits, not this PR.
Full theme inheritance past `hicolor` (future work, said in the
icon-themes ticket).
