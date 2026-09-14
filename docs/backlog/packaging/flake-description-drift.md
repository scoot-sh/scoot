---
title: "`flake.nix`'s top-level `description` and its package's `meta.description` are two independently hand-copied strings that can drift (NIT)."
status: "open"
area: "packaging"
priority: "nit"
blocked: null
---

# `flake.nix`'s top-level `description` and its package's `meta.description` are two independently hand-copied strings that can drift (NIT).

`flake.nix`'s top-level `description` and its package's
`meta.description` are two independently hand-copied strings that can
drift (NIT). Found by `flexwm-reviewer` reviewing item 16. Also, on
Darwin, `meta.description` still advertises "a scrolling-tiling Wayland
compositor that runs without a GPU" when the Darwin build is actually
just the `flexwm msg` client (`README.md` explains this correctly, but
`nix search`/`nix flake show` metadata would not). Cheap to fix whenever
`flake.nix` is next touched for another reason (e.g. the `src` filesetting
above) — bundling it there avoids paying for a second full evaluation/
rebuild cycle just for a string.
