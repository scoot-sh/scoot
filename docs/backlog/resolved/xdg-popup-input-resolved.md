---
title: "Popup input: keyboard focus never moves onto a popup, grabs are a no-op, layer-parented popups stay untracked — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Popup input: keyboard focus never moves onto a popup, grabs are a no-op, layer-parented popups stay untracked — RESOLVED.

## Resolution (2026-09-16)

The input half of popup support, split out when the mapping half resolved
(`xdg-popup-initial-configure-resolved.md`). Of the three gaps this entry
filed, two were real and one turned out to be already fixed — and
implementing it as written would have *broken* it.

### Gap 1 — `XdgShellHandler::grab` was a no-op: fixed

New `compositor/popup.rs` wires Smithay's `PopupGrab`/`PopupKeyboardGrab`/
`PopupPointerGrab` in, and — the part that needed deciding rather than
wiring — states where a grab sits in a focus model that already had three
answers in it.

**Precedence, highest first:**

1. **The session lock.** `PopupKeyboardGrab` *ignores* `set_focus` while it
   is live, so a grab left installed across a lock would route the user's
   password to whatever client had a menu open. This is not a theoretical
   ordering nicety: it is the difference between the lock screen receiving
   the password and a background client receiving it.
2. **An `exclusive` layer surface on `top`/`overlay`** — `layer_shell.rs`'s
   documented "takes the keyboard the moment it maps and keeps it until it
   unmaps". A launcher opened over a menu has to be typeable; without this
   it would be on screen with a text field nothing could type into, which is
   exactly the "one silently wins forever" state the entry warned about.
3. **The popup grab**, over the focused window *and* over a layer surface
   that got the keyboard from a click. That last distinction is load-bearing
   in the other direction: a bar's own dropdown must not be dismissed by the
   bar that opened it.

**Losing is spelled `popup_done`, not "unset the seat grab".** The protocol
lets a compositor dismiss a popup whenever it likes, and a menu left mapped
with its input taken away is a menu the user cannot get rid of. Both the
refusal path and the pre-emption path dismiss the popup tree outright.

**Checked at grant time as well as on later refreshes.** A client can ask for
a grab at any moment, including while the screen is already locked or a
launcher is already up; `refresh_keyboard_focus` alone would have granted it
and revoked it a moment later, flickering the keyboard through the menu on
the way back.

Three sites outside the new module carry the consequences, and each is a
place where a field or function had to mean the same thing at every
site that reads it:

- **`session_lock.rs`'s `drop_pointer_grab`** carried a standing note that
  *"if a keyboard or touch grab is ever added to this compositor, it has to
  be dropped here too"*. It has been — it is now `drop_input_grabs`, and it
  dismisses the popup grab **first**, because `PopupPointerGrab::unset`
  restores keyboard focus to the popup's root while the keyboard is still
  grabbed, which would put a `wl_keyboard.enter` on the wrong surface in
  between the lock's decision and its effect.
- **`layer_keyboard_focus`** now reports *why* a surface won, not just which
  one: `exclusive` pre-empts a grab and a click-focused `on_demand` surface
  does not, and a bare `Option<WlSurface>` could not tell the two apart.
- **`keyboard_on_layer`'s doc** was corrected. It said "the focus
  `refresh_keyboard_focus` last handed out"; with a grab swallowing that
  `set_focus`, it is now what the last refresh *derived*. Its one reader
  (`commit_layer_surface`'s gate) only decides whether to re-derive at all,
  so the looser meaning is safe — but it is the meaning, and the field must
  not be read as "a layer surface currently holds the keyboard".

**`settle_popup_grab`** gives flexwm's own derivation the last word once a
grab ends. Smithay restores focus to the grab's *root*, which is right for a
window and wrong for a `keyboard_interactivity: none` bar, which would end
up holding a keyboard it explicitly asked never to have. It runs from the
wayland display source (where a client's destroy is seen and nowhere else)
and after a pointer button (which is what dismisses a menu by clicking
outside it), both behind an `Option` check.

**Keybindings still win over a grab**, unchanged and now pinned by a test:
`input.rs`'s `key()` matches bindings in the filter Smithay runs *before*
`input_forward`, and only `input_forward` consults a grab. So the VT-switch
binds and `quit` stay reachable with a menu open — the same escape hatch
that makes an `exclusive` layer surface safe.

### Gap 2 — keyboard focus onto a popup: fixed *for grabbing popups only*

Deliberately narrower than this entry asked for, and the difference matters.
Focus moves onto a popup when it **grabs**, which is what the request is for
and what every menu, combo box and typeahead actually uses.

Focus does **not** move onto a popup that did not grab. As written ("even
without a grab ... keyboard-aware clients expect focus on the popup surface
while it is up") that is unsafe: a **tooltip is an ordinary `xdg_popup`**
(GTK's are), so "the focused window's topmost popup holds the keyboard"
means a tooltip appearing takes the keyboard away from the window the user
is typing into. niri, sway and mutter all draw the same line — popups get
keyboard focus through the grab, not through being mapped.

### Gap 3 — layer-parented popups: already worked; the fix as filed would have broken it

This entry's premise was wrong, and it was checked rather than assumed. A
popup parented to a layer surface already configures, maps, renders, hit-tests
and gets frame callbacks on `main` as it stood. The path:

- `xdg_surface.get_popup(None, positioner)` calls `XdgShellHandler::new_popup`
  with no parent yet, and flexwm's `track_popup` puts it in `PopupManager`'s
  *unmapped* list;
- `zwlr_layer_surface_v1.get_popup` then sets the parent;
- the popup's first commit reaches `PopupManager::commit`, which finds it in
  that unmapped list and promotes it into the parent's `PopupTree`.

Smithay's `LayerSurface` already walks `popups_for_surface` in its render
elements (`desktop/space/wayland/layer.rs`), its `surface_under` and its
`send_frame`, so nothing further was needed.

**Implementing `WlrLayerShellHandler::new_popup` would have introduced a
bug.** Tracking there is a *second* `track_popup` for a popup whose parent is
by then set, so `add_popup` inserts a second node for the same surface into
the layer surface's `PopupTree` — measured at 2 nodes for 1 surface, by
temporarily implementing the handler and counting
`PopupManager::popups_for_surface`. Every tree walk then sees the popup
twice, and `PopupTree::dismiss_popup` removes only one of the two, leaving a
dismissed menu on screen. The handler therefore stays unimplemented, with
that reasoning recorded at `XdgShellHandler::new_popup` (which is what
actually does the tracking) and pinned by an assertion in
`a_layer_parented_popup_configures_maps_and_draws`.

### A bug found in the bug-bash, not in review

Installing the grab keyboard-first (the shape anvil uses) breaks **nested**
grabs. `PointerHandle::set_grab` runs the *outgoing* grab's `unset`, and
`PopupPointerGrab::unset` hands keyboard focus back to the chain's root
whenever the keyboard is still grabbed with a serial matching that outgoing
grab. A client may legitimately reuse one input serial for both grabs — a
submenu opened by *hovering* has had no new input event to draw a fresh one
from — and then that condition is true of the keyboard grab being installed
right now. The parent menu's pointer grab silently removed the submenu's
keyboard grab and dropped focus to the toplevel, with the submenu still on
screen and unusable.

Fixed by installing keyboard-down, pointer, keyboard-up. Reproduced against
the keyboard-first order and confirmed fixed, both with a real client
(`a_nested_popup_grab_unwinds_to_its_parent`).

### Tests

13 new (14 total in the file, one moved here from `tests.rs`), in
`crates/flexwm/src/compositor/layer_shell/tests/popup.rs` — that file was
already the largest test file in the tree, so `tests.rs` became
`tests/mod.rs` (harness) plus `tests/popup.rs`, without pre-empting the
broader extraction `testing/large-test-file-organization.md` plans.

Grab and focus: the keyboard moves onto a grabbing popup and typed keys
really arrive there; it goes back to the window on destroy; a click over a
popup reaches the popup (the pre-existing hit test, pinned); a click outside
dismisses and a click inside does not; submenus nest and unwind; keybindings
still fire while a popup grabs the keyboard. Precedence: an `exclusive`
layer surface pre-empts an open grab *and* refuses a new one; a
click-focused `on_demand` surface does neither; a `none` bar does not keep a
keyboard when its own menu closes; locking takes the keyboard off a menu and
refuses a grab asked for while locked. Teardown: a client that dies mid-grab
leaves nothing behind; a nested popup grab unwinds to its parent rather than
losing the parent's own grab.

**Negative control**, run against the first grab commit (`8ac930a`, 12 tests
in the file at that point) before the later fixes landed: with
`XdgShellHandler::grab` stubbed back to a no-op, 9 of the 12 failed and
exactly the 3 pre-existing behaviours (mapping, drawing, the pointer hit
test) passed. Not re-run at the current tree; the two tests added since
(`keybindings_still_fire_while_a_popup_grabs_the_keyboard`,
`a_nested_popup_grab_unwinds_to_its_parent`) are both grab-dependent, so
re-running could only raise the failing count, not lower it.

### Not done, and deliberately

**The grab serial is not validated.** The protocol says a compositor *may*
ignore a grab whose serial does not name a real user action, and flexwm has
the machinery (`interaction_serials`, used by `xdg-activation-v1`). It is not
wired up here because flexwm records only key and button serials, never
`enter` serials, and Qt's `QWaylandInputDevice::serial()` is updated on
`pointer_enter`/`keyboard_enter` too — so a strict check would refuse
legitimate Qt menus in some orderings. Filed separately as
`protocols/popup-grab-serial-validation.md`. The security-relevant gates that
do *not* depend on a serial (the lock, and an `exclusive` layer surface) are
implemented and tested.

**No real-shell field confirmation.** Both probed Quickshell shells (DMS,
Noctalia) route their own menus through layer surfaces, with zero `xdg_popup`
wire traffic; the evidence here is a real minimal Wayland client through the
real dispatch loop, not a toolkit.

Original entry, left as written:

# Popup input: keyboard focus never moves onto a popup, grabs are a no-op, layer-parented popups stay untracked

The input half of popup support, split out when the mapping half
resolved (`docs/backlog/resolved/xdg-popup-initial-configure-resolved.md`):
a window's `xdg_popup` now configures, maps and draws, but it is
display-only. Three gaps, in the order a real menu needs them closed:

1. **`XdgShellHandler::grab` is a no-op** (`handlers.rs`). A client
   opening a menu asks for a popup grab (`xdg_popup.grab` + seat +
   serial); the protocol says the compositor should direct input to the
   popup tree until it is dismissed. Smithay has the machinery —
   `PopupManager::grab_popup` builds a `PopupGrab` that routes
   keyboard/pointer into the popup and unwinds focus on dismiss — but
   flexwm neither calls it nor falls back, so a menu is shown and then
   clicks/keys keep going to whatever held them. The grab is also what
   dismisses: nothing else ever sends `popup_done` today, so even
   Escape-to-close depends on this item, not just focus routing.
2. **Keyboard focus never moves onto a popup.** Even without a grab,
   keyboard-aware clients (e.g. an app menu with typeahead, a combobox)
   expect focus on the popup surface while it is up. Pointer hit-testing
   does reach popups already (`WindowSurfaceType::ALL` in
   `State::surface_under`), so clicks land on the right surface when the
   pointer is over one — but Escape-to-close, arrow-key navigation and
   typeahead all need the keyboard half.
3. **Layer-surface-parented popups are still untracked.**
   `WlrLayerShellHandler::new_popup` is deliberately not implemented
   (item 14's reasoning: tracking a popup that could never map was dead
   code — no longer true). A bar's own dropdown menus and tooltips
   (`xdg_popup` parented to a layer surface) configure fine once
   tracked, but nothing tracks them, so they still never map. Smithay's
   layer-shell handler signature hands flexwm the parent `LayerSurface`
   and the `PopupSurface`; `PopupManager::track_popup` accepts either
   parent kind.

The same end-to-end test shape the mapping fix used covers 1–2: map a
popup, assert a click over it reaches it, assert keyboard focus lands on
it under a grab, assert focus returns to the parent on dismiss. No
quickshell field confirmation yet — both probed shells route their own
menus through layer surfaces (zero `xdg_popup` wire traffic in either
probe), so the first real client will be an ordinary app toolkit (GTK
menus, Qt comboboxes).

Rough size: M (the grab machinery exists; the work is wiring it into
flexwm's focus model without regressing the layer-shell keyboard rules
`layer_shell.rs` documents — a popup grab must lose to them, or a
launcher's keyboard interactivity and a menu grab will fight).
