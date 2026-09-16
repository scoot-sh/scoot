---
title: "Quickshell's window list needs `wlr-foreign-toplevel-management-v1`; it ignores the `ext-` list flexwm now advertises."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Quickshell's window list needs `wlr-foreign-toplevel-management-v1`; it ignores the `ext-` list flexwm now advertises.

Found 2026-09-16 while implementing `ext-foreign-toplevel-list-v1`
(`resolved/foreign-toplevel-list-done.md`), by probing the client the
original entry was filed for rather than assuming it would follow the
standard.

## What was measured

Stock `quickshell` 0.3.1 (the exact build both the DMS and Noctalia probes
used, from the dev VM's nix store), a minimal `ShellRoot` whose only job is
to read `Quickshell.Wayland.ToplevelManager`, against `flexwm --headless`
with one real `foot` window open:

```
--- flexwm msg windows ---
[{"id":1,"app_id":"foot","title":"dev@flexwm-vm: ~"}]
--- quickshell ---
DEBUG qml: QS: initial count = 0
```

`WAYLAND_DEBUG=1` on the same run says why. flexwm offers the global and
quickshell simply does not take it:

```
wl_registry#2.global(7, "ext_foreign_toplevel_list_v1", 1)
```

— offered five times across four registries (main twice, plus three mesa
ones), bound zero times; `grep -n "bind.*foreign" /tmp/qs-wire.log` is empty. Its
binary carries a complete *wlr* client instead
(`zwlr_foreign_toplevel_manager_v1{,_listener,handle_toplevel,handle_finished}`
and `zwlr_foreign_toplevel_handle_v1handle_state`, …); the
`ext_foreign_toplevel_*` symbols in it are the scanner's interface tables
plus `ext_foreign_toplevel_image_capture_source_manager_v1`, which is a
different protocol.

So `ToplevelManager` — 29 use sites in DMS 1.5.3's QML, and Noctalia's
equivalent — is a `wlr-foreign-toplevel-management-unstable-v1` client. The
`ext-` list does not feed it, and the "Windows"/running-apps section of
both launchers stays empty until flexwm speaks the wlr protocol too.

## What this does and does not change

It does not undo the choice of protocol. `CLAUDE.md`'s rule (implement the
compositor-agnostic successor where one exists) still points at
`ext-foreign-toplevel-list-v1`, the pinned Smithay rev implements that and
nothing else, and it is what a standards-following client gets. The wlr
protocol is now the *compatibility* question it always was, with one fewer
unknown: the two shells this repo cares about need it.

## Why it is not a small follow-up

- **Nothing in Smithay implements it** at the pinned rev (grep: no
  `foreign_toplevel_management` anywhere), so this is a hand-rolled global,
  object lifecycle and event batching against `wayland-server` — the shape
  `ext_workspace.rs` already is, and about that size.
- **It is a control protocol, not an enumeration one.** Beyond
  title/app_id/output it carries `state` (maximized, minimized, activated,
  fullscreen) and accepts `activate`, `close`, `set_maximized`,
  `set_minimized`, `set_fullscreen`, `set_rectangle`. flexwm's core has
  `activate` and `close` and **no concept at all** of maximized, minimized
  or fullscreen — so the real work is deciding what those mean in a
  scrolling-column layout (or advertising them as unsupported and having a
  taskbar's minimise button silently do nothing), not the wire format.
- `set_rectangle` is a task-switcher animation hint; harmless to ignore.

A first cut could be enumeration + `activate` + `close` only, with the
state bits it can honestly answer (`activated`) and nothing else — which
would light up both shells' window lists and their click-to-focus, and
leave minimise/maximise for whenever the core grows them.

## Worth checking first, cheaply

Whether a newer quickshell binds `ext-foreign-toplevel-list-v1` when the
wlr global is absent. 0.3.1 does not, but this protocol is young and
quickshell moves fast; a five-minute re-probe of the current release with
the same minimal QML (kept in this entry's evidence, and trivially
rebuilt) answers it, and a yes would make this entry a compatibility
nicety instead of the thing standing between flexwm and two working
shells.
