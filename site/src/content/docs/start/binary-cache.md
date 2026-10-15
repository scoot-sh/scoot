---
title: Use the binary cache
description: "Skip compiling Smithay: point Nix at the public Cachix cache and download scoot's prebuilt binaries."
---

Every merge to `main` pushes built binaries for `x86_64-linux` and
`aarch64-linux` to the public Cachix cache `scoot-sh`. Without it, Nix
compiles Smithay and scoot's crates on your machine (minutes); with it,
you download. Opt in once:

## Use it

NixOS (`configuration.nix`):

```nix title="configuration.nix"
nix.settings = {
  extra-substituters = [ "https://scoot-sh.cachix.org" ];
  extra-trusted-public-keys = [
    "scoot-sh.cachix.org-1:QMj7CMw8uqZxrvqqm6SggdxTHz6Q4prt30ydDcXJXCo="
  ];
};
```

Home Manager (your home config — writes the same `nix.conf` for your user):

```nix title="home.nix"
nix.settings = {
  extra-substituters = [ "https://scoot-sh.cachix.org" ];
  extra-trusted-public-keys = [
    "scoot-sh.cachix.org-1:QMj7CMw8uqZxrvqqm6SggdxTHz6Q4prt30ydDcXJXCo="
  ];
};
```

Per user without the modules (`~/.config/nix/nix.conf`), the same two lines:

```ini
extra-substituters = https://scoot-sh.cachix.org
extra-trusted-public-keys = scoot-sh.cachix.org-1:QMj7CMw8uqZxrvqqm6SggdxTHz6Q4prt30ydDcXJXCo=
```

Then check it worked:

```sh
nix show-config | grep -E "substituters|trusted-public-keys"
```

Non-interactive (scripts, containers): the flake carries these two
values as `nixConfig`, and Nix asks whether to accept them on first use:

```sh
nix run --accept-flake-config github:scoot-sh/scoot#scootbar -- --version
```

The flake's `nixConfig` applies only to trusted users. A per-user `nix.conf` is
**not** enough on a multi-user (daemon) install: Nix ignores
`trusted-public-keys` from an untrusted user ("ignoring the client-specified
setting ... you are not a trusted user"), so add the key to the system
config (or make yourself a `trusted-users` entry).

## What you are trusting

Binaries built by CI from reviewed merges to `main`, verified by this
key:

```sh
curl -s https://app.cachix.org/api/v1/cache/scoot-sh
```

The `publicSigningKeys` it reports must equal the key in the config
above. A substituter can serve any store path your Nix asks for, so this trusts
CI's builds the way installing the flake already trusts its source.
Whoever holds the signing key can make your Nix
accept arbitrary binaries, which is why the key lives only in CI secrets and
the rotation runbook below exists.
Only `trusted-users` (root, and NixOS's `trusted-users` list) may add
substituters through the flake's `nixConfig`; everyone else opts in
through the system config above.

## What the cache holds

All four installable outputs on both Linux systems, as of `main`:

| Package | x86_64-linux | aarch64-linux | Download |
|---|---|---|---|
| `scoot` | cached | cached | ~3.7 / 3.4 MB |
| `scootbg` | cached | cached | ~0.9 / 0.8 MB |
| `scootbar` | cached | cached | ~1.2 / 1.1 MB |
| `scootbar-demo` | cached (tiny wrapper) | cached (tiny wrapper) | under 1 KB over `scootbar` |

The CPU-only minimal build is cargo-only
(`cargo build --no-default-features`) and is not cached: there is no
published minimal package to push.

Not cached, by decision: the docs site (built where it deploys, never
installed), the dev shell, and macOS builds (macOS CI is check-only — install the Darwin
client from source). The bar's own closure stays small: seven paths, 48.5 MB total
on `x86_64-linux`, with no font inside (only `scootbar-demo` carries
one).

One `follows` warning: the flake pins its own nixpkgs revision, and
Cachix holds binaries built from exactly that revision. With
`scoot.inputs.nixpkgs.follows = "nixpkgs"`, every store path differs,
the cache misses, and you compile locally. (More in [the desktop
page](../desktop/index.md#set-up-the-flake).)

## Cold versus cached

A fresh empty store on an Asahi M2, asking only for the bar:

```sh
nix --store ~/fx/cache-demo/store run github:scoot-sh/scoot#scootbar --accept-flake-config -- --version
```

prints `scootbar 0.1.0` in 14.6 seconds with 7 paths fetched (1.1 MB
download, 58.6 MB unpacked) — the bar itself coming from
`scoot-sh.cachix.org`, nothing compiled.

> **Symptom:** it is compiling anyway (Smithay scrolls past for
> minutes).
> The cache only has `main`'s exact store paths, so any difference
> misses. Diagnose in order: is the cache configured at all, did the
> flake prompt get refused, is `follows` on, is your tree ahead of
> `main`, or is the package one CI never pushes.

Check the config is active:

```sh
nix show-config | grep -E "substituters|trusted-public-keys"
```

- **Empty, or missing `scoot-sh`:** the two config lines are not
  active — re-check which file you edited and rebuild / restart the
  shell.
- **The flake asked about `nixConfig` and you said no:** re-run with
  `--accept-flake-config`, or use the `nix.settings` lines so no
  prompt is needed.
- **`follows` is on:** `scoot.inputs.nixpkgs.follows = "nixpkgs"`
  changes every store path (see above) — turn it off or accept the
  compile.
- **A local change, or a commit ahead of `main`:** your tree's store
  paths do not exist yet — they land on the cache with the next merge
  to `main`.
- **macOS, or the docs site:** not pushed by CI (see the table) —
  expected to build locally.

To confirm a single path is cached without downloading it:

```sh
curl -s https://scoot-sh.cachix.org/HASH.narinfo | head -5
```

A `StorePath:` reply means cached; `404 - Not Found` means it is not.
Replace `HASH` with the store path's hash (the first component of
`/nix/store/HASH-name`).

## For maintainers: pushes, trust boundary, rotation

Pushes come only from trusted CI. `.github/workflows/nix-build.yml`
triggers on `push` to `main` and manual `workflow_dispatch` only —
never on `pull_request`. The authenticated
Cachix step runs only on `push` to `refs/heads/main`
and is the sole step that receives `secrets.CACHIX_AUTH_TOKEN`; every other run takes the pull-only step,
which passes no token and references no secret. Signing is Cachix-managed: the project holds no private
key. The FlakeHub publish job runs only after both architectures push,
on `push` to `main`, over GitHub OIDC with no secret — and publishes
the flake only, never binaries (`include-output-paths: false`).

What a poisoned entry would require: a push to `main` containing the malicious build, since
only that path holds the token. A writer could push a branch with the
workflow's own gates edited — but dispatching that branch still takes
the token-less step, so it verifies without publishing.

Rotation runbook (no rotation has happened yet):

```sh
curl -s https://scoot-sh.cachix.org/nix-cache-info
```

1. **Auth token suspected or staff changed:** rotate it in the Cachix
   dashboard (cache settings), then replace the `CACHIX_AUTH_TOKEN`
   org secret. The next merge to `main` proves the new token; a manual
   dispatch before that proves the build without pushing.
2. **Signing key rotation:** rotate in the Cachix dashboard, then update
   the public key in `flake.nix` (`nixConfig`), this page, and
   [Install](./install.md) in one PR.
   Keep the old key in `extra-trusted-public-keys` alongside the new
   one until every still-used path is re-pushed, then remove it in a
   follow-up.

Retention and size: Cachix garbage-collects per its plan. What
each merge adds is what changed: a code-only merge re-pushes only the
rebuilt packages (each a few MB compressed, see the table); a lockfile
or fork-rev change rebuilds the five compiled-dependency derivations
(~420 MB each in the store, ~130 MB compressed — about 690 MB per
architecture). Packaging notes for distro builders live under
[Package scoot offline](../reference/packaging.md).
