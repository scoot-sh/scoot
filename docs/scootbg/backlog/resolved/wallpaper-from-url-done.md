---
title: "Wallpapers from a link, downloaded once and cached"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# Wallpapers from a link, downloaded once and cached

Filed 2026-10-04. Serves **daily-driving**: a look's wallpaper (and any
user's) can be a link, not just a file on disk — the maintainer's words,
"use the previous image", i.e. the vinyl-sunset illustration on Pixabay,
without ever committing a copy of it. (It serves computer use too, in that
an agent provisioning a VM can point at an image URL rather than smuggling
bytes, but daily-driving is the case that picks it.)

## The gap

`scootbg set` and scoot's `[wallpaper] image` take only absolute paths
(`crates/scootbg/src/cli.rs`, `crates/scoot/src/compositor/wallpaper/section.rs`:
anything else is refused). A Nix look's `image` is likewise a path
(`nix/modules/desktop.nix`, resolved with `toString` in
`nix/modules/home.nix`). There is no way to say "this URL", so the
vinyl-sunset look — whose illustration's Pixabay Content License forbids
passing it on standalone — falls back to a flat color unless the user
hand-downloads the file first (`docs/examples/vinyl-sunset/README.md`).

## What to do

Two halves, one behavior (downloaded once, cached):

1. **Nix (build time):** a look's (and a user's) wallpaper `image` can be
   `{ url = "..."; hash = "sha256-..."; }` as well as a path. The profile
   resolves it with `pkgs.fetchurl`, so it is cached in the Nix store and
   fetched once, and scootbg sees an ordinary store path. The hash is
   verified by Nix at build time.
2. **scootbg (run time), for people not using Nix:** `[wallpaper] image =
   "https://..."` with an optional `sha256 = "..."` (hex). The daemon
   keeps `$XDG_CACHE_HOME/scootbg/` (fallback `~/.cache/scootbg/`), keyed
   by the URL: a cache hit shows the file; a miss downloads it once, off
   the event loop, by spawning `curl` (no TLS/HTTP stack linked into
   scootbg — it must stay light; measure `.text`/RSS before and after per
   `lightest.md`), verifies the hash when given, writes atomically
   (temp + rename), then shows it. Until then — and offline, or when the
   download fails — the palette/background color shows, the reason is said
   once in the log, nothing blocks, and nothing retries in a tight loop
   (once per start: a new `set`, a changed section, or a reconfigured
   output re-attempts; nothing polls).

Edge cases to pin: a URL that returns HTML (fails decode, never cached),
a huge file (capped), redirects (followed, capped), partial downloads
(temp file, never renamed until complete), two instances racing (atomic
rename settles it), `file://` (refused: use the path), an unwritable
cache dir (loud error, background stays), `http` vs `https` (both fetch;
prefer `https`, pin `sha256`).

Deliberately **not** wired: the vinyl-sunset look does not fetch the
Pixabay illustration by default. Pixabay's Terms (§8) forbid "programs or
robots for automatic data collection and/or extraction" for unauthorised
purposes, and the Content License (§5) forbids distributing Content
standalone — naming wallpaper explicitly — where resizing/cropping still
counts as standalone. Automated fetching on the user's behalf is at best
unclear, so the look stays on its palette color and the README documents
the manual-download path. If Pixabay ever offers an explicit
machine-fetchable route, revisit.

## Not in this ticket

- A look registry / `look = "auto"` (the theming direction's own work).
- Retrying a failed download on a timer; validating TLS beyond what
  `curl` does; mirroring or re-hosting images.
- `scootbg` learning any other network behavior (rotation feeds, remote
  galleries).

## Resolution

Landed 2026-10-05 (PR #TBD): `scootbg set` / `[wallpaper] image` take
`http(s)` URLs with optional `sha256`, downloaded once by `curl` into
`$XDG_CACHE_HOME/scootbg/` (new `fetch` module; worker-thread fetch,
atomic rename, 32 MiB / 60 s caps, no new dependencies: `.text` +36,864 B,
idle RSS unchanged at 3.8 MB); state format v2 persists the URL; Nix
looks and users can use `{ url, hash }` via `pkgs.fetchurl`
(`nix/tests.nix`, over a `file://` fixture). The vinyl-sunset look
deliberately does NOT auto-fetch the Pixabay illustration (ToS §§5/8):
palette color stays, README documents the manual path. Evidence: `cargo
nextest run -p scootbg -p scootbg-mem` 478 passed (incl. 3 new
`download.rs` end-to-end tests with real pixels), `nix build
.#checks.aarch64-linux.scoot-modules` green, live headless screenshot
deterministic across captures.
