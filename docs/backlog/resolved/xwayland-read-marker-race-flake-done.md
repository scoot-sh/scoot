---
title: "Flake: xwayland test `read_marker` polls a file that exists-but-empty between redirect setup and echo — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Flake: `read_marker` reads empty between redirect setup and echo — DONE

## The entry as filed

> Filed 2026-09-26 from the PR #257 review (independent re-derivation, not
> the implementer's report): `read_marker` (`tests/mod.rs:81`, Phase-1-era,
> untouched by that diff) polls `read_to_string`, but `sh -c "echo … >
> marker"` creates the file at redirect setup *before* echo writes — a poll
> landing in that gap reads `""` and the `:9` assertion fails. Passed alone
> and on full re-run; same load-sensitive class plausibly explains #257's
> reported one-window dnd full-run flake (at n=1 the cap is one `HashMap::get`,
> off the dnd timing path).
>
> Fix shape (suggestion, not prescription): wait-for-nonempty or
> write-then-rename in the helper. Pinned by a stress loop, not a single
> green run.

## Resolution

Test-only fix in `crates/scoot/src/compositor/xwayland/tests/mod.rs`
(`git diff --stat`: one test file, +16/−2; no production code touched, no
README/protocols/CHANGELOG surface — states explicitly: nothing user-facing
changes). Chose the ticket's wait-for-nonempty shape over write-then-rename:
it stays inside the helper, so neither spawn command string changes, and the
emptiness check is on the raw text, not the trim — every spawn through this
helper echoes at least a newline, so a zero-byte read can only be the
redirect-setup gap, never a legitimate answer. A marker still empty at the
10 s deadline panics loudly (`the spawned child left its marker empty`)
rather than hanging, matching the existing never-wrote arm.

The mechanism was verified by reading, not assumed: both spawns go through
`sh -c "echo … > marker"`, and the shell creates/truncates the redirect
target before `echo` writes its first byte — the exact gap the old code
returned `""` from (and deleted the file on the way out, so the retry loop
never got a second chance).

## Evidence (dev VM, `ssh -p 2222 dev@localhost`, `/mnt/scoot` on branch `xwayland-read-marker-race`, `source ~/xw/env.sh` + `SCOOT_REQUIRE_XWAYLAND=1` where the live suites run)

- **Alone:** `cargo nextest run -p scoot --bins
  'compositor::xwayland::tests::display_reaches'` ×25: 25/25 pass
  (plus one initial single green run: 1 passed, 1792 skipped).
- **In-group:** `cargo nextest run -p scoot --features xwayland --bins
  'compositor::xwayland'` (the whole module, live suites required so no
  quiet skips — includes the dnd suite the ticket suspects): 3 sequential
  runs, 76/76 passed each.
- **In-group under load:** two concurrent full-module runs plus a CPU burner
  on each of the 4 vCPUs: both 76/76 passed.
- **Cheap set:** `cargo nextest run --workspace --no-fail-fast`: 2330
  passed, 0 failed, 25 skipped. `cargo test -p scoot --features xwayland`:
  1851 passed, 0 failed. `cargo clippy --workspace --all-targets` and
  `cargo clippy -p scoot --all-targets --features xwayland`: clean with
  `-D warnings`. `cargo fmt --all --check`: clean. `scripts/smoke-test.sh`:
  22 ok, rc=0.

## Found alongside

- **Sibling ticket, likely same root cause (left OPEN).**
  [`testing/xwayland-display-env-flake.md`](../testing/xwayland-display-env-flake.md)
  reports this same test reading `DISPLAY` as `""` where it expected unset
  — that is byte-for-byte this race's signature on the test's *second*
  spawn (`echo ${DISPLAY:-unset} > marker`): a gap poll returns `""`,
  whose trim is `""`, not `"unset"`. Not closed here: it is a separately
  filed item and the call belongs to the coordinator, but if it ever
  reproduces again after this fix, the redirect-gap theory is eliminated.
- **Dev VM disk was at 100% on arrival** (`/dev/vda` 32G, 30G used;
  `/var/cargo-target` 18G). Reclaimed 1.5G by deleting only 8-day-stale
  regenerable ship-tree copies under `/var/tmp` (`flexwm-*`, `v150/b/151`,
  all Sep 18–19, reproducible from git; nothing HANDOFF-referenced was
  touched). The 32G disk is still at 97% — worth a dedicated cleanup pass.
- **`scootbg`'s `a_live_socket_with_a_full_backlog_is_refused_without_hanging`
  fails under a 1024-fd ssh session** (`EMFILE` filling the backlog; needs
  >1024 sockets) and passes at `ulimit -n 65536`. Environmental, unrelated
  to this change (different crate, untouched), recorded so the next red
  gate does not chase it. The full-workspace nextest above ran raised.
