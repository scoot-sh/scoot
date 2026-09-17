---
title: "`scripts/smoke-test.sh` hardcodes some of its temp paths, so two concurrent runs can collide — RESOLVED: every temp path derives from `$SMOKE_PREFIX`"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `scripts/smoke-test.sh` hardcoded temp paths — RESOLVED

~~`scripts/smoke-test.sh` hardcodes some of its temp paths, so two
concurrent runs (e.g. two agents verifying different branches on the same
VM at once) can collide (LOW, pre-existing). The `SOCKET`/`LOG`
environment overrides the script honors don't cover every path it uses —
`/tmp/flexwm-smoke-config.log`/`-config.sock` and `-broken.*` are
hardcoded regardless of what `SOCKET`/`LOG` are set to.~~ — **RESOLVED
2026-09-17**. Every temp path in the script now derives from one
overridable prefix, `$SMOKE_PREFIX`; with it unset every default is
byte-identical to the historical hardcoded path.

## Path audit (whole script, not just the cited lines)

Found 12 hardcoded temp paths / templates, fixed all 12, deliberately
left 0:

| # | Old hardcoded path | Now | Notes |
| - | ------------------ | --- | ----- |
| 1 | `/run/user/$(id -u)/flexwm-smoke.sock` (`SOCKET` default) | `${SOCKET:-"$SMOKE_PREFIX.sock"}` when prefixed, legacy default otherwise | Explicit `SOCKET` still wins over the prefix |
| 2 | `/tmp/flexwm-smoke.png` (`SHOT` default) | prefix-derived / legacy | Explicit `SHOT` still wins |
| 3 | `/tmp/flexwm-smoke.log` (`LOG` default) | prefix-derived / legacy | Explicit `LOG` still wins |
| 4 | `/tmp/flexwm-smoke-appearance.toml` (`CONFIG` default) | prefix-derived / legacy | Explicit `CONFIG` still wins |
| 5 | `/tmp/flexwm-smoke-typed.txt` (`TYPED` default) | via new `TYPED_DEFAULT`, prefix-derived / legacy | Explicit `TYPED` still wins |
| 6 | `/tmp/flexwm-smoke-killed.png` (2 sites: `--out`, `rm -f`) | via new `KILLED`, prefix-derived / legacy | Had no env override before; now follows the prefix |
| 7–8 | `-config.sock` / `-config.log` in `run_config_bind_test` | via `CONFIG_SOCK` / `CONFIG_LOG` | Were fully hardcoded, ignored `SOCKET`/`LOG` |
| 9 | `mktemp /tmp/flexwm-smoke-config-XXXXXX.toml` | via `CONFIG_TEMPLATE` | `mktemp` names are random per run, but the template dir now follows the prefix |
| 10–11 | `-capital.sock` / `-capital.log` in `run_capital_bind_test` | via `CAPITAL_SOCK` / `CAPITAL_LOG` | Same as 7–8 |
| 12 | `mktemp /tmp/flexwm-smoke-capital-XXXXXX.toml` | via `CAPITAL_TEMPLATE` | Same as 9 |
| 13–15 | `-broken.sock` / `-broken.log` / `mktemp ...-broken-*.toml` in `run_broken_config_test` | via `BROKEN_SOCK` / `BROKEN_LOG` / `BROKEN_TEMPLATE` | Same as 7–9 |

(15 derived values from 12 previously-hardcoded paths — the ticket cited
2, the audit found the `-capital.*` triple too, which the ticket's line
range predates.)

Already-derived, unchanged: `$SHOT.check1` / `$SHOT.check2` (derive from
`$SHOT`), so they follow any `SHOT` or prefix automatically.

## Scheme and precedence

`SMOKE_PREFIX` unset → every default is exactly the historical path (new
`else` branch holds the old literals verbatim; verified by expanding the
block and diffing against `git show main:scripts/smoke-test.sh` — all 15
values identical modulo `$(id -u)`). `SMOKE_PREFIX` set →
`mkdir -p "$(dirname "$SMOKE_PREFIX")"` then every path is
`$SMOKE_PREFIX<suffix>`, unless the matching per-path variable
(`SOCKET`/`SHOT`/`LOG`/`CONFIG`/`TYPED`) is explicitly set, which wins for
that one path only. So existing callers setting `SOCKET`/`LOG`/`MODE`/
`FLEXWM` see zero change, and two concurrent runs with different prefixes
share nothing. The sub-test sockets/logs/templates are intentionally *not*
separately overridable — one prefix is the whole contract, not six new env
vars. The script header documents the scheme, the precedence, and the
same-prefix-still-collides caveat.

## The one behavior change beside paths

`TYPED` is embedded in a command line *typed into* the terminal
(`printf ... > $TYPED`), where script-side quoting doesn't apply — a
prefix with spaces would have broken that redirect. It is now embedded
quoted (`> "$TYPED"`), which is a no-op for the space-free default and
makes spaced prefixes work end to end. Every other expansion was already
quoted; the new variables are quoted at all 20+ use sites.

## Edge cases, decided and documented in the header

- **Trailing slash** (`SMOKE_PREFIX=/tmp/x/`): stripped; files become
  `/tmp/x.sock` etc., *not* files under `/tmp/x/`. A prefix is a filename
  prefix, not a directory. Proven: expansion gives
  `SOCKET=/tmp/edge-case-dir.sock`.
- **Spaces** (`SMOKE_PREFIX="/tmp/smoke with spaces/run"`): works —
  `mkdir -p`, all expansions, `mktemp`, and the typed redirect all proven
  with a spaced prefix.
- **Nonexistent parent dir**: created (`mkdir -p` on the dirname); proven
  with a two-deep nonexistent path. An explicitly-overridden `SOCKET` in a
  nonexistent dir still fails as before — pre-existing behavior, unchanged.
- **Same prefix twice**: still collides, by construction. Documented, not
  fixed — prefixes must differ.
- **`set -e` / `set -u`**: the new block introduces no unguarded command
  whose failure could abort the script (`mkdir -p` failing means the run
  can't write anything anyway, so aborting there is correct); no new
  unbound-variable exposure (`SMOKE_PREFIX` defaults to empty before use).

## Evidence (dev VM, `ssh -p 2222 dev@localhost`, NixOS aarch64)

Script-only change; binary `/var/cargo-target/debug/flexwm` untouched
(prebuilt 2026-09-17). Evidence captured against the uncommitted working
tree on top of `1bdf45f` (branch `fix/smoke-test-temp-prefix`).

- **No regression, default invocation** (no env vars):
  `bash scripts/smoke-test.sh > /tmp/smoke-default.log 2>&1` → `EXIT=0`,
  17 `^ok:` lines, 0 `BUG`. `/tmp/flexwm-smoke.log`, `-appearance.toml`,
  `.png` all written at the historical paths.
- **Concurrency, two prefixed runs at once**:
  `SMOKE_PREFIX=/tmp/smoke-a … & SMOKE_PREFIX=/tmp/smoke-b … & wait` →
  both reached the final `ok:` line; each log 17 `^ok:`, 0 `BUG`
  (`/tmp/smoke-a-run.log`, `/tmp/smoke-b-run.log` on the VM). File listing
  shows two fully disjoint sets (`/tmp/smoke-a{,.sock,.log,.png,…}` vs
  `/tmp/smoke-b{…}`) — no shared path between them.
- **Same-prefix collision control**: not run live (it would just re-prove
  that two writers share files); the collision shape is structural — with
  one prefix both runs compute identical paths — and is stated in the
  header rather than demonstrated.
- **Shell hygiene**: `bash -n` clean. shellcheck is installed on neither
  side (Mac: `command -v shellcheck` empty; VM: `NOSHELLCHECK`), so
  `bash -n` is the gate, plus the quoted-expansion audit above.
- **Standard set** (no Rust touched — freshness checks): `cargo test -p
  flexwm` ok (3 passed per binary incl. doctests tail), `cargo nextest run
  --workspace` 913 passed / 1 skipped, `cargo clippy -p flexwm
  --all-targets -- -D warnings` clean, `cargo fmt --check -p flexwm` clean
  (VM) and Mac-side `cargo fmt --check -p flexwm` exit 0.

## Deliberately not done

- **The `mktemp` cfgs are never deleted** (pre-existing leak — the VM had
  dozens of `/tmp/flexwm-smoke-broken-*.toml` before this change). Left as
  is: on failure the cfg is debugging evidence, and adding lifecycle
  handling is scope beyond the ticket. Flagged here so a future item can
  take it.
- **README**: no change. The README documents no smoke-test env contract
  (only the `scripts/` listing line) — verified by grep — so there is
  nothing to update; the new `SMOKE_PREFIX` contract lives in the script
  header, which is where `SOCKET`/`LOG`/`SHOT` were ever documented.
- **The rename**: untouched, still last per its own entry.
