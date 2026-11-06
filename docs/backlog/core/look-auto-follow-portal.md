---
title: "look auto --follow-portal: follow the desktop's light/dark setting"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# look auto --follow-portal: follow the desktop's light/dark setting

Filed 2026-11-06 from [`look-auto-switch`](look-auto-switch.md), which
landed `--schedule` only. Serves **daily-drive**, opt-in.

## The gap

`scoot look auto --light A --dark B --follow-portal` should swap when
something else flips `org.freedesktop.appearance color-scheme` (a settings
app, a phone-style quick toggle). The original ticket proposed a systemd
path unit. The setting lives in dconf, which is one binary database
(`~/.config/dconf/user`), so a path unit on it fires on every dconf write
by any program, not only a `color-scheme` change.

## What to do

1. A path unit (`PathChanged=` on the dconf database) starting a oneshot
   that reads `color-scheme` and applies the matching look. It must be
   cheap and idempotent: read the key, compare with the look already
   active, do nothing when they agree.
2. **The loop to pin.** `scoot look apply` writes `color-scheme` itself in
   the toolkit step (`crates/scootctl/src/look/toolkit.rs`). That write
   changes the database, which re-fires the path unit. The oneshot must end
   the cycle: it applies only when the setting's polarity disagrees with
   the active look's, and a swap whose polarity already matches writes the
   same value, so the second run sees agreement and stops. Test it with a
   recorded dconf and a counter: one external flip causes exactly one
   swap.
3. `PathChanged` fires on atomic replace of the file, which dconf does;
   check the pinned systemd's behaviour for a rename over the watched
   path, and for a database in a directory that does not exist yet.
4. Keep the own-if-marked unit rule, `--off` removing everything, and the
   refusal at `auto` time for a missing look. Share the unit code with
   `--schedule` (`crates/scootctl/src/look/auto/`).

## Not in this ticket

`--sun` ([`look-auto-sun`](look-auto-sun.md)).
