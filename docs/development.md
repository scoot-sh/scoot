# Developing scoot

Building, testing, and what CI checks. The engineering standards every
change is held to are in [`CLAUDE.md`](../CLAUDE.md).

## The dev shell

```sh
nix develop                     # every dependency, on Linux or macOS
cargo test --workspace          # the compositor only compiles on Linux
cargo nextest run --workspace   # one process per test -- the required runner
```

On Linux the shell also carries what `scripts/smoke-test.sh` drives (`foot`,
`jq`, ImageMagick, `wayland-info`), sets `$XDG_RUNTIME_DIR` when the box has
none (a container, a CI runner), sets `LANG=C.UTF-8` when no locale is set at
all (`LC_ALL`, `LC_CTYPE` and `LANG` all empty; an explicit one, `C`
included, is kept), and provides `soft-egl`, which runs one command against
Mesa's software EGL the way CI runs the GLES tests:

```sh
soft-egl cargo nextest run --workspace   # without it the GLES tests fail on a GPU-less box
scripts/smoke-test.sh                    # without soft-egl: don't hand it Mesa
```

## devenv

`devenv shell` gives a shell with the same contents (both read
`nix/dev-shell.nix`), and enters faster once warm because devenv caches
its evaluation. `scripts/devenv-bootstrap.sh` installs single-user Nix
and devenv on a disposable Linux box that has neither (a container, a
Claude Code on the web session); on a machine that already has Nix,
install devenv the usual way.

## The crates

| Crate | What it is |
|---|---|
| `crates/scoot-core` | the platform-independent layout engine (no Wayland, no I/O) |
| `crates/scoot-ipc` | the wire protocol and a client over it |
| `crates/scootctl` | the `scootctl` remote-control client |
| `crates/scoot` | the CLI and the Smithay-based compositor |
| `crates/scootbg` | the wallpaper daemon |
| `crates/scootbg-mem` | the only `unsafe` code scootbg has |

[`vm/README.md`](../vm/README.md) sets up a Mac-native NixOS VM to run the
Linux-only half in.

## CI

Every pull request runs `.github/workflows/ci.yml`. It checks:

- the tests above (`cargo test` and `cargo nextest`), `cargo fmt` and
  `cargo clippy -D warnings`;
- `scripts/smoke-test.sh` under `--headless`;
- an `ldd` check that the default build links no GPU stack
  (`libgbm`/`libdrm` are the live assertions; `libEGL`/`libGLESv2` are
  belt-and-braces, since both are `dlopen`ed and never appear in `ldd`
  either way);
- `nix fmt` over all tracked `.nix` files, and `nix flake check -L` (the
  Linux and macOS jobs each cover their own systems' outputs, modules and
  checks);
- a macOS `cargo check --workspace --all-targets` (on a Mac that is the
  `scootctl` client plus the compositor crate with its Linux halves
  cfg'd out).

Build and test steps run through `nix develop` (the smoke test's own
tools come via `nix shell` pinned to the same lockfile), so the flake
stays the only dependency list.

**Jobs are split by path.** A change under `crates/scootbg/` or
`crates/scootbg-mem/` alone runs only scootbg's own job (fmt, clippy,
tests, a no-`libc`-crate check, the release size) plus scootbg's
integration tests: on a headless scoot, and on a headless sway from the
pinned nixpkgs for outputs coming and going, with colors and images
checked by real pixels on both, fractional scales included. A
compositor-only change skips scootbg's job. Shared files (`Cargo.*`,
`flake.*`, `nix/`, `.github/`, and anything unlisted) run everything.

**Packages build on merge, not per PR.** `nix build .#scoot .#scootctl
.#scootbg` runs on every merge to main via
`.github/workflows/nix-build.yml`: it is a full release Smithay build the
cargo cache cannot reuse, so it would tax every push, and the every-PR
`flake check` already evaluates every output and runs the module suite.

**A green check is not full coverage.** A GitHub runner has no seat, no
VT, no `/dev/dri` and no GPU, so `--tty`, `--nested`, every GPU path and
all performance work stay manual on the dev VM. The workflow's header
says so in full.
