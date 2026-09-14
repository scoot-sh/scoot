---
item: "5b"
title: "VT-switch-back EPERM"
status: "done"
area: "backend"
pr: 9
commit: null
---

# VT-switch-back EPERM

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
pass, clippy/fmt clean, cross-platform build clean on macOS. Those
pause/reactivate cycles were real in the strongest sense, not just
libseat-event-level: confirmed 2026-09-13 that a `--tty` session over SSH
holds real DRM master and really does lose and reacquire it across a VT
switch (see the resolved DRM-master entry in the Backlog).

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
