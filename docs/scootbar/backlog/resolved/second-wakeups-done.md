---
title: "scootbar wakes about 40 times a minute at idle"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# scootbar wakes about 40 times a minute at idle

Filed 2026-10-05 from the five-desktop idle benchmark (Asahi M2,
`docs/benchmarks.md`), answering the open question in
`docs/backlog/resolved/session-launcher-idle-poll-done.md` ("file separately if
real"). Serves **daily-drive** (battery: wakeups are the idle-CPU
story once the launcher poll is fixed).

## The gap

It is real: the bar wakes 36-42 times per 60 s at idle with zero ticks
(`ev/scoot-c*-delta.tsv`, `vol=` column for `scootbar daemon`). The
likely source is the clock module ticking once a second to render a
minute-resolution clock (`clock.format = "%-I:%M %P"` in the measured
config) — waybar does the same (64-68 wakes/min with the same content),
so this is the going rate for a 1-second clock tick, not a scootbar
defect per se.

Scale: after the launcher-poll fix lands, the session's remaining
wakeups are roughly the manager (~200/min, also poll-driven today),
the bar (~40) and the compositor (~15). The bar would then be the
second-biggest waker.

## What to do

Confirm the source (strace the wakes for a minute, or gate the clock
tick behind "does the rendered minute change"), then wake the clock
only when its text changes (expected saving: ~35 wakes/min, from ~40
to the DND/unread feed's handful). Keep 1-second ticks for formats
that show seconds. Edge cases: DND toggle and notification count must
still update promptly (they are event-driven already); timezone
changes and suspend/resume must re-render.

## Not in this ticket

The launcher poll and the manager wakeups (that ticket); waybar's
higher rate (competitor observation, not ours); the bar's 4.6 MB PSS
(already 7-9x lighter than waybar — see the benchmark).

## Resolution (2026-10-05, PR #458)

Measured first, on the same Asahi M2, release builds of this tree,
headless scoot plus the benchmark's exact bar (`workspaces`, `clock`
with `%-I:%M %P`, `network` with `show-text = false`, `volume`,
`battery`), 60 s idle windows sampled from `/proc/PID/status`:
**36 wakes**, against the benchmark's 36/42/39. Per-module isolation
(same harness, one module placed): clock **2**, network **32–45**,
volume **0**, battery **0**, workspaces **0**.

The ticket's hypothesized cause was wrong, and that is the first
finding: the clock has ticked once a minute since the module API
(#324) — one absolute `timerfd` armed at the next minute boundary
(`TFD_TIMER_CANCEL_ON_SET`, so NTP steps and suspend/resume wake it
at once), one frame, one `wl_buffer.release`: 2 wakes a minute.
Verified, not changed. Formats with seconds still tick every second;
the classification (`Format::seconds`) is now tested for every
documented specifier, and a specifier the bar cannot classify never
reaches a tick — parsing refuses it at startup, so the 1 s fallback
is the refusal, not a runtime branch.

The actual idle cost was the **network module's signal re-read**: the
signal has no kernel event where the radio refuses CQM offload
(brcmfmac does), so a `timerfd` re-asks the station every 10 s,
armed only while WiFi is shown. Each tick cost the timer wake, the
station reply wake, and — nearly every time — a redraw plus the
compositor's release, because the change check compared the raw dBm
and an idle radio wanders constantly (sampled live off `query`:
−40…−49 dBm inside a minute, the shown level steady at 4 bars).

The change (`crates/scootbar/src/modules/network/mod.rs`): the
fingerprint compares the icon's `bars_for` level instead of the raw
dBm. The 10 s cadence stays — the signal has no event to replace it
with — and the exact dBm stays live in the tooltip and `query`, which
read the latest reply either way: fresh whenever the tooltip opens,
refreshed with the next redraw while one stays open.

Per-tick strace accounting (the deterministic evidence; run-to-run
counts on this shared box swing ±8 on IPv6 tempaddr churn and other
agents' radio traffic): before, 6 of 7 ticks redrew (release within
8 ms of the reply); after, **0 of 7 ticks redrew** — two wakes a tick
(the timer, then the reply), nothing further. Run counts, network
only: 32–45 before across six windows, 30–42 after across five; the
mechanism, not the noisy totals, is the proof. Remaining floor with
WiFi shown is ~12/min for the re-read plus the clock's 2 and whatever
real kernel events arrive (uevents, link/addr, roam/scan notices —
each drained into one re-read per turn, as the storm tests pin). The
"handful" hoped for is not reachable while the signal is polled; the
cadence is the justified one, kept deliberately.

Edge cases from the ticket, verified unchanged: DND/unread (`push`)
has no fd, timer or thread — event-driven already; timezone changes
re-read the zone file on every wake and steps/suspend cancel the
timer into an immediate re-render (pre-existing, tested).

Ratchet (`lightest.md` idle-wakeups and `.text` rows): release
`.text` +64 B (1,728,488 → 1,728,552), file size unchanged
(2,233,056 B), idle RSS level (5,792 kB both sides), no new
dependency, one thread throughout. No row regresses; nothing waived.

Tests: `signal_jitter_inside_a_level_redraws_nothing` (new — failed
before the fix, `Changed` where `Unchanged` was wanted, passes
after; also pins the tooltip's live dBm and the redraw across a
level), `seconds_decide_the_tick` extended to every documented
specifier class. Full `network` + `clock` suites: 127 passed.
Docs on the site (`scootbar/modules.md`: the Wakes row, the idle
bullet under Network).
