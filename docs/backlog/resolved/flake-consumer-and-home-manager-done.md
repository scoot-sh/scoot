---
title: "No documented way to consume scoot from another flake, and no home-manager or NixOS module — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# No documented way to consume scoot from another flake, and no home-manager or NixOS module — DONE

Requested 2026-09-19. Resolved 2026-09-20 as decided, with the ticket's
shape intact (both halves: documented consumption + both modules, thin).
No compositor or client code touched — Nix, docs, and eval tests only.

## What landed

- **Half 1 (docs):** README Install carries the consumer snippet plus a
  pointer; the fuller section is the new `docs/nix.md` (consumption,
  both modules' options, platform notes, session-entry behavior, portals
  install, failure modes, live-defaults reference).
- **`nix/modules/home.nix`** (`programs.scoot`): `enable`, `package`
  (null default — see below), free-form `settings` via
  `pkgs.formats.toml`, `configFile` path override, `sessionScript`
  (executable `scoot/session.sh`), `portals.enable` (default true)
  installing `resources/scoot-portals.conf` to the per-user
  xdg-desktop-portal lookup path — closing the remainder handed off by
  the session-environment ticket, which this module now owns.
- **`nix/modules/nixos.nix`** (`programs.scoot`): `enable` (installs
  `package`), `package` (null default + loud eval assertions), and
  `session.enable` (default false) adding a `scoot.desktop`
  (`Exec=<pkg>/bin/scoot --tty`, `DesktopNames=scoot`,
  `providedSessions = [ "scoot" ]`) to
  `services.displayManager.sessionPackages` — additive, verified
  against the pinned nixpkgs source.
- **`nix/tests.nix`** wired as `checks.<system>.scoot-modules`:
  standalone `evalModules` with stub options plus rendered-file content
  checks (empty → valid minimal, binds quoting round-trip, relocate,
  portals content, session-script shebang+exec, `.desktop` Exec,
  wrong-type renders).
- **Flake outputs:** `homeManagerModules.{default,scoot}` and
  `nixosModules.{default,scoot}`, each a one-`mkDefault` wrapper over
  the pure module defaulting `package` to the flake's own per-system
  build. Docs pointers updated (`docs/configuration.md` HM + portals
  paragraphs now point at `docs/nix.md`).

## Decisions (coordinator's three + two made explicit in the work)

1. **Both modules, thin** — shipped as decided. Config is per-user
   (home-manager), session wiring is system-level (NixOS); precedent
   (most compositors ship both) held.
2. **Free-form `settings`, no typed schema** — shipped as decided, with
   the reasoning recorded in the module comments: the config moves fast
   (three tables landed this month), a typed schema goes stale and then
   lies, `[binds]` is arbitrary keys anyway. Typed options exist only
   where they carry behavior beyond the file.
3. **Session entry additive, default OFF** — shipped as decided, with
   the reasoning recorded: the module never touches
   `defaultSession`/`default_session`/`initial_session`, and the
   login-screen change stays an explicit opt-in per the never-strand
   rule. Edge found while verifying: on an autoLogin box with no
   explicit `defaultSession`, adding *any* session package can move the
   autologin target (nixpkgs falls back to the list head) — documented
   in `docs/nix.md` (pin `defaultSession` explicitly), not fixed in
   code (nothing to fix; it is nixpkgs' fallback, and ours is one more
   list element).
4. **Portals default ON with `enable`** — decided in the work: the file
   is inert until a session names `XDG_CURRENT_DESKTOP=scoot`, and the
   per-user slot is highest-precedence, so it changes nothing outside a
   scoot session. Behind `portals.enable` for whoever manages backends
   otherwise.
5. **No overlay; `package` null-default + flake `mkDefault` wrapper** —
   decided in the work (the ticket listed overlays as missing but the
   decisions didn't ask for one): an overlay would duplicate the build
   graph into consumers' nixpkgs for no behavioral gain, and without
   one no `pkgs.scoot` exists to default to. The wrapper keeps one
   source of truth (the flake's own build) while the pure modules stay
   testable without the flake.

## Where the coordinator brief proved wrong (flagged, not silent)

- **"TOML render error surfaces at build"** (brief's bug-bash
  parenthetical for wrong-typed settings): wrong for free-form
  settings, by construction. A representable-but-wrong scoot type (a
  string for `layout.gap`) type-checks and *renders* — the refusal
  happens at session start, fail-safe (whole file discarded for
  defaults, session boots; proven live). What fails loud-and-early is
  the *non-representable* value (a function): the option type-check
  rejects it ("not of type 'TOML value'") at evaluation time, before
  anything builds (verified manually against the pinned nixpkgs). The harm
  the brief feared — a module writing a config the compositor rejects
  at startup, stranding a greetd session — does not materialize either
  way: the loader never refuses to boot over config content (the two
  deliberate startup errors are `[tty] gpu` and `[renderer] gles`,
  neither reachable from a type slip in `settings`).
- **"Stronger pin if cheap: empty-settings TOML parses to
  `Config::default()`"**: not built as a Rust-side assertion — with
  free-form settings there is no schema to drift, so the agreement
  requirement reduces to "the documented example matches the live
  emission". Kept as process, stated in `docs/nix.md` (pasted from a
  real emission with rev + date, regenerate-don't-edit): the mechanical
  pins are empty-renders-valid (tomllib) plus the live round-trip
  (below).

## Evidence (recorded, not narrated)

- `nix flake show` (Mac, aarch64-darwin): evaluates clean incl. new
  `homeManagerModules`/`nixosModules`/`checks` outputs.
- `nix flake check` (Mac): green, incl.
  `checks.aarch64-darwin.scoot-modules` (previously built).
- `nix flake check /tmp/scoot-flake` (dev VM, aarch64-linux, tree copy
  shipped by tar to dodge 9p permission noise): `all checks passed!`
  with all seven content oks in the build log
  (`scoot-modules-check> ok: ...` × 7).
- Two findings from probing the failure paths by hand (both fixed,
  both now pinned):
  - `session.enable` with `package = null` first failed with a bare
    `cannot coerce null to a string` (the `Exec=` interpolation blows
    up while evaluating the config value, before `assertions` are
    checked) instead of the module's helpful message. Fixed with a
    `cfg.package != null` conjunct on the `optional` (commented as
    load-bearing in `nixos.nix`); verified the helpful assertion text
    now surfaces.
  - Bare `lib.evalModules` collects `config.assertions` but never
    enforces them (enforcement is the host module system's job), so a
    check that merely forces the attribute proves nothing. The checks
    assert directly on the collected values instead — all-true for
    every configuration under test, all-false pinned for the known-bad
    combination above.
- Loader round-trip (dev VM, debug build of unmodified `main`
  `82371df` — this branch touches no `.rs` — at
  `/tmp/scoot-build/target/debug/scoot`, module-rendered bytes taken
  from the check's own store paths):
  - full settings (`.../0wqg5kblf409qv8j9pxgz6r2hs8yq106-scoot-config.toml`,
    gap/binds incl. quoting-needing `super+shift+/`/autostart):
    `--headless --config` boots, `msg windows` answers
    `{"type":"windows","windows":[]}`; log shows exactly one bind
    warning (the `/` bind, unknown keysym — per-bind fail-open, rest
    loaded) and one autostart warning (`spawn waybar` absent on the VM
    — per-entry fail-open, session up).
  - empty settings (`.../dcbs...-scoot-config.toml`, 0 bytes): boots
    silent, answers IPC — pure defaults.
  - wrong type (`.../kl57...-wrong-type.toml`, `gap = "wide"`): boots,
    answers IPC, log carries `could not parse config file; using
    defaults ... TOML parse error at line 2, column 7` — fail-safe live.
- `nixfmt --check` clean on all new/changed `.nix`. (`nix fmt` prints
  `unexpected end of input / expecting expression` — also on pristine
  `main` (verified via throwaway worktree), pre-existing, out of scope;
  `vm/compositor-deps.nix` was already unformatted per current nixfmt
  before this branch.)
- `cargo` suite untouched: no `.rs` files changed (`git status` shows
  only `flake.nix`, `nix/`, `docs/`, `README.md`).

## Left out, with why

- **Overlay** (`overlays.default`): see decision 5 above.
- **`extraPackages` for portal backends** (wlr/gtk/grim): genuinely
  useful, but outside the decided scope; without them portals degrade
  (documented in the conf), nothing fails. Fits a follow-up, not this
  ticket.
- **Display-manager integration beyond the entry file** (greetd
  `default_session` snippets, GDM defaults): explicitly out per the
  ticket's scope; the entry is the additive primitive, switching stays
  the user's system config.
- **Binary-cache/CI changes**: `nix flake check` required nothing.

