---
title: "`xdg-toplevel-icon-v1`: pixel-buffer icons are accepted but not exposed."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# `xdg-toplevel-icon-v1`: pixel-buffer icons are accepted but not exposed.

Follow-up from implementing the protocol
(`docs/backlog/resolved/foot-protocol-warnings-done.md`). A client may build
its `xdg_toplevel_icon_v1` from an icon *name*, from one or more square
`wl_shm` buffers, or both. flexwm exposes only the name, as
`WindowSnapshot::icon` over IPC; the buffers sit in Smithay's
`ToplevelIconCachedState::buffers` and nothing reads them, so a client that
supplies only pixels reads as having no icon at all.

Why it was left: handing pixels to an IPC client means re-encoding shm
buffers to PNG per query — the same work `screenshot.rs` does for the screen,
with the same rate-limiting question — and no consumer has asked for it. Most
real toolkits send the name, which matches the client's `.desktop` file and
is what a bar wants anyway.

What it would take: pick the buffer closest to a size the caller asks for
(the protocol guarantees each is square and `wl_shm`-backed), encode it the
way `screenshot.rs` does, and add it to the `windows` response as an optional
base64 field rather than inflating every window list with icon pixels nobody
asked for. Worth doing only once something — a bar over IPC, or an agent that
needs to recognize an app visually — actually wants it.
