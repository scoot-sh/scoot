---
title: "`xdg-activation-v1` had no input-serial gate, so an unfocused client could self-activate — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `xdg-activation-v1` had no input-serial gate — DONE

Found by independent review of `docs/backlog/resolved/foot-protocol-warnings-done.md`
(the four-protocols PR). Not a crash and not a regression — before that work
there was no way for a client to ask for focus at all — but the two bounds
`compositor/activation.rs` documents (`TOKEN_LIFETIME`, `MAX_TOKENS`) were
being described as the answer to focus-stealing when they are resource bounds
only, and did not stop the actual case.

A client with no keyboard or pointer focus, and no user interaction at all,
could call `get_activation_token`, then immediately `activate(token,
its_own_surface)`. The token was milliseconds old (well under
`TOKEN_LIFETIME`) and the table nowhere near `MAX_TOKENS`, so both checks
passed and focus moved to a surface the user never touched.

## What landed

A third rule, checked in `token_created` before either existing bound:

- The token's optional `set_serial(serial, seat)` is now required. `seat`
  must resolve (`Seat::<State>::from_resource`) to the compositor's own seat;
  `serial` must be one flexwm issued for a **key or button press or
  release**; and that event must have been **delivered to the client asking**
  (`XdgActivationTokenData::client_id`, which Smithay fills in from the
  sender and no client can forge). Missing serial, another seat's serial,
  another client's serial, and a stale or fabricated number are each refused
  with their own `debug!` line, and refused before the token table is touched
  at all — a client looping on unqualified tokens cannot evict anything or
  occupy a slot.
- Pointer *motion* deliberately does not qualify: it is continuous and
  passive (libinput reports it at 500-1000Hz, `refresh_pointer_focus`
  synthesizes it with no user involvement), so counting it would hand every
  client a permanently-refreshing serial.
- The check is at creation, never at redemption, exactly as this entry
  originally called for: a launcher's token is redeemed seconds later by a
  cold-starting child, by which point many newer events have happened.

## The design detail this entry did not anticipate

"Track the last input serial" — the obvious reading, and the one the
implementation brief started from — is wrong in both directions, and would
have broken the real-launcher flow this protocol exists for:

- A keyboard-driven launcher acts on the **press** and mints from that
  serial, but `State::press` sends the press and its release back to back
  within one call, with no dispatch in between. By the time the client's
  `commit` is handled, a single-slot tracker holds the *release*'s serial.
  `flexwm msg key Return` — the exact command that drove the verified
  `fuzzel` → `foot` trace in
  `docs/backlog/resolved/foot-protocol-warnings-done.md` — would have been
  refused every single time.
- Tracking only presses fails the other way: a GTK-style button activates on
  **release** and mints from a serial a press-only tracker never saw.

So `compositor/input/interaction.rs` keeps a fixed-size ring of the last 16
qualifying events (`[Option<(Serial, ClientId)>; 16]` plus a write index — no
allocation, written from the input path) and a token is honored if its serial
and client are one of them. Exact match against remembered values, never a
range: motion serials and the compositor's own focus serials (`shell.rs`'s
`set_focus`) come from the same global counter and fall between the
qualifying ones, so "between the oldest and the newest" would quietly admit
the passive events the gate exists to exclude.

## The second design detail, caught by independent review of the first fix

Matching the serial *by value alone* — which is what the first version of
this fix did — is not a check on interaction at all, it is a check on
arithmetic, and it is brute-forceable:

- `SERIAL_COUNTER` is process-global and shared with events that are not
  input. `xdg_surface.configure` and `xdg_popup.configure` draw from the same
  counter (pinned Smithay `0ff0098`, `src/wayland/shell/xdg/mod.rs:1524` and
  `:1853`), so **any** client can read a live value out of it for free: create
  an `xdg_surface`, commit with no buffer so it never maps, read the serial of
  the configure that comes back.
- Guessing from there costs nothing. A refused token posts no protocol error
  and Smithay sends `done` regardless, so a client can pipeline
  `get_activation_token` + `set_serial(X+1…X+64, wl_seat)` — all passing the
  seat check, since it is the real seat — and simply spend whichever one
  works. Any keystroke the user makes anywhere would eventually land inside
  the 16-entry ring.

The fix is the `ClientId` half above: an entry is only spendable by the client
that genuinely *received* that event. A guessed-but-numerically-correct serial
gets nowhere unless the guesser was the recipient, which requires real
interaction with the guesser. `input.rs` resolves the recipient at the moment
it records — `keyboard.current_focus()` for a key, `pointer.current_focus()`
for a button, each mapped to its owning client — so it is the client the event
is about to be *delivered to*, not one derived later from something that could
have changed. An event with no recipient (a key with nothing focused, a click
on bare desktop) is recorded as nothing: evidence for no one.

Pinned by `a_serial_delivered_to_another_client_is_refused_however_right_the_number_is`
(activation), `an_event_is_not_evidence_for_a_client_that_never_received_it`
and `a_button_that_reaches_no_surface_is_recorded_as_nothing` (input).

## Verified against a real launcher, not only in tests

The flow this protocol exists for was run end to end on the dev VM (`fuzzel`
1.14.1 → `foot`, headless, driven entirely over IPC) — once when the gate
first landed, and again after the `ClientId` half was added, since that
change touches the exact mechanism the evidence exercises. The
`WAYLAND_DEBUG` trace settles both design questions above with real numbers:

```
-> wl_keyboard@19.enter(4, wl_surface@3[0], [])   # focus, serial 4
-> wl_keyboard@19.key(6, 3230, 28, 1)             # Return pressed, serial 6
-> wl_keyboard@19.key(7, 3230, 28, 0)             # Return released, serial 7
<- xdg_activation_token_v1@30.set_serial(6, wl_seat@14)
DEBUG flexwm::compositor::activation: xdg-activation token created
DEBUG flexwm::compositor::activation: activating a window id=WindowId(1)
```

fuzzel mints from the **press** (6) while the newest serial the compositor
had issued was the **release** (7): a single-slot tracker would have refused
this exact token, every time, because `flexwm msg key` sends both halves
before any client can answer the first. And fuzzel is the client that
*received* `key(6, …)`, so the client-bound check passes for it — while
`foot`, a different client entirely, is the one that later redeems the token
and gets focus. That is the creation-time check doing exactly what it should:
the recipient rule binds whoever asked for the token, never whoever spends
it.

The same trace shows the limitation below concretely: fuzzel sends
`seat->kbd.serial`, which its `keyboard_enter` handler sets (`wayland.c:512`)
and its key handler overwrites (`wayland.c:970`). Chosen with the keyboard it
is a key serial; chosen with the mouse it would still be the *focus* serial
(4 here), which flexwm refuses. An attempt to drive that mouse path on the VM
was inconclusive -- clicking fuzzel's entry rows over IPC selected but never
executed an entry, so no token was minted at all.

## What it does not stop, by design

- A client the user really did interact with can activate itself off that
  interaction, including with more than one token minted from the same
  serial. The user just clicked it; `MAX_TOKENS` and `TOKEN_LIFETIME` are
  what bound how far that goes.
- A *focus* serial (`wl_keyboard.enter`) is refused, which is stricter than
  the protocol's loosest reading (Smithay documents the serial as one that
  "can come from an input or focus event"). Deliberate: flexwm focuses every
  newly mapped window itself, so a focus serial is something any client gets
  for free by mapping, and spending it later is exactly the steal this
  refuses. The cost is a launcher whose selection was made with the mouse
  while its own serial is still the focus one -- `fuzzel` is exactly that,
  since it only ever sends `seat->kbd.serial`. **What that costs depends on
  what was being activated, and the first version of this doc got it wrong
  by claiming it was simply harmless:**
  - *A fresh spawn* is unaffected: the app maps a window and mapping focuses
    it, which is where a launched app's focus came from before this protocol
    existed.
  - *Something already running* is not. A single-instance app (Firefox,
    Chromium, anything on `GApplication`) re-invoked from a launcher hands
    the token to its existing process, which activates a window that already
    exists — nothing maps, so nothing else focuses it and the activation
    simply does not happen. The same is true of the notification-daemon case
    (focusing the app a clicked popup came from), which is an activation of
    an existing window by definition.
- `XdgActivationState::create_external_token` (what the compositor would use
  to hand a token to a process it spawned itself — see
  `docs/backlog/protocols/activation-token-for-spawned-children.md`) never
  reaches `token_created` upstream, so it is unaffected by this gate.
