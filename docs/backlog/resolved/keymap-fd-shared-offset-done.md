---
title: "Second read()-based keyboard client gets an empty keymap"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
---

# Second read()-based keyboard client gets an empty keymap

Filed 2026-10-17 from the [keyboard-stack spike](../../../dev/spikes/scootui-keyboard.md#a-second-reader-gets-an-empty-keymap).
Serves **daily-drive**: scootlaunch and scootgreet read the keymap as ordinary
Wayland clients, and whichever maps second decodes every key as nothing.

## The gap

Every client's `wl_keyboard.keymap` fd is the same open file description:
Smithay's `KeymapFile` wraps one sealed memfd (`SealedFile::with_data`
seeks to 0 once, at creation, in the pinned fork) and `send()` passes that
fd to each new keyboard, and `SCM_RIGHTS` installs the same description in
the receiver, offset included. The first read()-based client to read moves
the shared offset to the end; every later client reads zero bytes and
compiles an empty keymap. Real toolkits mmap the fd (offset-independent)
and never notice.

Measured on the dev VM under `scoot --headless` (fresh server per row,
35,572-byte us keymap):

- client A reads 35,572 bytes, compiles; client B reads 0, keymap empty;
- after a restart with the clients swapped: the other client reads 35,572,
  the other reads 0.

A read()-based client works around it by seeking to 0 first (or mmap);
the spike does the seek. The fix belongs server-side (rewind per send, or
one description per client) so clients do not need the workaround.

## What to do

Rewind the sealed fd (or send a fresh description) on each keymap send, so
every client reads the full keymap regardless of order. Prove it with two
sequential read()-based clients with no seek, both reading the full
35,572 bytes. Pin the edge cases: keymap change mid-session (`change_keymap`
sends to all known keyboards), virtual-keyboard clients, and the
`SCM_RIGHTS` full-fd-table path the wayland fork already guards.

## Not in this ticket

The keyboard-stack pick itself (`scootui-keyboard-spike` decided
`xkbcommon`); client-side workarounds in scootui.

## What landed

PR #764. Fork commit `b65cbc5f` on `scoot/cursor-dmabuf-storage`, repinned here
(`crates/scoot/Cargo.toml`, `Cargo.lock`, `flake.nix` `outputHashes`,
`dev/forks.md`, the `CLAUDE.md` pin paragraph), plus
`crates/scoot/src/compositor/input/keymap_fd.rs` pinning it.

`SealedFile` gains `open_independent`: a read-only reopen through
`/proc/self/fd` with its own offset at 0. Seals live on the file, so the
copy stays sealed; it is closed once the send returns, and the receiver's
own copy outlives it. `KeymapFile::with_fd` hands each send that copy
instead of the shared fd, falling back to the shared description where it
cannot be opened (no procfs). Sends happen only on keyboard creation and
keymap change (`send_keymap` returns early on an unchanged id), so the
per-key path allocates nothing new.

Measured on the dev VM: before the repin, three new tests fail with
`client 1 read 0 of 35572 announced bytes`; after, all four pass -- a
sequential second reader, a `change_keymap` broadcast to all known
keyboards, virtual-keyboard clients, and a well-behaved survivor reading
whole keymaps while a second connection floods past the wayland fork's
queue cap and is disconnected by it. Full cycle on the same build:
`cargo nextest run -p scoot` 2877 passed / 42 skipped;
`cargo test -p scoot` 2835 passed in the bin target plus every
integration target green; `cargo clippy -p scoot --all-targets --
-D warnings` and `cargo fmt --check -p scoot` clean.
