---
title: "`wlr-output-management` reconfiguration: `apply`/`test` always fail."
status: "open"
area: "protocols"
priority: "low"
blocked: "multi-output support"
---

# `wlr-output-management` reconfiguration: `apply`/`test` always fail.

Split out of
[`output-management-read-only-done.md`](../resolved/output-management-read-only-done.md)
when the read half shipped (PR #49, 2026-09-16), the same way PR #46 split
the connection-cap item and PR #47 split the wlr foreign-toplevel one — so
what actually shipped and what did not are two separate records, not one
half-true one.

flexwm advertises `zwlr_output_manager_v1` at version 4 and answers every
`zwlr_output_configuration_v1.apply` and `.test` with `failed`. So
`wlr-randr --output <name> --pos 100,100`, `--scale`, `--transform`,
`--custom-mode`, `--mode` and `--off` all print `failed to apply
configuration` and exit non-zero, and a shell's Display page shows an error
rather than a change.

That is deliberate and is the right answer *today*: flexwm has exactly one
`Output`, created once in `headless::init_named` and never moved, rotated,
disabled or rescaled. There is nothing an `apply` could apply. The
alternative — accepting a configuration and silently doing nothing — would
give a user a settings page whose buttons appear to work, which is strictly
worse (see `compositor/output_management/configuration.rs`'s module doc for
the full reasoning, including why `cancelled` is not used either).

## What implementing this actually needs

Not the protocol work — the handler shape is already there, `refuse` just
has to become a real path. It needs the compositor to be able to *do* the
things a configuration asks for, none of which exist:

- **`set_mode` / `set_custom_mode`**: the *mechanism* now exists on two
  backends. `--nested` changes mode from the host's configure and `--tty`
  re-modesets on DRM hotplug (issue #48, `compositor/tty/hotplug.rs`), both
  through `State::resize_output`, which already propagates a new mode to
  `wl_output`, this protocol, layer surfaces and the layout. `--headless`
  still takes `--width`/`--height` once. What is missing for a client-driven
  `set_mode` is the *trigger*, not the plumbing: `tty/hotplug.rs` re-selects
  from what the connector offers rather than taking a size it is handed, so
  this needs a path that applies a caller-chosen mode (and refuses one the
  connector does not list) and an `apply`/`test` handler that routes into it
  — a much smaller piece of work than it was before #48, and still the
  natural first piece of this entry.
- **`set_position`**: meaningless with one output at `(0, 0)`. This is the
  multi-output item.
- **`set_scale`**: `State::output_scale` is resolved once from `[output]
  scale` and documented as fixed for the process's life —
  `output_scale.rs`'s `new_fractional_scale` sends the preferred scale at
  object-creation time precisely because it never changes. Live rescaling
  means re-asserting it per surface on every commit.
- **`disable_head`**: there is no "no output" state. Every render path,
  every layer-shell map and the core's own `OutputId(1)` assume one exists.
- **`set_adaptive_sync`**: no VRR support on any backend.

So this is gated on real multi-output support rather than being a
standalone item, and it should be picked up with it (or with whichever of
the bullets above lands first, one at a time — `set_mode` under `--tty`
would be the smallest real one). The read half needs no rework when it
happens: `apply`/`test` gain a success path and `HeadState` gains whatever
new state the change can produce, both inside the existing diff/`done`
batching.

## What a user loses meanwhile

Nothing they had before — the protocol was absent entirely until PR #49.
`README.md`'s "Display information" section says the refusal is deliberate
and points here, so "my Display page won't change the resolution" has
somewhere to land.
