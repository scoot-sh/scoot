---
title: "Seamless in scoot: a [wallpaper] config section"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Seamless in scoot: a [wallpaper] config section

**Part A, scootbg's half, landed 2026-09-27; part B, scoot's half and the
Nix modules, is open.** What part A delivered, where it departs from the
plan below, its measurements and what part B must still do are in
[Part A (scootbg): landed](#part-a-scootbg-landed) at the end; the plan
follows unchanged.

In scoot, the wallpaper should be one config section, with nothing else to
wire up.

```toml
# Planned. No scoot release accepts this yet: scoot's config rejects
# unknown sections and, when it does, ignores the WHOLE file (your binds
# and layout included). Do not add it until this item lands.
[wallpaper]
image = "~/Pictures/hills.jpg"   # or: color = "#1e1e2e"
mode = "fill"

[wallpaper.output."DP-2"]
color = "#101014"
```

## The rule: whichever you changed last wins

Stated once here, and the same way in the README and in
[restore-state-done.md](resolved/restore-state-done.md):

- Edit `[wallpaper]` (and start or reload scoot): the config's wallpaper
  shows.
- Run `scootbg set` afterwards: that wallpaper shows, and keeps showing
  across restarts and unrelated reloads, until you next change
  `[wallpaper]` itself.

### Mechanism: one command for everything from the config

`apply-config` belongs to this ticket: it moved here from the CLI ticket
([cli-and-ipc-done.md](resolved/cli-and-ipc-done.md#resolution)) when
that ticket's other items landed, because it needs ticket 9's saved state
([restore-state-done.md](resolved/restore-state-done.md)) for its
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

Ticket 9 ([restore-state-done.md](resolved/restore-state-done.md), and
[the Restore section](../README.md#restore) for the user-facing rules)
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
  (see [restore-state-done.md](resolved/restore-state-done.md)): scoot passes `scoot`, or
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

