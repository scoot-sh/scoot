---
title: Windows
description: "Floating windows, drag and resize, and window rules."
---

Tiling is the default; floating is the layer above each workspace's
strip for dialogs, pickers, and anything you choose with a rule. Whether
a window floats is decided when it first maps; the toggle changes it any
time (`Super+Shift+Space` floats the focused window, or puts it back in
the strip as a column right of the focused column).

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `[floating] auto` | bool | `true` | live (future windows) | Float a window when it first maps if it is a dialog, names a parent (transient), or has a fixed size. `false` turns all three off; `[[window_rule]]`s still apply. |
| `[floating] modifier` | string | `"super"` | live | The modifier held to drag floating windows: left button anywhere moves, right button resizes from the nearest edge or corner. Worth changing under `--nested`, where the host often keeps Super for itself. |

A floating window centers on its parent (or on the output), stays
inside the usable area, and keeps its centre as it resizes. The most
recently focused floats on top; `Super+Space` moves focus between the
floating windows and the strip (with a floating window focused,
strip-moving actions do nothing). Floating a column takes it out of the
strip; the strip underneath is otherwise untouched. A floating window
can go fullscreen and comes back floating.

## Window rules

Each rule is its own `[[window_rule]]` table; rules apply in file order
(a later matching rule overrides an earlier one per field it sets),
checked when a window first maps, after `[floating] auto` — so a rule
always has the last word. Re-applied live by `scootctl reload`, for
windows that map after it.

| Field | Type | Meaning |
|---|---|---|
| `match_app_id` | string (glob) | Matches the window's app id — for an X11 window, its `WM_CLASS` class (`scootctl windows` shows both). |
| `match_title` | string (glob) | Matches the window's title. |
| `float` | bool | `true` floats a matching window when it maps; `false` keeps it in the strip even if `auto` would float it. |
| `size` | `[width, height]` (logical px) | The size to ask a floating window for when it maps (`1..=65535` per axis, clamped to the usable area). No effect on a tiled window. |

A rule needs at least one matcher and must set `float`, `size` or
both; with both matchers, both must match. Matchers are **globs over
the whole string**, case-sensitive: `"foot"` matches only `foot`
(`"*foot*"` for anything containing it), `?` matches exactly one
character. An unusable rule (no matcher, a non-positive size) is
skipped with a warning; a reload refuses it by name on every reload
that finds it.

```toml
[[window_rule]]
match_app_id = "org.gnome.Calculator"
float = true

[[window_rule]]
match_title = "*Picture-in-Picture*"
float = true
size = [640, 360]

# A dialog you would rather have as a column:
[[window_rule]]
match_app_id = "org.gnome.Nautilus"
match_title = "*Properties*"
float = false
```
