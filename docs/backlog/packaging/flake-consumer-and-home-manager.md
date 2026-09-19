---
title: "No documented way to consume scoot from another flake, and no home-manager or NixOS module"
status: "open"
area: "packaging"
priority: "medium"
blocked: "sequenced behind the `scootctl` split and milestone 6 (user, 2026-09-19) — not a technical block"
---

# No documented way to consume scoot from another flake, and no home-manager or NixOS module

Requested 2026-09-19. scoot ships a flake and `README.md` explains how to
build and run it *from a clone* — but nothing tells a Nix user how to add
it to **their own** configuration, which is how a NixOS user actually
installs a compositor.

This has two halves, and only the first is documentation.

## Half 1 — document what already works (small)

Verified at `056bba6`, `flake.nix` exposes `packages`, `apps` and
`devShells` per system. So consuming it today already works:

```nix
inputs.scoot.url = "github:scoot-sh/scoot";
# then, in a module:
environment.systemPackages = [ inputs.scoot.packages.${pkgs.system}.default ];
```

That is enough to install the binary, and it is currently written down
nowhere. It belongs in `README.md`'s Install section — which the README
rewrite deliberately kept short, so this may be a short block plus a link
to a fuller `docs/` section rather than the whole thing inline.

Also worth stating honestly there: the flake covers **Apple Silicon and
Linux**, and on macOS the build gives the `scoot msg` client only.

## Half 2 — the module that does not exist (real work)

`flake.nix` exposes **no `overlays`, no `nixosModules`, no
`homeManagerModules`**. A user who wants what other compositors offer —

```nix
programs.scoot.enable = true;
programs.scoot.settings = { layout.gap = 8; };
```

— has to write the module themselves, including generating the TOML,
placing it at `$XDG_CONFIG_HOME/scoot/config.toml`, and wiring the session
(greetd entry, `.desktop` file, or `startwm.sh` for the webtop case).

Decisions this needs, which is why it is a ticket and not a patch:

1. **home-manager module, NixOS module, or both?** The config file is
   per-user, which points at home-manager; the session/greetd wiring is
   system-level, which points at NixOS. Most compositors ship both, thin.
2. **Does `settings` get a typed schema or a free-form attrset** rendered
   straight to TOML? Typed catches errors at build time and is what makes
   the module worth having; free-form never goes stale. `[binds]` is an
   arbitrary key/value table, which pushes toward free-form for that
   section at least.
3. **Does the module own the session entry?** Generating a greetd or
   display-manager entry is where the real convenience is, and also where
   the risk is — `CLAUDE.md` is explicit that a user must never be stranded
   out of their own desktop, so a module that makes scoot the session needs
   the same care as the `--tty` path itself.

## Related

`docs/backlog/config/default-config-command.md` — if a module renders the
config, the emitted-defaults work and the module's schema want to agree
about what the defaults are rather than drifting apart.
