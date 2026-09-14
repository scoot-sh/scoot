---
item: "16"
title: "nix build / nix run packaging"
status: "done"
area: "packaging"
pr: 26
commit: "868dd83"
---

# nix build / nix run packaging

`packages.<system>.default` is a real `rustPlatform.buildRustPackage`
derivation (no shelling out to cargo inside a shell), `apps.<system>.
default` points at its `flexwm` binary. Still one flake input; `cargoLock.
lockFile` needs no new machinery for a workspace, and `nativeBuildInputs`/
`buildInputs` reuse `vm/compositor-deps.nix` exactly as the dev shell
already does, so the package and the shell cannot drift apart.

**Three decisions worth the words:** (a) *the same package on macOS, not
a skipped output.* The crate already cfg's the compositor out on
non-Linux and `flexwm --headless` there exits with "the compositor only
runs on Linux", so the Darwin build is the `flexwm msg` client — exactly
what the dev shell's own Linux/Darwin split already assumes, and what
the README documents driving a VM with from a Mac. (b) *`doCheck =
false`.* The release profile sets `panic = "abort"`, which cargo ignores
for test targets, so `cargo test --release` rebuilds the whole dependency
tree unwinding — measured on Darwin (smallest tree): 10 crates recompile
after a complete `cargo build --release`, against 3 with `panic = "abort"`
removed; on Linux that second build would include Smithay. The
compositor's tests also need a writable `$XDG_RUNTIME_DIR` the sandbox
has no reason to provide. (c) *Smithay's git rev needs an
`outputHashes` entry* (a git source carries no crates.io checksum); a rev
bump fails the build loudly with the hash it got, so it can't drift.

No `LIBRARY_PATH` workaround is needed in the derivation, unlike the dev
shell's `shellHook`: the `-sys` crates' bare `-lfoo` resolves through
`NIX_LDFLAGS`, which the stdenv cc wrapper sets from `buildInputs`.

**Found while bug-bashing: the binary came out 1,224,312 bytes bigger
than this workspace's release profile asks for** — 4,843,776 against
3,619,464.
Cause, from nixpkgs' own source: `cargoBuildHook` exports
`CARGO_PROFILE_RELEASE_STRIP=false` ("let stdenv handle stripping"), and
stdenv's default takes debug info only, leaving `.symtab`/`.strtab` —
silently overriding the workspace profile's `strip = true`. Fixed with
`stripAllList = [ "bin" ]`. Measured, not assumed: `strip --strip-all` on
the unfixed binary reproduced exactly 3,619,464 bytes, kept
`cargo-auditable`'s non-allocated `.dep-v0` section (1,871 bytes, the
dependency manifest `nix build` embeds by default), and still ran.
Verified by building and running on the dev VM, not by inspection —
`scripts/smoke-test.sh` passes end to end against the Nix-built binary.
`flake.lock` is untouched (no new input), and the flake's `description`
lost its last "window manager" while it was open.
