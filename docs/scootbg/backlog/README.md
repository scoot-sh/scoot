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
9. [**Restoring the last wallpaper at startup**](resolved/restore-state-done.md)
   — RESOLVED 2026-09-27: every `set` and `clear` saved per output in
   `$XDG_STATE_HOME/scootbg/PROFILE` (a versioned, escaped line format,
   read defensively, written atomically off the loop), restored by
   `scootbg daemon` (`--profile NAME`, `--no-restore`); a moved image falls
   back with a warning, entries for absent outputs survive, the
   `apply-config` fingerprint is reserved; an image `set` sent as the
   daemon starts, and a restore, decode once (714–820 → 450–476 ms to a
   4K image on screen; a restored one 451–497 ms)
10. [**Seamless in scoot: a `[wallpaper]` config section**](resolved/scoot-integration-done.md)
    — RESOLVED 2026-09-27: part A, `scootbg apply-config --profile NAME
    JSON` (strict JSON schema, SHA-256 fingerprint over a canonical
    encoding, profile adoption, a detached daemon started race-safely,
    `{}` recorded with no daemon); part B, scoot's `[wallpaper]` section
    (read leniently so a mistake costs only the wallpaper, paths resolved,
    only the written keys sent), spawned at startup and on every reload,
    one run at a time with the newest section winning, a 40 s bound and
    a failed section retried, exit statuses logged; the NixOS and
    home-manager `programs.scoot.wallpaper.*` options and
    `overlays.default`; the smoke test checks a wallpaper pixel on each
    headless output
11. [Lowest resource use of any wallpaper daemon](lightest.md) — the
    release gate: v1 ships only when no competitor beats scootbg beyond the
    noise margin on any row both can do. Measured 2026-09-27
    (`scripts/scootbg-bench/`, on headless scoot and sway): **not passed**,
    awww holds 1.0–1.6 MiB less idle memory above the floor; every other
    row is a win or a tie. Re-run 2026-09-28 after the attribution and the
    cheap levers: still not passed (1.0–1.5 MiB). **The gate does not
    pass; its one failing class, idle memory above the floor (9 rows on
    each compositor), is waived for v1 by the user** (2026-09-28): it is
    mostly clean program code the kernel can reclaim, and closing it needs
    a separate daemon binary, which is post-v1. Every other row is a win
    or a tie
    - [Idle memory above the floor: the daemon's resident code](resolved/idle-code-pages-done.md)
      — the failing class, waived for v1: where the pages are, the levers
      tried, and the separate daemon binary as the post-v1 option
12. [**Tests: unit, and end to end on headless scoot**](resolved/testing-done.md)
    — RESOLVED 2026-09-28: a `cargo fuzz` target over decode, crop,
    scale and pack on the stable toolchain (no sanitizer), its seed
    corpus and past crashes replayed by an ordinary test; a live `set`
    that cannot be drawn checked end to end, with `query`'s new
    `draw_error` saying why; the precedence rule across a restart with
    two per-output overrides and a new `command`; hotplug on headless
    scoot recorded as a gap (covered on sway). The suites keep growing
    with every later milestone: [testing.md](../testing.md) says where
    each kind of test lives
    - [The scaler's output allocation aborts the daemon when memory is refused](resolved/scaler-oom-abort-done.md)
      — RESOLVED 2026-10-07: probe-then-refuse plus docs — `scale` probes
      the output, each scaled axis's weight tables and the row scratch and
      refuses the draw (`draw_failed` / `draw_error`) instead of letting
      the scaler end the daemon; the probe stays a heuristic under strict
      overcommit
13. [**Idle image retention**](resolved/image-retention-done.md) — CLOSED
    2026-10-05 as two artifacts, no retained copy: the 12 MB "anon" is
    misclassified shm pools (same `smaps.sh` off-by-one), and the lean
    cohort never drew the wallpaper (its image file was unreadable, so the
    compositor's background showed). Pinned by `tests/retention.rs`.
14. [**How swaybg shows the same wallpaper at 2.0 MB PSS**](resolved/swaybg-two-mb-done.md)
    — RESOLVED 2026-10-08 as a measured don't-build: swaybg drops its
    pool and buffer right after commit (source + `smaps`: 0 client maps),
    but under a pixman compositor the pool persists there at full size —
    the floor is an identical 8112 kB both sides, 3/3 rounds each — so
    adopting it would only move PSS, at a re-decode cost per re-commit.

## Milestone 2: motion

- [Transitions between wallpapers](resolved/transitions-done.md) — RESOLVED 2026-10-07: fade/wipe/grow/none + duration/easing, CPU into wl_shm, frame callbacks + presentation + timer pacing, damage-limited, mid-transition restart, 8 ms budget with degrade, zero idle cost (PR #501)
- [Animated wallpapers: GIF, APNG, animated WebP](animated-images.md)

## Milestone 3: extras

- [Wallpapers from a link, downloaded once and cached](resolved/wallpaper-from-url-done.md) — RESOLVED 2026-10-05: `set`/section URLs via `curl` + cache, state format v2, Nix `{ url, hash }`; vinyl-sunset deliberately not auto-fetched (Pixabay terms)
- [A wallpaper per workspace](resolved/per-workspace-done.md) (`ext-workspace-v1`) — RESOLVED 2026-10-08: `set --workspace` / `clear --workspace` (new protocol types), per-output actives from the manager bound only while mapped, transitions on switch, preloaded buffers, state v3; `[wallpaper]` keys split to [a follow-up](config-workspace-wallpapers.md)
- [Workspace wallpapers in scoot's `[wallpaper]` section](config-workspace-wallpapers.md)
- [A config file, and rotating through a directory](resolved/config-and-rotation-done.md) — RESOLVED 2026-10-08: the slideshow ships (`scootbg set DIR --every`, PR #514); the config file is split into [config-file](config-file.md), deferred per the ticket's "only if people ask"
- [A config file with per-output defaults](config-file.md) (split from the above; only if people ask)
- [More image formats: AVIF, JPEG XL, HEIF](more-formats.md)
- [Detached `cargo test` hangs the signal test: inherited `SIG_IGN`](signal-disposition-detached-tests.md) — open, low, filed 2026-10-08 from PR #514's review: the daemon relies on default dispositions, which a detached session inherits as ignored; reset them at startup or in the harness
- [Share tests hard-code 4K page rounding](resolved/scootbg-share-tests-page-size-done.md) — RESOLVED 2026-09-30: the tests take the host's page size (verified on the Asahi's 16 KiB pages)
