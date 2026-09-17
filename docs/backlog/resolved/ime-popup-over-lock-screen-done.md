---
title: "An IME popup over a lock screen's password field is tracked but never drawn — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# An IME popup over a lock screen's password field is tracked but never drawn — DONE

## Resolution

Fixed as the ticket sketched: the locked render path gathers
`PopupManager::popups_for_surface` for every current lock surface in
`lock_elements`, and `lock_post_frame` sends those popups frame callbacks
alongside the lock surfaces' own. An IME candidate window over the password
field is drawn at the caret, with frame callbacks, so an animated IME does
not stall behind the lock screen. Composition itself always worked; this was
visibility only.

## The trust decision

The ticket named this as the load-bearing choice, and it is recorded here
rather than left implicit: an IME client is **trusted with pixels over the
lock screen for the focused field's candidate window** — the ticket's option
(b), narrowed to exactly that surface. Three reasons, each traced to the
mechanism rather than asserted:

1. **An `xdg_popup` in a lock surface's tree is the lock client's own.**
   A popup names its parent by object id, and object ids live in
   per-connection namespaces, so a client can only parent to its own
   surfaces. The lock client already draws fullscreen and receives the
   password; its own menu grants it nothing.
2. **An IME popup's parent is assigned by the compositor, never by client
   naming.** Smithay parents it to the focused text field on creation and
   re-parents it on activation. While locked, keyboard (and with it
   text-input) focus is a current lock surface or nobody, so an IME popup in
   a lock surface's tree is there because the compositor put it there in
   service of the focused password field. A background window's own IME
   popup stays parented to that window -- and is dismissed outright when
   focus leaves it -- which the locked path never gathers.
3. **The candidate window shows the IME nothing new.** Composition already
   routes every composed keystroke through the IME client by design, so
   drawing its candidate window reveals nothing it does not already know.

What can never render through this path: windows, layer surfaces, xdg
popups from background clients (same-client parenting makes them
unplaceable in a lock tree), and background IME popups (compositor-assigned
parenting keeps them in their own window's tree). The PR #44 guarantee
stands unchanged around it: locking still dismisses an open xdg grab and
refuses new ones, so a menu left open at lock time can neither draw nor
receive the password.

## Shape of the change

- `session_lock.rs::surface_elements` gathers each current lock surface's
  popup tree ahead of that surface's own elements (front-most first, the
  order `Window`'s own elements use), placed with the layer-surface formula
  -- no parent geometry added back, since a lock surface draws at the
  output origin directly. For an IME popup the window and layer formulae
  coincide anyway: `parent_geometry` answers the default rectangle for a
  lock surface, so the candidate lands at the raw surface-local caret.
- `session_lock.rs::send_frames` sends each gathered popup's surface tree
  its callbacks, mirroring `Window::send_frame` exactly -- including its
  deliberate asymmetry: all tracked popups, not just element-producing
  ones, so a pre-first-attach callback is never the frame that stalls.
- No focus predicate in the gather loop, deliberately: with one output
  there is one lock surface holding the keyboard, and an IME popup can only
  be parented to the focused field, so an "unfocused lock surface's popup"
  cannot arise through either parenting path. Untestable on one output;
  decided by construction and stated here.
- Allocation: one `popups_for_surface` walk per lock surface per frame plus
  Smithay's own per-popup element `Vec` -- exactly what every window
  already pays per frame on the unlocked path. No new allocation shape; with
  no popup the walk finds an empty tree and appends nothing, which is the
  byte-identical no-IME behaviour the pre-existing blanking tests pin
  (green before and after, pixels unchanged).

## Edge cases, decided and pinned

- **No IME active**: element list identical to before -- the existing
  whole-screen blanking tests are the proof, unchanged and green.
- **IME popup open at the moment of locking**: dismissed, not kept. Focus
  leaving the window deactivates the IME against it (Smithay's own path),
  and the locked path never gathers the window's tree. Pinned by test.
- **Rapid lock/unlock with a popup mapped**: the unlock test cycles
  lock/unlock twice, asserting hidden-while-locked and
  drawn-plus-callbacks after each unlock.
- **Field disabled while locked**: the candidate is dismissed with it
  (deactivation path), pinned by test.
- **Pointer input to the candidate window**: unchanged and out of scope --
  this item is visibility, not input, per the ticket. The popup is not hit
  tested while locked (same as an unmapped-role surface); keystrokes reach
  the password field exactly as before.

## Tests

Five new, in `session_lock/tests/ime_popup.rs` (harness extended with text
input, input method, xdg-popup and per-popup frame-callback steps):

- `ime_popup_on_the_focused_lock_surface_is_drawn_and_gets_frames` --
  fail-first (failed on pixels pre-fix, passed after): candidate drawn at
  the caret over the lock screen, `activate` received, frame delivered.
- `a_background_windows_xdg_popup_is_not_drawn_over_the_lock_screen` --
  pin, green before and after: whole screen lock-colour, zero callbacks
  while locked (the PR #44 guarantee through the new element source).
- `a_background_windows_ime_popup_is_not_drawn_over_the_lock_screen` --
  drawn + animated pre-lock, gone (pixels and callbacks) post-lock.
- `unlocking_restores_popup_rendering_and_frames` -- two lock/unlock
  cycles, no stuck state either direction.
- `disabling_the_lock_screens_text_field_dismisses_its_popup` -- lifecycle
  close while locked.

Not driven live: a real IME (fcitx5/ibus) against a real locker, and
`msg screenshot` while locked with a candidate mapped -- the harness pixel
tests render with the real `PixmanRenderer` and read back the same
framebuffer a screenshot would, which is the evidence that exists. The
frame-callback arming subtlety found on the way (`wl_surface.frame` needs
its own commit to leave pending state) is documented at the harness step,
not worked around in the compositor.

Original entry, left as written:

# An IME popup over a lock screen's password field is tracked but never drawn.

Found by review while implementing `text-input-v3`/`input-method-v2`
(`docs/backlog/resolved/foot-protocol-warnings-done.md`). Not a crash and not
a regression -- before that work there was no IME at all -- but the one place
the feature does not do what the rest of it does.

An `ext-session-lock-v1` surface can hold a text field (that is what a lock
screen's password box is), and keyboard focus goes to it while locked, so
`zwp_text_input_v3` focus follows and an input method can be activated
against it. `State::parent_geometry` returns the default rectangle for such a
surface, which places the candidate window at the origin -- but that is moot,
because `headless.rs`'s locked branch **replaces the element list wholesale**
with `lock_elements(..)` rather than gathering popups, and `lock_post_frame`
sends no popup frame callbacks either. So the popup is tracked, positioned
and never rendered; an animated one would also be stalled for want of frame
callbacks.

Consequence: someone whose password needs an IME (a non-Latin passphrase)
gets no candidate window on the lock screen. Composition itself still works --
the text reaches the field -- so this is "cannot see what you are composing"
rather than "cannot type".

What it would take: gather `PopupManager::popups_for_surface` for each lock
surface in `lock_elements`, and send their frame callbacks alongside the lock
surfaces' own in `lock_post_frame`. The care needed is in `session_lock.rs`'s
own doctrine, not in the popup code: the locked render path is deliberately
"never gathered" rather than "drawn behind something opaque" (see that
module's doc), so adding a second source of elements to it has to preserve
that property -- an input-method client is not the lock client, and letting
an arbitrary client's surface render over the lock screen is exactly what
that design is refusing to do. Probably wants the popup restricted to the
lock client's own input method, or an explicit decision that an IME is
trusted while locked.
