# Backlog

Everything not yet scheduled or done, split by area. Each entry is one file
with YAML frontmatter (`title`, `status`, `area`, `priority`, `blocked`), so
the set is filterable without parsing prose:

```sh
rg -l 'status: "open"' docs/backlog                  # everything live
rg -l 'priority: "high"' docs/backlog                # what to pick next
rg 'blocked: ' docs/backlog/**/*.md | rg -v ': null' # what is waiting on something
```

`ROADMAP.md` is the index and carries the ordered work; this directory is
the detail. Prose is the original backlog entry, moved verbatim.

**Resolved entries** (struck through in the old monolith, kept for the
diagnosis history) live in [`resolved/`](./resolved/). They are archived, not
actionable.

## High priority

- [**`ext-idle-notify-v1` + `idle-inhibit-unstable-v1`**](./protocols/ext-idle-notify.md)
  — the automatic trigger session-lock currently lacks. The next protocol pick.

## Open

### Protocols
- [`xdg-activation-v1`](./protocols/xdg-activation.md) — clients asking each other to be focused
- [Foreign-toplevel management](./protocols/foreign-toplevel-management.md) — window enumeration for taskbars/switchers
- [`cursor-shape-v1`](./protocols/cursor-shape.md) — named cursor shapes without client-side themes
- [Popup input](./protocols/xdg-popup-input.md) — popups map/draw (initial-configure fix, resolved), but keyboard focus/grabs and layer-parented popups still missing
- [Layer surface with no buffer still holds its exclusive zone](./protocols/layer-surface-bufferless-exclusive-zone.md)
- [Unbounded `ext_workspace_manager_v1` binds per client](./protocols/ext-workspace-object-binding-cap.md)
- [Lock surfaces are per-output; flexwm has one output](./protocols/lock-surfaces-per-output.md)
- [Unbounded lock surfaces via duplicate `wl_output` binds](./protocols/lock-surface-duplicate-wl-output.md)
- [Lock blanks immediately instead of waiting for the first surface](./protocols/session-lock-blank-timing.md)
- [`locked` sent on rendered frame, not confirmed vblank](./protocols/session-lock-vblank-confirm.md)
- [Lock manager global offered to every client](./protocols/session-lock-global-restriction.md)
- [First click on a fresh lock screen reaches nobody](./protocols/session-lock-first-click.md)
- [Smaller/general protocol gaps (bundled)](./protocols/protocol-gaps-general.md)
- [Niche protocol gaps (bundled)](./protocols/protocol-gaps-niche.md)

### IPC / computer use
- [Targeted input injection without moving seat focus](./ipc/targeted-input-injection.md) — the computer-use gap (research)
- [`flexwm msg outputs` reports only the full rect](./ipc/msg-outputs-usable-rect.md) — bundle with a `PROTOCOL_VERSION` bump
- [No IPC way to ask whether the session is locked](./ipc/ipc-session-locked-query.md) — same bump
- [Screenshot capture is synchronous, no connection cap](./ipc/screenshot-sync-no-rate-limit.md)
- [A large `msg type` blocks the event loop](./ipc/msg-type-blocks-event-loop.md)

### Input
- [`msg key` hard-codes `_L` modifier keysyms](./input/msg-key-modifier-resolution.md)
- [`msg type`: dead keys, compose, inactive layouts](./input/msg-type-dead-keys-compose.md)

### --tty / backend
- [Config-file key for the DRM device](./tty/tty-gpu-config-key.md) — **gated on Asahi confirmation**
- [Background color not painted where no window covers](./tty/tty-background-not-painted.md)
- [Same-VT no-op VT-switch warning](./tty/vt-switch-same-vt-warning.md)
- [`O_CLOEXEC` request is a no-op at the libseat layer](./tty/tty-o-cloexec-noop.md) (informational)

### Core / config / rendering
- [`--width`/`--height` are unbounded `i32`s](./core/width-height-unbounded.md)
- [`[binds]` capital letter parses but never fires](./config/binds-capital-letter.md)
- [Per-frame `Vec` alloc in the cursor fallback path](./rendering/cursor-element-per-frame-alloc.md)
- [Cursor frames still tick while VT-paused](./rendering/cursor-frame-callback-when-paused.md)
- [`wl_surface.offset` doesn't move the cursor hotspot](./rendering/cursor-surface-offset-hotspot.md)
- [Cursor theme *name* support](./rendering/cursor-theme-name.md) — blocked on a license-clean asset

### Security
- [Unbounded total shm reservation per client](./security/shm-total-per-client-unbounded.md)

### Packaging / tooling
- [Nix `src = self` invalidates the build on doc-only edits](./packaging/nix-src-fileset.md)
- [`x86_64-darwin` in `systems` breaks `flake check --all-systems`](./packaging/flake-x86-darwin-system.md)
- [`flake.nix` description drift](./packaging/flake-description-drift.md) (nit)
- [`smoke-test.sh` hardcodes temp paths](./testing/smoke-test-hardcoded-temp-paths.md)
- [`smoke-test.sh` background check samples the cursor under `--tty`](./testing/smoke-test-background-cursor-sample.md)
- [Enhanced hardware/DRM testing ideas](./testing/hardware-testing-ideas.md) (research)

### Meta
- [Rename `flexwm` → `flex` / `flexctl` / `flexbar`](./meta/rename-flex-family.md)
