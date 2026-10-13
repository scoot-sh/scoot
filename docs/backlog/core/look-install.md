---
title: "scoot look install from a path, export and save, behind the trust boundary"
status: "open"
area: "core"
priority: "low"
blocked: "look-authoring"
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

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
