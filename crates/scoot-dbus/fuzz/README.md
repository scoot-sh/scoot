# scoot-dbus's fuzz target

One `cargo fuzz` target over the D-Bus client's wire parsers: arbitrary
bytes as D-Bus messages on a stream (the session bus's socket). Framing
must accept, wait or refuse without a panic; every framed message's
header must parse or refuse; the body is walked against its declared
signature and as each shape the tray, the media and the bluetooth modules
read (an item's `GetAll` through `read_item_props`, a player's `GetAll`
and `PropertiesChanged` through `read_player_props` and
`read_properties_changed`, a managed set, added and removed interfaces
and adapter, device and battery dictionaries and changes through
`src/bluez.rs`, the same functions the modules call, pixmap lists, name
lists, owner changes); the writer is round-tripped on input-derived
values. It compiles `src/proto.rs`, `src/mpris.rs`, `src/bluez.rs` and
`src/fuzz.rs` by `#[path]`; the seed corpus is frames in the shapes the
tray's fake bus speaks, plus a `GetAll` and a `PropertiesChanged`
marshalled by sd-bus (`src/fixtures/README.md`). Any panic is a finding:
the bar's release profile is `panic = "abort"`, so a panic there kills
the bar.

What the target checks is written once, in scoot-dbus's `src/fuzz.rs`,
compiled here the same way, so the target is one line. This crate is its
own workspace, with its own `Cargo.lock`: it is never built by
`cargo build --workspace`, nextest, clippy or the flake, and nothing here
reaches any shipped binary.

**In CI**, on every scoot-dbus change (which fans out to the `scootbar`
job, its consumer), that job builds the target and runs it for a fixed
budget: 1,000,000 runs of `dbus` from `-seed=1`, over the seed corpus and
`regressions/` (under a minute). Building it is what keeps the `#[path]`
includes from rotting: a change that compiles in scoot-dbus but not here
fails there. A finding's input is printed in base64 in the job's log.
Before building, the step runs `cargo fetch --locked` on this workspace
(cargo-fuzz has no `--locked` to pass on), so a `Cargo.lock` that no
longer matches `Cargo.toml` fails the job instead of being rewritten.
**On every `cargo test`**, scoot-dbus's `fuzz::tests` replays the seed
corpus and every file in `regressions/` through the same check on the
stable toolchain.

## Running it

The pinned stable toolchain with no sanitizer (`-s none`, as scootbg's);
`cargo-fuzz` comes from the pinned nixpkgs for this command only. From the
repository root:

```sh
devenv shell -- nix shell --inputs-from . nixpkgs#cargo-fuzz --command bash -c '
  cd crates/scoot-dbus &&
  mkdir -p /tmp/scoot-dbus-corpus-dbus &&
  cargo fuzz run -s none dbus /tmp/scoot-dbus-corpus-dbus fuzz/corpus/dbus \
    fuzz/regressions/dbus -- -max_len=70000 -timeout=10 -seed=1 -runs=1000000'
```

- **`-max_len`**: 70,000 covers the client's 1 MiB cap only in part, on
  purpose: whole capped messages are already in the seed corpus, and the
  budget is for parser breadth, not for discarding megabytes.
- **Give it a scratch corpus first**, as for scootbar: libFuzzer writes
  what it finds into the first directory, and `fuzz/corpus/` is the
  committed seed.
- A crash goes to `fuzz/regressions/dbus/` once fixed, named for what it
  was, so every later run replays it, fuzzing or not.

## Runs

| When | Commit | Target | Runs | Time | Findings |
| --- | --- | --- | --- | --- | --- |
| 2026-10-02 | `crates/` tree `7bc1a04525c8` | `dbus` (`-runs=1000000 -seed=1 -max_len=70000`) | 1,000,000 | 8 s | none |
| 2026-10-02 | `crates/` tree `7bc1a04525c8` | `dbus` (`-max_total_time=600 -max_len=70000`, seed 1) | 34,432,616 | 601 s | none |
