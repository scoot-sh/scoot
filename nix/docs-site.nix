# The docs site (site/): Astro Starlight over pnpm, built offline.
#
# `fetchPnpmDeps` vendors the locked dependencies into the store (hash-pinned,
# no network at build time beyond that hash); `pnpmConfigHook` installs them
# offline; then `astro build` renders the static site (including the
# `mdLinksToPages` integration, which rewrites relative `.md` links to pages
# in `*.html` only) and the five gate scripts run inside the sandbox:
# `check-llms` (llms.txt covers every page, every `.md` twin exists),
# `check-md-links` (no built HTML page links a raw `.md` twin, and every
# rewritten link resolves to a page in `dist/`), `check-assets` (every CSS
# `url()` and every HTML `src`/`href`/`srcset` naming an asset resolves to
# a file in `dist/`, base-aware; `test-check-assets` pins its fixture: one
# missing font plus one missing image must fail), `test-snippets` (fenced
# toml/sh blocks are sound in `.md` and `.mdx`), `test-mdx-twin` (the
# fixture `.mdx` page converts to plain Markdown with fences intact, and a
# broken variant trips the `<[A-Z]` JSX gate — both directions) and
# `check-nix` (`nix-instantiate --parse` over
# every fenced nix block, so the flake/module snippets cannot rot at the
# syntax level; evaluation is pinned in nix/tests.nix, which
# `nix flake check` runs).
# `$out` is the static `dist/` tree, ready for GitHub Pages.
#
# Reproducing the `pnpmDeps.hash` below: set it to `""`, run
# `nix build .#docs-site`, and copy the `got: sha256-…` value back here.
{
  lib,
  stdenv,
  nodejs-slim,
  pnpm,
  pnpmConfigHook,
  fetchPnpmDeps,
  nix,
}:

stdenv.mkDerivation (finalAttrs: {
  pname = "scoot-docs-site";
  # Keep in step with site/package.json.
  version = "0.1.0";

  # The site tree only, never a developer's `node_modules/` or a stale
  # root `dist/` (either would ride into the store; a stale `dist/` would
  # shadow the build). The `dist` exclusion is the site root only:
  # `test-check-assets`'s fixtures carry miniature `dist/` trees of their
  # own (`site/scripts/fixtures/`), which must reach the sandbox.
  src = builtins.path {
    path = ../site;
    name = "scoot-docs-site-src";
    filter =
      path: type:
      let
        base = baseNameOf path;
      in
      base != "node_modules" && path != ../site + "/dist";
  };

  pnpmDeps = fetchPnpmDeps {
    inherit (finalAttrs) pname version src;
    inherit pnpm;
    fetcherVersion = 4;
    hash = "sha256-+qelfN3saaBuFWIyOO3D9m+U8XQ72Cb5JAiWR+y+gxQ=";
  };

  nativeBuildInputs = [
    nodejs-slim
    pnpm
    pnpmConfigHook
    nix
  ];

  buildPhase = ''
    runHook preBuild

    pnpm build
    pnpm check
    pnpm check-md-links
    pnpm check-assets
    pnpm test-check-assets
    pnpm test-snippets
    pnpm test-mdx-twin
    pnpm test-nix

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    cp -r dist $out

    runHook postInstall
  '';

  meta = {
    description = "scoot documentation site (Astro Starlight, with llms.txt)";
    homepage = "https://github.com/scoot-sh/scoot";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
})
