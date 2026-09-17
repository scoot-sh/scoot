---
title: "Screen capture forces `Xrgb8888`'s undefined fourth byte opaque, and that is most of what a capture costs — RESOLVED (conditional)."
status: "resolved"
area: "resolved"
priority: "low"
blocked: null
---

# Screen capture forces `Xrgb8888`'s undefined fourth byte opaque, and that is most of what a capture costs.

Found while addressing a review finding on the screencopy PR
([`screencopy-capture-done.md`](../resolved/screencopy-capture-done.md)) about
*how* that byte is written. Measuring the "how" turned up a bigger question
about the "whether", which is a behaviour decision rather than an optimization
and so is filed instead of taken.

## What happens today

`screencopy.rs` advertises `Xrgb8888` first and `Argb8888` second, and a
capture into an `Xrgb8888` buffer has every pixel's fourth byte forced to
`0xff`. The stated reason (see the module doc's "Buffer formats") is that
`[appearance] background_color` may have an alpha below 1.0, so the
framebuffer really is translucent in places — and a client that took that
alpha at face value would render a translucent "screenshot" of an opaque
screen. `Xrgb8888` says "no alpha here", and the forcing makes the bytes agree
with that.

## Why it is worth questioning

The format's fourth byte is **undefined**. A conforming client ignores it;
`grim` 1.5.0 demonstrably does (its output is an `8-bit/color RGB` PNG with no
alpha channel at all, measured). wlroots-derived compositors do not force it.

And it is expensive. Measured on the dev VM over a 1920x1080 frame:

| | `opt-level=0` | `opt-level=3` |
| --- | --- | --- |
| row `memcpy` alone, no forcing | 0.84 ms | 0.20 ms |
| row `memcpy` + forcing four pixels at a time (what ships) | 33.7 ms | 0.92 ms |

End to end over a whole `grim` capture that is ~10.3 ms in release and ~63 ms
in a dev build, dropping the forcing would be roughly **13% off a release
capture and 77% off a debug one** — far more than any of the three
implementations of the forcing differ from each other, which is what the
original review finding was about.

## The case for keeping it

Not forcing means the byte carries the framebuffer's own alpha. With the
default opaque `background_color` that is `0xff` everywhere anyway, so nothing
changes for nearly every session — the cost is only paid to defend against a
client that both asks for `Xrgb8888` *and* reads its fourth byte, which is a
client bug. But it is a client bug with a bad failure mode: a buffer blitted
onward as `Argb8888` with a `0x00` alpha is invisible rather than merely
wrong, and "screen-share shows nothing" is an expensive thing to debug from
the other end.

## What a decision needs

- Whether any real client is known to read the byte (check quickshell's
  `ScreencopyView` texture upload path, and whatever `wf-recorder`/OBS do with
  an `Xrgb8888` screencopy buffer).
- Whether the cheaper answer is to stop forcing, or to stop *advertising*
  `Argb8888` and its translucency problem instead, or to keep both and force
  only when `background_color`'s alpha is actually below 1.0 — that last one
  costs nothing in the default configuration and keeps the guarantee where it
  was actually needed. It is probably the right answer and was not taken here
  only because it is a behaviour change filed mid-review.

Rough size: S.

## Resolution

Decided as the ticket's third option: force the fourth byte only while
`[appearance] background_color`'s alpha is actually below 1.0. Costs nothing
in the default opaque configuration, keeps the guarantee exactly where it was
needed. `Argb8888` stays advertised second and is untouched (it never forced).

What shipped (`screencopy.rs`): `xrgb_needs_forcing(alpha)` — exact
`< 1.0`, no epsilon (a config parses to `byte/255.0`, so `1.0` is the only
opaque value spellable and `254/255` the nearest translucent one) — read off
`State::appearance` once per frame tick in `service_captures` and threaded
through `deliver` into `write_capture`, where `opaque` is now
`is-Xrgb && force`. No config reload exists today (nothing writes
`appearance` after `State::new`), so the per-tick read is trivially current;
it also cannot go stale if a reload ever lands. The tick is synchronous on
the event-loop thread, so no TOCTOU between the check and the writes.

Byte-identity argument (pinned by test, not reasoned): windows composite
source-over onto the framebuffer, which preserves opaqueness of the
destination — so with an opaque background every read-back pixel already
carries `0xff` and the old pass OR'd `0xff` onto `0xff`. The new
`an_xrgb_capture_over_an_opaque_background_is_the_framebuffer_byte_for_byte`
asserts both the premise (framebuffer all-`0xff`) and the conclusion
(capture == framebuffer). The existing translucent-background tests
(including the offset+stride combined one) still force and still pass; a
fail-first neuter of the conditional fails exactly those three.

Evidence: see the PR (benchmark table: micro row-memcpy vs forcing at opt 0
and opt 3, plus release `--headless` end-to-end `grim` before/after with
opaque background and post-change with translucent background).
