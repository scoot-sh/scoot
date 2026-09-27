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

## Milestone 1: colors and images, in scoot (v1)

The first shippable version: a color or an image per output, `scootbg set`
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
2. [**The crate, the daemon and the control socket**](resolved/crate-and-daemon-done.md)
   — RESOLVED 2026-09-26: `scootbg daemon`, `query`, `version`, `kill`;
   the socket (lock-guarded), the allocator and the shm buffer type in
   `scootbg-mem`; the Nix package and the CI split
3. [**One background layer surface per output, across hotplug**](resolved/outputs-and-layer-surfaces-done.md)
   — RESOLVED 2026-09-27: outputs tracked through hotplug, one
   `background` surface each (configured, nothing drawn yet), `query`
   lists them; a failed `accept` rests the listener instead of ending the
   daemon; checked on headless scoot and headless sway
4. [**Solid colors through single-pixel buffers**](resolved/solid-color-done.md)
   — RESOLVED 2026-09-27: `scootbg set '#rrggbb' [--output NAME]` and
   `scootbg clear`; single-pixel buffer + viewport, else a 1×1 `wl_shm`
   buffer + viewport, else a full-size one; replies after a sync round
   trip, without blocking the loop; `query` reports what each output
   shows; checked by real pixels on headless scoot and headless sway
5. [**The CLI and control protocol**](resolved/cli-and-ipc-done.md) —
   RESOLVED 2026-09-27, across tickets 4 and 6: `set` with a color or an
   image path, `clear`, `query`, `kill`, `version`, `--help` everywhere,
   replies after the compositor has the change; `apply-config` moved to
   ticket 10, which it exists for
6. [**Decoding images and fitting them to an output**](resolved/images-decode-and-fit-done.md)
   — RESOLVED 2026-09-27: `scootbg set PATH` with `--mode
   fill|fit|stretch|center|tile`, `--fill` and `--filter`; PNG, JPEG and
   WebP decoded and scaled on a worker thread, EXIF orientation applied in
   the packing pass; a file that cannot be shown is an error that changes
   nothing, and the newest request wins; checked by real pixels on
   headless scoot and headless sway
7. [**Drawing at real device pixels on scaled outputs**](resolved/hidpi-fractional-scale-done.md)
   — RESOLVED 2026-09-27: images (and the full-size color fallback) at
   the surface's device pixels, from `wp_fractional_scale_v1` under a
   viewport (the protocol's rounding, measured exact where snapping to the
   mode was not), else the integer `preferred_buffer_scale` or
   `wl_output.scale`; a stale smaller scale gives way; `query` reports
   `surface.scale` and `surface.pixels`; 44% fewer buffer bytes at 1.5;
   checked by a one-pixel checker on headless scoot and headless sway
8. [**Buffers, memory and zero idle cost**](resolved/memory-and-idle-done.md)
   — RESOLVED 2026-09-27: the resource budget measured and published
   (idle, memory for 1× 1080p, 1× 4K and 2× 4K, startup); outputs of one
   size showing one image share its pixels (one memfd and pool, a
   `wl_buffer` each: 2× 4K 69.2 → 36.8 MB RSS), and an output plugged in
   later shares them with no decode; memfds closed once pooled; no spare
   buffer kept at rest; a surface the compositor never configures no
   longer holds up replies
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
