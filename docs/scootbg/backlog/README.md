# scootbg backlog

scootbg's own backlog, kept apart from the compositor's
([`docs/backlog/`](../../backlog/README.md)). Same format: one file
per item, YAML frontmatter (`title`, `status`, `area: "scootbg"`,
`priority`, `blocked`), so the set filters the same way:

```sh
rg -l 'status: "open"' docs/scootbg/backlog
rg -l 'priority: "high"' docs/scootbg/backlog
```

Items move to `resolved/` here when done, with a `-done` suffix, as in the
compositor's backlog.

## Milestone 1: colours and images, in scoot (v1)

The first shippable version: a colour or an image per output, `scootbg set`
to change it, a `[wallpaper]` section in scoot's config, and numbers
showing it is the lightest. Roughly in order; each is one PR through the
full per-feature cycle.

1. [**Choosing dependencies**](resolved/dependencies-done.md) — RESOLVED
   2026-09-26, by measurement: plain `wayland-client`, `zune-jpeg` +
   `png` + `image-webp` directly, `pic-scale-safe` (no `unsafe`), a
   hand-rolled CLI, `serde_json` for the socket, a line-format state file,
   and a pure-Rust large-allocation allocator to return the heap. No C
   beyond std's, and the only `unsafe` sits in a small `scootbg-mem`
   crate
2. [The crate, the daemon and the control socket](crate-and-daemon.md)
3. [One background layer surface per output, across hotplug](outputs-and-layer-surfaces.md)
4. [Solid colours through single-pixel buffers](solid-colour.md)
5. [The CLI and control protocol](cli-and-ipc.md)
6. [Decoding images and fitting them to an output](images-decode-and-fit.md)
7. [Drawing at real device pixels on scaled outputs](hidpi-fractional-scale.md)
8. [Buffers, memory and zero idle cost](memory-and-idle.md)
9. [Restoring the last wallpaper at startup](restore-state.md)
10. [Seamless in scoot: a `[wallpaper]` config section](scoot-integration.md)
11. [Lowest resource use of any wallpaper daemon](lightest.md) — the
    release gate: v1 ships only when no competitor beats scootbg beyond the
    noise margin on any row both can do
12. [Tests: unit, and end to end on headless scoot](testing.md) — grows with
    every item above, not batched at the end

## Milestone 2: motion

- [Transitions between wallpapers](transitions.md)
- [Animated wallpapers: GIF, APNG, animated WebP](animated-images.md)

## Milestone 3: extras

- [A wallpaper per workspace](per-workspace.md) (`ext-workspace-v1`)
- [A config file, and rotating through a directory](config-and-rotation.md)
- [More image formats: AVIF, JPEG XL, HEIF](more-formats.md)
