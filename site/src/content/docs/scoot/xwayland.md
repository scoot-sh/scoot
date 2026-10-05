---
title: XWayland
description: "Running X11 apps — the packages, the switch, and the trust model."
---

X11 apps need two things the default packages do not carry: the
`xwayland` build feature, and the `Xwayland` binary on the compositor's
`PATH`. Two Linux-only packages carry both — and only if you need X
apps should you pay their ~344 MiB of closure (all but ~12 MiB of it
Xwayland's own):

| Package | Meaning |
|---|---|
| `scoot-xwayland` | `scoot` with the `xwayland` feature |
| `scoot-gpu-xwayland` | `scoot-gpu` with it |

```nix
programs.scoot = {
  enable = true;
  package = inputs.scoot.packages.${pkgs.system}.scoot-xwayland;
  session.enable = true;
};
```

The package is only half: XWayland still starts only when asked, by
`--xwayland` or `[xwayland] enabled`. Either half alone is loud rather
than silent: the knob with the default package logs that the build has
no XWayland support, and an XWayland build whose `PATH` lacks the
binary logs `` `Xwayland` must be on PATH `` with the path it
searched. Both then run a Wayland-only session.

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `[xwayland] enabled` | bool | `false` | restart only | Run an XWayland server inside the session (flag `--xwayland` ORs with it). |
| `[xwayland] fractional` | `"sharp"` / `"light"` | `"sharp"` | live | What the X server draws at a fractional output scale: `ceil(scale)` for sharp X apps at ~4x the buffer memory, or `floor(scale)` for blurry upscaled ones at ~1/4. Integer scales draw at themselves either way. |

X windows map into the layout like any other: they tile in columns,
dialogs float, `[[window_rule]]` `match_app_id` matches their
`WM_CLASS` class — and they take focus by themselves only when nothing
is focused, when they belong to the focused X app, or when scoot
started them. The clipboard and primary selection cross between X and
Wayland both ways (X apps touch them only while an X window holds the
keyboard — never while locked); drag-and-drop works in every
direction. `DISPLAY` is exported to spawned children while the server
is believed live.

Read the trust model before turning it on: **any X client can keylog
and read other X clients by design.** That is not a scoot bug to fix —
it is X11. Every protocol consequence (focus policy, seals, limits)
is in [Protocols](./protocols.md#xwayland-opt-in).
