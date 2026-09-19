---
title: "Flake: the spawn close-on-exec pin asserted fd numbers, not fd identity — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Flake: the spawn close-on-exec pin asserted fd numbers, not fd identity — DONE

## The entry as filed

CI run 35460052576 went red on a **docs-only** PR. The failing step was
`cargo test --workspace` — not `cargo nextest run --workspace`, which was
green in the same run — and the failing test was
`compositor::activation::tests::spawn::a_spawned_child_inherits_no_close_on_exec_fd`
(`crates/scoot/src/compositor/activation/tests/spawn.rs:587` as it then
stood):

```
a State::spawn child inherited a close-on-exec compositor fd: [0, 1, 142, 145, 16, 2, 3, 4, 5]
```

Earlier runs of the same code were green, so the failure was
nondeterministic, and no production code was anywhere near the diff.

## The mechanism: a test bug, wrong in both directions

The test held marker fds across a real `State::spawn`, had the child list
`/proc/self/fd`, and asserted on the markers' **raw fd numbers**. A number
names no particular open file description, so the assertion could go wrong
either way:

- **A number the marker vacated gets re-used, and reads as a leak.** This is
  what CI hit. `execve` closes the close-on-exec markers, freeing their
  numbers; the child's own fds (the shell's redirect, the listing process's
  directory fd) are then allocated the *lowest free* numbers — exactly the
  ones just vacated. A marker at fd 3 comes back as the child's own fd 3.
- **A number another fd occupies gets inherited, and reads as a leak.** Under
  `cargo test` every test shares one process, and neighbours open and close
  fds throughout (`gamma_control/tests.rs` opens a deliberately plain
  `libc::pipe`, for one). Anything non-close-on-exec in the process is in the
  child's table, on whatever number it holds.
- **And, less visibly, `/proc/self` was the wrong table to read.** `self`
  resolves against whichever process opens the path, so `ls /proc/self/fd`
  reported `ls`'s own table, not the shell's — near enough by inheritance to
  look right, but it is the listing process's directory fd that shows up as
  the low-numbered "leak".

nextest, which runs each test in its own process, structurally cannot see
either of the first two. `cargo test` runs them in one process and can — the
asymmetry `CLAUDE.md`'s "Verification and evidence" section describes, and
exactly what CI keeps a `cargo test` step for.

## Resolution: identify fds, don't count them

Test-only (`activation/tests/spawn.rs`); no production code changed. The
claim is unchanged and nothing is weakened — `State::spawn` must leak no
close-on-exec compositor fd, the plain marker must still be **present** as
the positive control, and every marker shape from the
[close-on-exec audit](./spawn-fd-cloexec-audit-done.md) is still held across
the spawn, still premise-checked with `F_GETFD` and still liveness-checked
after the listing. What changed is how each marker is *named*:

| marker | was | is |
|---|---|---|
| file (close-on-exec, and the plain positive control) | `/dev/null`, by fd number | a fresh file with a path nothing else in the process opens, by the canonical path the child's `readlink` reports (removed on `Drop`) |
| socket pair ends, and the `try_clone` | by fd number | by the `socket:[<inode>]` in the target, inode from `fstat` on this side |
| eventfd | by fd number | by a distinctive starting count, read back from the `eventfd-count:` line of the child's `fdinfo` dump |

An open fd's path and inode are exclusively its own for as long as it stays
open, which is what the liveness check at the end of the test guarantees, so
none of these can be confused with a neighbour's fd or with the child's own.

Two supporting changes the above needs:

- The child reads `/proc/$$/fd` — the shell's own pid, which is the process
  that did the inheriting — and reports `fd <number> <target>` per fd plus a
  `grep -H ''` dump of `fdinfo`.
- It writes that listing under a temporary name and `mv`s it into place.
  `read_probe` returns on the first **non-empty** read, and the listing is no
  longer one small write, so without the atomic rename the test would read a
  half-written listing and the positive control would flake — a new flake in
  place of the old one.

The probe needed a shell and coreutils before this change and still does;
which coreutils changed (`ls` before, `readlink`/`grep`/`mv` now).

### The eventfd, and the check that was deliberately *not* used

`readlink` reports every eventfd as `anon_inode:[eventfd]`, naming no
instance, so "is *this* eventfd in the child" has no target to match. The
obvious substitute — assert the child's table holds **no** eventfd at all —
is strictly stronger than needed and was rejected anyway: one foreign
non-close-on-exec eventfd in the shared `cargo test` process (a graphics
driver under the GLES suites is a plausible source, and not one this test
can rule out) would turn this back into exactly the interference flake being
fixed here, pointing at `State::spawn` again.

So the marker is created with a distinctive starting count
(`0xEF00_0000 | pid`), which `/proc/<pid>/fdinfo/<n>` reports and no other
eventfd in the process carries (calloop's pings start at 0 and are read back
to 0 after every wake). The count is parsed out of the line rather than
string-matched, so the kernel's `%16llx` padding changing cannot silently
turn the check into a no-op, and a parent-side premise assertion fails loud
if the field ever stops reporting the value — the same "fail loud, not weak"
shape the `F_GETFD` premise checks already had.

## Evidence

Captured on the dev VM (aarch64 NixOS, kernel 6.18.50, rustc 1.97.1,
`CARGO_TARGET_DIR=$HOME/fdfix-target`, `CARGO_INCREMENTAL=0`), against the
tree at `17a6bd7` plus uncommitted scratch instrumentation where the
experiment needed it. Exact commands and raw output are in the PR.

**Fail-first, deterministic** (not raced for): the child parks an unrelated
`/dev/null` on the number the close-on-exec marker holds in the parent —
`eval "exec ${2}</dev/null"` — which is the CI failure's own mechanism made
repeatable. The pre-fix assertion goes red every time:

```
the close-on-exec marker holds fd 12 in the parent
the child's listing: [0, 1, 12, 2]
a State::spawn child inherited a close-on-exec compositor fd: [0, 1, 12, 2]
```

The same parking under the identity checks is green, and the listing shows
why — fd 12 is a `/dev/null` that is *not* the marker, and the positive
control is present under its own name at fd 13:

```
fd 0 pipe:[245509]
fd 1 /tmp/scoot-spawn-fds-64485-0.tmp
fd 10 pipe:[248011]
fd 12 /dev/null
fd 13 /tmp/scoot-spawn-fd-marker-64485-1
fd 2 pipe:[248011]
fd 3 pipe:[248027]
```

**Sensitivity, re-proven per marker** (the audit proved the old shape's
markers the same way; the identification mechanism changed, so this is
re-derived, not inherited): clearing close-on-exec on one marker after its
premise check makes exactly that marker's identity check go red — file
marker by path, both socket ends and the clone by inode, the eventfd by
count.

## Residuals

- The narrow gap the audit stated is unchanged: fd shapes with no
  per-instance identity *and* no std constructor (epoll, timerfd, the udev
  netlink socket) still have no marker here. They come only from C libraries
  whose flags the audit measured live.
- No README change: test-only, no user-facing surface.
