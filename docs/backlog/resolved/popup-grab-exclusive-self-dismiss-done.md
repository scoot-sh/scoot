---
title: "An `exclusive` layer surface's own popup grab is refused and its menu dismissed by the very surface that opened it — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# An `exclusive` layer surface's own popup grab is refused and its menu dismissed by the very surface that opened it — DONE

Found by independent review of `resolved/xdg-popup-input-resolved.md`
(PR #44). An `exclusive` layer surface's own dropdown flashed open and
instantly closed: `popup_grab_outranked()` refused the new grab, and
`refresh_keyboard_focus` pre-empted an existing one, whenever
`layer_keyboard_focus()` reported `exclusive: true` — without checking
whether the exclusive surface *is* the grab's own root. The module doc and
README rule 3 already claimed the bar's own dropdown survives; true for a
click-focused `on_demand` surface, false for an `exclusive` one.

## What landed

One clause at each of the two sites, both comparing against the grab's
*root* surface rather than just asking whether something is exclusive:

- **Refusal** (`popup.rs`): `popup_grab_outranked()` takes the root
  `find_popup_root_surface` already computes in `grab_popup` and answers
  false when the exclusive surface *is* that root. A grab rooted on a
  window while a launcher is up is still refused; a grab asked for while
  locked is still refused.
- **Pre-emption** (`shell.rs`): `refresh_keyboard_focus` skips the dismiss
  when the held grab is rooted on the exclusive surface. The root comes
  from the live grab state — `popup_grab_rooted_on`, read off the held
  grab's `keyboard_grab_start_data().focus` the way `popup_grab_holder`
  (PR #55) already does — since no grab request is in hand on that path.
  No grab at all, or a root that is already gone, answers false: the
  dismissing direction.

`popup.rs`'s precedence doc now states the exception (rule 2 wins over
everything but the surface's own menu; rule 3 names the exclusive root
explicitly), and README's "Popup menus" section drops the "one case this
doesn't hold for yet" caveat — the claim is now true as written.

## Tests

Four new in `layer_shell/tests/popup.rs`, all real `exclusive` layer
surface + real `xdg_popup` grab + seat-keyboard assertions, no hand-set
fields:

- `an_exclusive_layer_surfaces_own_popup_grab_is_accepted` — grant time:
  grab accepted, no `popup_done`, keyboard on the popup, keys reach it.
- `an_exclusive_layer_surface_does_not_pre_empt_its_own_popup_grab` —
  pre-emption: the own grab plus a nested submenu off the same root both
  survive a `refresh_keyboard_focus` forced by mapping a window.
- `unmapping_the_pre_empting_launcher_does_not_resurrect_a_dismissed_grab`
  — the third-party unmap edge: a window-rooted menu dismissed by a
  *different* launcher stays dismissed (`popup_done` stays 1, no grab)
  once that launcher unmaps; the keyboard goes back to the window.
- `unmapping_the_exclusive_root_mid_grab_leaves_its_own_menu_up` — the
  own-root unmap edge: unmapping the root mid-grab neither dismisses its
  menu (`popup_done` stays 0) nor moves the keyboard off it.

Fail-first: the first, second and fourth FAIL unfixed — the fourth at its
grant assert, since the refusal dismisses with `popup_done` before the
unmap step ever runs (`configured` false, and `popup_done` already 1).
The first two fail with the keyboard stuck on `Layer(0)`
(`left: Some(Layer(0))` vs `Some(Popup(1))`). The third passes unfixed —
it pins behavior that must keep working. The preserved shape the other
way (a different exclusive surface still refuses and pre-empts) was
already pinned by `a_popup_grab_is_refused_while_a_launcher_holds_the_keyboard`
and `an_exclusive_layer_surface_pre_empts_an_open_popup_grab`; both still
pass unchanged.

## Deliberately not covered

- No hot-path benchmark: the grant path runs per menu opened, and the
  pre-emption path adds one `Option` comparison per focus derivation —
  no allocation, next to the layer-map walk both paths already pay
  (the same stated-not-measured call PR #56 made).
- No other precedence changes; `popup-grab-blocked-by-ime-grab` stays its
  own entry.

Original entry, left as written:

# An `exclusive` layer surface's own popup grab is refused and its menu dismissed by the very surface that opened it.

Found by independent review of `docs/backlog/resolved/xdg-popup-input-resolved.md`
(PR #44). Not a crash, and the precedence rule it violates is otherwise
correctly enforced — this is the one case the rule's own doc overclaimed.

`popup.rs`'s `popup_grab_outranked()` refuses a new grab, and `shell.rs`'s
`refresh_keyboard_focus` pre-empts an existing one, whenever
`layer_keyboard_focus()` reports `exclusive: true` — without checking
whether the exclusive surface *is* the grab's own root. So a
`gtk4-layer-shell` launcher on `overlay`/`top` with `keyboard_interactivity:
exclusive` that opens its own dropdown (a settings menu, an emoji picker)
gets refused immediately: `grab_popup` sees its own exclusive surface
outranking it and sends `popup_done` on the spot, so the dropdown flashes
open and instantly closes — dismissed by the surface that opened it, not by
something else winning.

This is exactly the failure `popup.rs`'s module doc and `README.md`'s
"Popup menus" section both claim rule 3 prevents ("a bar's own dropdown is
not dismissed by the bar that opened it") — true for a click-focused
`on_demand` layer surface, false for an `exclusive` one, since the two share
one check today.

Low priority: neither DMS nor Noctalia (the two Quickshell shells probed
against flexwm) route any menu through `xdg_popup` — both use layer
surfaces for their own dropdowns — so this has not been field-hit yet, and
`exclusive` layer-shell clients with their own popups are uncommon (most
exclusive surfaces are launchers, which tend to use their own internal
list-navigation UI rather than a real `xdg_popup`).

What it would take: `popup_grab_outranked()` (and the corresponding
pre-emption check in `shell.rs`) need the grab's *root* surface, not just
whether something else is exclusive — `find_popup_root_surface` already
computes this in `grab_popup`. One clause: don't outrank when the exclusive
surface *is* the root the grab belongs to.
