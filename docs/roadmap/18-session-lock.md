---
item: "18"
title: "ext-session-lock-v1"
status: "done"
area: "protocols"
pr: 25
commit: "901ae4d"
---

# ext-session-lock-v1

**Smithay has a helper for this one** (unlike `ext-workspace-v1`):
`wayland::session_lock` owns the three interfaces, the surface role, the
configure/ack cycle and the "is this the object that holds the lock"
check behind `unlock_and_destroy`. What flexwm owns is the policy —
when to accept a lock, what a locked frame contains, where input goes,
and what happens when the lock client dies. That lives in
`compositor/session_lock.rs`.

**The lock state is one field, deliberately.**
`SessionLock::owner: Option<ExtSessionLockV1>` — `is_some()` *is* "the
session is locked", so there is no second boolean to disagree with it.
`abandoned()` (locked, owner not alive) is derived from the same field.
Written `Some` in exactly one place (the `lock` handler) and `None` in
exactly one (the `unlock` handler); a VT switch, a session pause, a
failed render and a dying client all leave it untouched, which is what
makes the lock survive them. This shape was chosen against item 5b's
worked example: a `locked: bool` beside the owner would have been two
fields that can mean different things at different sites.

**What locking changes, each as a branch taken *before* the unlocked
path rather than an ordering tweak on top of it:** the render element
list is built from the lock module alone (an opaque backdrop plus the
mapped lock surfaces — a window, a bar, a wallpaper and the ring are
never gathered); frame callbacks go only to lock surfaces; keyboard
focus is a lock surface or nobody; pointer hit-testing sees only lock
surfaces, *and* pointer focus is explicitly re-derived at every lock
transition (`State::refresh_pointer_focus`) because `wl_pointer.button`
goes to whatever the pointer last *entered* — without that the first
click after a lock still lands in the window underneath; `Bound::Action`
keybindings are forwarded to the lock client instead of firing; and
`State::act` itself refuses while locked, which is the backstop that
covers the IPC `action` request, `ext-workspace-v1`'s `activate`, and
any caller added later.

**The backdrop is a real element, not just `render_output`'s
`clear_color`.** A clear colour is not part of any element's damage, so
under `--tty` (the one backend that passes a real buffer age) a frame
whose elements did not change can report no damage and leave the
previous pixels on the scanout buffer. A persistent `SolidColorBuffer`
(the pattern `decorations.rs` already establishes) makes locking,
unlocking and the switch to the abandoned colour real damage. The clear
colour is set to the same colour anyway, as a second line of defence.

**Crash recovery, the most safety-critical decision here:** the session
stays locked (the protocol's own rule), the screen turns **solid red**
so a user can tell a crashed locker from one drawing black, and a new
client may **take the lock over** — confirmed immediately when the lock
it replaces had already drawn its blanked frame, and made to wait for one
otherwise (see round two) — and unlock after authenticating. Not
guessed: sway's `lock.c` (`handle_abandon` paints exactly this red,
`handle_session_lock` replaces an abandoned lock) and niri's
`Niri::lock` (replaces a lock whose client `is_alive()` is false) both
do the same, checked against their actual sources. The honest cost is in
`README.md`: while a lock is abandoned, any client that can reach the
wayland socket can take it over and unlock — the same-uid boundary the
IPC socket already has, and the price of having a recovery path at all.

**IPC while locked:** injected keyboard and pointer input is *not*
refused, because it goes through the same focus paths a real keyboard
does and so can only reach the lock surface — which is what lets an
agent drive a lock screen. `Request::Action` is refused with an error
saying so. `windows`/`outputs`/`screenshot` still answer; a screenshot
reads the same framebuffer, so it shows the lock screen and nothing
behind it. All four stated plainly in `README.md` rather than left to be
discovered.

**One real bug found by hardware bug-bashing, not by any test:**
`kill -9` on a lock client that had already destroyed its lock surface
left the screen **black instead of red**. A disconnecting client
destroys protocol objects; it does not commit a surface or press a key,
so nothing marked the screen dirty and the last frame drawn stayed up.
The same case *with* a live lock surface happened to work, because that
surface's destruction goes through `handlers.rs`'s `destroyed` hook —
i.e. the signal was correct only by accident of teardown order. Fixed
with `State::refresh_lock_backdrop`, called from the one place a
disconnect is observed (the wayland display source), comparing the
backdrop buffer's *own* colour against what the lock state says it
should be, so there is no remembered flag to fall out of step. The
regression test for it needed two clients — written with one first, and
the negative control passed, because a single client owning both the
window and the lock has `remove_window` request a redraw as a side
effect.

**Deliberately deferred, each with a Backlog entry below:** an IPC way
to *ask* whether the session is locked (needs a `PROTOCOL_VERSION` bump,
so it bundles with the two already waiting); waiting for lock surfaces
before blanking (niri waits up to 1s to avoid a black flash, at the cost
of rendering the unlocked session for that second — flexwm blanks
immediately, which is the conservative half of that trade); confirming
`locked` on a real vblank rather than on a queued page flip;
security-context filtering of the global; per-output lock surfaces once
multi-output exists; and `ext-idle-notify-v1`, without which nothing
locks the session automatically.

**Tests: 26 new (346 total, against 320 on the merge base)** — 20 with
the protocol's first landing, six more from round two below — in
`session_lock/tests.rs`, driving real `wayland-client` connections
(including a second one, for the takeover) through a real `State` and a
real `PixmanRenderer`. Every "nothing is visible" assertion checks
*every pixel of the frame*, not samples, and every "nothing received
input" assertion is made from what the client was actually sent
(`wl_keyboard.enter`, `wl_pointer.button`, `xdg_toplevel.close`) rather
than from a field inside the compositor. Covered: a window blanked off
the screen, an `overlay` layer surface blanked too, the lock surface's
own pixels, a lock surface with no buffer yet, a destroyed lock surface
falling back to a solid colour, unlock restoring the session, `locked`
withheld until a frame is drawn (driven by taking the render target
away, so no timing luck is involved), a second lock refused with
`finished`, keyboard and pointer reaching only the lock surface, a
`Super+Q` bind not closing a window, `State::act` and the IPC `action`
both refused, the session staying locked when the client dies, the
abandoned colour appearing with nobody asking for a redraw, takeover +
unlock, a replacement locker after one died unconfirmed, an empty
session locking and unlocking three times (the damage case), and an
output resize reconfiguring the lock surface.

**Hardware verification** and **benchmarks**: see PR #25's description
for the exact commands, raw histograms and jiffies figures, all captured
on the dev VM's real `--tty` `virtio-gpu` device at 1600x1000. In
summary: a locked frame contains exactly the lock surface's colour plus
the 136-pixel cursor and nothing else; 44 keystrokes typed over IPC
while locked reached the lock client and left no trace in the terminal
behind it; a VT switch away and back left the lock screen
pixel-identical (`magick compare -metric AE` = 0); `kill -9` turned the
screen red and kept refusing actions; a second locker took over and
unlocked cleanly. Balanced interleaved benchmark, 18 reps per side
across three batches, the order of the two binaries alternated between
reps — because item 10 measured a real run-position effect, and an
unbalanced first attempt here duly produced a phantom 11% regression:
150 corner-to-corner pointer jumps, `c93a100` mean **37.6** jiffies
(20–44) against this branch's **37.2** (28–45) — fully overlapping, no
measurable difference; idle 0 jiffies/10s both ways. The frame-callback
suppression is worth a real number of its own: a terminal running
`while true; do date; done` costs the compositor **229 jiffies/10s**
unlocked and **2 jiffies/10s** with that same client still running
behind a lock (225 again the moment it unlocks), which is why
`handlers.rs`'s per-commit `request_render` needed no extra gate while
locked.

**Round two: any client could permanently steal every future lock
screen's keyboard.** The most severe finding of the session, and it needed
no race, no crash and no privilege. `ext_session_lock_v1.destroy` is legal
*before* `locked` has been sent — Smithay's `lock.rs` refuses it only
while its own `LockStatus` says that object holds the lock, and that
status stays `Unlocked` until the confirmation actually runs. So a client
could send `lock`, `get_lock_surface` and `destroy` as one batch, and
unlike a dying client it kept both its connection and the `wl_surface`
under that lock surface. Every reader here filtered on
`LockSurface::alive()`, which asks about the `wl_surface` and nothing
about the lock, so the surface stayed registered forever: when the user's
real locker ran later, the zombie was still first in the list, so it drew
in front and it held the keyboard. The user's password went to the
attacker keystroke by keystroke while the locker that never saw them could
never authenticate and so could never unlock. Reproduced end to end before
it was fixed: on the unfixed tree the attacking client's report reads
`keys: 16` against the real locker's `keys: 0`.

The obvious fix — compare against `owner` — closes the takeover half and
not the other one: during the abandoned-but-not-yet-replaced phase the
stale surface's lock *is* still `owner`. The predicate has to be all three
of alive, `== owner`, and `owner` itself still existing, and it is now one
free function (`is_current`) behind one accessor (`SessionLock::current`)
that is the only reader of `surfaces` — so a reader added later cannot ask
the weaker question by forgetting the rest. Filtering is the guarantee;
a stale surface is also *dropped*, with both focuses re-derived, at the
dispatch that observed the `destroy` (`refresh_lock_state`, renamed from
`refresh_lock_backdrop`), at the next locked frame, and at a takeover.
Re-deriving pointer focus is not optional: `wl_pointer.button` goes to
whatever the pointer last *entered*, so filtering the hit test alone would
have left the zombie receiving every click — which it did, measured, two
buttons' worth.

Two more findings from the same round. **A takeover could confirm
`locked` with an unlocked frame on screen**: the fast path reasoned "the
outputs are already blanked", true only if the *previous* lock had ever
been confirmed. It now fast-confirms only when the state it replaces had
`pending == None` on entry, read before the dead-`pending` sweep that
would otherwise erase the distinction; everything else routes through the
same `pending` mechanism a fresh lock uses. (niri's `Niri::lock` draws the
same line — it fast-confirms from `Locked`, never from `Locking(_)`.)
And **the `README` overstated the `--tty` `locked` guarantee**: it claimed
the frame had been copied to the scanout buffer with a flip requested,
but `Tty::present` returns without doing either whenever the session is
inactive or a previous flip is still pending — the latter being, in the
code's own words, an ordinary frequent throttle. Corrected to the real,
weaker guarantee rather than closed, because closing it means carrying the
confirmation through the flip; see the backlog entry, which now says that
precisely.

Six new tests, all red on `ace80a1` first and green after (the four
reproductions review wrote, incorporated rather than discarded, plus the
pointer/click half of the attack and a control proving the fast-confirm
path still exists for a takeover that has genuinely earned it).

**Round two hardware verification**, all captured on the dev VM's real
`--tty` `virtio-gpu` at 1600x1000 against `e1af444`, driving the attack
from a purpose-built wayland client (no off-the-shelf locker will destroy
a lock object it has just been granted):

- The attack, both shapes — surface never mapped, and surface mapped with
  a full-screen magenta buffer *after* its lock was already destroyed,
  which is stronger than anything the unit tests do. In both, the screen
  is the abandoned red: 1,599,864 of 1,600,000 pixels `#FF0000`, the other
  105 being the cursor, and **zero** `#FF00FF` pixels in any frame of the
  run. The attacker is handed `KB_ENTER` and then `KB_LEAVE` 0ms later —
  the compositor takes the keyboard back in the same dispatch cycle that
  observed the `destroy`.
- With the real locker then up and drawing green: `flexwm msg type
  hunter2` plus a click gives **attacker KEY=0 BTN=0, locker KEY=14
  BTN=2**. On `ace80a1` the same shape of test reads attacker `keys: 16`
  and locker `keys: 0`.
- Finding 2, deterministically: one client sending `lock(A)`,
  `get_lock_surface`, `destroy(A)`, `lock(B)` as a single batch — so B
  necessarily arrives while A is unconfirmed and the desktop is still on
  screen. Log reads `already_blanked=false`, the confirmation is deferred
  to the next drawn frame (`session lock confirmed` 24ms later; the client
  sees `LOCKED` at +28ms against +2ms on the fast path), and it is not a
  hang. The fast path still exists: both ordinary takeovers in the same
  runs log `already_blanked=true` and confirm in 1–2ms.
- VT switch away and back while locked: AE = 0 outside the two 16x16
  cursor boxes, i.e. the lock screen is pixel-identical. (The raw AE of
  120.5 is entirely the cursor, which the script itself had moved between
  the two shots — connected-component analysis shows exactly two 16x16
  regions, 136 px each, at the two pointer positions.)
- `kill -9` on the real locker still turns the screen red and keeps
  refusing IPC actions; a third locker takes over that red screen and
  draws its own.

**Benchmark**, same commit, on the one path this adds per-event work to:
the locked pointer hit test. 4000 `pointer_move` requests over a *single*
IPC connection (a first attempt at 150 requests spawned 150 `flexwm msg`
processes, whose noise swamped the signal — 33–65 jiffies within one
side), 10 balanced reps per side with the order alternated: `ace80a1`
mean **27.2** jiffies (25–30), this fix **28.4** (26–31), fully
overlapping, Welch p ≈ 0.13. Locked-idle is **0 jiffies/10s** on both.
The difference is at or under this rig's noise floor and there is no
mechanism for more: the filter is one `alive()` and one object-id compare
per lock surface per read, of which there is one per output, with no
allocation anywhere. The *unlocked* paths are untouched by construction —
every change sits behind an `is_locked()` check or inside an
`if self.forget_lock_surface(..)` that is false while unlocked — so this
item's own round-one unlocked numbers (above) still stand. (Written as
"item 16's" when this entry *was* item 16; it is a self-reference, and is
spelled that way now so the next renumber can't break it again.)

**Round three** re-reviewed the round-two fix adversarially — seven more
attack variants beyond the original three, including a stronger one (an
attacker surface mapped with real full-output pixels *after* its lock was
already gone) — and **none reproduced**: the keyboard-hijack hole is
closed. One real, non-security finding survived that pass, plus four
documentation/consistency items, all fixed in the round-three commit:

- **MEDIUM, protocol compliance: destroying only the lock *surface* left
  its last frame on screen.** A client may destroy its
  `ext_session_lock_surface_v1` and keep the lock, the `wl_surface` and
  the connection — legal, and what a locker does when an output is
  removed under it. Smithay's `ExtLockSurfaceUserData::destroyed` resets
  `last_acked`, so the surface stops producing render elements, but
  *nothing asked for a frame*: the destroyed surface's pixels stayed up
  until some unrelated damage (a pointer motion, the backdrop turning
  red) happened along, against the protocol's own "the compositor must
  fall back to rendering a solid color". Pre-existing, not a round-two
  regression (reproduced on `ace80a1` too). Fixed where that destruction
  is the only visible: `dispatch.rs`'s hand-written blanket
  `Dispatch::destroyed`, which now calls `State::lock_surface_destroyed`
  for that one interface — `SessionLockHandler` has `lock`, `unlock` and
  `new_surface` and no callback for a surface going away. The interface
  test is the same compile-time-folded `TypeId` comparison the three
  request guards there already use, so every other interface's
  destruction pays nothing.
- **LOW: the grab drop was asymmetric across the lock lifecycle.** Only
  `lock()` dropped a pointer grab; the other transitions (a surface
  dropped by `refresh_lock_state`, by the render loop's cleanup, by
  either destruction hook) re-derived focus and left any grab installed —
  and a grab outlives focus changes *by design*. Fixed by making all five
  call one `State::lock_transition` (grab, both focuses, redraw); only
  `unlock` deliberately does not, since that hands the session back. Two
  grabs can actually be live: a drag-and-drop, and — found while fixing
  this, contradicting the round-two note that DnD was the only one —
  Smithay's *implicit click grab*, which `DefaultGrab::button` installs on
  every press. That is what makes the fix testable without a DnD fixture,
  and the new test holds a button down across a role-object destroy.
  Nothing installs a keyboard grab today, which is why the same asymmetry
  was never a keystroke leak; `drop_pointer_grab`'s doc says where one
  would have to be dropped if that ever changes.
- **Doc accuracy: "both behaviors match what sway does" was wrong** for
  the newer half. sway's `handle_session_lock` does replace an abandoned
  lock and refuse a live one the way flexwm does, but it confirms
  **unconditionally** — no blanked-frame gate, fresh lock or takeover
  alike. The conditional confirmation round two added is **niri's**
  (`Niri::lock` fast-confirms only from an already-`Locked` state), which
  the module doc and PR body already attributed correctly; only
  `README.md` said sway. Fixed there.
- **Doc completeness:** the README's own caveat list was missing the
  first-click-on-a-fresh-lock-screen gap (its own backlog entry below),
  which sits beside caveats of the same severity, and said nothing about
  the destroyed-surface fallback above. Both added.
- **NIT: an invariant the round-two fix quietly depends on** is now
  written down on `SessionLock::pending`: while it is `Some`, its lock
  object is the same object as `owner` (both written together in
  `lock()`). Without that, the dead-`pending` sweep could clear a
  `pending` belonging to a lock that never blanked the screen and leave
  the *next* lock fast-confirmed over a visible desktop.

**Round-three verification, all against `166210b`.** Nothing executable
changed after that commit: every later diff to a `.rs` file is a doc
comment (on `State::lock_surface_destroyed`, recording the pointer-enter
observation below and what the `is_locked` gate does *not* ask),
alongside this record itself.

- Dev VM: `cargo test -p flexwm` **372 passed**, `cargo clippy -p flexwm
  --all-targets -- -D warnings` clean, `cargo fmt --all --check` clean.
  macOS: 13 passed (the non-Linux subset), clippy and fmt clean. The 372
  counted, not estimated: 346 at `4d6799e`, plus the **23** that came
  with merging item 17 (`git grep -c '#\[test\]'` across
  `crates/flexwm/src` goes 320 → 343 between `868dd83` and `6bfd29e`:
  21 in `tty/gpu.rs`, 2 in `cli.rs`), plus **3** new here — the fourth
  test named below is an existing one rewritten, not an addition.
- **The two new regression tests fail without the hook**, which is the
  point of having them: with `redraw_after_lock_surface_destroyed`
  disabled, `destroying_only_the_lock_surfaces_role_falls_back_without_waiting_for_damage`
  reads `pixel (0, 0) is [224, 32, 224, 255]` — the destroyed surface's
  literal pixels — and `a_lock_transition_drops_a_grab_a_click_left_behind`
  fails on the grab. Both pass with it.
- `scripts/smoke-test.sh` under **`--headless`** and, inside `cage`,
  **`--nested`**: both exit 0 with all 12 `ok:` checks, on
  `/var/cargo-target/debug/flexwm`
  `sha256:3aafe38d29b2cc975880936d6a085f16a3520cb56606a84edeb22bb16ce5e077`.
- **Real `--tty` hardware, run once per binary so the evidence
  discriminates** (`~/hw-r4-role-destroy.sh`, artifacts in
  `/tmp/hw-r4-before` and `/tmp/hw-r4-after` on the dev VM; a locker
  puts a 1600x1000 magenta lock screen up, holds it 4s, destroys **only**
  its `ext_session_lock_surface_v1`, and the screen is then read ~3s
  later with no pointer motion, no client commit and nothing else
  touching it):
  - pre-fix `4d6799e` (`sha256:8f481bac…`): `3-role-destroyed` is
    **1,599,864 magenta pixels** and `magick compare -metric AE` against
    the locked frame is **0** — the stale frame, exactly the finding.
  - post-fix `166210b` (`sha256:3aafe38d…`): the same read is
    **1,599,895 black pixels** (the remaining 105 are the cursor), AE
    **799,932** against the locked frame. The session is still locked on
    both (`flexwm msg action close` refused), and the rest of the
    lifecycle still works from there: `kill -9` on the locker turns the
    screen fully red, and a third locker takes that red screen over and
    draws its own blue.
  - One behaviour worth recording rather than leaving to be rediscovered:
    the post-fix log shows the locker getting `PTR_ENTER` at the moment
    of the destroy. That is `refresh_pointer_focus` finding the orphaned
    `wl_surface` — which still has its buffer and input region — in the
    hit test. It is the lock owner's own surface, it is no longer drawn,
    and nothing else can be focused while locked, so this is a
    consequence of leaving the orphan registered (see
    `State::lock_surface_destroyed`'s doc for why there is no public way
    to drop it), not a new path anywhere.
- **Round three's own core attack scenario re-run against `166210b`**
  (`~/hw-lock.sh attack-mapped`, artifacts in `/tmp/hw-attack-mapped`),
  because this round touches the same dispatch/render-request code:
  attacker **KEY=0 BTN=0**, the abandoned screen is fully red with no
  attacker magenta anywhere, the real locker's screen is fully green and
  gets the keystrokes, an IPC action is refused, and a VT switch away and
  back leaves the lock screen pixel-identical (AE 120.5, entirely the two
  16x16 cursor boxes the script itself moved between shots — the same
  figure round two recorded).
- **Benchmark on the one per-event path this adds work to**: the
  `destroyed` arm now runs a `TypeId` comparison for every destroyed
  object of every interface. 200,000 `wl_region` create/destroy pairs
  (the cheapest object there is, so nearly all of the measured work *is*
  the destruction path), 5 alternating reps per side, debug builds on
  both sides — the worst case for a comparison a release build folds to
  a constant. `4d6799e` mean **200.4** jiffies (186–211), `166210b`
  **198.6** (190–208), Welch t = 0.33, p ≈ 0.75; the "after" side is
  nominally *faster*, i.e. the difference is noise. ~10µs per
  create+destroy round trip on this rig, unchanged.
