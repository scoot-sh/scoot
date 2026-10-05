# The docs site (site/): Astro Starlight over pnpm, built offline.
#
# `fetchPnpmDeps` vendors the locked dependencies into the store (hash-pinned,
# no network at build time beyond that hash); `pnpmConfigHook` installs them
# offline; then `astro build` renders the static site and the two gate scripts
# run inside the sandbox: `check-llms` (llms.txt covers every page, every
# `.md` twin exists) and `test-snippets` (fenced toml/sh blocks are sound).
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
}:

stdenv.mkDerivation (finalAttrs: {
  pname = "scoot-docs-site";
  # Keep in step with site/package.json.
  version = "0.1.0";

  # The site tree only, never a developer's `node_modules/` or a stale
  # `dist/`: both would ride into the store (or worse, shadow the build).
  src = builtins.path {
    path = ../site;
    name = "scoot-docs-site-src";
    filter =
      path: type:
      let
        base = baseNameOf path;
      in
      base != "node_modules" && base != "dist";
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
  ];

  buildPhase = ''
    runHook preBuild

    pnpm build
    pnpm check
    pnpm test-snippets

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
