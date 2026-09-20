---
title: "`smoke-test.sh` defaults to a shared target dir, which has now handed three agents someone else's binary — RESOLVED: invoking-tree defaults, same-build pairing, loud failure, header identity"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `smoke-test.sh` defaults to a shared target dir — RESOLVED

~~`scripts/smoke-test.sh:33`:~~

```sh
SCOOT=${SCOOT:-/var/cargo-target/debug/scoot}
```

~~`/var/cargo-target` is the dev VM's *shared* target directory. When two
agents work concurrently — which is now normal — whichever built last owns
that path, so the script silently tests a binary from a different branch.~~

~~**This happened three times in one session, to three different agents**,
and each caught it a different way: one by a live check showing one output
where two were expected, one by `strings … | grep -c "ignoring a host
configure"` returning 0, one by a reviewer's smoke run passing against the
wrong tree. Two of those were near-misses that would have produced a
confident wrong verdict — a green smoke run against code you did not build
is worse than a red one.~~

~~The failure is silent by construction: the path exists, the binary runs, and
nothing in the output says which tree it came from.~~ — **RESOLVED
2026-09-20**. All three of the ticket's options landed together, extended to
the `scootctl` split (which the ticket predates): no default names an
absolute machine-specific path, both binaries' path + source + mtime print
in the run's first lines, and any unresolvable binary fails loudly before
anything launches.

## Scheme and precedence

- Explicit `SCOOT` / `SCOOTCTL` always win — existing callers and CI (which
  set both) see zero change. `SMOKE_PREFIX` behavior is untouched.
- `SCOOT` unset → `$CARGO_TARGET_DIR/debug/scoot` when `CARGO_TARGET_DIR`
  is set (the builder's real output dir — export a private one and the
  default follows your build), else
  `<this-script's-repo>/target/debug/scoot`, resolved from the script's own
  location (`BASH_SOURCE`), not the cwd, so the script runs from anywhere.
- `SCOOTCTL` unset → the `scootctl` next to `SCOOT`, always — **the
  same-build pairing is kept deliberately, not split**: a compositor from
  one tree driven by a client from another is the same wrong-verdict class
  this default exists to prevent, so there is no `SCOOTCTL` default
  independent of `SCOOT`. (This answers the post-split question the ticket
  predates: deriving from `SCOOT`'s dir stays, for both explicit and
  defaulted `SCOOT`.)
- Either binary missing, not a regular file, or not executable → `error:`
  + `exit 1` before any socket, log or compositor exists. The old
  mid-script `SCOOTCTL` check is removed as subsumed (it could only fire if
  the binary vanished mid-run).
- The run's first lines print `SCOOT=` / `SCOOTCTL=` each with its source
  (`environment`, `default from CARGO_TARGET_DIR`, `default from the
  invoking tree`, `default next to SCOOT (same build)`) plus `ls -l` (mtime
  and size).

## The ticket's first preference was a no-op on the dev VM — verified, not assumed

`$CARGO_TARGET_DIR/debug/scoot` taken literally resolves to the *same*
shared path on the dev VM, because `CARGO_TARGET_DIR=/var/cargo-target` is
set globally there for every checkout (confirmed live:
`CARGO_TARGET_DIR=/var/cargo-target`). So the invoking-tree default alone
changes nothing where the three incidents happened; the weight on that
machine is carried by the other two halves — the header (a wrong binary is
now visible in the log instead of inferred later) and the loud failure (a
`CARGO_TARGET_DIR` pointing at a tree without built binaries fails naming
the path, with no fallthrough to any shared location). Concurrent agents
there should still prefer an explicit `SCOOT` (or a private
`CARGO_TARGET_DIR` exported for both build and run, which the default then
follows). The script header says exactly this.

Related, found while verifying: with `CARGO_TARGET_DIR` unset the dev VM
default would have picked up `/mnt/scoot/target/debug/scoot` — stale
2.0 MB pre-rename artifacts (Sep 20 12:02 / 03:26, vs the real 137 MB
binary), not buildable or removable over the 9p mount by the dev user.
Another silent-wrong-binary shape the header now exposes rather than fixes.

## Edge cases, decided

- **`CARGO_TARGET_DIR` set, no binaries there**: loud fail naming the
  resolved path (`default from CARGO_TARGET_DIR`), exit 1 — proven live
  (T1 below). No fallthrough to the shared path.
- **Neither env var set, no tree-local binary**: loud fail naming the
  script-relative path (`default from the invoking tree`) — proven live
  from a different cwd against a copied tree (T5), which also proves the
  resolution is script-location, not cwd.
- **Explicit `SCOOT` / `SCOOTCTL` missing**: loud fail naming the value and
  its source — an improvement over the old shape, where a missing explicit
  `SCOOT` surfaced only as "the control socket never appeared" (T2/T3).
- **Bare name on PATH** (`SCOOT=scoot`): pinned to `command -v`'s full path
  *before* pairing/checks/header, so the `SCOOTCTL` pairing, the `-f`/`-x`
  checks and `ls -l` all see one file. `dirname` of a bare name would be
  `.` (the cwd — the wrong tree by construction); resolving first keeps
  the same-build invariant on PATH too. Not on PATH at all → loud fail
  (T4). A `command -v` hit without a slash (relative `PATH` entry) is
  anchored at `$PWD` (the script never cds, so this is exact).
- **Not a regular file** (e.g. a trailing-slash directory, which passes
  `-x` via the search bit): the check is `-f && -x`, so it fails loudly
  with the same message.
- **Empty-string `SCOOT`/`SCOOTCTL`**: treated as unset (the old `:-`
  semantics, preserved exactly).
- **`set -u` / `set -e`**: all new expansions use `${VAR:-}` guards; the
  `command -v` lookup carries `|| true` with an explicit emptiness test;
  the `BASH_SOURCE` resolution runs only in the branch that needs it, so
  the common path cannot be broken by an odd invocation context.
- **Spaces in paths**: every new expansion quoted; proven end to end with
  `SMOKE_PREFIX="/tmp/smoke with spaces/b"` (run B).

## Evidence (dev VM, `ssh -p 2222 dev@localhost`, NixOS aarch64)

Script-only change; binaries prebuilt 2026-09-20 12:15, untouched by the
diff (rebuilt later only by the Rust verification set below — log excerpts
are as captured). Evidence captured against the uncommitted working tree on
top of `fda38d7` (branch `backlog/smoke-test-binary-default`).

- **Shell hygiene**: `bash -n` clean (Mac-side). shellcheck is installed on
  neither side, so `bash -n` is the gate, as in the temp-prefix record.
- **Negatives, all exit 1 with an `error:` naming path + source**:
  T1 `CARGO_TARGET_DIR=/tmp/empty-target` → `no compositor binary at
  SCOOT=/tmp/empty-target/debug/scoot (default from CARGO_TARGET_DIR)`;
  T2 `SCOOT=/nonexistent/scoot` → `... (environment)`; T3 explicit `SCOOT`
  ok + `SCOOTCTL=/nonexistent/scootctl` → `no client binary ...`;
  T4 `SCOOT=definitely-not-a-binary` → `names no file and is not on
  PATH`; T5 `env -u CARGO_TARGET_DIR` from `/tmp` against a copied tree
  with no binaries → `no compositor binary at
  SCOOT=/tmp/faketree/scripts/../target/debug/scoot (default from the
  invoking tree)`.
- **Run A, default invocation, zero env** (`/tmp/smoke-a.log`): exit 0,
  18 `^ok:`, 0 `BUG`. Header shows
  `SCOOT=/var/cargo-target/debug/scoot (default from CARGO_TARGET_DIR)`
  + `ls -l` (`137293216 Sep 20 12:15`) and the `scootctl` next to it.
- **Run B, bare name on PATH + foreign cwd + spaced prefix**
  (`"/tmp/smoke with spaces/b-run.log"`, `PATH=/tmp/fakebin:$PATH
  SCOOT=scoot`, cwd `/tmp`): exit 0, 18 `^ok:`, 0 `BUG`. Header shows
  `SCOOT=/tmp/fakebin/scoot (environment (on PATH at /tmp/fakebin/scoot))`
  and `SCOOTCTL=/tmp/fakebin/scootctl (default next to SCOOT (same
  build))`; spaced-prefix artifacts all land under `/tmp/smoke with
  spaces/`.
- **Runs C+D, concurrent pair proving disjoint binaries**
  (`/tmp/smoke-c-run.log`, `/tmp/smoke-d-run.log`): both exit 0, 18
  `^ok:` each, 0 `BUG`. C (default env, `/tmp/smoke-c`) tests the shared
  binary (Sep 20 12:15); D (`env -u CARGO_TARGET_DIR` + copied tree,
  `/tmp/smoke-d`) tests byte-identical copies under a different path with
  a different mtime (Sep 20 12:23) — the two headers differ in both path
  and mtime, so neither run could have tested the other's binary.
- **Harness mishap, recorded**: the first C/D attempt pointed the outer
  `> log` redirect at the same path the prefix derives `LOG` from, so the
  backgrounded compositor truncated the script's own log (exit codes
  survived; headers/counts did not). Re-ran with distinct outer paths.
  Outer capture logs must never equal `$SMOKE_PREFIX.log`.
- **Standard set** (no Rust touched — `git diff --stat` shows only
  `scripts/smoke-test.sh` plus docs): `cargo fmt --check -p scoot` clean,
  `cargo clippy -p scoot --all-targets -- -D warnings` clean,
  `cargo nextest run --workspace` 1156 passed / 4 skipped (dev VM).
  Benchmark: n/a — the change is a script preamble, no hot path.

## Deliberately not done

- **Same-prefix collision, mktemp leaks, shared pgrep visibility**: out of
  scope per the ticket's own record (the earlier smoke-prefix record's
  known-left items) — unchanged by this diff.
- **README**: no change. The README documents no smoke-test env contract
  (verified by grep — only the `scripts/` listing line and the CI
  paragraph), so there is nothing to update; the new binary contract lives
  in the script header, where `SMOKE_PREFIX` already lived.
- **`vm/README.md` troubleshooting**: one pointer added — the shared-target
  entry now notes the script fails loudly on a missing default and prints
  binary identity in its header, i.e. what to read instead of `ls`-ing the
  binary by hand.
