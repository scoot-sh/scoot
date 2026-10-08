---
title: "A config file, and rotating through a directory (milestone 3)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# A config file, and rotating through a directory (milestone 3)

- For use outside scoot (inside scoot, `[wallpaper]` in scoot's config
  covers this): `~/.config/scootbg/config.toml` with per-output defaults,
  fit mode, filter and transition settings. The CLI keeps working without
  it. Only if people ask; the CLI plus an autostart line may be enough.
- `scootbg set DIR --every 30m [--shuffle]`: slideshow from a directory,
  with one timer, no polling of the directory.

## Resolution (2026-10-08, PR #514)

Shipped the slideshow half. The config-file half is not built: the ticket
itself says "only if people ask", so it is split into
[config-file](config-file.md) and deferred. No one has asked.

`scootbg set DIR --every DURATION [--shuffle]` cycles the directory's
regular files (listed once, never polled), one per whole-minutes interval
(`1m`–`7d`), sorted or shuffled once, each with the usual `--mode`,
`--fill`, `--filter` and `--transition`. The first file goes through the
normal image path (the reply waits for it); later steps advance on the
loop's poll timeout — no timerfd, no extra poll slot, no wakeups without
a slideshow, at most one a minute with one. One slideshow runs at a time;
a new `set`, a `clear` or a changed `apply-config` stops it (an unchanged
section leaves it running, like a `set` made since); a restart shows the
last image without resuming. `query` reports it (`rotation`, absent when
idle, so static replies are byte-identical). Same protocol (old daemons
refuse the new fields loudly as a targetless `set`); no state-format or
section change; no new dependencies.

Tests, each proven to fail without its fix (revert-run-restore): pure
`--every` parsing/listing/shuffle units; CLI usage units (directory
detection, refusals); protocol round-trip/refusal units; daemon
start/refusal units; `tests/rotation.rs` end to end on headless scoot
(start/report/stop with real pixels, timer advance red→blue→red on a
debug-shortened interval, misuse refusals, apply-config precedence,
zero wakeups with no slideshow). One existing test updated to the new
contract (`tests/image.rs`: a bare directory is now a CLI usage error
naming `--every`, still changing nothing). One real bug found by the
suite: an extra poll slot broke fd-exhaustion survival (`poll` EINVAL
past a lowered `RLIMIT_NOFILE`); fixed by driving the slideshow off the
poll timeout instead of a new fd.

Verification: `cargo nextest run -p scootbg -p scootbg-mem` with
`SCOOTBG_REQUIRE_SCOOT=1`; `cargo clippy -p scootbg -p scootbg-mem
--all-targets -- -D warnings`; `cargo fmt --check -p scootbg
-p scootbg-mem`; `cargo deny check`; `scripts/backlog check` (only the 3
known pre-existing problems); site `cli.md` updated (`nix build
.#docs-site` for the gates). Ratchet (release, 1×1920×1080 headless scoot, 3 runs per side, 60 s
settle + 60 s window; loads 0.24–1.20): size +65536 B (+4.0%:
1643296 → 1708832), `.text` +21280 B (+1.8%: 1204292 → 1225572) —
inside the margin and far below every competitor, no new dependencies.
Idle with a static wallpaper: RSS 12096 → 12160 KiB (+64, resident
code), PSS above the floor 2679 → 2744 KiB (+65, +2.4%, inside the 5%
margin; it joins the idle-memory class already waived for v1 — reported,
not waived), wakeups 0/60 s on all 6 runs both sides, 1 thread. With a
`--every 1m` slideshow (2 runs, 130 s window covering 2 steps, loads
13.4/13.4): 14 and 6 wakeups (3–7 per step: the timeout plus one decode),
1 thread throughout, cycling a→b→a as `shows` confirms.
Docs: `site/src/content/docs/scootbg/cli.md` (commands, a Slideshows
section, the `rotation` key).
