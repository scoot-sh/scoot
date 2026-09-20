---
title: "Nothing says how to start a bar, a launcher or a browser at session start — and `--` takes exactly one command — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Nothing says how to start a bar, a launcher or a browser at session start — and `--` takes exactly one command — DONE

Asked by the user, 2026-09-19: *"How do we handle startup exec of other
processes like Waybar, fuzzel, browsers, etc? Do we make it easy for startup
desktop-env type things?"* Resolved 2026-09-20 with all three of the
ticket's shapes: (1) the startup model documented, (2) `[autostart]` built,
(3) the home-manager mapping left as a pointer for the flake ticket.

## What landed

**Shape 1 — "Starting a session" documented** (`docs/configuration.md`,
with a pointer from `README.md`'s Configuring section). A real
`session.sh` (wallpaper, bar, notification daemon, `exec`-ing a terminal),
the webtop `/defaults/startwm.sh` variant (`exec scoot --nested -- ...`,
the deployment the field reports came from), and the composition rule:
config declares the session baseline, the script carries the behavior.

**Shape 2 — `[autostart]` built**, per the coordinator decision (three of
four named peers put startup commands in the config, so that is where
users look; cost is small). Schema and semantics as decided, no redesign
needed once in the code:

```toml
[autostart]
commands = ["spawn waybar", ...]
```

- Full action grammar through the shared parser — `scootctl::action`, the
  same function `[binds]` values parse through (the ticket cited it as
  `cli::action`; post-split it lives in the `scootctl` crate, and
  `config::parse_bind` was already reusing it there, so `parse_autostart`
  follows that precedent exactly, including the trailing-text check).
- No spawn-only restriction; a non-spawn action at startup is the user's
  choice, documented as such. Entries run through `State::act`, so they get
  the same path (and the same session-lock gate, which passes by
  construction — a fresh session starts unlocked) as a keybind or IPC
  request.
- Flat spawn list: no ordering, no conditionals, no supervision. The
  ticket's exclusion stands, and the docs say where the fancier half lives
  (the session script) and where the missing half lives (a service
  manager; on webtop, the container).
- Ordering with `--`: `[autostart]` entries run FIRST, in file order, then
  the `--` command. The spawn site is `compositor::run` right where the
  ticket said it would be — after `ipc::init`, the `set_var` block and the
  reaper install — so autostart children inherit `WAYLAND_DISPLAY`,
  `SCOOT_SOCKET`, the session environment and a fresh activation token,
  and are reaped, exactly like `--` and `spawn` binds.
- The double-spawn footgun is documented, not prevented: config autostart
  plus a script spawning the same bar yields two bars — the same class as
  two `spawn` binds, the user's composition to fix.

**Shape 3 — home-manager stays a pointer.** One sentence in "Starting a
session" mapping `programs.scoot.settings` / a session script onto the
state/behavior split, owned by
[`flake-consumer-and-home-manager.md`](../packaging/flake-consumer-and-home-manager.md).

## The load-bearing property: fail-open per entry

A malformed autostart entry that killed a `--tty` session — losing every
client — is the user-facing harm this change could have introduced, so
fail-open is designed in, not defaulted into:

- A bad *entry* (unknown action, missing argument, trailing text, empty
  string) is skipped with a warning naming just that entry; every other
  entry still runs, the rest of the file still applies, and the session
  always starts. Same rule `[binds]` follows, pinned by
  `an_invalid_autostart_entry_is_skipped_and_the_session_still_starts`.
- A mistyped `commands` *field* (`commands = "spawn waybar"`) is a
  whole-file parse error — full defaults, never a refusal — the same as
  any other mistyped field, pinned alongside the entry-level test so the
  two levels don't get conflated later.

Fail-first framing, honestly stated: pre-fix there was no per-entry parse
at all — an `[autostart]` table was an unknown table, so
`deny_unknown_fields` discarded the *whole file* (binds included; the
existing `an_unknown_field_falls_back_to_full_defaults_not_an_error` pins
that semantics). Post-fix the file applies and only the bad entry drops.
Sensitivity was proven the other way too: with `parse_autostart` neutered
to always fail, 5 of the 9 new tests fail (the 4 that pass are the
empty/missing/wrong-type/unknown-key ones, which correctly don't depend on
successful parsing).

## Evidence (dev VM, `ssh -p 2222 dev@localhost`, branch `backlog/startup-autostart`)

Unit (9 new tests in `compositor::config::tests` — round-trip in file
order, missing table, empty list, fail-open pin, trailing-text, non-spawn
acceptance, wrong-type field, unknown-key rejection, 100-entry list):

```sh
cd /mnt/scoot && cargo test -p scoot --bin scoot autostart
# 9 passed; 0 failed (1048 filtered out)
```

Full verification set, same tree:

```sh
cd /mnt/scoot && cargo nextest run --workspace
# Summary [31.567s] 1171 tests run: 1171 passed, 4 skipped
cd /mnt/scoot && cargo clippy -p scoot --all-targets -- -D warnings  # clean
cd /mnt/scoot && cargo fmt --check -p scoot                          # clean
cd /mnt/scoot && SMOKE_PREFIX=/tmp/smoke-autostart scripts/smoke-test.sh
# exit 0, 19 `ok:` lines, no failures
```

Live proof 1 — autostart spawns, session answers, instant-exit reaped
(config: two `spawn touch` entries plus `spawn true`; `--headless`,
explicit `--socket`/`--config`):

```sh
setsid /var/cargo-target/debug/scoot --headless --socket /tmp/sa.sock \
    --config /tmp/sa-config.toml > /tmp/sa.log 2>&1 < /dev/null &
SCOOT_SOCKET=/tmp/sa.sock /var/cargo-target/debug/scootctl windows
# {"type": "windows", "windows": []} — session up
# /tmp/sa-first and /tmp/sa-second both exist (touched by the session)
# `ps --ppid $SCOOT_PID` empty, zero `defunct` system-wide — `true` reaped
# /tmp/sa.log: three `spawned` lines in file order, then `scoot is up`
```

Live proof 2 — fail-open (config: `"not-a-real-action"` then `spawn touch
/tmp/sb-good`):

```sh
# /tmp/sb-good exists; `scootctl windows` answers; /tmp/sb.log carries:
# WARN scoot::compositor::config: skipping an invalid [autostart] entry
#   command=not-a-real-action
#   reason=unknown argument `not-a-real-action` (try --help)
```

Live proof 3 — ordering (config autostart `spawn touch /tmp/sc-auto`,
`-- touch /tmp/sc-dash`): both files exist, session up, and `/tmp/sc.log`
shows the autostart spawn (`...35.987344Z ... ["touch", "/tmp/sc-auto"]`)
before the `--` spawn (`...35.987739Z ... ["touch", "/tmp/sc-dash"]`).

Bug-bash edges covered: empty list (unit), malformed entry (unit + live),
instant-exit entry (live, no Z), `--` + autostart together (live, both run
in order), autostart with no `--` (live proof 1), 100-entry list (unit —
parse cost only, paid once at startup on a cold path, so no bound).
Benchmark: n/a for the same reason — nothing here runs on a hot path
(`into_actions` runs once at startup; `act` per entry is the same call a
keybind makes).

## Left out, with why

- **Supervision/restart** — the ticket's own exclusion; stands.
- **Home-manager module** — pointer only; the flake ticket owns it.
- **`--print-default-config`** — separate ticket; but the README rule's
  in-ticket half is done (`[autostart]` is in `docs/configuration.md`'s
  table list, field docs, and example config, and `README.md` points at
  "Starting a session").
- **Backends beyond headless** — the spawn site in `compositor::run` is
  shared by headless/nested/tty (verified by reading, not by running each:
  the code path is backend-independent, and `--nested`/`--tty` differ only
  earlier in `run`). Live proof is `--headless`; a `--nested`/`--tty`
  re-run would exercise the same lines.
