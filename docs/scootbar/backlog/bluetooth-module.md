---
title: "Bluetooth module: adapter and device state over BlueZ"
status: "open"
area: "scootbar"
priority: "low"
blocked: "dbus-client"
milestone: "M6"
---

# Bluetooth module

Filed 2026-09-29 (research: Waybar #688 asks for fuller Bluetooth). Serves
**daily-drive**.

BlueZ is D-Bus: `org.bluez` objects with `PropertiesChanged` signals, through the
[shared D-Bus client](dbus-client.md). All signals, no polling.

- Adapter on/off and connected-device count, a state class, and the connected
  device's name (bounded and sanitized text, like
  [window titles](window-title-module.md)).
- Click toggles power; picking a device to connect is a menu, through the
  launcher's dmenu mode or [popups](popups.md), never the bar's own protocol code.
- `Unavailable` and zero cost with no adapter or no BlueZ; a device that vanishes
  mid-connect is not an error.
- Battery level of a connected device only if BlueZ reports it; do not poll.

## Done when

State follows a real adapter and a real headset, and the module costs nothing
with Bluetooth off or absent.
