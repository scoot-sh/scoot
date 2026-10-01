# The order file's probe: the review's repros, before and after (2026-10-01)

For [the README's "The order file"](../../README.md#the-order-file). Machine: Asahi M2
(aarch64, NixOS, dev shell: GNU ld 2.46, gold 1.16, `rustc 1.97.1`; lld 21.1.8 and mold 2.42.1
from nixpkgs). Each tree has **its own `CARGO_TARGET_DIR`**: a shared one reuses the other tree's
build-script output (same relative path, older mtime), which made the first "after" runs
meaningless; they are discarded. `<old>` = `477854b2e` (the first `build.rs`), `<new>` = the code
of `46239a432` (`crates/scootbar` and `scripts/` identical to the pushed head; later commits are docs).

All three repros are `cargo clean -p scootbar` then `cargo build --release -p scootbar`. The
wrappers are `cc` scripts first on `PATH`: **A** uses GNU ld for the build script's trial link
(`-x c -`) and `-fuse-ld=lld` for every other link (what rustc's own default lld does for the real
link on x86_64 since 1.90); **B** is `RUSTFLAGS="--codegen link-arg=-fuse-ld=lld"`; **C** builds
with a flag-accepting `cc`, touches `main.rs`, then builds with a `cc` that refuses
`--section-ordering-file` first on `PATH` (an old binutils).

| repro | `<old>` | `<new>` |
|---|---|---|
| A | `ld.lld: error: unknown argument '--section-ordering-file=.../hot-text-d503d12c8cbbed12.ld'`, `could not compile scootbar (bin "scootbar")`, exit 101 | `Finished release`, exit 0 (probe through rustc reaches lld: skipped) |
| B | same `ld.lld` error, exit 101 | `Finished release`, exit 0 |
| B, `-C link-arg=-fuse-ld=lld` | `Finished`, exit 0 (the spelling the old parser read) | `Finished`, exit 0 |
| C, step 1 | `Finished`, exit 0 | `Finished`, exit 0 (applied) |
| C, step 2 | `ld: unrecognized option -Wl,--section-ordering-file=...`, `could not compile scootbar`, exit 101 | `Compiling scootbar ... Finished`, exit 0 (the PATH change reran the script, which now skipped) |

Also on `<new>`: a debug build then a changed `PATH` is `Finished ... in 0.03s` (no rebuild);
a release build then a changed `PATH` to the same linker recompiles scootbar (17.7 s) and the
same `PATH` again does nothing (0.03 s); a refusing `cc` then an accepting one gives
`hot-text order file applied`.

## Per linker, at the final tree

`RUSTFLAGS='-C link-arg=-fuse-ld=X' SCOOTBAR_ORDERFILE_VERBOSE=1 CARGO_PROFILE_RELEASE_STRIP=false
cargo build --release -p scootbar`, then `orderfile.py check --binary B --min-coverage 0` (its
FAIL on a linker that ignored the file is the check working):

```
== gnu-ld-default: build rc=0; hot-text order file applied
== bfd: build rc=0; hot-text order file applied
== gold: build rc=0; hot-text order file skipped: the linker takes the option but not as an order file: it accepts a file that is none
== lld: build rc=0; hot-text order file skipped: the linker does not take the option (= note: ld.lld: error: unknown argument '--section-ordering-file=...
== mold: build rc=0; hot-text order file skipped: the linker does not take the option (= note: mold: fatal: unknown command line option: --section-ordering-file=...
== check gnu-ld-default   8 64 KiB windows; 518632 bytes (100%) in the main stretch, 8 windows   ok
== check bfd              8 64 KiB windows; 518632 bytes (100%) in the main stretch, 8 windows   ok
== check gold            16 64 KiB windows; 420804 bytes (81%) in the main stretch, 13 windows   FAIL
== check lld             18 64 KiB windows; 416392 bytes (80%) in the main stretch, 14 windows   FAIL
== check mold            18 64 KiB windows; 416392 bytes (80%) in the main stretch, 14 windows   FAIL
```

The first version passed the real file to gold (it takes the option, as a list of section
names): 17 windows against the 16 above without it, `.text` 1413280 against 1413308 bytes, and a
different file (`cmp`: differ at byte 25): a changed layout for no saving. The control probe
(a file that is no order file) is what now skips it: GNU ld refuses that file, gold does not.

Idle `r-xp` `Rss` of the bar under headless scoot (`quick.sh`, 20 s settle, the tree's debug
`scoot`): cargo release build **640 kB**; `nix build .#scootbar` at the same tree **576 kB**.
The cargo release binary is **byte-identical** to the benchmarked one: sha256
`860185c2d678e3c8f42d65c445e73401133f36f37d6000da383638f5b0e1d893` (built at the head's code),
so the A-B evidence stands.
