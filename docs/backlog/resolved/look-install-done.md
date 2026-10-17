---
title: "scoot look install from a path, export and save, behind the trust boundary"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-17"
---

# scoot look install from a path, export and save, behind the trust boundary

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves
**daily-drive**. Low: the security-sensitive part, after the basics.

## The gap

There is no way to take a look from someone else or to hand one over, and
anything arriving from elsewhere crosses the trust boundary of the design
doc's section 8.3.

## What to do

- `scoot look install PATH [--trust]` into `installed-looks/` (never
  `user-looks/`, so its origin is on disk): copies regular files only (a
  symlink, device, FIFO or socket refuses the install), and only the file
  kinds a look uses, by name and place (the design doc's section 8.3 list:
  `look.toml`, `preview.png`, `.png`/`.jpg`/`.jpeg`/`.webp`/`.gif`
  wallpapers, `fonts/*.ttf|otf`, the five named `apps/` leaves, `LICENSE*`,
  `COPYING*`, `README.md`); any other file refuses the install by name.
  Every referenced path resolved inside the look directory, file count and
  total size capped, `license` required, the name of a shipped, declared or
  user-authored look refused, `apps/` static leaves dropped unless `--trust`
  (printed first). An installed look can be applied but never extended.
- A wallpaper given by URL in an installed look is fetched, verified
  against its `sha256` and cached at install time (the install refuses if
  that fails), so a swap to it never makes a network request (R5; the URL
  grammar in `look-format` already refuses userinfo, IP literals, ports and
  local names).
- **The install-time fetch must not reuse scootbg's redirect-following
  fetch as is (F1).** scootbg runs `curl --location --max-redirs 5 --proto
  http,https --proto-redir http,https` (`crates/scootbg/src/fetch.rs:64,347-353`),
  so a public URL that passes the grammar can redirect to `127.0.0.1` or a
  LAN host, and a public name can resolve to a private address (DNS
  rebinding). For an installed look: follow no redirects (or re-validate
  every hop against the grammar and the address rule); resolve the host
  first, refuse a loopback, link-local, private, unique-local or
  unspecified address, and pin the connection to the checked address
  (`curl --resolve host:443:ADDR`) so a second lookup cannot swap it. Test
  each refusal: a redirect to loopback, a name resolving to RFC 1918 and
  `fe80::/10`, and a rebinding resolver.
- `installed-looks/` is already in `look-apply-cli`'s catalog search, so an
  installed look appears in `list` and `apply` with no change there.
- `scoot look export NAME`: a tarball of a source look, regular files only.
- `scoot look save NAME`: the user's own appearance and color values as a
  source look in `user-looks/` (user-authored, so extendable).

## Evidence to record

A test per refusal (symlink, `..`, absolute path, a file kind outside the
list such as a `.so`, a shipped or user-authored name, missing license, an
oversized image, `apps/` without `--trust`), and a test that `extends`
never resolves to an installed look.

## Not in this ticket

Git URLs and `lookPacks` (`look-share`, later).

## Resolution (2026-10-17, PR #644, fix 2026-10-17)

`scoot look install PATH [--trust]` takes a look directory or a plain
`.tar` tarball into `installed-looks/`: regular files only (symlinks,
hardlinks with `nlink != 1`, devices, FIFOs and sockets refuse), only
the file kinds a look uses by name and place, 64 files / 96 MiB total
(32 MiB images, 16 MiB fonts, 64 KiB everything else), `license`
required, never under a shipped, declared or user-authored name,
`apps/` leaves dropped unless `--trust` (printed first). Both paths
read with the same per-kind caps (`O_NOFOLLOW`, at most `cap + 1`,
regular-file recheck), so a real wallpaper installs from a directory
as it does from a tarball. A URL wallpaper is fetched at install time
— no redirects, the host resolved and classified (loopback, private,
link-local, unique-local and the rest refuse), the checked address
pinned with `curl --resolve`, the bytes verified against `sha256` and
the image magic — into scootbg's cache (unique `part-pid-counter`
names), so the swap resolves the cache hit and never makes a network
request. `scoot look export NAME [--out FILE]` writes a deterministic
ustar tarball (unique `part-pid-counter` name) that installs back byte
for byte; symlinks skip with a note instead of baking outside bytes.
`scoot look save NAME` snapshots the user's
`[appearance]`/`[wallpaper]`/`[colors]` as a user-look extending the
recorded live look; its wallpaper file reads capped, and a URL
wallpaper names the reinstall its first swap may owe. Works without
Nix; tests never touch the real HOME/XDG.

Evidence: 35 unit tests in `crates/scootctl/src/look/install/tests.rs`
(one per refusal, hardlink refusal, large image and font from dir and
tar, `extends`-never-installed end to end, the export/install and save
round-trips, export symlink skip, save URL reinstall note, the fetch
refusals with the curl argv pinned, mutation-proved on the directory
symlink, hardlink, capped-read, tar kind, and redirect/rebinding
guards) and 3 CLI parse tests; nextest 469 passed, `cargo test`
162+9+4+4+282+8 passed, DejaVu-only `FONTCONFIG_FILE`, scratch
HOME/XDG, plus real-binary `scoot look install` of `looks/moonrise`
(copied and renamed) from dir and tar. Docs in the same PR
(`looks/index.md` Sharing).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
