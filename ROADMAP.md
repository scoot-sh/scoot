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

   **5a. Merge authority history.** `gh pr merge` was denied by the
   auto-mode classifier ("Merge Without Review") when attempted autonomously
   on PR #8 — a harness-level gate. The user approved merging PR #8 in chat;
   for PR #9 that approval didn't carry forward and a fresh check-in was
   asked for and given. After that, the user made it standing policy (see
   `CLAUDE.md`): once `flexwm-reviewer` has reported back and the
   coordinating session has actually weighed the diagnosis, merge without
   asking again — first exercised merging PR #9 and #10 together.

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
   clean skip, no `EPERM`; (2) after a real reactivation cycle, `change_vt`
   works normally again for a fresh switch-away/back over IPC. 93/93 tests
   pass, clippy/fmt clean, cross-platform build clean on macOS.

   `flexwm-reviewer`'s first formal pass (PR #9): **no blocking findings** —
   independently re-verified the whole test/clippy/fmt trio, traced every
   read/write site of both `active` and `session_paused` and confirmed they
   never conflate (the two write-site sets are structurally disjoint, not
   just correct by current ordering), checked the fix's premise against the
   pinned Smithay source (`Event`'s two variants are exhaustive, no third
   path can move session state behind flexwm's back) and against the actual
   `seatd` binary's own refusal strings on the dev VM. Two small
   observations addressed before merge: the skip's log level was raised
   `debug!` → `info!` (an explicitly requested action being silently
   discarded needs a trace at the default log level, not just "no spurious
   error"), and `session_paused: false` at `init` — which asserts a state
   rather than observing an event, the same shape as the mistake that made
   `active` wrong the first time — got a doc comment explaining why it's
   actually safe (`init`'s own `session.open()` call already fails first if
   the session isn't active). Two more were logged as backlog, not fixed
   here: an existing pre-PR `warn!` on "already on that VT" that predates
   this change, and the IPC-VT-switch note below, sharpened by this review.

   General note, not a fix, sharpened by `flexwm-reviewer`'s pass: every IPC
   input request (`pointer_move`/`pointer_button`/`scroll`/`key`/`type`)
   bypasses `libinput`'s suspend the same way `change_vt` did — `change_vt`
   was the only one that turned into a hard `libseat` error, so it's the
   only one fixed here. The concrete, sharper version of why this matters
   for `flexwm-vision`'s "an agent doing computer-use tasks in a VM is a
   first-class client" goal: an agent that sends a VT-switch-away over IPC,
   then a VT-switch-back over IPC, gets a clean skip instead of an error on
   the second call — but the compositor is still off-screen, and IPC is the
   agent's only input channel, so it cannot un-pause itself. An
   IPC-initiated VT switch away is currently a one-way door for anything
   whose only input is IPC. See the backlog entry below.

   Process note: a `pkill -f` self-match gotcha came up during this item's
   testing — a bare pattern that literally contains the invoking shell's own
   command line kills the calling shell too; worked around with the bracket
   trick `pkill -f 'flexw[m]'`.

   **5c. ~~IPC-initiated VT switch is a one-way door~~ — DONE**, PR #11.
   Picked up from the Backlog, sharpened by `flexwm-reviewer`'s PR #9 pass
   (see 5b above). Resolution chosen deliberately over gating/rejecting: 5b's
   own hardware bug-bash relies on IPC being able to trigger `ChangeVt`, so
   blocking it there would break that test path. Instead the *reply itself*
   now tells an agent whose only input/output is IPC what just happened,
   since IPC has no route to a log line the way a human at the console does.

   `Tty::change_vt` (`tty/mod.rs`) now returns a new `VtSwitchOutcome`
   (`Requested`/`Ignored`/`IgnoredPaused`/`Failed`) instead of `()`, threaded
   up through `input::key`'s new `KeyOutcome { intercepted, vt_switch }`
   (replacing its old bare `bool`) and `press()`'s new
   `Result<Option<VtSwitchOutcome>, String>` (replacing `Result<(), String>`,
   accumulated with `Option::or` across every press in the combo, not just
   read from the main key, so it stays correct even if a future binding
   changed which key can carry one) to `ipc.rs`'s `Request::Key` handler —
   `press()`'s one caller. A new `Response::Warning { message }` variant
   fires for `Requested` and `IgnoredPaused`; `Failed` gets `Response::error`
   (libseat itself refused the request — not "success with a side effect,"
   an actual failure); plain `Ignored` (no `--tty` backend at all) and no VT
   binding matched stay a plain `Ok`. `PROTOCOL_VERSION` bumped 1 → 2:
   `Response` is internally tagged, and this project's own
   `unknown_request_types_are_rejected` test proves an unrecognized tag is a
   hard decode error for an older client, not something it can shrug off —
   so this addition does break old clients' decoding, contrary to the first
   pass's assumption that it was purely additive. `flexwm msg` prints a
   `Warning`'s JSON to stdout the same way every other response does (so
   `flexwm msg key ... | jq .` still works and reflects what happened) plus
   a human-readable line on stderr; exits `0` either way (a warning isn't a
   failure, `Response::error` still is).

   `Requested` means "libseat accepted the request," not "a switch is
   guaranteed" — found empirically on the dev VM, not assumed: requesting
   the VT this session is *already showing* also returns `Ok(())`, with
   `seatd`'s own log reading "Could not set next session: requested session
   is already active" and no pause at all. `libseat_switch_session`'s own C
   doc says the same thing plainly ("does not imply that a switch will
   occur"). The warning's wording accounts for this directly ("requested...
   if it takes effect...") rather than asserting a pause that may not
   happen — deliberately still warning on this no-op case rather than
   trying to suppress it (that would need tracking which VT this session
   currently occupies, which nothing here does today — see the Backlog entry
   below).

   **`flexwm-reviewer`'s first pass on this item (against the pre-fix
   version) found real issues, all fixed before this write-up**: (1) the
   actual ticket gap — the switch-*back*-while-paused retry over IPC still
   replied with a bare `Ok`, with the "why nothing happened" explanation
   only in the compositor log, i.e. exactly the ambiguity 5b's own problem
   statement calls out. Root cause: the original single `Ignored` variant
   collapsed "no `--tty` backend" and "session is paused" into one case,
   so `ipc.rs` couldn't warn on the second without also (wrongly) warning
   on the first. Fixed by splitting `Ignored`/`IgnoredPaused` as described
   above. (2) `Failed` silently read as success (`Response::Ok`) — fixed to
   `Response::error`. (3) the warning text claimed "only real hardware
   input... can reactivate it," which the review's own hardware repro
   disproved (`chvt 1` from an ordinary ssh shell reactivated it, no
   physical keyboard involved) — reworded to "a VT switch from outside this
   compositor — a physical Ctrl+Alt+Fn, or `chvt N` from any shell on this
   machine." (4) the `PROTOCOL_VERSION` bump above, confirmed necessary by
   the project's own decode-error test rather than left unbumped on a
   "probably fine" assumption. (5) two doc-comment inaccuracies: `KeyOutcome`
   claimed `keyboard.input`'s `None` case meant "no active keyboard focus
   target," but the pinned Smithay rev's `input_from_source` (`src/input/
   keyboard/mod.rs`) shows `None` actually comes from either a keycode
   already held by another input source, or the filter returning `Forward`
   — focus isn't involved; and `VtSwitchOutcome`/`press()`'s docs said a
   switch-away makes the compositor "unreachable over IPC," which this same
   PR's own hardware evidence disproves (`flexwm msg key`/`flexwm msg
   windows` both work fine while paused) — narrowed to what's actually true:
   the one channel that could switch the session *back* is what's lost, not
   IPC reachability generally. (6) `press()` read `vt_switch` from only the
   main key's press, correct today only because nothing but the hardcoded
   VT bindings constructs a `ChangeVt` — made robust instead of
   documentation-dependent via the `Option::or` accumulation described
   above. Re-verified in full afterward: 93/93 tests, clippy/fmt clean on
   the dev VM guest, cross-platform build clean on macOS,
   `scripts/smoke-test.sh` full pass under `--headless` (confirming
   `Ok(None)` — no `ChangeVt` binding outside `--tty` — stays a plain `Ok`
   with no spurious warning where this doesn't apply), and all five
   `--tty` hardware scenarios re-run on the dev VM's real `seatd`-backed
   session, including the previously-missing one: retrying the switch-back
   combo over IPC while genuinely paused now gets `Response::Warning`
   ("ignored: this session is already paused..."), not a bare `Ok`, and
   still no `EPERM` (5b's fix unregressed). Exact commands and raw
   command/output blocks for every scenario are recorded in PR #11's
   description rather than narrated here. `Failed → Response::error` is
   code-traced, not hardware-exercised — there's no cheap way to force a
   real libseat error on this VM, same caveat 5b made about forcing
   `drm.activate` to fail. No unit test constructs a live `Tty`/`State` to
   exercise any `VtSwitchOutcome` variant directly — same reasoning as 5b:
   no existing fixture for that, hardware bug-bash plus code tracing is this
   project's established way of verifying this class of session-state
   correctness.

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

- **Suppress the same-VT no-op case of the IPC VT-switch warning (5c).**
  `change_vt`'s `VtSwitchOutcome::Requested` also fires — with a hedged
  warning, per 5c — when the requested VT is the one the session is already
  showing on, since libseat itself returns `Ok(())` for that request too
  (confirmed on real hardware; see 5c's verification). A precise fix would
  need `Tty` to track which VT it currently occupies and compare before
  calling `session.change_vt`, so this case can be `Ignored` instead of a
  hedged `Requested`. Not done in 5c because libseat exposes no query for
  "what VT is this session on" at `init` time — the number would have to be
  sourced some other way (the kernel's own active-VT ioctl on the console
  fd, perhaps) and 5c's warning-not-guarantee wording already makes this a
  false-positive risk, not a silent-failure one, so it wasn't worth blocking
  5c on. Low priority: cosmetic (an agent gets told to be careful once for
  no reason), not correctness-affecting.

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
