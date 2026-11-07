---
title: "look auto --follow-portal: follow the desktop's light/dark setting"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
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

## Landed (2026-11-07)

`scoot look auto --light A --dark B --follow-portal`: a path unit
(`scoot-look-auto.path`, `PathChanged=` on `<config>/dconf/user`) starting
the same oneshot service the schedule uses, which now takes either trigger
(`crates/scootctl/src/look/auto/`, the decision in `portal.rs`).

- **Loop.** The run compares the setting's polarity with the active look's
  (resolved through `extends`, not by name) and applies only on
  disagreement, re-reading the setting after a swap because systemd drops
  its watches while the service runs. Test: `portal_tests.rs`, the real
  toolkit step against a scripted dconf, every write treated as a re-fire;
  one external flip is one swap.
- **A D11 change the loop test exposed.** `apply_one_key` left `color-scheme`
  alone whenever it differed from what the tool last wrote, so after an
  external flip a later manual apply of the other polarity did not write
  it, and the next unrelated dconf write reverted the apply. A key whose
  live value equals the look's wanted value, with a record, is now owned
  again with no write (`toolkit.rs`).
- **systemd 261.2 (`path.c`, and measured on a user manager).** A rename
  over the watched file fires once per rename. A missing directory is
  watched at the nearest existing ancestor and fires when the file appears
  (the `PathChanged` existence check in `path_spec_check_good`);
  `MakeDirectory=` is never set, since it would create the database path
  as a directory. The default service start limit (5 per 10 s) put the path
  unit into `failed` after five spaced writes, so the portal service sets
  `StartLimitIntervalSec=0`.
- **Not done.** The unit does not run at manager start (`PathChanged` has no
  initial fire); `DCONF_PROFILE` other than the default is not followed;
  settings kept outside dconf are not read.
