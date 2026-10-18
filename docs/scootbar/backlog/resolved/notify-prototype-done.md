---
title: "scootnotify go/no-go prototype, out of the workspace: own the name, draw a card, compare with mako"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-17"
---

# scootnotify prototype: the go/no-go gate

Filed 2026-10-12 from the [scootnotify design](../../../../dev/design/scootnotify.md#goals-non-goals-and-the-case-for-it).
Serves **daily-drive**. It decides whether scootnotify is built or mako stays, **before** any
extraction is spent on it.

## What to do

A throwaway binary **outside the workspace** that `#[path]`-includes scootbar's drawing, text
and D-Bus sources, the pattern `crates/scootbar/fuzz/Cargo.toml:28-65` already uses. It owns
the name and answers `Notify` with one card on an overlay surface, and nothing else.

The memory row is close to certain to pass: scootbar idles at 3.6 to 4.3 MB, and stock mako
was measured at 9,424 kB RSS (#579 review). The rows that can fail are text quality and hangs.
Measure them in the same harness as mako:

- idle RSS/PSS, wakeups in 60 s, and with one card up;
- a real-client corpus rendered beside mako: notify-send plain, markup, `-i` name and path,
  `-h int:value:`; libnotify `image-data`; Firefox; Thunderbird; an Electron app; a GTK4 app
  through the portal; at least one CJK and one Cyrillic body, plus Arabic if the shaping spike
  has landed;
- a 5 MiB `Notify` answered, not hung.

## Acceptance

The design's gate, recorded here with the numbers:

- idle RSS at most half of mako's, and 0 wakeups;
- a legible corpus, judged by the maintainer from the screenshots;
- every call answered.

## Verdict: GO, on one condition (measured 2026-10-18)

`notify-server` is built, including the small `LimitsExceeded` reply for
over-cap method calls that the design already prescribes. The full record
is `dev/spikes/scootbar/notify-prototype.md` with screenshots in
`dev/spikes/scootbar/notify-prototype/`; the throwaway source is deleted.

- Idle RSS 3,140 kB against mako's 8,432 kB in the same harness (37%,
  gate: at most half), 0 wakeups in 60 s both sides. One card up: +76 kB
  RSS, ≤1 wakeup in 60 s. (DejaVu-only font loading; five full fonts
  read into heap cost 21.6 MB — a prototype artifact, since the real
  server memory-maps fonts the way `font.rs` already does. The row is
  engine-only: mako had no CJK fonts here, and a mapped CJK fallback is
  unmeasured. See record.)
- Corpus (notify-send suite, `image-data`, CJK, Cyrillic, Arabic, value,
  actions, replace, stack): legible throughout; Arabic draws wrong in
  the way the text-shaping NO-GO predicted. Firefox, Thunderbird, an
  Electron app and a portal app did not run (no GUI apps on the headless
  VM). Maintainer judges legibility from the side-by-sides.
- Small calls are all answered (ids, capabilities, racy close of an
  unknown id gets the spec's empty Error). A 5 MiB `Notify` hangs on
  unmodified scootbar D-Bus code (skipped silently, sender waits out its
  timeout; mako answers promptly) and is answered in under a millisecond
  with the prescribed `LimitsExceeded` reply, validated in this spike.
  That reply rides with `notify-server` or the gate's third row fails in
  the real thing too.

## Resolving this gate

The tickets it guards name it in prose ("GO verdict recorded in notify-prototype"), which `scripts/backlog resolve` does not strip ([why](../../../../dev/design/shell-apps.md#how-the-gates-stay-closed)). Record the verdict here, then resolve. On **go**, a human deletes that clause from the guarded tickets. On **no**, a human resolves the epic and the guarded tickets as won't-do, citing this evidence.

## Evidence the review needs

The rows with the SHA and method, both sets of screenshots, and the prototype's source (out of
the workspace, deleted after).

## Not in this ticket

Anything kept: the real server is `notify-server`.
