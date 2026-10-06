---
title: Screenshots
description: "Capture outputs reproducibly over IPC: the request, the pointer, physical vs logical pixels, retry rules."
---
Capture the screen reproducibly — the same path the docs' own screenshots use, on every backend including `--headless` with no display at all.

## Take one

```sh
scoot msg screenshot --out /tmp/shot.png
scoot msg screenshot --output 2 --out /tmp/second.png
scoot msg screenshot --no-cursor --out /tmp/clean.png
```

| Flag | Type | Default | Meaning |
|---|---|---|---|
| `--output ID` | int | `1` (first output) | Which output to capture; every output has a framebuffer of its own, so the capture is that output's own pixels. An id naming no output is refused rather than answered with another output's pixels. A powered-off output is refused too, naming the recovery. |
| `--out FILE` | path | stdout | Where the PNG goes; without it, the PNG bytes go to stdout. |
| `--no-cursor` | switch | draw it in | Leave the pointer out (below). |

A screenshot taken straight after a `scootbg set` shows it — the set
returns once the change is on screen — the reproducible pair the docs'
own captures use.

## Physical pixels, logical coordinates

`screenshot` captures the framebuffer at full physical resolution,
while `windows` and `outputs` report logical rectangles. Convert with
`physical = logical * scale`, rounded down where an edge lands
mid-pixel — and per output: two screens can run at different scales,
so convert a window's rectangle with the scale of the output the
window is on (its `output`), never the first output's. (Full rule in
[Rules an agent needs](./index.md#rules-an-agent-needs).)

## Pre-LUT: captures show the unmodified frame

`screenshot` reads the framebuffer, which sits *before* the display's
gamma LUT in the pipeline — so a night light (`wlsunset`, `gammastep`,
or the desktop profile's [night light](../desktop/index.md#night-light))
never shows in a capture. The file holds the compositor's own pixels:
compare two screenshots across a warming boundary and they are
byte-identical (modulo the pointer, [below](#the-pointer-in-a-screenshot)).
To observe the ramp itself, read the CRTC gamma back on the `--tty`
session (`drm_info`, `modetest`) instead of capturing it.

## Retry rules

Captures are paced: one per connection per 16 ms frame (a second inside
the same frame is refused — and note the reply order: the refusal
arrives *first*, so match replies by content, not position), one in
flight per connection, four in flight across every client. On the
`--tty` GPU scanout tier a capture can also be refused with a retry
while the screen shows a client's buffer directly — treat both
refusals as retryable. The full list, with what each costs and what to
do, is [what the socket refuses](./troubleshooting.md#what-the-socket-refuses).

### The pointer in a screenshot

**A screenshot shows the pointer, the same way on every backend and
renderer**, unless the request says not to. On the wire that is an optional
`cursor` field on the request:

```json
{"type": "screenshot"}
{"type": "screenshot", "cursor": false}
```

Omitted means drawn in (`SCREENSHOT_CURSOR_DEFAULT` in `scoot-ipc`);
`false` leaves it out, and `scoot msg screenshot --no-cursor` sends that.
"The same everywhere" is the point: an agent's script sees the pointer the
same way under `--headless` and `--nested` (whose screens have no drawn
cursor at all), on `--tty`'s default renderer, and on the GPU scanout tier
(whose cursor rides a hardware plane) — the pointer's own image, hotspot and
scale, at its position on the captured output and on no other output. A
fresh session's pointer sits at the centre of the first output, so a
default screenshot shows it there until something moves it. Leave it out
to diff two screenshots without the pointer showing up as a difference, or
to read the pixels it would cover. (Until this field existed, `--headless`
and `--nested` screenshots never showed the pointer.)

The field was added without an IPC protocol bump: a request that omits it is
the old request byte for byte, so older clients are unaffected. The flip
side: a server that predates the field silently ignores it rather than
refusing it, so on one of those `cursor: false` has no effect, and whether
the pointer shows depends on the backend as it used to (drawn only under
`--tty`, and missing where a cursor plane carries it). A client cannot tell
such a server apart: `version` reports the same protocol number either way.
