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
- **D. Workspaces + output management + pointer.** `ext-workspace-v1`
  group per output; `wlr-output-management` head per output (read half —
  the `apply`/`test` refusal stays as documented); `input.rs` pointer
  clamp over the union (or the output under the pointer — decide from the
  focus decision above, record it).
- **E. `--tty` multi-CRTC (hardware-gated).** One connector is chosen at
  startup and hotplug *switches* rather than *adds*; driving two at once
  is a different shape. Needs real two-connector hardware (Asahi) — like
  `gpu-vs-cpu-measured.md`, this phase waits on the user's machine and
  does not block A–D.

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
