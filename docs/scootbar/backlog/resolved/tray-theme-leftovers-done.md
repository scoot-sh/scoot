---
title: "Tray themed icons: small leftovers from the #497 and #502 reviews"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
---

# Tray themed icons: small leftovers from the #497 and #502 reviews

Filed 2026-10-07 from the independent reviews of #497 (themed icon names)
and #502 (its hardening). Serves **daily-drive**. None of these is a
defect that bites a real desktop today; each is a doc or a hardening step
worth doing the next time someone is in `theme.rs`.

## Status (2026-10-08): all six items landed in #524

Conservative choices throughout (maintainer away): docs qualified rather
than behavior changed for item 2; `openat2` contained open added behind
the existing `canonicalize` check with an `ENOSYS` fallback for pre-5.6
kernels for item 1 (trivially reversible: delete `open_contained` and
call `read_file` on the canonical path); relative entries skipped for
item 5; notes only for item 6. No new dependency (`Cargo.lock`
unchanged); no hot-path allocation (lookup stays once per `GetAll`
answer on the bus turn, never per frame).

1. **Intermediate-component swap closed with `openat2`
   `RESOLVE_BENEATH`** (`theme.rs`: `read_if_inside_impl` seam,
   `open_contained`, `read_file_fallback`, `openat2_missing`;
   `rustix` 1.1.4 already carries `openat2`, so no new package). The
   `canonicalize` check stays as defense in depth; the contained open
   refuses any escape past the base with `EXDEV`, atomically. Same-user
   threat model documented next to the trailing-symlink note (payoff no
   more than `kill`, impact capped at decoding a PNG the attacker could
   already read; the base directory itself remains peer-controlled).
   Cost, fixture theme, 200 lookups each, Asahi M2: old plain open
   117.0/117.4/117.2 us per lookup vs contained
   143.9/127.5/126.5 us (load 3-5) — about 10 us a lookup, against the
   50 ms re-read floor. Test that fails before:
   `an_intermediate_swap_after_the_check_stays_contained` (seam swaps
   `22x22` for a symlink outside between check and open; old plain open
   decodes the outside file, contained open stays hidden; proven by
   revert-run-restore on the M2).
2. **Attention-name sentence qualified** (site tray docs, `proto.rs`
   `attention_icon_name` doc, resolved `tray-icon-themes-done.md`
   Status): the attention name wins while `NeedsAttention` for items
   without a pixmap; an item that sent a pixmap keeps it while alarmed.
   No behavior change. Doc-only, no test.
3. **Dead `Indexed` arm pinned with a comment** (`theme.rs` `decode`):
   png 0.18 with `EXPAND` expands palette images to `Rgb`/`Rgba` before
   the frame (`output_color_type`), so the arm is unreachable and palette
   PNGs decode; the arm stays so a decoder upgrade that stops expanding
   surfaces as a refused icon. Test pinning it:
   `a_palette_png_decodes_through_expand` (4x4 indexed PNG decodes;
   passes before and after — comment-only).
4. **Stale measurement text corrected**
   (`resolved/tray-icon-themes-done.md`): the "NOT captured" paragraph
   replaced with the captured numbers (release file +131,072 B, `.text`
   +65,760 B, `.rodata` +24,568 B vs `main` at `ae10e90bc`; idle
   wakeups/fds/threads unchanged, idle RSS +~150 kB inside noise) and the
   2026-10-07 waiver pointer to `lightest.md`; the
   `AttentionIconName` sentence qualified as in item 2. Doc-only.
5. **Relative `XDG_DATA_DIRS` entries skipped** (`theme.rs`
   `data_dirs_from` keeps only absolute entries; the variable is the
   bar's own environment, worst case a miss). Test that fails before:
   `relative_xdg_data_dirs_entries_are_skipped` (proven by
   revert-run-restore on the M2).
6. **`.PNG` and `LOOKUP_SIDE` noted** (code comments at the strip site
   and `LOOKUP_SIDE`, site tray docs parenthetical): only lowercase
   `.png` is stripped (`.PNG` stays hidden); `LOOKUP_SIDE` 24 stays the
   documented HiDPI compromise (bus turn knows no output size; the cache
   scales smoothly). Pinned by the `.PNG` assertion in
   `a_name_with_an_extension_resolves_like_the_bare_name` (passes before
   and after — note-only).

## Items

1. **Directory-component swap between `canonicalize` and `open`**
   (`crates/scootbar/src/modules/tray/theme.rs`, `read_if_inside` and
   `read_file`). `O_NOFOLLOW` refuses a trailing symlink only; a same-user
   peer controlling its own `IconThemePath` directory could swap an
   *intermediate* component after the containment check. `openat2` with
   `RESOLVE_BENEATH` closes it. Same-user threat model (payoff no more than
   `kill`), impact capped at decoding a PNG the attacker could already
   read. At least say so in the code comment next to the trailing-symlink
   note.
2. **Attention name ignored when a pixmap is present**
   (`modules/tray/item.rs`, the `from_pixmap` branch). An item that sends a
   main pixmap plus an attention *name* keeps its main icon while alarmed.
   Either honor the name or qualify the sentence in the tray docs ("While
   `NeedsAttention` the attention name is drawn instead of the main one"
   holds for items without a pixmap).
3. **Dead `Indexed => return None` arm after `EXPAND`** (`theme.rs`).
   png 0.18 maps `Indexed` to `Rgb`/`Rgba` before the frame is returned, so
   the arm is unreachable and palette PNGs do decode (proven by a
   review test). A comment saying so keeps the next reader from "fixing"
   palette support.
4. **Stale measurement text in the resolved ticket**
   (`resolved/tray-icon-themes-done.md`, Status section still says the Linux
   numbers were not captured). They were: the PR body, the proof round and
   the reviews carry them, and the maintainer's size waiver of 2026-10-07 is
   in `lightest.md`.
5. **Relative `XDG_DATA_DIRS` entries** pass through unresolved
   (`theme.rs`); the env is the bar's own, and the worst case is a miss.
   Skip them explicitly or document the tolerance.
6. **`.PNG` (uppercase) is not stripped** and `LOOKUP_SIDE = 24` is a
   compromise at HiDPI (the bus turn has no output size; the cache scales
   smoothly). Note only, unless a real app trips over either.
