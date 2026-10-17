---
title: "look apply refuses the whole look when the installed foot is older than initial-color-theme"
status: "open"
area: "core"
priority: "high"
blocked: null
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

## Not in this ticket

The musl/Alpine installer refusal and the webtop tag docs (separate tickets from the same test).
Whether the bar/launcher fragments have similar version floors — check while here, but file separately.
