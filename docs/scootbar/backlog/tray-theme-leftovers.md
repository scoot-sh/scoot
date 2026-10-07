---
title: "Tray themed icons: small leftovers from the #497 and #502 reviews"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# Tray themed icons: small leftovers from the #497 and #502 reviews

Filed 2026-10-07. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Filed 2026-10-07 from the independent reviews of #497 (themed icon names)
and #502 (its hardening). Serves **daily-drive**. None of these is a
defect that bites a real desktop today; each is a doc or a hardening step
worth doing the next time someone is in `theme.rs`.

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
