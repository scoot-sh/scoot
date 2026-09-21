---
title: "Docs gaps found converting a real NixOS config to the flake"
status: "open"
area: "meta"
priority: "medium"
blocked: null
---

# Docs gaps found converting a real NixOS config to the flake

Filed as gh issue #178 (read it — six numbered parts from actually doing
the conversion; this entry tracks them, it does not replace them).

1. **CHANGELOG rename entry misses the session-identity half.** The
   2026-09-18 entry covers binary/crates/socket/`$SCOOT_SOCKET`/config
   path/client-visible strings but not: `XDG_CURRENT_DESKTOP=flexwm` →
   `=scoot` (exported to every child), `DesktopNames=flexwm` →
   `DesktopNames=scoot` in session `.desktop` files, `flexwm-portals.conf`
   → `scoot-portals.conf`. These live in *other people's* config, which is
   why they bite.
2. **Repo move recorded nowhere user-facing.**
   `yackey-labs/flexwm` → `scoot-sh/scoot` appears only in the resolved
   rename record. It belongs in `CHANGELOG.md` (and arguably a line in
   README/`docs/nix.md`) — it's the URL consumers pin, and GitHub's
   redirect makes a stale origin silently work.
3. **`docs/nix.md` needs a "Migrating from a hand-rolled packaging"
   section**: `${pkg}/bin/flexwm` → `${pkg}/bin/scoot` in wrappers (builds
   fine, fails at the login screen — Nix never checks the binary exists);
   `xdg.configFile."flexwm/config.toml"` → module `settings` (stale path
   boots happily on defaults — the fail-safe looks like success, silently
   dropping scale and binds); the `homeManagerModules`/`nixosModules`
   split and that the HM module replaces hand-rolled `xdg.configFile`.
4. **Live-defaults reference advertises a renderer the package can't run.**
   `docs/nix.md`'s `--print-default-config` emission shows
   `backend = "pixman"` with no note that the flake package cannot do
   `gles` (see #177) nor `gpu-scanout`. Say what the *packaged* binary
   supports where the consumer decides. Same for `[tty] gpu`.
5. **No end-to-end session example.** The NixOS section shows
   `session.enable = true` (bare compositor, nothing running — see #171)
   with no worked greeter-entry + shell-inside-session + config example.
   ~20 lines (`writeShellScriptBin` wrapper `scoot --tty -- <shell>` +
   session package with `providedSessions` + `programs.scoot.settings`)
   would answer it — and doubles as what #171 eventually replaces.
6. **Smaller**: `docs/roadmap/README.md` row 6 says planned,
   `06-gpu-pipeline.md` says in-progress (also noted in #177);
   `docs/nix.md` never mentions `scoot msg reload` (edit-rebuild-reload is
   the actual loop — one line in the home-manager section).

Docs-only. Verify every added claim by running (emission output, flake
eval, help text) — the file-audit bar from the README rewrite applies.
