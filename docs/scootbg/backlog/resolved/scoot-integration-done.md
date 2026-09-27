---
title: "Seamless in scoot: a [wallpaper] config section"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# Seamless in scoot: a [wallpaper] config section — RESOLVED

**Both halves landed 2026-09-27.** Part A, scootbg's `apply-config`:
[Part A (scootbg): landed](#part-a-scootbg-landed). Part B, scoot's
`[wallpaper]` section, its spawning and the Nix modules:
[Part B (scoot and Nix): landed](#part-b-scoot-and-nix-landed). The plan
follows unchanged, except that the example below no longer carries its
"do not add this yet" warning.

In scoot, the wallpaper should be one config section, with nothing else to
wire up.

```toml
[wallpaper]
image = "~/Pictures/hills.jpg"   # or: color = "#1e1e2e"
mode = "fill"

[wallpaper.output."DP-2"]
color = "#101014"
```

## The rule: whichever you changed last wins

Stated once here, and the same way in the README and in
[restore-state-done.md](restore-state-done.md):

- Edit `[wallpaper]` (and start or reload scoot): the config's wallpaper
  shows.
- Run `scootbg set` afterwards: that wallpaper shows, and keeps showing
  across restarts and unrelated reloads, until you next change
  `[wallpaper]` itself.

### Mechanism: one command for everything from the config

`apply-config` belongs to this ticket: it moved here from the CLI ticket
([cli-and-ipc-done.md](cli-and-ipc-done.md#resolution)) when
that ticket's other items landed, because it needs ticket 9's saved state
([restore-state-done.md](restore-state-done.md)) for its
fingerprint. That state now exists (see [below](#what-ticket-9-provides));
`apply-config` itself does not yet: the CLI neither parses nor advertises
it.

Everything scoot does with scootbg goes through one command,
`scootbg apply-config --profile NAME '<json>'`, where the JSON is the
`[wallpaper]` section's wallpaper values (`{}` when the section is
absent). scoot never uses `set`, `clear` or `daemon` itself.
`apply-config`:

1. Computes a fingerprint of the values, over a **canonical encoding**:
   scootbg parses the JSON and re-encodes it with sorted keys before
   hashing, never trusting the sender's byte order (a `HashMap` of
   per-output tables serializes in a different order every process). The
   values are taken after scoot resolves paths, and **`command` is not
   among them**: it is scoot's own key for finding the binary, and with
   the home-manager module it is a store path that changes on every
   upgrade, which would otherwise re-apply the config and wipe a
   `scootbg set` pick on each one.
2. If a daemon answers on the socket, hands it the section. If none does,
   becomes the daemon itself with the section as its starting point. Two
   racing starts (scoot's and an `[autostart]` entry's, say) are settled
   by the socket bind: the loser forwards to the winner, so the config's
   values are never dropped. A daemon started by `apply-config` uses its
   `--profile`; a forwarded `apply-config` carries its profile, and **the
   daemon adopts each one**: from then on it compares,
   restores and records (later `scootbg set`s included) in that profile's
   state, whatever profile it started with. The config owns the profile,
   so which process won the bind never changes where state lives.
3. The daemon compares the fingerprint with the one in that profile's
   state, which records the last section applied *from the config*:
   - **Different:** the section changed since it was last applied (or
     was never applied). Apply it (an empty section means clear) and
     record the new fingerprint.
   - **Same:** the section has not changed. Keep what is showing, or
     (at startup, or on adopting a profile) restore that profile's saved
     state, which includes any later `scootbg set`.

That delivers the rule in every order, because every config-origin
change records its fingerprint and nothing else does:

| Sequence | Result |
|---|---|
| section A, `set X`, restart | A unchanged, so X is restored |
| section A, reload with B, `set X`, restart | B unchanged since its reload, so X |
| section A, reload with B, restart | B shows |
| section removed by reload, later re-added as A | `{}` was recorded at removal, so A differs and shows |
| no daemon yet, section added by reload | `apply-config` starts the daemon |
| an `[autostart]` entry also starts `scootbg daemon` | whichever binds first, the daemon ends up on scoot's profile, so a `set X` from a previous boot is restored either way |

One order it cannot see: the section removed *while scoot is not
running* and re-added unchanged before the next start. No `{}` was ever
applied, so the old fingerprint matches and the last saved state (which
may be a `scootbg set` pick) restores. The config did not change between
the two runs that scoot saw, so this is still "unchanged", and it is
documented rather than worked around.

### What ticket 9 provides

Ticket 9 ([restore-state-done.md](restore-state-done.md), and
[the Restore section](../../README.md#restore) for the user-facing rules)
built the state this mechanism compares against, and left these hooks:

- **The fingerprint has its line already.** The state file (version 1)
  reads and keeps `fingerprint VALUE` (escaped like every field, at most
  `state::format::MAX_FINGERPRINT`, 256 bytes: a hex hash fits) and
  `profile NAME`; every save writes back the fingerprint it read, so a
  `scootbg set` never loses it. `apply-config` needs no version bump: it
  adds a setter on `state::Saved` (only it calls one) and saves as `set`
  does.
- **`--profile` exists** on `daemon`, checked by `state::Profile::parse`
  (1 to 64 of `A-Z a-z 0-9 . _ -`, no leading `.`, no `..`);
  `apply-config --profile` takes the same parser.
- **Loading and showing are separate.** `daemon::restore::load` reads a
  profile's file (every problem a warning, never fatal) and returns the
  table to save into; `restore::apply(state, record, show)` fills it and,
  when `show`, the daemon's choices. A daemon started by `apply-config`
  loads without showing, compares fingerprints, then applies either the
  section or the saved choices, rather than restore first and flash.
- **Adopting a profile mid-life** is not the start-up path: by then the
  outputs are configured, so after `apply` the daemon must `reconcile`
  every output (start-up needs none: nothing is configured yet), and the
  previous profile's writer should be flushed (`Saved::flush`) before
  `state.saved` is replaced. The restored choices take new generations,
  newer than every earlier request, which is right: adopting is the
  newest event.
- **`apply-config '{}'` with no daemon** writes the file itself:
  `state::load`, the choices cleared and the fingerprint set, and
  `state::saver::write_atomic` (public for this), synchronously, since
  the process exits straight after.
- A restore saves nothing, and `--no-restore` still reads the file, so
  neither can erase a fingerprint.

## How scoot drives it

- **Startup, and every reload, while `[wallpaper]` exists:** spawn
  `scootbg apply-config`, alongside `[autostart]`. No autostart entry is
  needed. Spawning on every reload, not only when the section changed,
  costs one short process and is a no-op when nothing changed (the
  fingerprint matches), and it means `scootctl reload` brings back a
  daemon that crashed, since scoot does not supervise it. Spawn order
  does not decide which client commits first, so nothing here promises
  the wallpaper appears before a bar.
- **No section at startup: scoot spawns nothing.** A user without
  scootbg installed never sees a missing-binary warning, and a user who
  runs `scootbg daemon` from `[autostart]` without a section keeps their
  `scootbg set` pick (a first `apply-config '{}'` would count as "never
  applied" and clear it).
- **Removing the section** is a reload with `{}`: the wallpaper clears,
  and the daemon keeps running (so a later `scootbg set` still works).
  `apply-config '{}'` with no daemon running records both the fingerprint
  and the clear in that profile's state, then exits, rather than starting
  a daemon only to clear; a later `scootbg daemon --profile scoot` shows
  nothing, as the config asked.
- **The profile** names the state `apply-config` restores and records
  (see [restore-state-done.md](restore-state-done.md)): scoot passes `scoot`, or
  `scoot-nested` under `--nested`, so a nested session and its host keep
  separate state.
- **Paths are resolved by scoot**, which owns its config's meaning: `~/`
  expanded and a relative path taken against the config file's directory,
  before encoding. scoot spawns without a shell, so nothing else expands
  them.
- **Never wait on scootbg from the event loop.** `scootbg set` only
  finishes once scoot has processed its Wayland commit, so scoot waiting
  for it on its own loop thread would deadlock. Every scootbg invocation is
  spawned and left to the existing SIGCHLD reaper; if a failed status
  should be logged, the reaper gains that ability (today it discards
  statuses, `child_reaper.rs`), still without blocking.
- **One-way coupling.** scoot depends on scootbg's command line, never its
  crate, and scootbg never reads scoot's config. The scoot side lives in
  the compositor (config parsing, spawn, reload), not in `scoot-core`,
  which stays platform-independent.
- **Failure is loud but harmless.** A missing `scootbg` binary, a bad path
  or a decode error logs a warning and leaves `background_color` showing.
  The session always starts, as with `[autostart]`.
- **The gap before the first frame.** Until scootbg commits its first
  buffer, scoot shows `background_color`. Measure that gap on `--tty`. If
  it is visible, the goal is to shorten it (commit sooner, decode less),
  not to paint a different placeholder color, which would be its own
  flash. A redundant `scootbg daemon` in `[autostart]` that wins the bind
  first restores the `default` profile, then adopts scoot's, which can
  flash an old wallpaper; the `[wallpaper]` docs tell users to drop that
  entry.

## Finding the binary (packaging)

The `scoot` package deliberately contains only `scoot` (`scootctl` is its
own package, flake.nix, gh #172), so `scootbg` is its own package too, and
scoot must be able to find it:

- scoot runs `scootbg` from `PATH` by default, with `[wallpaper] command`
  to point elsewhere (an absolute path).
- The home-manager module installs the `scootbg` package whenever its
  rendered settings contain `wallpaper`, and sets `command` to that
  package's store path, so a Nix user gets it working with no extra step.
- A missing binary is the fail-open warning above, naming the `command`
  it tried and how to install it.

### NixOS consumers

(User, 2026-09-27: "make sure ticket 10 enhances the nix setup for NixOS
consumers", and "it should flow well with scoot".)

**The bar: turning scoot on is enough.** A NixOS user who writes
`programs.scoot.enable = true` and puts a `[wallpaper]` section in their
config gets a wallpaper. They add no second option, no package, and no
`command` path. scootbg is a detail of scoot there, not a second thing to
set up. The same holds for home-manager. Both modules spell the options
the same way, under `programs.scoot.wallpaper.*`, so one reads like the
other.

Today the NixOS module (`nix/modules/nixos.nix`, `nixosModules.scoot`)
owns the scoot binary and the opt-in session entry, and nothing of
scootbg. After this item, a NixOS user must get a working `[wallpaper]`
with no hand wiring. That includes a user without home-manager, and one
who launches scoot from the greeter entry.

- **`programs.scoot.wallpaper.enable`** installs
  **`programs.scoot.wallpaper.package`** system-wide, so the default
  `[wallpaper] command = "scootbg"` resolves on `PATH` for any user and
  for the greeter-launched session.
  - **It defaults to `programs.scoot.enable`.** Installing a ~1.5 MB
    binary changes nothing until a config asks for a wallpaper, so this is
    not a behaviour change, unlike the login entry, which stays opt-in.
  - Setting it to `false` opts out, for someone using another wallpaper
    daemon.
- **One pinned pair.** scoot and scootbg come from the same flake
  revision by default, so the `apply-config` protocol scoot speaks is
  always the one the installed scootbg understands. If a user pins only
  one of them, `apply-config`'s protocol version check reports the
  mismatch in scoot's log. It must not fail silently.
- **Package default:** the flake wrapper injects
  `self.packages.${pkgs.system}.scootbg` with `mkDefault`, the same
  pattern as `programs.scoot.package`. A direct-module user (no flake)
  who turns it on without setting `package` gets a loud eval failure,
  not a session that silently has no wallpaper.
- **No per-user config from the NixOS side.** The config file stays
  per-user, in the home-manager module, as it does for scoot, and the
  NixOS module only guarantees the binary. `docs/nix.md` says so and
  shows the pairing next to the existing "complete session" example:
  home-manager `settings.wallpaper` plus NixOS `wallpaper.enable`.
- **scootbg is Linux-only** (`flake.nix` exposes `packages.*.scootbg`
  only where `isLinux`, and its `meta.platforms` is Linux). On Darwin the
  home-manager wrapper already defaults `programs.scoot.package` to
  `null`. `wallpaper.package` does the same there, and so does the
  overlay: it provides `pkgs.scootbg` only on Linux. A Darwin config
  that edits a `[wallpaper]` section for a Linux box must evaluate,
  not fail on a missing attribute.
- **An overlay:** `overlays.default` providing `pkgs.scoot`,
  `pkgs.scootctl` and `pkgs.scootbg`.
  - `docs/nix.md` currently says "there is no overlay", and that is why
    direct-module users must set `package` by hand.
  - Decide it in this item, and record the reason either way.
  - If it is added, the modules default to `pkgs.scootbg` where it
    exists, the flake wrappers keep injecting their own build, and the
    `docs/nix.md` caveat goes.
- **Checks:** extend `nix/tests.nix`, the hermetic `evalModules` checks
  that `nix flake check` runs. Cover the option off (no package), the
  option on (the package in `environment.systemPackages`), the
  direct-module failure without a package, the flake wrapper's default,
  and the overlay if it is added.
- **An end-to-end VM test:** a `nixosTest` that boots a scoot session
  and sees a wallpaper pixel would prove the whole path. Add it if CI
  can carry its cost; decide, and record which.
- **Docs, in the same PR:** `docs/nix.md`'s NixOS module table and prose
  (the new options, the pairing, the overlay), and the packaging line in
  `docs/scootbg/README.md`.

## Docs

`docs/configuration.md` gets a `[wallpaper]` section (with `command`), the
root `README.md` a line under "Running", and `docs/nix.md` the module
behaviour, all in the PR that lands this.

The scoot half goes through scoot's own review bar (it is compositor
code), with the smoke test extended to check a wallpaper pixel on each
headless output.

## Part A (scootbg): landed

Landed 2026-09-27 on branch `claude/scoot-screenshot-verify-0ocx5p`: the
code at `4a3cdf6`, this record in the docs-only commit after it. The
ticket stays open for part B.

### What landed

- **`scootbg apply-config [--profile NAME] JSON`** (`crates/scootbg/src/apply.rs`,
  `cli.rs`): `--profile` as for `daemon` (`state::Profile::parse`,
  default `default`); the JSON one argument, at most 64,512 bytes.
- **The section** (`section.rs`): parsed and validated strictly: `image`
  (absolute, no NUL, at most 4095 bytes), `color` (`#rrggbb`), never both;
  `mode`, `fill`, `filter` with an image only; `output` tables by
  connector name (not empty, at most 256, each the same five keys);
  `command` accepted and ignored. Refused, as a usage error (exit 2): an
  unknown key at either level, a key given twice (serde's derive for
  fields, a binary search for output names), `null`, a value of the wrong
  type, an array where an object belongs (`Object<T>`, since a derived
  struct also reads an array), trailing text, non-UTF-8. The daemon
  re-validates what it gets through the same type.
- **The fingerprint** (`Section::canonical`, `sha256.rs`): SHA-256, hex, of
  compact JSON with keys in byte order at both levels, strings as given,
  `command` left out, every other key present as it was. A user can check
  it: `printf '%s' "$canonical" | sha256sum`.
- **The daemon's side** (`daemon/config.rs`): adopt the profile if it is
  another (load its state, an every-output `clear` at the adoption's
  generation, `Saved::adopted_at` so nothing older lands in the new
  file); compare fingerprints; different: apply the section as a `set` of
  everything and record the fingerprint in the same write
  (`Saved::applied_config`, the only setter); the same: keep what shows,
  or on adopting show the profile's saved state. Then every output is
  stamped and reconciled, and the reply waits like a `set`'s. A daemon
  *started* by `apply-config` does this before serving anyone
  (`config::start`), from the section.
- **Starting the daemon** (`apply.rs`): with nothing answering and the
  display's lock free, `apply-config` re-execs itself (`/proc/self/exe`)
  as `apply-config --serve --profile=NAME CANONICAL`, stdin and stdout
  `/dev/null`, stderr inherited, working directory `/`; that process calls
  `setsid(2)`, then runs the daemon with the section as its start. The
  caller then retries its connection (every 1 ms, 5 s in all) and sends the
  section as to any daemon, so it gets a reply and an exit status. A
  lock held with nothing answering is a daemon starting: waited for. A
  started daemon that loses the lock forwards its section to the winner
  and exits.
- **`{}` with no daemon**: under the lock, the profile's state file gets
  `all clear` and the fingerprint (`state::load`, `format::encode`,
  `saver::write_atomic`), unless it has that fingerprint already; a file
  that cannot be read, or is a newer scootbg's, is left alone (exit 1).
- **Protocol**: `apply-config` is additive to protocol 1
  (`{"protocol":1,"type":"apply-config","profile":…,"config":{…}}`,
  both fields required; on other requests they stay ignored unknown
  fields); `query` gains a top-level `profile`. The command sends
  `version` first on the same connection: another version is a warning,
  another protocol or `unknown request` (a daemon that predates
  `apply-config`) an error naming both builds and what to do.
- **Exit status**: 0 applied or unchanged, and on screen (or `{}`
  recorded); 1 no daemon started or reached in 5 s, no reply in 30 s (one
  bound for the whole exchange), the connection closed before the reply,
  another protocol or too old, an image in the section that is not a file
  (the rest applied), a failed draw, the state file not written; 2 usage,
  a refused section included.
- **Docs**: the root README's scootbg section, `docs/scootbg/README.md`
  (a new `apply-config` section with the schema, the fingerprint, the
  precedence table, starting, exit codes, timing; the protocol; the
  budget), `scootbg apply-config --help`, `query --help`.
- **Tests**: unit tests for the section (schema, strictness, canonical
  encoding, fingerprints, bounds, 20,000 generated inputs), SHA-256 (FIPS
  vectors, and lengths 55, 56, 63, 64, 65, 119, 120, 128 against
  coreutils), the `{}` writer, reply classification, the protocol, the
  CLI, the responder, `Saved`'s fingerprint and adoption floor.
  `tests/config.rs`, 20 tests on headless scoot, added to CI's
  integration job: every row of the precedence table across a real
  restart (the compositor killed, a new one over the same state
  directory, `apply-config` run as scoot's start-up would), the order it
  cannot see, per-output tables and an image, a missing image, refused
  sections, `{}` with no daemon, eight racing starts, a start racing a
  plain `daemon`, `set` racing `apply-config` (the file agreeing with the
  screen), no compositor, the daemon SIGKILLed mid-apply, `apply-config`
  killed mid-wait, and a fake daemon on the socket of another version,
  another protocol, and one that predates `apply-config`.

### Departures from the plan, and why

- **`setsid` in a re-exec'd child, not a double fork.** `fork` is not
  reachable from safe Rust (`forbid(unsafe_code)`), and `pre_exec` is
  `unsafe` too, so the child calls `setsid` itself (rustix's safe call).
  No second fork is needed: `apply-config` is the short-lived
  intermediate, and it exits without waiting, so the daemon is reparented.
  `setsid` is what keeps a signal to the caller's process group away from
  it. Checked: the daemon's `/proc/PID/stat` has ppid 1, and its pid as
  both process group and session, no tty.
- **The section is sent twice on a cold start**: once as the started
  daemon's starting point (so the saved state never flashes first), then
  by `apply-config` as to any daemon (so it gets a reply and an exit
  status; the fingerprint then matches, so nothing changes). The ticket
  said "becomes the daemon itself": the caller becoming the daemon would
  leave scoot no exit status to log, and would not be detached.
- **The old profile's writer is not flushed on adopting.** `Saved::flush`
  would stall the loop up to 2 s on a slow disk. The writer thread finishes
  on its own (it holds its own reference); the old `Saved` is kept
  (`State::retired`) and flushed with the current one on the way out.
- **An image in the section that is not a file** is an error reply at
  once, after the rest is applied, and stays saved (as a restored image
  that has gone does). Its output shows nothing.
- **A gone image now shows nothing on its output**, in a restore too,
  rather than the every-output choice: the docs already said "that output
  shows the compositor's own background", and on adopting, falling back to
  an older choice would have been wrong. `restore::apply` became
  `restore::put` with an origin for its messages and a list of problems.
- **Each table stands alone** (an output's image does not inherit the top
  level's `mode`/`fill`/`filter`), as `scootbg set` does. The plan did not
  say; this is the simpler rule, and it is documented.
- **The canonical encoding is literal**, as the plan says: strings as
  given, so `#1E1E2E` and `#1e1e2e`, or an explicit `mode = "fill"` and
  none, are different sections. Normalizing would make a cosmetic edit
  not count as "changing the section", which the rule is phrased in.
- **`{}` holds the lock while it writes** (race safety with a daemon
  starting). A `scootbg daemon` started in those milliseconds finds the
  lock held and exits "already running". Only a reload that removes the
  section, at the same moment as a hand-started daemon, can meet it.
- **The binary grows by 106,496 B** (1,569,640 → 1,676,136 stripped). By
  symbol, about 27 KB is `std::process::Command` (the only way to start a
  process without `unsafe`), 23 KB the strict serde parse, 20 KB the client
  half; the output-table sort was replaced by sorted insertion (−12,288 B).
  Still `libc.so.6`, `libm.so.6`, `libgcc_s.so.1` only; no `libc` crate.

### Measurements

**Setup.** The Claude Code web container (x86_64, 4 CPUs, no GPU, root).
`scootbg` release at `4a3cdf6` (sha256 `6fb730cd…`, 1,676,136 B); base
`4e2d82b` built in its own worktree and target directory (`634c8133…`,
1,569,640 B). The compositor a debug `scoot --headless --outputs 2`
(1600×1000 each), empty config. Script: `bench.py` in the session's
scratch record: each round with a fresh state directory, `apply-config`
of a color with no daemon (it starts one), five unchanged, five changed,
five `set`s, `kill`; `{}` with no daemon twice (written, then already
recorded); then a plain `scootbg daemon` and a `set` retried until it
succeeds, from `Popen`. Each time is from `subprocess.run` to exit, which
is after the change is on screen and the compositor has it. 10 rounds a
run, two runs; medians, (min–max):

| ms | run 1 | run 2 |
|---|---|---|
| cold `apply-config` (starts the daemon) | 6.09 (4.80–7.90) | 6.05 (4.98–8.93) |
| cold `daemon`, then `set` | 3.80 (3.48–4.68) | 4.48 (4.02–8.64) |
| warm, unchanged | 1.77 (1.55–6.86) | 2.10 (1.70–8.59) |
| warm, changed | 2.24 (1.88–4.45) | 2.60 (2.11–7.11) |
| warm `set` | 2.19 (1.85–6.21) | 2.44 (2.00–4.31) |
| `{}`, no daemon, written | 2.28 (2.02–3.64) | 2.50 (2.11–2.93) |
| `{}`, no daemon, already recorded | 1.48 (1.36–1.78) | 1.62 (1.42–3.30) |

A cold `apply-config` costs about 2 ms more than `daemon` then `set`: one
more process start, and a spawn. The retry interval was 5 ms at first,
which cost about 3 ms of every cold start (median 7.27, 7.16–8.15): now
1 ms. A changed `apply-config` costs what a `set` does; an unchanged one
a little less (nothing is drawn).

**Idle**, the daemon `apply-config` started, a color, after 1 s settled,
then 30 s: 0 voluntary and 0 involuntary context switches, 0 user and 0
system ticks (`/proc/PID/status`, `stat`); 1 thread, 8 fds, VmRSS
3,976 kB, RssAnon 224 kB; ppid 1, process group and session its own pid,
no tty.

**A stopped daemon** (`SIGSTOP`): `apply-config` exits 1 after 30.3 s,
"the daemon did not answer within 30 s (the change may still happen)";
on `SIGCONT` the daemon carries on.

### Verified where

On the container above, at `4a3cdf6`:

- `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1
  SCOOTBG_TEST_SWAY=…sway-1.12/bin/sway devenv shell -- soft-egl cargo
  nextest run -p scootbg -p scootbg-mem`: 419 passed, 2 skipped (the
  `#[ignore]`d benchmarks).
- `cargo test -p scootbg -p scootbg-mem` (same variables): every suite
  passed, `tests/config.rs` 20 of 20.
- `cargo clippy -p scootbg -p scootbg-mem --all-targets -- -D warnings`,
  `cargo fmt --check -p scootbg -p scootbg-mem`, `RUSTFLAGS="-D warnings"
  cargo build --release -p scootbg`: clean; `ldd` and the `libc`-crate
  check as CI runs them: clean.
- `nix build .#scootbg --option sandbox true` (with the proxy's CA bundle
  as an extra sandbox path): built.
- **The tests catch what they claim**, checked by breaking the code:
  ignoring the fingerprint (always applying) fails four tests (rows 1, 2
  and 6, and the unseen order); putting `command` in the canonical
  encoding fails `a_set_survives_an_unchanged_section`; skipping adoption
  fails `an_autostart_daemon_adopts_the_profile`; skipping `setsid` fails
  `a_section_starts_the_daemon`.

### Not verified, and why

- **`--tty`, the dev VM**: not reachable from this container. The gap
  before the first frame at login is part B's to measure there.
- **A real scoot driving it**: part B. The tests stand in for scoot by
  running `apply-config` as it will.
- **A systemd user session as the subreaper** that reaps the detached
  daemon: here PID 1 reparents it (and reaps lazily: exited daemons sat as
  zombies of PID 1 for a while). A subreaper that never reaps would keep
  one zombie per daemon exit; none known does that.
- **A permission-denied state file** for the `{}` path: as root,
  permission bits do not bind; a directory in the file's place stands in.

### What part B must do

The scoot side, in scoot's own review bar, plus the Nix work above:

- **Config**: a `[wallpaper]` section (the keys above, `output."NAME"`
  tables, `command`, default `scootbg`), resolving `~/` and relative
  `image` paths against the config file's directory, then encoding it as
  JSON in any key order. Send only keys the user wrote: scootbg refuses
  unknown ones, and an explicit default counts as a change. `command` is a
  string (scootbg refuses anything else).
- **Spawning**: `COMMAND apply-config --profile scoot JSON` (`scoot-nested`
  under `--nested`) at start-up and on every reload while the section
  exists, `{}` on a reload that removes it, nothing at start-up without
  one. Leave stdin, stdout and stderr as for `[autostart]` (stderr to
  scoot's log: the daemon writes there too; never a pipe scoot reads).
- **Ordering**: two reloads in quick succession start two `apply-config`s
  that race, and the older section can land last. Do not start one while
  the previous one runs (queue the newest), or pass them in order.
- **Exit statuses**: the reaper logs a non-zero status with the command:
  1 is a runtime failure (stderr already says which), 2 a section scootbg
  refused (a scoot/scootbg mismatch, or a value scoot did not check). A
  missing binary is the warning above.
- **Nix**: the NixOS and home-manager options, the overlay decision, the
  checks and the docs listed under [NixOS consumers](#nixos-consumers).
- **Docs and tests**: `docs/configuration.md`, the root README's
  "Running", `docs/nix.md`; the smoke test checking a wallpaper pixel on
  each headless output; the `--tty` gap before the first frame.
- **A bounded queue** (review of PR #293, F4): the queue of `apply-config`
  runs above must not wait on one forever. A run can take 5 s to find a
  daemon and 30 s for its reply; stop waiting on a run after about 40 s,
  log it, and move on to the newest queued section (the run itself keeps
  going and exits on its own).
- **Clean file descriptors** (F4): spawn `apply-config` with no
  descriptors of scoot's but stdio (everything else `CLOEXEC`, as std's
  `Command` leaves them; anything scoot opens without it would be inherited
  by `apply-config`, and from it by the long-lived daemon).
- **A hung disk on the `{}` path** (F4): with no daemon, `apply-config
  '{}'` writes the state file (`fsync`, `rename`, directory `fsync`)
  holding the display's lock, and that is not bounded: on a hung NFS home
  it waits as long as the kernel does, and a `scootbg daemon` started
  meanwhile finds the lock held and exits "already running". Not bounded
  here, because it cannot be cheaply: a timeout would have to give up on a
  thread still inside `fsync` while the lock is held, and a process in an
  uninterruptible `fsync` cannot even exit to release it. So part B's
  bounded queue (above) is what keeps scoot moving: it stops waiting on
  that run, and the next reload's run finds the lock and waits for it,
  bounded by its own 5 s.

  - **What moving on costs** (re-review of PR #293, N1):
    - That next run fails after its 5 s ("a daemon holds the lock but did
      not answer"), and its newer section is not applied.
    - When the stuck `fsync` finally ends, the old `{}` is what the state
      file records.
  - So part B logs a failed run and keeps its section queued as the
    newest, rather than treating the failure as final. The next successful
    run then applies it.
- **A `stat` on the wallpaper client's loop** (re-review of PR #293, N2):
  - scootbg checks that each image in a changed section exists, and
    re-checks missing ones on an unchanged apply, on its Wayland loop.
  - On a hung network mount that stalls scootbg (not the compositor), as
    restore at start-up already could.
  - Recorded here, not fixed: moving the `stat` off the loop belongs with
    the other slow-disk work.

## Review of PR #293

Review found no crash, hang or data-loss blockers, and one finding that
had to be fixed before merge (F1). All fixed on the same branch in
`0c1c48d` and `dc8196f` (the second a regression the first introduced,
found while re-verifying: see F5); this record in the docs-only commit
after them.

- **F1 (must fix): a missing image exited 0 on a cold start or an
  unchanged apply, and a reload after the file came back never showed
  it.** A started daemon's `config::start` said its problems only on
  stderr; the caller's own request then found the fingerprint matching,
  and the "same" branch checked nothing. Now the "same" branch re-checks
  the section's images that are not showing (`config::recheck`): an entry
  whose *saved* choice is still the section's own (the same path and look)
  but whose live choice is not is `stat`ed, put back if the file is there
  (`Choices::fill`, at the generation it was chosen at), and reported if
  not (exit 1). The cold path is covered by the same check: the caller's
  request, the first reply it gets, arrives unchanged and re-checks.
  **The decision on `set`:** a `set` or `clear` for that output, or for
  every output, replaces the saved entry, so it is no longer the
  section's and is neither put back nor reported; and putting back at the
  old generation (not a new one) makes the image no newer than it was, so
  it cannot undo a newer request still decoding, and putting back the
  choice for every output drops no per-output choice (a new generation
  would drop every older one, `set`s included). That is the precedence
  rule unchanged: the section's choice fills in only where it is still
  the last thing chosen. Tests: `a_missing_image_is_reported_until_it_is_back`
  (a per-output image missing at a cold start: 1; unchanged: 1; a `set`
  for the other output; the file appears: 0, shown, the `set` stands; the
  every-output image missing, a `set` for one output, the file appears:
  the others show it, the `set` stands; a `set` on the very output: 0
  from then on, the file appearing changes nothing; across a restart).
  Against the tree without the recheck it fails on the first cold run
  ("left: Some(0)" with the missing-image warning on stderr: the
  reviewer's evidence); with the put-back at a fresh generation it fails
  on "the set for this one stands".
- **F2: two writers for one profile could race** (A→B→A within one
  save). Adopting a profile whose retired writer is still busy now waits
  for that write (up to 2 s, `SAVE_GRACE`) before reading the file, so the
  file read is the newest, and if it is still stuck takes that writer back
  (`Saved::take_writer`) rather than start a second. Retired writers are a
  list; idle ones are dropped (nothing to lose), the rest flushed on exit,
  each given the grace. Unit test: `a_writer_is_taken_back_only_for_its_own_profile`.
  `rapid_adoptions_keep_each_file_newest` (20 rounds of `set`, adopt B,
  adopt A) is a consistency check only: it passed with the fix taken
  out, three runs of three, since a save takes about 0.3 ms here and each
  `apply-config` a few.
- **F3: the forwarder could lose the section.** A `--serve` that lost the
  lock and then finds no daemon serving and the lock free (the winner
  recorded `{}`, or died before binding) now tries once more to be the
  daemon itself; twice at most, and no process starts another.
  `a_loser_whose_winner_never_serves_becomes_the_daemon` (the test holds
  the lock, then drops it) fails without the retry ("no daemon answered,
  and none was starting").
- **F4 (part B):** in [What part B must do](#what-part-b-must-do): a
  bounded queue, clean descriptors, and the `{}` path's `fsync` under the
  lock, documented as unbounded, with why.
- **F5: exec the running binary, not `current_exe`'s text.** First landed
  (`0c1c48d`) as an exec of `/proc/self/exe` always, which named the
  daemon's process `exe` (the kernel takes `comm` from the exec'd file
  name): `pgrep scootbg`, `pkill scootbg` and `ps -C scootbg` no longer
  found it. Found while re-verifying (a `pgrep -x scootbg` found no
  daemon). Now (`dc8196f`) the `current_exe` path is exec'd while it is
  the same file (device and inode) as `/proc/self/exe`, and
  `/proc/self/exe` only when the path names another file or none (an
  upgrade meanwhile), so a different build never runs; the path alone
  without `/proc`. `a_section_starts_the_daemon` checks `/proc/PID/comm`
  is `scootbg` (fails with `exe` against `0c1c48d`'s F5).
- **F6:** connects are tried every 1 ms for the first 50 ms, then every
  15 ms. A cold daemon answers well inside 50 ms, so cold start is
  unchanged (below).
- **F7:** the root README's exit-1 list names the connection closed before
  an answer and a daemon too old for `apply-config`.

**Numbers at `dc8196f`** (same setup and script as
[Measurements](#measurements); release sha256 `39e2c361…`, 1,684,328 B,
+8,192 over `4a3cdf6`). Medians (min–max), ms, two runs:

| | run 1 | run 2 |
|---|---|---|
| cold `apply-config` | 6.90 (4.85–7.74) | 5.61 (4.89–10.54) |
| cold `daemon`, then `set` | 4.27 (3.70–4.64) | 4.01 (3.50–5.20) |
| warm, unchanged | 1.96 (1.58–10.04) | 1.86 (1.53–9.15) |
| warm, changed | 2.55 (2.04–4.68) | 2.31 (1.92–4.32) |
| warm `set` | 2.39 (1.98–5.71) | 2.34 (1.92–4.95) |
| `{}`, no daemon, written | 2.52 (2.24–3.11) | 2.37 (2.27–2.86) |
| `{}`, no daemon, recorded | 1.51 (1.43–2.37) | 1.76 (1.36–2.03) |

Cold raw, run 1: 7.14, 5.60, 5.52, 7.74, 7.06, 6.74, 7.63, 7.44, 4.85,
5.20; run 2: 5.59, 5.00, 7.92, 5.63, 6.78, 10.54, 5.26, 5.55, 7.73, 4.89.
At `0c1c48d` (the retry backoff in, F5's first form): 6.23 and 6.46; at
`4a3cdf6`: 6.09 and 6.05. The spread is the host's (the `daemon`+`set`
baseline moves with it), not the backoff, which never engages on a cold
start here. Idle, the daemon `apply-config` started at `dc8196f`: comm
`scootbg`, ppid 1, its own process group and session, no tty, 8 fds, 1
thread, VmRSS 3,968 kB; 0 context switches and 0 CPU ticks in 30 s.

**Verified at `dc8196f`:** `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1
SCOOTBG_TEST_SWAY=…sway-1.12/bin/sway devenv shell -- soft-egl cargo
nextest run -p scootbg -p scootbg-mem`: 424 passed, 2 skipped (the
`#[ignore]`d benchmarks); `cargo test` (same variables): every suite
passed, `tests/config.rs` 23 of 23; clippy `-D warnings`, `fmt --check`,
`RUSTFLAGS="-D warnings"` release build, `ldd` (libc, libm, libgcc_s) and
no `libc` crate: clean; `nix build .#scootbg --option sandbox true`:
built.

## Part B (scoot and Nix): landed

Landed 2026-09-27 on branch `claude/scoot-screenshot-verify-0ocx5p`: the
code at `9464cf2`, a Darwin-only fix to the Nix checks at `4646074`, and
this record in the docs-only commit after them.

### What landed

- **`[wallpaper]` in scoot's config** (`crates/scoot/src/compositor/wallpaper/section.rs`):
  `image`, `color`, `mode`, `fill`, `filter`, `output."NAME"` tables of
  those five, and `command` (a string, default `scootbg` from `PATH`).
  `~` and `~/` expand against `HOME`; a relative path resolves against the
  config file's directory, made absolute against the working directory
  but never through symlinks (a home-manager config is a link into the
  store, and "relative" means the directory the user sees); `.` components
  are dropped, `..` kept. `command` resolves the same way when it has a
  `/`, and stays a bare name for `PATH` otherwise. The JSON carries only
  the keys written (an explicit empty `output = {}` and empty output
  tables included, `command` never), from a `BTreeMap`, so it is the same
  bytes every run; it is refused past 64,512 bytes (scootbg's own bound),
  as are an empty `image` or `command`, a NUL in `command`, `~` with no
  `HOME` and a path that is not UTF-8.
- **Spawning** (`wallpaper.rs`): `COMMAND apply-config --profile
  scoot|scoot-nested JSON` at startup (before `[autostart]`) and on every
  reload while the section exists, `{}` on the reload that removes it
  (through the removed section's own `command`), nothing at startup
  without one. The child gets the session environment `State::spawn`
  gives (now one function, `State::session_command`, less the activation
  token only `spawn` mints) and scoot's stdio, and is never waited on.
- **The reaper** keeps statuses now: `child_reaper::wait` returns
  `Running`, `Exited(status)` or `Gone`; `reap_children` sweeps
  `spawned_children` as before and then the wallpaper runs, one bool check
  per `SIGCHLD` when none is tracked. Each end is logged with its pid,
  command and whether it had been abandoned: 0 info; 1 "a runtime failure,
  which scootbg describes above"; 2 "scootbg refused the section" (a value
  scoot passes through, or a scootbg that does not match); other codes,
  signals and `ECHILD` warnings of their own.
- **The run queue** (`wallpaper/queue.rs`, pure, 18 unit tests): one run
  at a time; of the sections submitted meanwhile only the newest runs; a
  run is abandoned after 40 s (still reaped and logged; one calloop timer
  per run, re-armed if it fires early, removed when the run ends); a failed
  run of the newest section (any non-zero end, a signal, `ECHILD`, or a
  spawn that failed) is *held*, and retried on the next trigger, which is
  a reload (with the section unchanged, absent or invalid alike) or an
  abandoned run ending, never its own failure (no retry loop); an
  abandoned run of an older section that ends is followed by the newest
  again, since it may have landed after it; at most 8 abandoned runs.
- **Reload** reports `wallpaper` when the section's values changed (added,
  edited, removed) and `wallpaper.command` when only `command` did, and
  refuses a broken section by name, keeping the running one. It applies
  under lock (the background layer is not in a locked frame).
- **A missing binary** is one warning per attempt: "cannot run scootbg for
  the [wallpaper] section: the command was not found. Install scootbg
  (...) or set `command` in [wallpaper] to its path. The session carries
  on with its background color; the next reload tries again", with the
  command and the OS error.
- **Nix.** NixOS: `programs.scoot.wallpaper.enable` (default
  `programs.scoot.enable`) installs `programs.scoot.wallpaper.package`
  system-wide; enabled with no package is an assertion naming the option.
  home-manager: `programs.scoot.wallpaper.enable` (default `settings ?
  wallpaper`) installs the package and renders `settings.wallpaper.command`
  as `lib.getExe' package "scootbg"` unless the user set one (only for a
  table; anything else renders as written for scoot to refuse). Both
  `wallpaper.package` default to `pkgs.scootbg or null`, and `package` to
  `pkgs.scoot or null`; the flake wrappers inject `mkDefault` of the
  flake's own builds (`null` for scootbg where the flake builds none: HM on
  Darwin). `overlays.default` adds `scoot`, `scootctl` and, where it
  builds, `scootbg`.
- **Tests.** scoot: 51 new unit tests in `wallpaper/tests/`: the
  section's parse, refusals, path resolution and JSON, 23, among them the
  whole-file regression through `config::load` (a `[wallpaper]` beside
  `[binds]` and `[layout]`, both still applied); the queue, 18; the glue
  on a live `State` with a stand-in `scootbg` script, 10: argv, profile,
  `WAYLAND_DISPLAY`, reload and removal, serialization under two quick
  reloads, a failed run's status logged and retried, a missing binary, the
  bound with a 300 ms patience, a new `command`, and the reload report
  through `State::reload`.
  scootbg: `tests/scoot_config.rs`, scoot's own section driving scootbg
  end to end (start-up, a changed reload, a `set` surviving an unchanged
  reload, the removal), in CI's integration job. `nix/tests.nix`: 41 new
  pins and four new file checks. `scripts/smoke-test.sh`: see below.
- **CI.** The Linux job builds `-p scootbg` beside scoot, keeps it aside
  with the default binaries and runs the smoke test with
  `SMOKE_REQUIRE_SCOOTBG=1`, so its wallpaper section cannot skip. The
  path classification is unchanged: a scootbg-only change skips the Linux
  job, and the pair is still checked by `tests/scoot_config.rs` in the
  integration job, which runs on either side's change.

### Decisions, and departures from the plan

- **`[wallpaper]` is read leniently, not with `deny_unknown_fields`.**
  Every other table is strict, and an unknown key anywhere discards the
  whole file at startup. Held to that, a typo in a wallpaper key would cost
  a user their binds, the very regression this item had to avoid. So the
  section has a hand-written `Deserialize` that never fails: unknown keys
  and wrong types are collected as problems (every one, not just the
  first), the value drained through `IgnoredAny` (no allocation, recursion
  bounded by `toml`'s own limits; a `toml::Table` passthrough is the
  expensive target `parse_or_defaults`' doc warns about). Startup logs the
  problems and runs nothing; a reload refuses the field and applies the
  rest, as `[floating] modifier` does. Values' meaning (color syntax, mode
  names) stays scootbg's to check (exit 2, logged): one parser for those
  rules.
- **An empty `[wallpaper]` is a section** (`{}`), as the plan implies: the
  generated starting config keeps the whole table commented out, header
  included, and the docs say so.
- **stdin is inherited too**, as the plan says ("as for `[autostart]`"),
  not `/dev/null`: `apply-config` never reads it, and the daemon it starts
  puts its own on `/dev/null`.
- **Retry triggers, precisely.** N1 asked for "the next trigger or
  reload". A trigger here is a reload (whatever it says about the section)
  or an abandoned run ending; a run's own failure is not, or a scootbg that
  always fails would be run in a loop. A refused section (2) is held like
  any failure: retrying it costs one short process per trigger, and it is
  the same rule to reason about.
- **A late run is followed by the newest section**, beyond the plan: the
  hung `{}` of F4 finishing after a newer section was applied would
  otherwise leave the older section in force. It needs a run to have been
  abandoned first, so it costs nothing otherwise.
- **The overlay: added.** It provides the flake's own builds, the same
  derivations as `packages`, not a rebuild against the consumer's nixpkgs
  (`final.rustPlatform`): that would tie the build to whatever Rust their
  nixpkgs carries (the workspace needs edition 2024) and split the scoot
  and scootbg pair, which is the thing `apply-config`'s protocol check
  exists to catch. With it the pure modules have something honest to
  default to, so docs/nix.md's "there is no overlay" caveat is gone.
  Neither `scoot`, `scootctl` nor `scootbg` exists in nixpkgs at the
  pinned rev (checked with `nix eval`), so `pkgs.scoot or null` guesses
  nothing.
- **A new eval failure for direct NixOS module users.** `wallpaper.enable`
  follows `enable` as the plan says, so a direct-module user without the
  overlay who upgrades now gets the assertion until they set
  `wallpaper.package`, apply the overlay or opt out. The CHANGELOG says so.
- **No `nixosTest`.** It would add a NixOS system build and a VM boot to
  every CI run, for a path the smoke test and `tests/scoot_config.rs`
  already prove end to end (a real scoot, its section, scootbg, pixels on
  each output); the modules' own part is pinned by evaluation. Recorded in
  docs/nix.md.
- **The home-manager side has `wallpaper.enable` too**, so both modules
  spell their options the same way (the plan asked for that); it defaults
  to the settings having a `wallpaper` table, which is the plan's "whenever
  its rendered settings contain `wallpaper`".
- **The starting config's header** (`--print-default-config`) had four
  lines emitted with ten spaces before their `#`; the lines this item
  edited are fixed (the `[xwayland]` block's indentation is untouched).

### File descriptors (F4)

Measured live by the smoke test (every run from now on): the `command`
wrapper records its fd table at exec, and no target of the compositor's
own fds past stdio (less what the compositor inherited from the test's
shell) may be in it. At `9464cf2`: "ok: apply-config started with stdio
and nothing of scoot's (6 fds in its table; 10 compositor targets
checked)". Sensitive: with every compositor fd's close-on-exec cleared just
before the spawn (a throwaway change, reverted), the same check failed
naming 15 of them (eventpolls, eventfds, the timerfd, both sealed memfds,
the wayland lockfile, both listener sockets, the spare `/dev/null`s).
Source audit of what was added since the 2026-09-18 audit
(`docs/backlog/resolved/spawn-fd-cloexec-audit-done.md`): the `SIGHUP`
eventfd and the reaper's (`EFD_CLOEXEC`, and each `dup` given
`FD_CLOEXEC` back before anything can spawn), the per-output memfd
(`MemfdFlags::CLOEXEC`), the `--nested` GBM and `--tty` render-node opens
(`OFlags::CLOEXEC`), Smithay's XWayland sockets, lock and display pipe
(`CLOEXEC`, cleared only in the X server's own `pre_exec`, after the
fork). No straggler.

### Measurements

Container as for part A (x86_64, 4 CPUs, no GPU). Release builds:
base `5b373a2` (scoot sha256 `b0e2b5d4…`, 6,998,112 B) and head `4646074`
(scoot `d8824ba3…`, 7,080,320 B, +82,208 B; scootbg unchanged, `c2ea42c2…`).
Scripts `bench.py`, `startup.py` and `firstframe.py` in the session's
scratch record. `scoot --headless --outputs 2`.

- **Startup** (Popen until the control socket answers `version`, base and
  head interleaved, 40 each, a config with only `[layout]`): base median
  8.85 ms (7.31–13.61), head 8.54 ms (7.46–12.02). With a `[wallpaper]`
  (5 rounds, two runs): 9.34 and 9.82 ms; one `fork`/`exec`, not waited on.
- **Reload** (`{"type":"reload"}` written to reply read, 250 per run, 50
  ms apart so each `apply-config` finishes first), medians (p90):

  | | run 1 | run 2 |
  |---|---|---|
  | base, no `[wallpaper]` | 0.523 (0.618) | 0.520 (0.666) |
  | head, no `[wallpaper]` | 0.520 (0.652) | 0.542 (0.657) |
  | head, `[wallpaper]` unchanged | 1.011 (1.182) | 1.096 (1.291) |

  The half millisecond with a section is the spawn of `apply-config` on the
  reload path; every one of the 51 runs per session succeeded.
- **Start to wallpaper on screen** (Popen until scoot logs that
  `apply-config` exited 0, which it does once the wallpaper is on screen
  and the compositor has it; fresh state, so the daemon cold-starts; 10
  runs): median 13.69 ms (12.77–15.96); from scoot's first log line to the
  spawn 4.29 ms, spawn to applied 3.97 ms.
- **Idle**, 10 s after a 1 s settle: 0 CPU ticks and 0 context switches
  for scoot, with and without a section, base and head.
- **Hot paths**: nothing on the render, input or IPC dispatch paths
  changed; the reaper gained one bool check per `SIGCHLD`, and `spawn` is
  the same calls reordered through `session_command`.

### Verified where

At `9464cf2` unless noted, on the container above:

- `devenv shell -- soft-egl cargo nextest run --workspace`: 2557 passed,
  26 skipped.
- `devenv shell -- soft-egl cargo test -p scoot`: 1843 passed, 23 ignored.
- `cargo clippy -p scoot --all-targets -- -D warnings`, `cargo fmt --check
  -p scoot`, and the same for `-p scootbg -p scootbg-mem`: clean.
- `devenv shell -- scripts/smoke-test.sh` (no `soft-egl`,
  `SMOKE_REQUIRE_SCOOTBG=1`): rc 0, 34 `ok:` lines, the wallpaper section's
  twelve among them. With `SCOOTBG=/nonexistent/scootbg`: rc 0, "skipped:
  no scootbg binary"; with that and `SMOKE_REQUIRE_SCOOTBG=1`: rc 1 at
  once, naming the missing binary.
- `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1
  SCOOTBG_TEST_SWAY=…sway-1.12/bin/sway devenv shell -- soft-egl cargo
  nextest run -p scootbg -p scootbg-mem`: 425 passed, 2 skipped; `cargo
  test` with the same variables: every suite passed, `scoot_config` 1 of
  1, `config` 23 of 23.
- At `4646074`: `nix flake check -L` (sandboxed): all checks passed;
  `nix build .#checks.x86_64-linux.scoot-modules --rebuild`: every file
  check printed, the four new ones included; `nix eval` of
  `checks.aarch64-darwin.scoot-modules.drvPath` and
  `checks.aarch64-linux...`: evaluated (every pin holds on those systems;
  the first evaluation of the Darwin one failed on a NixOS pin, fixed in
  `4646074`). `nix fmt --check` over every tracked `.nix`: clean.
- **The tests catch what they claim**, each by breaking the code: runs
  allowed to overlap (5 tests fail, the live overlap test among them); a
  failure never held (4); never abandoning (7, the live 300 ms one
  included); a late older run not followed by the newest (2); `[wallpaper]`
  dropped from `FileConfig`, the whole-file regression (4); unwritten keys
  sent as `null` (5); relative paths not resolved (5); the NixOS default
  off, the home-manager `command` not injected, the NixOS wrapper without
  scootbg, and an overlay that adds `scootbg` on Darwin (each fails the
  module check).
- The configs in docs/configuration.md (the section's example and the
  full example) and the README's Configuring example, run through a real
  `scoot --headless`: no parse error, and the section recognized.

### Not verified, and why

- **`--tty`, and the gap before the first frame at a real login:** no dev
  VM from this container. On headless, start to wallpaper is 13.69 ms
  (above); the `--tty` number is still to take.
- **`nix build .#scoot`** at the head: not run (the container's disk had
  6 GB free, a sandboxed Smithay build needs more headroom than that
  leaves). The new files are under `crates/`, inside the package's
  fileset; CI's `nix-build.yml` builds it on `main`.
- **A real NixOS or home-manager evaluation:** the checks use stand-in
  options, as the existing ones do; `nix flake check` evaluated the modules
  as NixOS modules.
- **A Mac:** the Darwin check was evaluated from Linux, not built on
  Darwin; CI's macOS job does that.
- **The 40 s bound in real time:** the glue's test runs it at 300 ms
  (`Queue::set_patience`, test-only); the constant and the queue's rules
  are unit-tested at 40 s with a fake clock.

### For the next tickets

- **Measure the `--tty` gap** before scootbg's first frame at login, on the
  dev VM (this item's plan asked for it; still open).
- **N2** (a `stat` on scootbg's loop for each image) stays with the other
  slow-disk work.
- **The reload reply** says "applied" when the section was handed over;
  if an agent needs the outcome, a later protocol addition could carry it,
  but it would mean waiting on scootbg, which this item deliberately never
  does.
- **[lightest.md](../lightest.md)** can now measure scootbg as a scoot
  user runs it: started by `apply-config` from the config.
