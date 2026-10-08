---
title: "Bluetooth module: adapter and device state over BlueZ"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
---

# Bluetooth module

Filed 2026-09-29 (research: Waybar #688 asks for fuller Bluetooth). Serves
**daily-drive**.

BlueZ is D-Bus: `org.bluez` objects with `PropertiesChanged` signals, through the
[shared D-Bus client](resolved/dbus-client-done.md). All signals, no polling.

- Adapter on/off and connected-device count, a state class, and the connected
  device's name (bounded and sanitized text, like
  [window titles](window-title-module.md)).
- Click toggles power; picking a device to connect is a menu, through the
  launcher's dmenu mode or [popups](resolved/popups-done.md), never the bar's own protocol code.
- `Unavailable` and zero cost with no adapter or no BlueZ; a device that vanishes
  mid-connect is not an error.
- Battery level of a connected device only if BlueZ reports it; do not poll.

## Done when

State follows a real adapter and a real headset, and the module costs nothing
with Bluetooth off or absent.

## Resolution (2026-10-03, PR #399, code commit `1cde0dbe`)

Landed the bluetooth module behind the Cargo feature `bluetooth` (in
`default`; the size row's regression was waived by the maintainer on
2026-10-03, see [lightest.md](../lightest.md#m6-bluetooth-module-level-cost-measured-2026-10-03)). What the VM proves,
item by item (every code fix below has a test that failed before it;
`crates/` tree `1cde0dbe`, dev VM aarch64 rustc 1.97.1):

- **System-bus support** (`crates/scootbar/src/dbus/conn.rs`,
  `link.rs`): `system_bus_path_for` (`DBUS_SYSTEM_BUS_ADDRESS` when it
  names a path, else `/run/dbus/system_bus_socket`) and
  `Link::start_on` (only the stderr noun differs), plus one symmetric
  `Writer::i16`. The only `src/dbus/` edits in the PR (the tray-review
  hardening agent works there concurrently). Test:
  `dbus::tests::the_system_bus_address_names_a_path_or_falls_back`.
- **Wire readers** (`src/dbus/bluez.rs`, `std` only, in the `dbus` fuzz
  target like `mpris`): `GetManagedObjects`, `InterfacesAdded/Removed`,
  per-interface `GetAll`, `PropertiesChanged`; counts bounded
  (128 props, 32 interfaces, 128 names), wrong-typed properties skipped
  alone, misshapen bodies refused whole. Tests: `bluez/tests.rs`
  (small world, unknown interfaces, wrong types, overlong lists,
  trailing bytes, bad paths, invalidated) and the fuzz corpus replay.
- **State machine** (`src/modules/bluetooth/session.rs`): owner-tracked
  `org.bluez` (signals believed only from the owner, `NameOwnerChanged`
  only from the bus), full set from one `GetManagedObjects`, single
  `GetAll` re-reads (50 ms floor), at most 8 adapters / 64 devices,
  cleaned names (120 B). An oversize answer keeps the last state: one
  retry on the coalesce timer, then the next signal (a re-read loop found
  by the oversize test was fixed the same day). Adapter power dominates
  a stale connected flag (found by the real-daemon test). Tests: 20
  scripted-bus cases and 4 real-daemon cases (zero/two adapters, external
  toggle, forged senders, restart, crash, hotplug, storm throttle, no
  Name/Alias, long names, non-UTF-8, Battery1 appear/vanish).
- **Module** (`src/modules/bluetooth/mod.rs`): connected device's name +
  charge / `on` / `off` / empty; click toggles `Adapter1.Powered`
  (fire-and-forget `Set`, verified on the wire); `menu` spawns
  `menu-command` with the device list; 100 ms draw throttle (at most two
  draws for 400 flips). Config `[bluetooth]`, help, registry, `cli.md`
  (`## Bluetooth`, table, query, config), `testing.md`, fuzz README.
- **Idle cost** (`dev/benches/scootbar/m6-bluetooth-vm/`): zero wakeups
  with no bus, bus without BlueZ, and idle scripted BlueZ showing
  `Headset 72%`; +65,536 B on disk (+66,224 B loaded) over `main`,
  feature-off +3,032 B loaded. `Cargo.lock` unchanged.
- **Verification**: `fmt --check` clean; `clippy -D warnings` clean on
  all 41 feature combos; `nextest` and `cargo test` green except
  `agent layout_rectangles_are_where_a_click_lands...`, which fails
  identically on unmodified `origin/main` in this environment
  (pre-existing, not this PR).

**Not proven here** (no machine involved had an adapter): a real
power toggle, `Battery1` pacing on real hardware, real
`GetManagedObjects` scale, Waybar beside it. Filed as
[bluetooth-real-hardware](bluetooth-real-hardware.md) (M6) — a human at
the Asahi box. No real-headset evidence is claimed.
