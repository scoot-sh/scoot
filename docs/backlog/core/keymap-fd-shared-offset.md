---
title: "Second read()-based keyboard client gets an empty keymap"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Second read()-based keyboard client gets an empty keymap

Filed 2026-10-17 from the [keyboard-stack spike](../../dev/spikes/scootui-keyboard.md#a-second-reader-gets-an-empty-keymap).
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
