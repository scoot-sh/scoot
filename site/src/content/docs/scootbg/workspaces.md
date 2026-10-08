---
title: A wallpaper per workspace
description: "Show a different wallpaper on each workspace with scootbg — set --workspace, switching, preloading, and what it costs."
---

A different wallpaper on each workspace: while workspace 2 is active,
the outputs show one image; on workspace 1, another. It follows the
standard `ext-workspace-v1` protocol scoot already speaks, so it works
on any compositor with it — not only scoot — and costs nothing when
unused (the daemon binds the protocol only while a mapping exists: no
extra wakeups, verified idle).

```sh
scootbg set '#1e1e2e'                              # the base, every output
scootbg set ~/Pictures/focus.jpg --workspace 2     # while workspace 2 is active
scootbg set ~/Pictures/code.jpg --workspace 2 --output DP-1  # ... on one output
scootbg clear --workspace 2                        # take that mapping back off
scootbg query                                      # actives and mappings, one JSON line
```

Workspaces are named by the compositor: `"1"`, `"2"`, ... on scoot (the
1-based position, as `query` lists it per output under `"workspace"`).
Other compositors announce their own names (`"web"`, `"code"`); the
mapping keys on that string either way. On scoot the names are
positions, not identities: closing the last window on workspace 3 drops
it and renumbers what follows, and a mapping for `"3"` follows position
3. A mapping for a workspace that does not exist yet simply waits —
mapped workspaces are preloaded, so the first switch to one is instant
too.

## How switching works

Switch workspaces as usual (keybinding, bar, `scoot msg action
focus-workspace-index 1`): the output fades — or lands at once —
through the transition the mapping was set with (`--transition fade`
beside `--workspace`, as [`set` takes them](./cli.md#commands)). `query`
reports the switch the same way it does a `set`: `"shows"` is what is
on screen, and `"workspace"` names the active one. A switch never waits
for a client: there is no request in flight, so nothing answers — it
just lands.

```sh
scootbg set ~/Pictures/calm.jpg --workspace 1 --transition fade --duration-ms 800
scootbg set ~/Pictures/loud.jpg --workspace 2 --transition wipe --angle 90
```

Each output switches on its own: on two outputs, workspace 2 on the
left screen never disturbs the right one's. A mapping without
`--output` covers every output; one with `--output` covers only that
connector (kept by name, so it survives unplug and replug). Where both
cover an output, and where the base `set` covers it too, the newest
change wins — one timeline: a `set` newer than a `set --workspace`
covers it until a still newer `set --workspace`. `clear --workspace`
takes one mapping off (that output falls back to its own wallpaper);
`clear` without it leaves the mappings in place under the newer clear.

## What it costs

Switching must be instant, and decoding is not (about 400 ms for a 4K
JPEG, measured): so every mapped image is decoded and scaled in the
background when it is set — and again when an output is reconfigured to
a new size — and its buffers are kept while the mapping stands. The
price is one output-sized buffer per mapped image per size: about
33 MB at 3840×2160 `XRGB8888`, shared between outputs of one size
showing one image (the second output costs a `wl_buffer`, not the
pixels). Four workspaces with four 4K images hold about 132 MB over the
floor; unmapped workspaces hold nothing, and `clear --workspace` frees
its image's buffers at once. Colors cost nothing anywhere (no shared
memory on compositors with single-pixel buffers). Past 64 live mappings
a `set --workspace` is refused naming the cap — clear one first.

Mapping the same file on two workspaces decodes and holds it twice (one
buffer per mapping per size): 64 mappings of one 4K file hold about
64 × 33 MB. Each `set` re-reads the file (it may have changed), so the
daemon never shares pixels between mappings by path; the 64-mapping cap
bounds the worst case. Map colors, or fewer images, where memory is
tight.

Without the compositor's workspace protocol (sway has none), mappings
are recorded and saved all the same, and apply once a compositor with
one is: the base wallpaper shows meanwhile, and the daemon says so once
on stderr. A group the compositor announces with zero outputs, or more
than one, cannot say which output is on which workspace, so it is
ignored (the base shows there) rather than guessed.

## Saved and restored

Every `set --workspace` and `clear --workspace` is saved in the
profile's state file like any choice (version 3: an older scootbg reads
nothing from such a file, and never writes it away), and restored at
the next start — transition included. The `[wallpaper]` section does
not manage workspace wallpapers: `apply-config` leaves the mappings
alone, and a profile adopted from it keeps the file's mappings.

> **Symptom:** *the workspace wallpaper never appears; the base stays.*
> The compositor has no `ext-workspace-v1` (the daemon says so once on
> stderr), the workspace name does not match what the compositor
> announces (`query`'s `"workspace"` shows the exact string to map), or
> a newer `set` covers the older mapping. `query`'s `"workspaces"` lists
> what is mapped.
>
> **Symptom:** *switching shows the old wallpaper for a moment first.*
> The image had no preloaded buffer when the switch came: it was set
> while its output had no size yet, or the output just resized and the
> background re-render had not landed. It heals itself (the next switch
> is instant); if it persists, the daemon's stderr says why the draw
> failed (`draw_error` in `query` says it too).
>
> **Symptom:** *`set --workspace` says too many workspace wallpapers are
> mapped.*
> Past 64 mappings: `scootbg query` lists them under `"workspaces"`;
> `scootbg clear --workspace NAME [--output NAME]` frees one.

## See also

- [CLI reference](./cli.md#commands) — when `set --workspace`
  returns, and its exit statuses.
- [Transitions](./transitions.md) — the kinds a switch animates
  through.
- [Restore](./restore.md) — profiles, the state file, and what
  `--no-restore` skips.
