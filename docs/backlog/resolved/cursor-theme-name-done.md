---
title: "Custom/client cursor support — DONE: client surfaces (item 8), size/colour (item 13), drawn per-shape cursors and real xcursor theme loading (issue #40)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Custom/client cursor support — DONE

~~(a) honor `CursorImageStatus::Surface` by rendering the client's actual
supplied buffer~~ — landed as item 8. ~~(b) a user/config-level override for
the fallback shape's size and colour~~ — landed as item 13. ~~(c) drawing a
different shape per requested name, and honoring a theme *name*, both blocked
on sourcing a license-clean cursor theme~~ — **DONE**, 2026-09-15, with
`wp-cursor-shape-v1` (`docs/backlog/resolved/foot-protocol-warnings-done.md`).

## The framing that was wrong

This entry used to say per-shape cursors were "blocked on sourcing a
license-clean theme, not on code", and that reading stood unchallenged long
enough to be repeated in the first draft of the cursor-shape work. It
conflated two different things:

- **Shipping** a cursor theme in this repository — genuinely blocked. niri's
  assets are GPL and Adwaita's are not MIT-clean, per `CLAUDE.md`, and
  nothing MIT-clean has been vetted.
- **Reading the theme already installed on the user's machine** at runtime —
  *not blocked at all, and never was*. The file belongs to whoever installed
  it; flexwm neither redistributes nor derives from it. It is what sway,
  niri, Hyprland and Smithay's own `anvil` do, and the parser (the
  `xcursor` crate) is MIT.

The user caught this from the other end while reviewing the cursor-shape
work: *"when we mouse over gtk it shows a real cursor"* — i.e. clients were
already loading themes from the same machine flexwm had decided it could not
read one from.

## What landed

- `cursor/theme.rs` resolves `[appearance] cursor_theme`, else
  `$XCURSOR_THEME`, else `default`, and draws that theme's own artwork for a
  named shape. Images are loaded when a client first asks for that shape (an
  event path, never the render path) and cached, including negatively.
- `cursor/shapes.rs`'s ten drawn shapes remain, as the fallback when no theme
  is installed — which is not hypothetical: a webtop or minimal container
  frequently has none, and that is a first-class flexwm target.
- `compositor::run` exports `XCURSOR_THEME`/`XCURSOR_SIZE` to children, so a
  client that loads a theme itself picks the same one the compositor draws.

## What this does *not* close

Nothing about shipping an asset, which remains both blocked and unnecessary.
The one real remaining gap is HiDPI: theme images are picked at the nominal
`cursor_size` and drawn at buffer scale 1, so a scaled output gets a cursor
at its logical size rather than a sharper image from the theme's larger
variant. That is pre-existing behaviour (the drawn shapes have always worked
this way), not something this introduced — but it is now fixable, because
there is a theme with multiple sizes to pick from. Worth an entry of its own
if HiDPI `--tty` becomes a daily-driver case.
