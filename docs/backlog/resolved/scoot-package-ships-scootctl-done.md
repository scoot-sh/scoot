---
title: "packages.scoot ships scootctl too, contradicting the documented package split — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# packages.scoot ships scootctl too, contradicting the documented package split — RESOLVED

## What it said

Filed as gh issue #172. `packages.scoot` had no `cargoBuildFlags`, so
`buildRustPackage` built the whole workspace and `$out/bin` carried both
binaries (`scoot` 5593312 + `scootctl` 529440 bytes, per the issue's `ls`
proof) — contradicting the flake's own `scootctl` package comment ("Just
this crate, not the whole workspace") and `docs/nix.md` ("the standalone
remote-control client"). Practical effect: `programs.scoot.enable` put a
redundant second client on `PATH`, plus 529 KB of closure nobody asked
for. Either fix direction was left open: `-p scoot`, or document the
bundle.

## Resolution

**`cargoBuildFlags = [ "-p" "scoot" ]` in the `scoot` package — packaging
matches the docs, not the reverse.** The split is the documented design
(the `scootctl` derivation comment, `docs/nix.md`, the module wrappers
treating the two as distinct installables), and the `scootctl` derivation
already demonstrates the exact shape, so there was nothing to design —
only to mirror. No fallback was needed: `-p scoot` builds cleanly on both
reachable systems and changes nothing structural.

Behavior-preserving for the shipped `scoot` binary, by construction, not
by assumption: the `scootctl` *binary* unit enables no features on any
shared crate (it depends on bare `scoot-ipc` plus `serde_json`; only the
`scoot` edge requests `scoot-ipc`'s `core` feature), so under
`resolver = "3"` the `scoot` unit graph is identical with or without the
second binary unit. The `scoot msg` alias is untouched — it is part of
the `scoot` binary, not a second one.

Docs touch-ups, only where text meets the new reality: the flake's
`packages` comment now pins "`-p scoot` so `$out/bin` carries only the
`scoot` binary", and `docs/nix.md`'s split sentence says "the compositor
alone". Module option descriptions mention no installed binaries beyond
"the binary"/"package" — no contradiction, left alone. Explicitly out of
scope (separate tickets, untouched): the gpu-scanout package question
(#177), CI coverage (#173), and all compositor/client code (zero `.rs`
files changed; `Cargo.lock` and `devShells` untouched).

## Evidence (record, branch `backlog/scoot-package-split`)

Where each command ran is stated; store paths are raw. Base: `main` at
`b042654`, dirty tree carrying only `flake.nix` + `docs/nix.md`.

- Mac (aarch64-darwin), `nix build .#scoot .#scootctl --print-out-paths`:
  `/nix/store/csf3ggg1bafcc0hbpjgh14rzrk3lk38p-scoot-0.1.0`,
  `/nix/store/c9178d373r39mi9kdq1v82ib4mfc1rcc-scootctl-0.1.0`.
  `ls` proof — `packages.scoot` bin holds ONLY `scoot` (621776 bytes),
  `packages.scootctl` bin holds ONLY `scootctl` (620624 bytes).
- Darwin `scoot` still honest: `<scoot>/bin/scoot --headless` prints
  `scoot: the compositor only runs on Linux; \`scoot msg\` works
  everywhere`, exit 1 (non-Linux `start_compositor` returns `Err`, and
  `main` maps any error to failure by design).
- Dev VM (aarch64-linux, via the 9p mount `/mnt/scoot`, verified to carry
  the fix at `flake.nix:96`),
  `nix build /mnt/scoot#scoot /mnt/scoot#scootctl --print-out-paths`:
  `/nix/store/q5pl3xh2mgh1dp4s5si93nf434ryga2h-scoot-0.1.0`,
  `/nix/store/cd40269lhprxcc70zcrlc03i0wvwcf9m-scootctl-0.1.0`.
  `ls` proof — `packages.scoot` bin holds ONLY `scoot` (5658968 bytes);
  `packages.scootctl` bin holds ONLY `scootctl` (529440 bytes — the exact
  figure from #172's pre-fix `ls`, i.e. exactly the redundant closure is
  gone).
- Linux builder (`linux-builder :31022`) was NOT usable: every remote
  derivation failed fetching from `cache.nixos.org` with `Problem with
  the SSL CA cert ... /etc/ssl/certs/ca-certificates.crt` — the known
  `NIX_SSL_CERT_FILE` launcher gotcha (`vm/README.md`), an environment
  fault predating this change (all failures are cache fetches, none reach
  the build). Not restarted per policy; the dev VM build above is the
  Linux proof instead.
- Module checks: `nix build
  /mnt/scoot#checks.aarch64-linux.scoot-modules` →
  `/nix/store/qwkqwnfv1f4ni55qribxa22kh5sqbzfb-scoot-modules-check`;
  Mac `nix flake check` green for darwin (`formatter`, both `apps`,
  `scoot-modules`; linux systems omitted — no working linux builder from
  this host, see above).
- Wrapper defaults still resolve to working packages (applied-eval of the
  flake wrappers, `package.content.pname`): linux HM + NixOS → `scoot`;
  darwin HM + NixOS → `scootctl` (= each system's `packages.default`).
  `apps.<system>.default/.scootctl` `program` paths point at the
  just-built store binaries on both systems; `nix flake show` lists the
  full `apps` table.
- Live consumer paths on the dev VM with the packaged binaries: nix-built
  `scoot --headless` boots (`scoot is up wayland="wayland-1"
  renderer=pixman`), packaged `scootctl windows` and packaged `scoot msg
  windows` against that session both return `{"type": "windows",
  "windows": []}`, exit 0 each.
- `nix fmt -- --check flake.nix` clean (flake formatter output).
- Benchmark: n/a — eval-time Nix change; nothing runs per-event or
  per-frame. No Rust code changed, so the cargo verification set does not
  apply (stated, not skipped silently).
