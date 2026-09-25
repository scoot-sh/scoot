# Milestone 19: multi-output (more than one monitor at a time)

Promoted from `docs/backlog/core/multi-output.md` (still the detailed spec —
read it first; this file is the staging plan, not a rewrite). The biggest
user-facing gap scoot has; `README.md`'s "Not yet" list leads with it.

## What already landed

The foundation (PR #150, `State.outputs` id-keyed collection,
`--headless --outputs N` creating N virtual outputs side by side, each
with its own `wl_output`, logical position, and scrolling strip in the
core). Almost everything below is therefore testable on the dev VM with
no second monitor. `crates/scoot/src/compositor/outputs.rs`'s doc on
`Outputs::primary` enumerates the single-output-assumption sites — every
`primary()` caller is one — and is more precise than a fresh survey.

## Decisions (coordinator's, recorded before Phase A)

- **Scrolling strip: one per output** (already the foundation's shape, and
  niri's answer). Not revisited.
- **Focus across outputs reuses the within-output rule.** No new
  focus-follows-pointer doctrine: the pointer position picks the output,
  then the existing focus derivation applies unchanged. If the code proves
  the existing rule assumes singleness somewhere, that site is a bug in
  this milestone's scope, not a reason to invent a second rule.
- **`scoot-core` models strips per output id; the compositor owns outputs.**
  No core redesign: the foundation's split stands unless a phase proves a
  workspace-set-per-output concept must move into the core (flag it, don't
  silently redesign).
- **Renderers: both must work.** Pixman-first implementation is fine, but
  every phase proves its pixels under `SCOOT_TEST_RENDERER=gles` too (the
  coalesce precedent: byte-identical suites under both). No GLES-only or
  pixman-only per-output behavior without a measured reason.
- **Single-output stays byte-identical.** Every phase keeps the one-output
  session pixel- and wire-identical (existing suite + screenshot hashes);
  multi-output is additive.

## Phases (in order — each is one implementer invocation + review + merge)

- **A. Render + capture per output.** One render target per output
  (`State::render` draws the primary's framebuffer and nothing else today
  because `State.backend` is one `Backend`). Un-refuse `screenshot
  --output N`; serve `screencopy` of non-primary outputs
  (`capture_constraints`); per-output gamma (`get_gamma_control`);
  per-output frame callbacks. Exit: two virtual outputs each show their
  own strip live; screenshot/screencopy per output pinned; single-output
  byte-identical.
  - **DONE (2026-09-21, Phase A PR).** `State::backend: Option<Backend>` is
    now `State::backends: HashMap<OutputId, Backend>` — one render target
    per output, built with the session's renderer by both `init_named` and
    `add_output`, so no output ever exists without its own pixels. `render`
    walks outputs in creation order, taking and returning each target (no
    per-frame allocation: an `Arc` bump per output plus the frame's own
    elements). Screenshots resolve the asked output's target
    (`screenshot_refusal` now refuses only unknown ids); screencopy records
    each session's output at `new_session` and serves each output's due
    sessions from its own read-back; gamma holds one live control per
    output with transfers scoped to the output; frame callbacks go to the
    windows/layers overlapping each output's own geometry. Single-output is
    byte-identical: full nextest + `cargo test` green, and a 400x300 empty
    IPC screenshot hashes identically (`0d5affbe…fc0db`) on main and on the
    branch. Live proof on the dev VM (`--headless --outputs 2`): a `foot`
    window on output 1 appears in output 1's IPC screenshot and `grim -o
    headless` but not in output 2's (`w2`/`g2` hash the empty session);
    `wayland-info` shows both `wl_output`s. Benchmarks (release, dev VM,
    800x800, min-of-5 with ranges stated alongside — never best-of without
    spread, per project rule): single-output before/after overlapping (pixman
    empty 74–77µs → 67–75µs; the 8-window scene is noisier, 100–131µs →
    68–145µs across runs — noise dominates, no structural regression, the
    added work is O(1) map ops); two-vs-one scaling ~2x pixman
    (47→101µs empty, 68→145µs 8-window) and ~1.5x gles/llvmpipe
  (1.70→2.58ms empty). Per-output memory: one framebuffer (~6.4 MiB at
  1600x1000 pixman, same for the GLES renderbuffer plus its own EGL
  display/context) plus its damage tracker per output. `locked`
    confirmation still fires on the first blanked frame — waiting for
    *every* output's is phase C's, recorded there.
- **B. Layer shell per output.** `refresh_layer_zone` per output;
  `layer_hit` under the pointer; `layer_keyboard_focus` + render's
  frame-callback/cleanup walks over all maps;
  `foreign_toplevel_management` `output_enter` per output a window is on.
  (Admission + unmap already resolve per surface — foundation did those.)
  - **DONE (2026-09-21, Phase B PR).** `refresh_layer_zone` walks every
    output, translating each map's `non_exclusive_zone` by that output's
    origin and filing one `OutputUsableAreaChanged` per moved zone, with a
    single `apply()` covering every output that moved (one rectangle compare
    per output on the no-change path; no allocation -- an `Arc` bump per
    output). `layer_hit` resolves the output under the pointer first
    (`output_under`: first geometry in creation order containing the point,
    miss over no output) and hit-tests that output's map, which fixes
    pointer motion, clicks and `focus_under_pointer` at once. The no-output
    choice stays primary ("the compositor chooses"), now documented in
    `docs/protocols.md` alongside the per-output zone/focus rules.
    `layer_keyboard_focus` derives per output with the pointer's output
    first and the rest in creation order -- the tie-break two `exclusive`
    surfaces need, and what keeps a mapped launcher usable with the pointer
    on the other screen (any output's `exclusive` still outranks every
    window, as one screen's did); `clicked_layer` membership is checked
    across all maps. The render tail's dead-layer sweep asks for a zone
    re-derivation plus focus refresh when *any* output's map dropped a
    surface (was: primary only). `output_enter` resolves each window's own
    output from its drawn bounding box (`output_of_window`, primary fallback
    while unmapped -- where new windows open), at announce, at manager bind
    and at `wl_output` bind. Single-output is byte-identical: full nextest
    green, and the one-output motion path measures after-≤-before (release,
    dev VM, 200k `pointer_move` x5: 444-533ns/event before, 351-382 after
    — disjoint ranges, direction safe, absolute cost ~0.4µs/event negligible;
    the after-faster direction has no mechanism in the diff and is read as
    environmental noise, not a speedup claim; two-output 407-541 before,
    396-437 after -- an early
    138ns two-output reading never reproduced and is recorded as a
    governor/warmup artifact, not a number). Fail-first pins: a bar on
    output 2 shrinks only output 2 (zone leak), clicking it focuses it
    (wrong-output focus), both-bars, disconnect-release, and
    pointer-output-wins for two `exclusive` launchers (5 of the 6 new
    layer-shell tests fail pre-fix; the 2 new `wlr-output-bind` tests pass
    pre-fix as regression pins, not fail-first pins). Live proof on the dev
    VM (`--headless --outputs 2`, real waybar 0.15.0): bar on `headless-2`
    gives usable `(400,30,400,270)` with output 1 whole; IPC screenshots and
    `grim -o headless-2` agree (bar pixels on output 2's top rows, background
    on output 1's); bars on both hold `(0,40,400,260)` + `(400,30,400,270)`;
    killing the output-2 bar restores `(400,0,400,300)` with output 1
    untouched. Pixel suites green under `SCOOT_TEST_RENDERER=gles` too.
- **C. Session lock per output.** Drop the single-output fallback in
  `new_surface`; size each surface to its own output; **`locked` waits for
  *every* output's blanked frame** (security-relevant — the bug the
  vblank-confirmation work exists to prevent, across screens); focus rule
  one surface per output; locked render path per output. Heaviest review
  of the five.
  - **DONE (2026-09-21, Phase C PR).** `new_surface` resolves the named
    output with no fallback (unresolvable is ignored, unreachable past
    startup); `configure_output` replaces `configure_all`, so a resize
    reconfigures only the output that moved; `locked` fires only once every
    output's blank is recorded (`SessionLock::confirmed`, cleared wherever
    `pending` is written); keyboard goes to the pointer's output's surface
    with fallback to the first current one (the milestone focus decision --
    derivation still happens at map/unmap/transition, never on bare motion,
    exactly like layer-shell); pointer hit-testing resolves each surface
    against its admitted output; the locked frame draws each output's own
    surface at framebuffer-local origin (the old path drew every surface at
    the output's *global* origin into an output-sized target, so a second
    output's surface landed entirely off-screen -- found live, pinned
    fail-first); frame callbacks and presentation feedback are scoped to the
    output's own surfaces. The duplicate-output refusal is untouched.
  - **The confirm rule, exactly:** an output records on a drawn locked
    frame; no surface there counts on its backdrop frame (so a locker
    covering some outputs still gets `locked`, and zero-surface locks
    confirm as pinned); an admitted-but-undrawn surface with a live role
    blocks its output (its screen shows a placeholder, not the locker's
    blank); a destroyed role never blocks (detected via Smithay's own
    attribute reset, so no post-destroy commit is needed -- a silent teardown
    cannot wedge a pending lock). The undrawn half applies only with more
    than one output: with exactly one, the first blanked frame confirms
    whatever the surface state, the long-pinned single-output semantic, so
    the whole existing lock suite reads the new code as the old. A surface
    admitted after confirm is sized and shown with no second confirmation
    (decided + pinned). `--tty` is provably untouched: its branch never
    reaches the new bookkeeping (short-circuit case analysis in the render
    tail), and it has one output until phase E -- code-read + harness, no
    live `--tty`, stated.
  - **Single-output byte-identical:** full nextest (1281 passed) + the whole
    lock suite green unchanged, pixman and GLES (`SCOOT_TEST_RENDERER=gles`
    lock/layer/outputs: 198 passed); `clippy --workspace --all-targets`,
    `fmt --check --all`, smoke (19 ok) clean. No hot path touched (the new
    bookkeeping runs only on frames drawn while a lock awaits confirmation;
    the render filter is per locked frame, which renders on demand, not
    continuously) -- so no benchmark is owed, stated not skipped.
  - **Fail-first pins (10 new tests, 7 fail pre-fix):** both-surfaces cover
    own outputs (sizes white-box + per-output censuses); keyboard follows
    the pointer's output at map/unmap with first-surface fallback;
    output-1-only blanks output 2 with no surface and still confirms;
    `locked` waits for the last blank (admitted-undrawn holds it, mapping
    releases it); duplicate refusal per physical output (regression pin);
    differs-per-output sizes (120 + 80x60, configures + censuses);
    unlock restores both outputs (regression pin); full destroy
    mid-confirmation unblocks; role-only destroy + silence unblocks (pins
    the no-commit carve-out); post-confirm late surface needs no second
    confirmation.
  - **Live proof on the dev VM (`--headless --outputs 2`, 400x300):**
    pixman -- single-batch lock + both surfaces admitted pre-first-frame,
    forced-render screenshots while output 2 was admitted-undrawn (out1
    120000 magenta, out2 120000 black, `locked` still 0 after a sync
    barrier), then `LOCKED` only after output 2 mapped (out1 magenta, out2
    cyan, 120000 each); output-2-only takeover (out1 black, out2 cyan,
    `locked` +2ms, configure 400x300); locker death turns both screens
    solid red (120000 each). GLES -- both surfaces live (magenta/cyan
    censuses, `locked` +20ms). Foot blanked off both screens throughout.
    Method note: the locktool's `dispatch_pending` settle never drains the
    client socket, so `locked` timestamps skew late -- every ordering
    reading above was taken through a roundtrip barrier, and promptness
    (confirm on the first completing frame) is the harness's claim, not the
    log's.
- **D. Workspaces + output management + pointer.** `ext-workspace-v1`
  group per output; `wlr-output-management` head per output (read half —
  the `apply`/`test` refusal stays as documented); `input.rs` pointer
  clamp over the union (or the output under the pointer — decide from the
  focus decision above, record it).
  - **DONE (2026-09-21, Phase D PR).** Three halves, no core change — the
    milestone's no-redesign tripwire did not fire, and why not is load-bearing:
    new windows always open on the first output (`shell.rs` files
    `WindowOpened` with the first output's id, and every focus path keeps the
    focused output there), so every non-first output's workspace list is
    permanently the single empty workspace and `FocusWorkspaceIndex`
    (focused-output-relative) suffices for every reachable switch. A switch
    pending on a non-focused output is ignored rather than misrouted, loudly
    in debug builds — the branch a future cross-output-move phase turns into
    a real output-targeted action.
  - **`ext-workspace-v1`: one group per output** (`ext_workspace.rs`).
    `Manager` holds a `Group` per output (group object, positional handle
    slots, staged `pending_activate`); `published` is one snapshot per
    output with a reused `current` scratch, so the per-`apply` refresh
    allocates nothing and sends nothing (no events, no `done`) when every
    snapshot matches. `refresh_workspaces` diffs per output through the
    unchanged `diff.rs` and closes with one `done` per manager; `announce`
    builds one group block per output (group, capabilities, that output's
    `output_enter`s, handles) before that `done`; `workspace_group_output_bound`
    enters only the bound output's group; `commit` checks bounds and
    already-active against the *group's* output's list. Single-output is
    wire-identical: one group block + one `done`, exactly the old sequence.
  - **`wlr-output-management`: one head per output**
    (`output_management.rs`). `published` is one `HeadState` per output,
    each manager one `Head` per output (with its own modes, retired
    innermost-first like before); updates touch only the heads whose output
    moved, and one `done` still closes the batch. `headless::add_output`
    now runs `refresh_output_heads`, so an output added after a bind still
    announces its head. `configuration.rs` untouched — `apply`/`test`
    still answer `failed`, pinned with two heads bound.
  - **Pointer clamp: the union of the outputs** (`input.rs`,
    `clamp_to_output_union`). Decided from the focus decision as the
    milestone requires: the pointer picks the output, so the clamp must let
    motion *reach* the next output — clamping to the output under the
    pointer would trap relative motion on one screen forever (pinned: held
    at 199 pre-fix). Half-open bounds mirror `output_under`, so the seam
    pixel belongs to the output on its right; uneven outputs leave dead
    zones where the pointer may rest and misses exactly like an off-output
    absolute move (absolute motion is never clamped). A single output takes
    the old per-extent expression exactly (fast path), so one-output
    sessions neither behave nor measure differently.
  - **Benchmark (owed: the clamp runs per relative-motion event).**
    Release, dev VM, 200k oscillating `pointer_move_relative` x5 with 20k
    warmup (pre-warmup runs showed the known governor artifact — two ~180ns
    readings among ~700ns — recorded, not used): single-output 561ns/event
    median before (558–588), 665ns without the fast path (629–672,
    disjoint — real), 567ns with it (512–620, overlapping — residual
    gone); two-output 576ns before, 737ns after on the union loop, which
    has no production producer until phase E (`--headless` moves absolutely,
    single-output `--tty` takes the fast path) and is disclosed, not
    claimed. Absolute cost sub-µs either way: ~0.6% of one 16ms frame per
    second at a 1kHz device rate.
  - **Bind/refresh allocation check.** Refresh allocates nothing (reused
    `changes` + `current` buffers, snapshot swap); bind allocates O(#outputs)
    small vecs at client-bind rate, the same class as the pre-existing
    per-workspace vec — no new flood path, and the shared 8-bind budget is
    untouched.
  - **Single-output byte-identical:** full nextest (1297 passed) with every
    pre-existing suite green unmodified, the whole ext-workspace +
    output-management + input + outputs + layer-shell set green under
    `SCOOT_TEST_RENDERER=gles` too (240 passed), smoke green, and a live
    400x300 output-2 screenshot hashing to phase A's known-empty
    `0d5affbe…` while output 1 shows the window.
  - **Fail-first pins (17 new tests, 11 fail pre-fix):** clamp trapping,
    seam ownership, far-edge stop and dead-zone rest (4 fail, 1 absolute-miss
    pin passes throughout); one-head-per-output bind, late-add announce and
    release-isolation (3 fail, stop + 2 refusal pins pass throughout);
    one-group-per-output bind, late-`wl_output` enters and
    activate-on-output-2-leaves-output-1 (3 fail, first-output switch +
    stop pins pass throughout); the 338-event relative fling across the
    seam entering output 2's bar (fails held at 199 pre-fix).
  - **Live proof on the dev VM (`--headless --outputs 2`, 400x300):**
    pixman — `wayland-info` shows both `wl_output`s, `wlr-randr` both heads
    (`headless` at 0,0, `headless-2` at 400,0, current+preferred modes);
    `foot` opens on output 1 (IPC `"output": 1`), output-1 shot shows it
    while output-2's hashes the known-empty session; pointer driven to
    (600,150) and clicked with the session stable and window focus sane;
    the ext-workspace bind burst dumped from a real client (two groups,
    per-group `output_enter`s, one `done`). GLES — both heads in
    `wlr-randr`, per-output IPC screenshots differ correctly, `grim -o
    headless-2` serves the screencopy path. No live `--tty` (single output
    there until phase E — stated).
- **E. `--tty` multi-CRTC (hardware-gated).** One connector is chosen at
  startup and hotplug *switches* rather than *adds*; driving two at once
  is a different shape. Needs real two-connector hardware, and note what
  that means now that `gpu-vs-cpu-measured` has been *answered* on the
  Asahi machine (2026-09-21) while this stayed blocked: the constraint
  there was the GPU, which that machine has; the constraint here is a
  **second connector**, which it does not — only `card2-eDP-1` exists
  until a USB-C/DP-alt display is attached. Does not block A–D.
  - **DONE (2026-09-25, phase E PR; E1 `b782b06`, E2 `2bd7d47`).** The
    second connector arrived: the M2 Air drives an external 1920x1080
    Samsung over USB-C DP alt mode on the experimental `fairydust` kernel
    (`card2-DP-1`, CRTC 68, beside `eDP-1` on CRTC 50; `drm_info`: each
    encoder reaches exactly one CRTC, gamma size 0 on both).
  - **E1, enumeration + startup.** `gpu::find_all` collects every
    connected connector with a mode in kernel order (the first entry is
    exactly the old first-wins answer, so one connector is driven as
    before), each with the CRTCs its encoders reach. `tty/crtcs.rs` matches
    connectors to CRTCs from `possible_crtcs` (maximum bipartite matching,
    free-first, an earlier connector never displaced) instead of the old
    "first CRTC that accepts a surface", which only worked on the DCP by
    luck of order (Smithay refuses a CRTC only once its primary plane is
    claimed). One `Head` per connector (`tty/head.rs`: connector, name,
    presenter, mode size, output id) under a `Tty` that keeps only
    session/device state; the `Presenter` enum moved to
    `tty/presenter.rs`. Startup degrades, never refuses: one head that
    builds is enough, and a connector whose CRTC, surface, buffers or
    `wl_output` fails is left dark with a warning. One tier per session:
    the first head decides, and a later head that cannot join the GPU
    tier stays dark rather than mixing tiers (`State::renderer` is one
    value, and `resize_output` rebuilds from it). Capped at `MAX_OUTPUTS`.
    Outputs are created side by side (`add_output_with`, measured off the
    previous output's logical geometry, so the shared scale is respected).
  - **Every per-output tty path keyed by `OutputId`:** present, buffer age,
    retry, `scanout_mut` (so one screen's frame can never be queued on
    another's swapchain, incl. the capture-forced composite), gamma size
    and ramp (per-output sizes in `GammaControlState`, a CRTC switch fails
    only its own output's control), and vblank routing by CRTC. A
    `DrmEvent::Error` names no CRTC, so it settles every head and confirms
    nothing (the `invalidate_scanout` asymmetry argument). Absolute pointer
    and tablet map across the output union. `resize_output_of(id, ..)`
    files the output's real origin and repacks the outputs to its right.
  - **Session lock, the security-relevant part.** Phase C held on `--tty`
    only because it had one output: `blank_flip` was one `Option<u64>`, and
    each head numbers its flips from zero, so head 1's flip-0 vblank would
    have confirmed a lock whose blanked frame on head 2 (also flip 0) was
    still in flight. Now `blank_flips` is per output, a completion must
    match its own output's flip, `locked` needs every output recorded,
    Phase C's placeholder rule (an admitted, undrawn surface blocks its
    output) applies on the tty path too, and the fallback timeout stands in
    for missing vblanks, never for missing surfaces. Pinned fail-first
    (`session_lock/tests/tty_multi_output.rs`, 9 tests). Live: both screens
    blank (swaylock colour on both IPC screenshots) and the log's
    confirmation reads "every output's blanked frame reached scanout" ~60 ms
    after `locking the session`, on both tiers.
  - **E2, hotplug add/remove.** `tty/hotplug/heads.rs` re-plans every head
    at once (pure, 12 pins): a plugged-in connector gets a head on a CRTC no
    lit head holds (lit heads are never re-routed), a driven connector that
    goes away is removed, a mode change is followed per head, and only when
    nothing driven is left do the #48 single-output rules apply to the
    primary (move to another connected connector, or hold the last frame).
    The last output is never removed. `State::remove_output` takes an
    output away with clients connected, in protocol order (layer surfaces
    `closed`; capture sessions `stopped`, parked frames failed; gamma
    `failed`; `ext-workspace-v1` group removed after its workspaces leave;
    foreign-toplevel `output_leave` now, `output_enter` from the `apply()`;
    scanout feedback reverted), then drops the render target, withdraws the
    `wl_output` global (destroyed after 5 s, so a racing bind survives),
    files `OutputRemoved`, repacks, forgets the output in the lock wait,
    re-homes the pointer and refreshes once. Pinned with real clients:
    `outputs/removal.rs` (8), plus head retirement (`output_management`),
    group removal order (`ext_workspace`), leave/enter (`foreign_toplevel_management`).
  - **Live on the Asahi machine** (final product code `2bd7d47`, `nix
    build .#scoot-gpu`, `~/fx/e-live.sh`, both tiers; pixman also at
    `b782b06` earlier): `scoot msg outputs` names `eDP-1` (1707x1067 at
    `[output] scale = 1.5`) and `DP-1` (1280x720 at x=1707); a window on
    each output (`Super+Shift+period` carried the second across); the
    pointer driven onto DP-1 is drawn there; per-output IPC screenshots
    show each window on its own screen only; VT switch away and back logs
    a full modeset on both CRTCs and both screens come back; the lock
    covers both. CPU (jiffies, 30 s, `foot` printing every 20 ms): pixman
    idle 0.00%, both screens damaged 37.27%, eDP-1 only 36.20% (vs
    single-output `main` `0785420` 35.20/35.67% for the same eDP-1
    workload, one full-width window); GPU tier idle 0.00%, both 13.73%,
    eDP-1 only 9.27% (single-output `main` 9.43/8.63%, same two-column
    layout). Hot paths benchmarked (release, thin LTO, both trees on the
    Asahi machine): relative motion 318-319 ns (main) vs 315 ns (branch)
    with one output, 368-369 vs 367-368 ns with two; headless
    `render_frame_cost` best-of-5 empty 41.8/41.8 µs (main) vs 41.3/40.4 µs,
    8 windows 57.1/57.5 vs 57.2/57.7 µs, and the two-output render
    overlapping likewise -- no measurable change. Page flips with both
    screens damaged: 705 in 10 s across the two heads (about 35/s each,
    below 60/s each: flips never exceed one per CRTC vblank).
  - **What hardware could not show.** The unplug: with this kernel DP-1
    keeps reading `connected` after the cable is pulled (no HPD loss
    reaches userspace), so removal is harness-verified only. The GPU tier
    ran with Mesa's paths passed by environment (`GBM_BACKENDS_PATH`,
    `__EGL_VENDOR_LIBRARY_DIRS`) because the booted generation has no
    `/run/opengl-driver`; no system change was made. Gamma: both CRTCs
    report size 0, so each output advertises the fallback and a `set_gamma`
    would answer `failed` there.
  - **Decided, recorded.** Frame pacing: each head flips at most once per
    its own vblank (its own flip tracker/`DrmCompositor`), but a render
    walks every output, so damage on one screen costs the other a no-damage
    pass and a frame callback round; measured at ~1 pp of CPU with the
    second screen idle (above), so per-output render scheduling is left as
    an optimisation, not a correctness gap. Output ids are never reused, so
    a monitor unplugged and replugged returns under a new id and the
    default output-2 binds stop reaching it (documented; follow-up in the
    remainder ticket).
- **F. Cross-output window moves + focus (staged 2026-09-21, the last
  "partially implemented" gap).** All of A–D deliberately left windows
  opening on output 1 with no way across: no move primitive, no
  focus-output primitive (keyboard users are stuck on output 1 — focus
  only follows the pointer), and no `output_leave` pairing. Shape (decided,
  see below): `MoveFocusedWindowToOutput` + `FocusOutput` core actions,
  IPC variants, config grammar, NO default binds (document; binds get
  their own decision like the workspace-index ones did), `output_leave`
  pairing wherever `output_enter` already fires, focus follows the moved
  window (mirroring move-to-workspace-index's carry-and-follow), keyboard
  on the source output falls back per existing rules. Out: per-output
  scale/mode surface, runtime output add/remove, default binds.
  - **DONE (2026-09-21, Phase F PR).** Single diff, no split: the leave
    pairing turned out to be one refresh method plus its hook, not a
    sprawl -- see below.
  - **Core** (`scoot-core`, no redesign -- the tripwire did not fire:
    `reshape` stays for within-output actions, the two new actions are
    plain `World` methods). `MoveFocusedWindowToOutput(OutputId)` takes the
    focused window (always on the focused output, by construction) into the
    target's *active* workspace right of its focused column, carrying the
    column preset, and follows it there; the source keeps `take`'s
    neighbour focus. `FocusOutput(OutputId)` sets `focused_output` -- the
    focused window derives as the active workspace's focused window, or
    `None` on an empty output (decided from what focus means on an empty
    output today: exactly that `None`, and the arrangement reports it).
    Unknown ids are ignored in both (mirroring the out-of-range workspace
    rule -- focus can never strand on a nonexistent output, and a move can
    never lose a window), as are move-with-no-focus and move-to-current.
  - **Wire + grammar, additive, no `PROTOCOL_VERSION` bump.**
    `MoveFocusedWindowToOutput { output }` / `FocusOutput { output }`
    (`snake_case` tags, `u64` output *ids* -- stable for the session, unlike
    workspace positions); the unknown-tag decode check proves an older
    server answers a new tag with `Error` and keeps serving, the same
    degradation the move-index action shipped under. Shared
    `scootctl::action` arms (`move-window-to-output ID`, `focus-output
    ID`) cover CLI, `scoot msg` alias and config `[binds]`/`[autostart]` by
    construction; `action_string` emits both back byte-identically (pinned).
    `scoot --help`'s embedded grammar updated alongside (containment test).
  - **Compositor integration.** `State::act`'s lock gate and IPC's
    `Request::Action` refusal cover both with no new code (pinned under
    lock: refused, arrangement identical). `FocusOutput` spends
    `clicked_layer` and takes the already-there fast path (one `Copy`
    compare); `MoveFocusedWindowToOutput` does neither -- the exact cut
    `MoveWindowToWorkspaceIndex` already holds (a carry changes arrangement;
    the click-spend boundary test fails if it ever lands in the match).
  - **`output_leave` pairing: the wlr handle only.** Checked all three
    lists against their protocol XMLs: `ext-foreign-toplevel-list-v1` has
    no output events at all (zero `output` mentions in its XML -- nothing
    to pair); `ext-workspace-v1`'s group `output_leave` fires when an
    output is *removed from a group*, and groups never change outputs
    (pinned: a move sends no group enter/leave while workspaces do move);
    `wlr-foreign-toplevel-management-v1`'s handle `output_leave` is the one
    moves owe. Each handle's last-told output is now stored (core id, set
    at announce/bind to exactly what the `enter` named) and `apply` diffs
    the fresh arrangement against it: leave-old + enter-new + `done` on
    exactly the moved window's handles, per-client objects (the
    wrong-client panic guard included), `done` only when something was
    said. Close sends `closed` with no `leave` (pinned -- the handle's
    death is the `closed`); workspace switches within an output send
    nothing (unchanged, pre-existing). Unmap has no boundary in scoot
    (handles live creation-to-destruction), so there is no unmap path to
    pair.
  - **Pointer rule, recorded.** Moves and output-focus never touch the
    pointer (only startup centres it, only motion clamps it): after a
    carried follow the keyboard is on the target output while the pointer
    rests on the source, until motion re-resolves it -- which only matters
    for layer/exclusive derivation, never for which window holds the keys.
  - **Single-output byte-identical:** full nextest green unmodified (plus
    the whole ext-workspace + wlr + input + config sets green under
    `SCOOT_TEST_RENDERER=gles` too), smoke green, and the one-output motion
    path untouched (no hot path changed -- see benchmark note).
  - **Benchmark (none owed, stated not skipped).** The two actions are
    per-request cold (keybinding/IPC rate, like every other action);
    `arrange` already walks all outputs and is unchanged; the one addition
    to a shared path is `apply`'s membership walk -- one map lookup plus
    one `Option` compare per window, allocation-free, sending nothing when
    nothing moved. Nothing here runs per event or per frame, so there is
    nothing to measure before/after.
  - **Fail-first pins (new tests, red proven where behavior is new):**
    core move/focus/preset/unknown/empty/rapid-sequence (5 red on the
    no-op placeholder, green on the behavior); wlr leave+enter+done both
    directions (red with the `apply` hook disabled, green restored) plus
    close-without-leave and silent-to-outputless-client (green throughout);
    ext-workspace group-stability across a move; IPC carry+follow,
    unknown-output, cross-output focus both ways, already-there
    no-`apply`, click-spend boundary both directions; config bind
    parse+emit round-trip; scootctl parse; IPC wire shape + unknown-tag
    rejection; lock refusal of both actions via IPC and via `act` with the
    arrangement byte-identical.
  - **Live proof on the dev VM (`--headless --outputs 2`):** see the PR
    description for the wire logs (leave/enter per output on a live taskbar
    client) and the per-output screenshots (window pixels move screens).

## Verification per phase (in addition to the standard set)

`--headless --outputs 2` throughout (side-by-side virtual outputs, no
second monitor): per-output screenshots compared, protocol dumps
(`wayland-info`, `wlr-randr`) per output, fail-first tests for every
refusal being lifted. `--nested`/`--tty` single-output regressions via
smoke. Phase E additionally needs the Asahi runbook treatment.

## Explicitly not in this milestone

`wlr-output-management` `apply`/`test` becoming real (refusal stands as
documented — multi-output makes it *possible*, not *required*); named
workspaces; per-output scale/mode configuration surface.
