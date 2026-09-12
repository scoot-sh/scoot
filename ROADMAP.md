# flexwm roadmap

The ordered list of milestones, worked through with the cycle in
`CLAUDE.md`. Update this file in the same PR that lands each item — it's the
authoritative, git-tracked history of what shipped, what was found in
review, and why.

## Order

1. ~~Nested backend~~ — DONE, merged to `main` at `494048b`. Independent
   review found 2 real low-probability bugs (resize-failure wedge,
   skipped-frame-never-retried) + 1 unneeded `unsafe impl Send`; all fixed
   and re-verified (`--headless`/`--nested` smoke test, 19 tests, clippy
   clean) before merge.

2. ~~Keybindings layer~~ — DONE, merged to `main` at `f3723b3`, PR #4. Vim
   motions (h/j/k/l) + Super, intercepted at `input::key`'s filter closure
   (the single choke point shared by IPC/nested/future-tty input). Matches
   unshifted level-0 keysym + tracked modifiers (not case-folded shifted
   symbols) — deliberate deviation from initial spec, matches niri/sway
   convention. Independent review found 2 issues, both fixed before merge:
   (a) `press()` could leak a stuck modifier if the combo's main key resolved
   by name but wasn't on the actual keymap — fixed by resolving the main
   keycode before pressing any modifiers; (b) documented an invariant on
   `suppressed_keys` (only `key()` may mutate it) against a desync risk
   relevant once the tty backend's libinput device-teardown exists.
   `crates/flexwm/src/compositor/keybindings.rs` holds the pure, unit-tested
   lookup table — the seam a config-file loader parses `flexwm_ipc::KeyCombo`
   strings into.

3. ~~Real tty/DRM backend~~ — DONE, merged to `main` at `0a90dc9`, PR #5.
   `--tty`: libseat session + raw `DrmSurface`/dumb-buffer scanout (no GBM —
   this Smithay rev has no `Bind<DumbBuffer>` for pixman, so presenting is a
   memcpy, confirmed necessary not just simplest) + libinput. Mirrors
   `nested::Host`'s shape (`Tty::present` same byte-slice signature). VT
   switching added as `keybindings::Bound::ChangeVt` (kept out of
   `flexwm_core::Action` — session concern, not layout, keeps the core
   platform-independent). Two independent review rounds; second found 3 real
   issues, all fixed: (a) `reactivate()` used to skip `libinput.resume()` on
   a failed `drm.activate`, which could strand the keyboard dead alongside a
   dead display with no recovery path — fixed so every recovery step runs
   independently; (b) `DrmEvent::Error` didn't free the pending buffer slot
   the way `VBlank` did, leaking it forever after repeated occurrences —
   fixed via a shared `flip_settled()`; (c) `build.rs`'s `pkg-config`
   build-dependency was wrongly gated behind `cfg(target_os = "linux")`,
   which broke `cargo check`/`cargo build` on the Mac dev host (build-deps
   compile for the *host*, not the target) — fixed by ungating it. Verified
   on the dev VM's real `virtio-gpu` KMS device: real scanout, VT-switch
   round trip (pause/reactivate + forced full modeset on the way back), idle
   CPU ~0, a real `/dev/uinput`-injected keystroke proven to reach a client.
   Out of scope, stated: cursor rendering, DRM hotplug, multi-GPU/output,
   DPMS, output scale, key-repeat.

4. Window decorations and a config file for "more visually pleasing options."
   Split into two PRs. **4a config file — DONE, merged to `main` at
   `3b62292`, PR #6.** `crates/flexwm/src/compositor/config.rs`: TOML at
   `$XDG_CONFIG_HOME/flexwm/config.toml` (falls back to `~/.config/flexwm/`),
   `--config PATH` flag; `[layout]` + `[binds]` sections. serde stays out of
   `flexwm_core` (conversion by hand in the `flexwm` crate). Failure
   semantics are the safety-critical part: explicit `--config` missing = hard
   error; default path missing = silent defaults; malformed/typo'd config =
   logs and falls back to full defaults, *never* blocks startup (a
   compositor that won't boot over a typo is a lockout with no recovery on
   real hardware). Bind collisions are detected and the whole group skipped
   rather than an arbitrary "last wins." `--tty`'s Ctrl+Alt+Fn VT-switch
   bindings always override a colliding config bind, applied after the
   config loads. 51 tests, verified via `scripts/smoke-test.sh` under all
   three backend modes including real `--tty` hardware.
   **4b decorations — DONE, merged to `main` at `21b9c3e`, PR #7.**
   `crates/flexwm/src/compositor/decorations.rs`: niri-style focus ring
   (width clamped to at most half the gap at load time) drawn in the
   layout's own gap + a background color (`render_output`'s `clear_color`,
   not an element — bottom-most by construction) + `zxdg_decoration_manager_v1`
   answering `ServerSide` when `prefer_no_csd` (default true).
   `headless.rs::render()` moved off `space::render_output` to
   `space_render_elements`+`OutputDamageTracker::render_output` directly so
   windows draw on top of the ring (element order is back-to-front,
   `.iter().rev()`). Ring buffers are persistent per-window (`SolidColorBuffer`,
   stable `Id`, updated not rebuilt). Verified via real pixel-sampled IPC
   screenshots across all three backends including real `--tty` hardware.
   `imagemagick` added to `vm/configuration.nix` for pixel-reading.

5. ~~Cursor rendering for `--tty`~~ — DONE, merged to `main` at `546534b`,
   PR #8. `crates/flexwm/src/compositor/cursor.rs`: a persistent,
   procedurally-generated arrow bitmap (no copied cursor-theme asset —
   deliberately license-clean per `CLAUDE.md`) rendered front-most, gated to
   `--tty` only. Damage-scoped presentation: `tty/buffers.rs` tracks each
   dumb buffer's own age (the two slots don't age in lockstep) so
   `tty::present()`'s memcpy scopes to the damaged bounding box instead of
   the full frame on every mouse-motion-triggered render. Independent review
   found two real issues, both fixed before commit: (a) an off-by-one in the
   age calculation (`generation - last_written + 1`, not `generation -
   last_written` — the render being peeked-for hasn't happened yet) that
   would have left stale pixels on the scanout buffer under slot alternation,
   invisible to every existing check since they all read the pixman
   intermediate, never the actual DRM buffer this lived in; (b)
   `write_region`'s pitch/offset copy math had zero test coverage, closed by
   extracting pure `age`/`copy_region_rows` helpers with regression tests.
   Both fixes are unit/source-verified against the pinned Smithay damage
   tracker, not empirically screenshot-verified — screenshots structurally
   can't observe the scanout buffer. Benchmarked via jiffies-delta: idle
   unchanged at 0, ~7 jiffies/150 small local moves vs ~42/150 near-full-frame
   corner jumps (the latter ≈ pre-fix cost) — confirms the scoping fix is
   real. `CursorImageStatus::Named`/`::Surface` both draw the same fallback
   shape — see Backlog.

   **5a. Merge needs a check-in every time, not a one-time unblock.**
   `gh pr merge` was denied by the auto-mode classifier ("Merge Without
   Review") when attempted autonomously on PR #8 — a harness-level gate the
   standing "merge authorized without further check-in" instruction can't
   override. The user then approved merging in chat for that specific PR;
   that approval doesn't carry forward to future PRs (see `CLAUDE.md`).

   **5b. ~~VT-switch-back `EPERM`~~ — DONE**, PR #9. Root cause confirmed by
   real reproduction on `--tty` hardware: `State::change_vt` (`tty/mod.rs`)
   had no gate on session-paused state. Real hardware input never triggers
   this (`SessionEvent::PauseSession` correctly calls `libinput.suspend()`
   first, pre-existing item 3) — but the IPC `key` request is a second,
   independent input path that bypasses `libinput` entirely, so switching
   away for real then sending the switch-back combo over IPC while genuinely
   paused reliably hit `libseat`'s `EPERM` on `VT_ACTIVATE`.

   First fix attempt gated on the existing `Tty::active` field and was
   **wrong, caught by the coordinating session before it reached a separate
   reviewer** — the incident `CLAUDE.md`'s "independent review is mandatory"
   section is named after: `active` means "DRM master held," not "session
   active" — it also goes `false` inside `reactivate()` when `drm.activate`
   itself fails, a state item 3's own review made deliberately recoverable
   ("dead DRM device, working keyboard, retry the VT switch"). Gating
   `change_vt` on `active` silently broke that retry path. Corrected fix: a
   separate `Tty::session_paused` field, set `true` only in the
   `PauseSession` arm and cleared `false` in the `ActivateSession` arm
   *unconditionally, before* `reactivate()` runs — so `session_paused` can
   never depend on whether that call's own `drm.activate` succeeds.
   `change_vt` gates on `session_paused`, not `active`. Re-verified on real
   hardware both ways: (1) the original IPC-while-paused repro still gets a
   clean debug-level skip, no `EPERM`; (2) after a real reactivation cycle,
   `change_vt` works normally again for a fresh switch-away/back over IPC.
   93/93 tests pass, clippy/fmt clean, cross-platform build clean on macOS.

   General note, not a fix: every IPC input request (`pointer_move`/
   `pointer_button`/`scroll`/`key`/`type`) bypasses `libinput`'s suspend the
   same way `change_vt` did — `change_vt` was the only one that turns into a
   hard `libseat` error, so it's the only one fixed here, but if IPC input
   reaching a VT-switched-away client ever matters later, this is the known
   cause, not a fresh investigation.

   Process note: a `pkill -f` self-match gotcha came up during this item's
   testing — a bare pattern that literally contains the invoking shell's own
   command line kills the calling shell too; worked around with the bracket
   trick `pkill -f 'flexw[m]'`.

6. A real GPU rendering pipeline, added at the end after everything above is
   stable — an actual goal, not just a "don't foreclose it" constraint.
   Likely a GLES/Vulkan Smithay renderer as an alternative to pixman,
   selected per-backend (e.g. tty backend prefers GPU when a real DRM/GBM
   device supports it, headless/nested/webtop keep pixman) rather than
   replacing CPU rendering outright, since GPU-free operation for
   webtop/no-GPU boxes stays a hard requirement. This is exactly why the
   render-target/presentation split from item 1 onward matters: it's the
   seam a GPU renderer slots into.

## Backlog (unordered — pick up whenever it fits)

- **Custom/client cursor support.** Item 5 shipped a fixed, procedurally-
  generated triangle for every `CursorImageStatus` variant — `Named` (a
  requested xcursor theme name) and `Surface` (a client-supplied cursor
  image, e.g. a text-input I-beam or a resize arrow) both currently draw the
  exact same shape, ignoring what was actually requested. Two independent
  pieces, either could land alone: (a) honor `CursorImageStatus::Surface` by
  rendering the client's actual supplied buffer as the cursor element (same
  render-element machinery `cursor.rs` already has, different source
  buffer) — likely the more valuable half; (b) a user/config-level override
  (cursor theme name, size, or color in `config.toml`'s `[appearance]`-shaped
  section) for the fallback shape itself. Keep the license constraint from
  `CLAUDE.md` in mind if (b) ever means loading a real xcursor theme (niri's
  assets are GPL, Adwaita's aren't MIT-clean).
