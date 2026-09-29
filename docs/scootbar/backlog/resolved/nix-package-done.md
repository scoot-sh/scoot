---
title: "Nix package: `packages.scootbar`, overlay, `nix run`, CI and eval tests, from the first milestone"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M1"
resolved: "2026-09-29"
---

# Nix package — RESOLVED

Filed 2026-09-29. Serves **daily-drive**: a bar nobody can install with one
line is not iterating with users. It ships with the **first** milestone that
runs, not after config and modules; every later milestone then lands already
installable. The NixOS and home-manager modules follow in
[nix-modules-and-stylix](../nix-modules-and-stylix.md).

Modeled on how scootbg is packaged in `flake.nix` (its own derivation, overlay
entry and eval tests in `nix/tests.nix`).

## What to build

- **`packages.scootbar`**: `rustPlatform.buildRustPackage` with
  `cargoBuildFlags = [ "-p" "scootbar" ]` so only the one binary is in
  `$out/bin`, `stripAllList`, `doCheck = false` (CI runs the tests), `meta`
  description read from `crates/scootbar/Cargo.toml` like `scootbgDescription`
  (no drift), `mainProgram = "scootbar"`. Linux only; the crate builds a stub
  elsewhere, so no Darwin package, as scootbg.
- **Source fileset**: `src` already unions `Cargo.toml`, `Cargo.lock` and
  `crates/`; add `scootbar/fuzz` to the exclusion beside scootbg's so the fuzz
  crate never enters the build (`nix-src-fileset-done.md`).
- **Overlay** `pkgs.scootbar` next to `pkgs.scoot` and `pkgs.scootbg`, and
  `nix run .#scootbar`.
- **No font in the package's closure.** scootbg's benchmark counts installed
  disk with its non-glibc closure, so a bundled font would show up as a loss.
  The bare binary takes a font from `--font` (a config key later) or a short
  fixed list of well-known directories, and otherwise **refuses to start with a
  message naming how to give it one**; the module and Stylix supply the path. On
  NixOS those directories are usually empty, so `nix run .#scootbar` needs
  `--font` or the demo output below.
- **A demo output, `packages.scootbar-demo`**: `scootbar` wrapped with a small font
  from nixpkgs as its default `--font`, so `nix run .#scootbar-demo` works on a
  clean NixOS box on day one. It is a separate output: the bare `scootbar` (and
  its measured closure) never carries the font, and the modules do not use it.
- **Cargo features as a variant**: `--no-default-features --features ...` is
  reachable by `.override`, tested to build.
- **CI**: a `scootbar` path filter; `nix flake check -L` covers the new output
  on every PR; `nix build .#scootbar` main-only with the others
  (`ci-nix-packaging-done.md`); an `ldd` assertion that the binary links only
  libc, libm and libgcc_s.
- **Eval tests** in `nix/tests.nix`: the overlay provides `pkgs.scootbar` on
  Linux and not on Darwin.
- **Docs**: a short section in `docs/nix.md` (what, why, the font rule); the
  README changes only if what scoot is changes.

## Done when

`nix run github:scoot-sh/scoot#scootbar` starts the bar from a clean checkout,
CI builds it, and the closure is measured and recorded for the
[resource ratchet](../lightest.md).

## Resolution

Resolved 2026-09-29 on branch `feat/nix-package`.

### What landed

- **`packages.<system>.scootbar`** (x86_64-linux, aarch64-linux; none on
  Darwin): `nix/scootbar.nix`, called from `flake.nix` with
  `pkgs.callPackage` and the flake's shared `version`, `src` and
  `cargoLock`. `cargoBuildFlags = [ "-p" "scootbar" ]`, `stripAllList`,
  `doCheck = false`, `meta.description` read from
  `crates/scootbar/Cargo.toml`, `mainProgram = "scootbar"`,
  `platforms = linux`. No `buildInputs`: the bar links only what std links.
- **Source fileset**: nothing to do. #324 had already added
  `crates/scootbar/fuzz` to the exclusion beside scootbg's.
- **Cargo features by `.override`**: the package file takes
  `buildNoDefaultFeatures` and `buildFeatures` as its own arguments and
  passes them to `buildRustPackage`. So
  `scootbar.override { buildNoDefaultFeatures = true; }` builds the bar
  with no modules, and adding `buildFeatures = [ "clock" ]` picks modules
  one by one. This is why the package is `callPackage`d: `overrideAttrs`,
  as `scoot-gpu` uses, reaches only the derivation's own attributes, not
  `buildRustPackage`'s arguments.
- **`packages.<system>.scootbar-demo`**: `nix/scootbar-demo.nix`, a
  `writeTextFile` script at `$out/bin/scootbar`, checked by `shellcheck`
  at build time. It runs the bare build with DejaVu Sans
  (`dejavu_fonts.minimal`, one 742.6 KiB file) as its default `--font`.
- **Overlay**: `pkgs.scootbar` on Linux, the same derivation as
  `packages`. **`apps`**: `scootbar` and `scootbar-demo` (Linux).
- **Eval pins** in `nix/tests.nix`, which fail when a regression breaks
  them (checked by breaking each one):
  - the overlay's `scootbar` is the flake's build, and it has no demo;
  - Darwin's overlay and `packages` have neither;
  - the default build names no features, and both overrides reach
    `cargoBuildNoDefaultFeatures`/`cargoBuildFeatures`;
  - the demo's script runs the bare build and names the font;
  - no input of `scootbar` names a font.
- **CI**: `nix-build.yml` (main only) builds `.#scootbar` and
  `.#scootbar-demo` and runs both `--version`s. It then checks the
  binary with `ldd` (only libc, libm and libgcc_s; no libEGL, libgbm or
  libwayland), fails if any path in the closure names a font, and builds
  the no-modules override through `--impure --expr`, grepping its
  `--help` for "built with no modules". Every PR's `nix flake check -L`
  already evaluates the new outputs and runs the pins. The `scootbar`
  path filter in `ci.yml` already existed. `flake.nix` and `nix/` fall
  into its "everything" branch, so a packaging change runs the scoot
  job's `nix flake check`.
- **Docs**: a "The status bar: scootbar" section in `docs/nix.md` (plus
  the package list, the platform note and the overlay). A pointer in
  `docs/scootbar/cli.md#fonts`, a Nix line and the closure row in
  `docs/scootbar/README.md`, and a `CHANGELOG.md` entry.

### Decisions (the ticket left these open)

- **The demo is a shell script, not `makeBinaryWrapper --add-flags`.** A
  default `--font` has to go after `daemon`. It must be left out when the
  user gives `--font` (the bar refuses a flag given twice), and it must
  never come before `daemon --help` (`daemon` takes `--help` only as its
  first argument). A binary wrapper can only prepend or append
  unconditionally. The script walks the arguments knowing that every
  `daemon` flag takes a value, so in `--clock-format --font` the
  `--font` is a value and not a flag. It inserts the font right after
  `daemon`, so a trailing flag with no value still gets its own error.
  The cost is one bash start, once, and only on the demo: `--version`
  took 2,307 µs bare and 4,016 µs through the script (the mean of 100
  runs each, timed with `date +%s%N`).
- **The demo with no arguments runs `daemon`**: `nix run .#scootbar-demo`
  shows a clock. Everything that is not `daemon` passes through untouched
  (`--help`, `--version`, `help daemon`, an unknown word).
- **The demo is in `apps`, not in the overlay.** It is in `apps` because
  the `apps` comment keeps each runnable output explicit, and running it
  is what the demo is for. It is not in the overlay because it is
  something to run, not to build on: a system installs `scootbar` and
  gives it a font from its own font setup. That matches the ticket's "the
  modules do not use it".
- **`dejavu_fonts.minimal`, not `dejavu_fonts`**: it is DejaVuSans.ttf
  alone (742.6 KiB against the full family), the face the bare binary
  looks for first anyway.
- **The feature override is built on main, not on every PR.** A
  `checks` entry would make every PR's `nix flake check` compile the bar
  again. The ticket keeps `nix build` main-only, so the per-PR proof is
  the eval pin (the flags reach the cargo hook) and the main-only build
  proves they compile.

### Deviations

- **The bare `nix run .#scootbar` needs `-- daemon`**, and on a box with
  no font where the bar looks it also needs `--font F`. The ticket's
  "done when" reads as a bare `nix run` starting the bar. The CLI has
  always required the `daemon` command, and changing that is outside a
  packaging ticket. With no arguments the binary exits 2 with
  `scootbar: missing command (try --help)`. The no-argument experience is
  `scootbar-demo`, as the ticket's own demo output intends.
- `nix run github:scoot-sh/scoot#...` itself is unverified until this
  merges. The same thing was run against the local flake (below).

### Evidence

All on x86_64-linux (the web container; no dev VM involved, since
nothing here needs `--tty`), from the worktree
`/tmp/impl-nix-package`. The nix files, `flake.nix` and the source
fileset are byte-identical from the uncommitted tree through commit
`75ec396`. That is shown by the derivation paths: the drvPath evaluated
before committing (`/nix/store/fsv7x4mrnrvxbx32w3fx0n2q3qr2a9m1-scootbar-0.1.0.drv`)
is the one the committed head evaluates to (re-checked below). The
container runs Nix with `sandbox = false`. The first build's cargo left
`/homeless-shelter/.cargo` behind, which made Nix refuse the next build
there. Every build after that ran with `--option sandbox true`.

```
$ nix build .#scootbar .#scootbar-demo -L          # cold: real 2m13s
  cargoBuildHook flags: -j 4 --target x86_64-unknown-linux-gnu --offline --profile release -p scootbar
  Finished `release` profile [optimized] target(s) in 1m 04s
$ nix build --option sandbox true .#scootbar .#scootbar-demo --print-out-paths
/nix/store/kqpp38wv9nhxyghj7m9710n31bscx3lp-scootbar-0.1.0
/nix/store/7ddp40jbr3yq9lx9r8byvgyfl79x43kf-scootbar-demo-0.1.0

$ stat -L -c '%s bytes' result/bin/scootbar
845152 bytes
$ file -L result/bin/scootbar
ELF 64-bit LSB pie executable, x86-64, ..., interpreter /nix/store/n51dhmdbik1kfrsm62j5knavmigwrl1a-glibc-2.42-84/lib/ld-linux-x86-64.so.2, ..., stripped
$ ldd result/bin/scootbar
	linux-vdso.so.1
	libgcc_s.so.1 => /nix/store/0vqb1mcas5j8dv6bhbrshinlgsg6bvgi-gcc-15.3.0-lib/lib/libgcc_s.so.1
	libc.so.6 => /nix/store/n51dhmdbik1kfrsm62j5knavmigwrl1a-glibc-2.42-84/lib/libc.so.6
	/nix/store/n51dhmdbik1kfrsm62j5knavmigwrl1a-glibc-2.42-84/lib/ld-linux-x86-64.so.2
$ result/bin/scootbar --version; result-1/bin/scootbar --version
scootbar 0.1.0
scootbar 0.1.0
$ nix run .#scootbar -- --version; nix run .#scootbar-demo -- --version
scootbar 0.1.0
scootbar 0.1.0

$ nix path-info -rsSh result          # the bare bar's closure
libunistring-1.4.2      2.0 MiB    2.0 MiB
libidn2-2.3.8         359.5 KiB    2.3 MiB
xgcc-15.3.0-libgcc    193.0 KiB  193.0 KiB
glibc-2.42-84          33.4 MiB   36.0 MiB
gcc-15.3.0-libgcc     193.1 KiB  193.1 KiB
gcc-15.3.0-lib          9.8 MiB   46.0 MiB
scootbar-0.1.0        825.8 KiB   46.8 MiB
$ nix path-info -S result result-1
scootbar-0.1.0        49078696
scootbar-demo-0.1.0   51697520     (adds bash 1.8 MiB and dejavu-fonts-minimal 742.6 KiB)
$ nix path-info -r result | grep -i font || echo "no font in scootbar closure"
no font in scootbar closure
```

The closure row: **49 MB** (49,078,696 bytes) with no font, against the
M0 baselines' 720 MB to 1,232 MB measured the same way (`nix path-info
-S`). Published in [the README](../../README.md#baselines).

The feature override, built exactly as CI builds it:

```
$ nix build --option sandbox true --impure --expr \
    '(builtins.getFlake (toString ./.)).packages.${builtins.currentSystem}.scootbar.override { buildNoDefaultFeatures = true; }'
/nix/store/2y1396pd14jif56g6cjayzc2c3vj3r6w-scootbar-0.1.0            # real 1m01s
$ result/bin/scootbar --help | grep -F 'built with no modules'
A bar on every output, reserving its space (built with no modules).
$ stat -L -c %s result/bin/scootbar; ldd ...
652624 bytes; linux-vdso, libgcc_s.so.1, libc.so.6, ld-linux (nothing else)
```

Flake checks:

```
$ nix flake check -L --option sandbox true
scoot-modules-check> scoot-modules: all file-content checks passed
all checks passed!                                       (rc=0; x86_64-linux)
$ nix flake check --all-systems --no-build
all checks passed!                                       (rc=0; evaluates aarch64-linux and aarch64-darwin too)
$ nix eval --raw .#checks.aarch64-darwin.scoot-modules.drvPath   # the Darwin pins are asserted at eval
/nix/store/3g33kqsaa71wrdxas4s904gx47par0iw-scoot-modules-check.drv
$ git ls-files -z '*.nix' | xargs -0 nix fmt -- --check
(no output, rc=0)
```

Outputs per system (`nix eval .#packages.<system> --apply builtins.attrNames`):
x86_64-linux and aarch64-linux gain `scootbar` and `scootbar-demo`, and
their `apps` gain the same pair. aarch64-darwin is unchanged
(`default scoot scoot-gpu scootctl`).

Negative checks, each a temporary edit reverted after the run, with
`nix build .#checks.x86_64-linux.scoot-modules`:

- `"scootbar"` taken out of the overlay list:
  `error: attribute 'scootbar' missing` at
  `assert overlaid.scootbar.drvPath == built.scootbar.drvPath`.
- `"scootbar-demo"` added to the overlay list:
  `assertion '(! ((overlaid) ? scootbar-demo))' failed`.
- `buildNoDefaultFeatures` not passed to `buildRustPackage`:
  `assertion '((scootbarNoModules).cargoBuildNoDefaultFeatures && ...)' failed`.

**The refusal**, with `/usr/share/fonts` hidden by a tmpfs in a private
mount namespace. This container has Debian's DejaVu at the first
well-known path, and none of the other seven exist:

```
$ unshare --mount --propagation private sh -c 'mount -t tmpfs none /usr/share/fonts; result/bin/scootbar daemon'
scootbar: no font: none of the usual font files is usable; give one with `--font PATH` (a .ttf or .otf file, such as DejaVuSans.ttf)
exit=1
```

The same line was printed inside a headless scoot session
(`scoot --headless --width 1280 --height 400 -- result/bin/scootbar
daemon`, fonts hidden the same way), whose screenshot shows no bar.

**The demo starting a bar**, in the same namespace, with the cargo-built
scoot at `/tmp/sb-clock-target/debug/scoot`:
`scoot --headless --width 1280 --height 400 --socket $RT/scoot-ipc.sock
-- result-1/bin/scootbar` (no arguments), then `scootctl wait-idle` and
`scootctl screenshot --no-cursor`. The screenshot is 1280x400 and shows
the bar with `11:04 am` centered. The bar's process line shows the wrapper
exec'd the bare build with the font:

```
/nix/store/kqpp38wv9nhxyghj7m9710n31bscx3lp-scootbar-0.1.0/bin/scootbar daemon --font /nix/store/zqhby0xidpi0xsafsbl4l7dc72imqqq6-dejavu-fonts-minimal-2.37/share/fonts/truetype/DejaVuSans.ttf
```

The bare build with `daemon --font <that path> --clock-format '%a %d %b
%H:%M' --right clock` drew `Tue 29 Sep 11:09` at the right.

**The demo's argument handling**, with its script's `bar=` pointed at a
stub that prints its argv (DEJAVU is the store path above):

```
demo                                     -> [daemon] [--font] [DEJAVU]
demo --help                              -> [--help]
demo --version                           -> [--version]
demo daemon                              -> [daemon] [--font] [DEJAVU]
demo daemon --help                       -> [daemon] [--help]
demo daemon -h                           -> [daemon] [-h]
demo daemon --font /x.ttf                -> [daemon] [--font] [/x.ttf]
demo daemon --font=/x.ttf                -> [daemon] [--font=/x.ttf]
demo daemon --height 30 --font /x.ttf    -> [daemon] [--height] [30] [--font] [/x.ttf]
demo daemon --clock-format --font        -> [daemon] [--font] [DEJAVU] [--clock-format] [--font]
demo daemon --font-size 20               -> [daemon] [--font] [DEJAVU] [--font-size] [20]
demo daemon --font-size=20 --right clock -> [daemon] [--font] [DEJAVU] [--font-size=20] [--right] [clock]
demo daemon --height                     -> [daemon] [--font] [DEJAVU] [--height]
demo foo                                 -> [foo]
demo help daemon                         -> [help] [daemon]
```

And through the real binary: `daemon --font /nonexistent.ttf` gives
`cannot use the font /nonexistent.ttf: No such file or directory`
(exit 1), `daemon --height` gives `` `--height` needs a value `` (exit 2),
and `daemon --help` prints the daemon page.

Not verified here: the aarch64-linux build (only evaluated), and a real
NixOS box, where the store is mounted read-only and the demo's font would
be mapped rather than read. No Rust changed, so the cargo suites were not
re-run for this entry. CI's scootbar jobs run them.
