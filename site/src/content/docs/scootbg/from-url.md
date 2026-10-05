---
title: Wallpaper from a link
description: "Point the wallpaper at a URL: download-once caching, sha256 pins, and every failure mode."
---

Point `set` — or scoot's `[wallpaper]` image — at a link: downloaded once, cached, then shown like any other image.

## A wallpaper from a link

Point `set` (or scoot's `[wallpaper] image`) at an `http://` or `https://`
URL and the daemon downloads it once and caches it, then shows the file
like any other image. Until the download lands — and when it fails — the
compositor's background shows and the error says why; nothing blocks and
nothing retries in a loop.

```sh
scootbg set https://example.com/hills.jpg
scootbg set https://example.com/hills.jpg --sha256 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `image` (a URL) | string, `http(s)://`, at most 4096 bytes | — | Downloaded once, then shown. Needs `curl` on `PATH`; `file://` and other schemes are refused (give the path itself). |
| `--sha256` / `sha256` | 64 hex digits, as `sha256sum` prints | none | Pins the download's bytes: anything else fails instead of showing. With a path, refused. Applies live: changing it re-downloads. |

**How it works.** The cache is `$XDG_CACHE_HOME/scootbg/`
(`~/.cache/scootbg/` when `XDG_CACHE_HOME` is unset, empty or relative),
one file per URL, named for the URL's SHA-256. A file already there is
shown as is (re-checked against `sha256` when one is pinned); otherwise
the worker thread — never the event loop — runs
`curl --fail --location` (redirects followed, five at most; `http` and
`https` only), capped at 32 MiB and sixty seconds, verifies the pin when
given, checks the bytes start as PNG, JPEG or WebP, and renames the
temporary file into place atomically. Two daemons racing on one entry
both download; the rename settles it, and the cache never holds a partial
file. An error page, a file past the cap, a hash mismatch, no network, or
no `curl` is an error naming the URL, and nothing is cached — a fixed
link is picked up by the next request. A failed download is tried again
on a new `set`, any `apply-config` (even an unchanged reload — scoot
sends one per reload), a reconfigured output, or a restart;
nothing polls. Prefer `https`, and pin `sha256` so a changed byte fails
loudly instead of landing on screen.

**The cache is yours to inspect and clear.** To re-download a link (its
image changed upstream and no `sha256` pins the old bytes), remove its
file from the cache directory and `set` the URL again; to drop every
download, remove the directory (`scootbg` re-creates it). `query`'s
`shows` names both the cached file (`image`) and the link (`url`).

**When the link does not show, in order:** `scootbg set` prints the
reason (and the daemon's stderr has it once). No `curl` on `PATH` says
so; a host nothing resolves or answers names `curl`'s exit; a 404 or
other HTTP error says `exited 22`; an error page (a login wall, say) says
the bytes "do not start as a PNG, JPEG or WebP" and caches nothing; a
file past 32 MiB says so (link something smaller); a `sha256` mismatch
prints both hashes (copy the actual one into the config, or drop the
pin); a cache directory nobody can write names it. Offline at startup,
the background shows until the next `set`, `apply-config`, reconfigure,
or restart with the network back.

## From Nix: fetch at build time

A look's `image` — and a `settings.wallpaper.image` you write — takes
`{ url, hash }` as well as a path or a plain string URL: the profile
fetches it once with `pkgs.fetchurl` (the hash verified by Nix, the
file cached in the store), and scootbg sees an ordinary file. A plain
string URL works too, and then scootbg itself downloads and caches it
at runtime (above).

```nix
# Fetched at build time (verified, in the store, no network at runtime):
programs.scoot.settings.wallpaper = {
  image = {
    url = "https://example.com/hills.jpg";
    hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
  };
  mode = "fill";
};
```

```nix
# Or fetched by scootbg at runtime (cached under ~/.cache/scootbg/):
programs.scoot.settings.wallpaper = {
  image = "https://example.com/hills.jpg";
  sha256 = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
  mode = "fill";
};
```

A set without string `url` and `hash` fails evaluation. A per-output
`output.<name>.image` takes the same `{ url, hash }` set. Runtime
downloads need `curl` on `PATH`: the flake's `scootbg` package carries
it (appended after your own `PATH`, so yours still wins).
