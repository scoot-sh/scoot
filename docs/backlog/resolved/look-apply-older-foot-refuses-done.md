---
title: "look apply refuses the whole look when the installed foot is older than initial-color-theme"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-26"
---

# look apply refuses the whole look when the installed foot is older than initial-color-theme

Filed 2026-10-17 from the webtop field test (T-webtop). Serves **daily-drive**:
the documented `install.sh --look ginger-night` one-liner cannot succeed on Debian 13
(and by the same mechanism Debian 12 / Ubuntu 24.04, whose foot is older still) with foot installed.

## The gap

`scoot look apply` renders a foot fragment whose `[main]` section always sets
`initial-color-theme=dark`, then validates it with `foot --check-config`. foot 1.21.0
(Debian trixie, `apt` newest — no backports available) rejects the key:

```
scoot: foot fragment refused by foot --check-config: err: config.c:1099: /config/.cache/scoot/rendered/ginger-night-8c47985e7316d338/foot.ini:5: [main].initial-color-theme: dark: not a valid option: initial-color-theme
```

Reproduced for `ginger-night` AND `moonrise` (same `[main]` header, so all five shipped
looks fail identically) inside `lscr.io/linuxserver/webtop:debian-xfce` (Debian 13,
glibc 2.41, aarch64, Nerd Fonts installed so the font gate passes). Refusal is clean —
`look current` still says "no look applied yet", no user configs written — but the whole
look is lost because of one key for one app. The fragment is renderer-generated (no
`initial-color-theme` in the look catalog), so no look authoring can avoid it.

## What to do

Pick one (in order of preference):
1. Probe `foot --version` at render time and omit keys the installed foot does not know,
   with a note naming the key and the foot version that introduces it. Keeps fail-closed
   for genuinely broken fragments while old-but-working foots get the look.
2. If probing is too fragile, fall back to warn-and-skip the foot fragment (note, never error)
   when validation fails on an unknown-option error specifically — same "optional apps: notes,
   never errors" rule the docs already promise for absent apps.
3. At minimum, document the minimum foot version per look / in the install page, and have
   install.sh's missing-dependencies section flag an older foot the way it flags a missing font.

Pin the edge cases: a foot too old to even answer `--version`; a fragment that is broken beyond
one unknown key (must still refuse); `foot --check-config` itself missing (older foot? then skip
validation with a note, or refuse — decide and test both).

## Resolution (2026-10-27)

Option 1 (probe) with the refusal shape as fallback, plus option 3 (docs + installer flag).

Version archaeology against foot's own history: `initial-color-theme=1|2`
lands after 1.22 (first released in 1.23.0); the rename to `dark`/`light`
and the `[colors-dark]`/`[colors-light]` sections land after 1.25 (first
released in 1.26.0). So every foot before 1.26 refuses the themed
fragment — before 1.23 on the key, on 1.23–1.25 on the `dark`/`light`
values and the section names — and 1.28 removed the legacy `[colors]`
section, so one shape cannot serve both eras. Affected: foot < 1.26
(Debian 13's 1.21.0, Debian 12's and Ubuntu 24.04's older still).

Behavior: `foot --version` is probed at render time; an older foot
renders the legacy single-`[colors]` fragment (same hues, that era
parses) up front, with a note naming the key and the 1.26 floor. An
unparseable version renders modern and downgrades once off the
`--check-config` refusal instead. A fragment broken beyond the themed
keys still refuses the whole look (fail-closed); a foot without
`--check-config` itself leaves the legacy fragment unchecked with a
note. Never half-applied: the swap applies everything with a foot
fragment the installed foot parses, or refuses cleanly before anything
is recorded. `install.sh` flags an older foot the way it flags a
missing font (note, never a refusal); the install page and the look
docs name the 1.26 floor.

Evidence: failing test first
(`an_older_foot_downgrades_the_fragment_instead_of_refusing_the_look`
failed with the ticket's exact message, then passed); `nextest -p
scoot-look -p scootctl` 538 passed; `cargo test` likewise; `clippy -D
warnings` and `fmt --check` clean; `test-install.sh` all passed
including the new old-foot case; the legacy bytes validate under a
real foot 1.21.0 built from source on the dev VM, whose `--check-config`
refuses the themed bytes with the ticket's message. The one `cargo
test -p scoot` / `nextest -p scoot` failure
(`a_refused_managed_window_costs_the_server_nothing`, 273 fds vs the
264 cap) reproduces byte-identically on pristine `origin/main` —
pre-existing, owned by open PR #757.

## Not in this ticket

The musl/Alpine installer refusal and the webtop tag docs (separate tickets from the same test).
Whether the bar/launcher fragments have similar version floors — check while here, but file separately.
