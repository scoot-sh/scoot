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
  must resolve (`Seat::<State>::from_resource`) to the compositor's own seat,
  and `serial` must be one flexwm issued for a **key or button press or
  release**. Missing serial, another seat's serial, and a stale or fabricated
  number are each refused with their own `debug!` line, and refused before
  the token table is touched at all — a client looping on unqualified tokens
  cannot evict anything or occupy a slot.
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
qualifying serials (`[Option<Serial>; 16]` plus a write index — no
allocation, written from the input path) and a token is honored if its serial
is any one of them. Exact match against remembered values, never a range:
motion serials and the compositor's own focus serials (`shell.rs`'s
`set_focus`) come from the same global counter and fall between the
qualifying ones, so "between the oldest and the newest" would quietly admit
the passive events the gate exists to exclude.

## Verified against a real launcher, not only in tests

The flow this protocol exists for was re-run end to end on the dev VM after
the gate landed (`fuzzel` 1.14.1 → `foot`, headless, driven entirely over
IPC), and the `WAYLAND_DEBUG` trace settles the design question above with
real numbers:

```
-> wl_keyboard@19.enter(4, wl_surface@3[0], [])   # focus, serial 4
-> wl_keyboard@19.key(6, 3219, 28, 1)             # Return pressed, serial 6
-> wl_keyboard@19.key(7, 3220, 28, 0)             # Return released, serial 7
<- xdg_activation_token_v1@30.set_serial(6, wl_seat@14)
DEBUG flexwm::compositor::activation: xdg-activation token created
DEBUG flexwm::compositor::activation: activating a window id=WindowId(1)
```

fuzzel mints from the **press** (6) while the newest serial the compositor
had issued was the **release** (7): a single-slot tracker would have refused
this exact token, every time, because `flexwm msg key` sends both halves
before any client can answer the first.

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
  while its own serial is still the focus one -- its token is refused, and
  the app it started is focused on map regardless.
- `XdgActivationState::create_external_token` (what the compositor would use
  to hand a token to a process it spawned itself — see
  `docs/backlog/protocols/activation-token-for-spawned-children.md`) never
  reaches `token_created` upstream, so it is unaffected by this gate.
