//! Touchpad swipe gestures: a three-finger swipe scrolls the strip under the
//! fingers and settles on a column; a vertical one steps workspaces or
//! windows. `[gestures]` in the config.
//!
//! libinput reports a multi-finger swipe as `begin`, a run of `update`s with
//! the centre's motion, and an `end` (cancelled when the finger count
//! changes). [`State::gesture_swipe_begin`] and its two siblings are the whole
//! interface; the libinput arms in `tty/mod.rs` call them.
//!
//! # All or nothing
//!
//! A swipe is decided once, at its `begin`, and kept to its `end`:
//!
//! - **Claimed** (`fingers == swipe_fingers`, an axis is configured, the
//!   session is unlocked, an output has the focus): scoot handles the whole
//!   swipe and the focused client hears none of it.
//! - **Forwarded** (anything else): the whole swipe goes to
//!   `pointer.gesture_swipe_*` exactly as it came.
//!
//! The route is remembered in [`Gestures::route`], not re-derived from the
//! config, so a reload, a lock or a removed output in the middle of a swipe
//! cannot send a client an `update` or `end` for a `begin` it never got, or
//! the other way round. A `begin` that arrives with the previous swipe still
//! open (libinput never does this) closes the old one first: a forwarded swipe
//! gets a cancelled `end`.
//!
//! This compositor does not advertise `zwp_pointer_gestures_v1` yet, so no
//! client can bind the swipe objects and a forwarded swipe reaches nobody.
//! The forwarding is in place so the rule above already holds when the global
//! lands.
//!
//! # What a claimed swipe does
//!
//! The first ~[`LOCK_DISTANCE`] units of motion pick an axis for the rest of
//! the swipe. Horizontal tracks the fingers: the strip is drawn shifted by the
//! swipe (a purely visual offset on the tiled windows of one output, the same
//! idea as the slide animation -- the committed buffers move, no client is
//! asked to resize, the core's arrangement and hit-testing stay at the
//! targets). Columns scrolled out of view are drawn too while the swipe
//! lasts ([`Gestures::reveal`]), or the strip would open onto bare background.
//! Past the ends of the strip the shift is rubber-banded.
//!
//! On release the nearest column to the view -- or, past [`FLICK_VELOCITY`],
//! the next one in the direction of the flick -- takes the focus, by the same
//! `focus-column` actions a key would run. The core scrolls the view into the
//! new column itself, so the offset is then re-based onto the new targets and
//! eased to zero with the `[animation]` curve (snapped when animation is off).
//! Vertical does not track: past [`VERTICAL_DISTANCE`] it fires one
//! `focus-workspace` or `focus-window` on release.
//!
//! # Cost
//!
//! Idle, the whole feature is `Gestures::output == None` checks. Per update
//! event: float arithmetic and a render request. The column list and reveal
//! list are pooled `Vec`s, refilled once per swipe.

use std::time::Instant;

use scoot_core::{Action, Arrangement, Horizontal, OutputId, Placement, Vertical, WindowId};
use smithay::backend::input::InputTime;
use smithay::input::pointer::{GestureSwipeBeginEvent, GestureSwipeEndEvent};
use smithay::utils::SERIAL_COUNTER;

use super::State;
use super::animation::{AnimationSettings, Easing, viewport_position};

mod config;
#[cfg(test)]
mod tests;

pub(super) use config::GesturesFile;

/// Fingers a swipe needs to be scoot's when `swipe_fingers` is unset.
pub(super) const DEFAULT_FINGERS: u32 = 3;
/// Content motion per libinput swipe unit when `sensitivity` is unset: one
/// logical pixel. Measured through libinput on a synthetic 10 units/mm pad,
/// 0.8 mm of finger travel arrived as 8.4 units, so about 10 units (pixels)
/// per millimetre there; a real pad's scaling may differ.
pub(super) const DEFAULT_SENSITIVITY: f64 = 1.0;
pub(super) const MIN_SENSITIVITY: f64 = 0.1;
pub(super) const MAX_SENSITIVITY: f64 = 10.0;

/// Motion, in scaled units, before the swipe commits to an axis.
const LOCK_DISTANCE: f64 = 12.0;
/// The release speed, in px/s along the strip, at which a swipe is a flick
/// and goes to the next column rather than the nearest.
const FLICK_VELOCITY: f64 = 500.0;
/// Scaled vertical travel a swipe needs to fire its action on release.
const VERTICAL_DISTANCE: f64 = 100.0;
/// An update this long after the last one means the fingers rested: the
/// speed so far no longer describes the release.
const VELOCITY_STALE_MS: u32 = 100;
/// Floor for the time between two updates, so two events stamped the same
/// millisecond cannot make an infinite speed.
const MIN_DT_MS: u32 = 4;
/// How far past either end of the strip the rubber band can pull, as a
/// fraction of the usable width.
const OVERSCROLL: f64 = 0.15;
/// A column counts as "ahead" of the view's edge once it is this many pixels
/// past it, so a column resting exactly on the edge is not skipped.
const AHEAD_EPSILON: f64 = 1.0;
/// Keeps the accumulated motion a sane `f64` whatever the device sends.
const MAX_MOTION: f64 = 1.0e7;

/// What a horizontal swipe does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HorizontalMode {
    FocusColumn,
    None,
}

impl HorizontalMode {
    pub(super) fn parse(text: &str) -> Option<Self> {
        match text {
            "focus-column" => Some(Self::FocusColumn),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::FocusColumn => "focus-column",
            Self::None => "none",
        }
    }
}

/// What a vertical swipe does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VerticalMode {
    FocusWorkspace,
    FocusWindow,
    None,
}

impl VerticalMode {
    pub(super) fn parse(text: &str) -> Option<Self> {
        match text {
            "focus-workspace" => Some(Self::FocusWorkspace),
            "focus-window" => Some(Self::FocusWindow),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::FocusWorkspace => "focus-workspace",
            Self::FocusWindow => "focus-window",
            Self::None => "none",
        }
    }
}

/// The resolved `[gestures]` table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureConfig {
    pub(super) fingers: u32,
    pub(super) horizontal: HorizontalMode,
    pub(super) vertical: VerticalMode,
    /// The content follows the fingers (the macOS default); off, it goes the
    /// other way.
    pub(super) natural: bool,
    pub(super) sensitivity: f64,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            fingers: DEFAULT_FINGERS,
            horizontal: HorizontalMode::FocusColumn,
            vertical: VerticalMode::FocusWorkspace,
            natural: true,
            sensitivity: DEFAULT_SENSITIVITY,
        }
    }
}

impl GestureConfig {
    /// Whether any axis does anything: with both off, swipes are not scoot's.
    fn enabled(&self) -> bool {
        self.horizontal != HorizontalMode::None || self.vertical != VerticalMode::None
    }

    /// Content motion per input unit, signed by the `natural` setting.
    fn scale(&self) -> f64 {
        if self.natural {
            self.sensitivity
        } else {
            -self.sensitivity
        }
    }
}

/// What the swipe in progress is doing, decided at its `begin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// No swipe open.
    Idle,
    /// Handed to the pointer's focus; its `update`s and `end` go there too.
    Forwarded,
    /// Scoot's; the client hears none of it.
    Claimed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// One column of the strip as it was laid out when the swipe began: where its
/// left edge sits on screen at rest.
type ColumnX = i32;

/// The swipe in progress.
#[derive(Clone, Debug)]
struct Claim {
    output: OutputId,
    /// The config as it was at `begin`: a reload mid-swipe changes the next
    /// swipe, not this one.
    config: GestureConfig,
    /// The offset a swipe that caught a settling strip started from (zero
    /// otherwise): the strip's own position, not finger motion, so it takes
    /// no part in picking the axis.
    carried: f64,
    /// Motion since the start, scaled and signed so that positive is content
    /// moving right / down.
    motion: (f64, f64),
    /// Picked once [`LOCK_DISTANCE`] is covered.
    axis: Option<Axis>,
    /// The strip's current visual offset, in pixels: `motion.0` run through
    /// the rubber band.
    shift: f64,
    /// Where `shift` may range without resistance, and how far past it pulls.
    lo: f64,
    hi: f64,
    limit: f64,
    /// `shift` per second, smoothed.
    velocity: f64,
    last_time: u32,
}

/// The offset after a release, easing to zero while the new targets take
/// over.
#[derive(Clone, Copy, Debug)]
struct Settle {
    from: i32,
    start: Instant,
    duration: std::time::Duration,
    easing: Easing,
}

/// All gesture state of the seat.
#[derive(Debug)]
pub(super) struct Gestures {
    pub config: GestureConfig,
    pub(super) route: Route,
    claim: Option<Claim>,
    settle: Option<Settle>,
    /// The output whose strip is shifted: set from `begin` until the settle
    /// lands. `None` is the idle state every hot path tests first.
    output: Option<OutputId>,
    /// Left edges of the output's columns at rest, in strip order. Pooled.
    columns: Vec<ColumnX>,
    /// Every tiled window of the output's active workspace: drawn (and kept
    /// mapped) while the swipe lasts, even scrolled out of view. Pooled.
    reveal: Vec<WindowId>,
}

impl Default for Gestures {
    fn default() -> Self {
        Self {
            config: GestureConfig::default(),
            route: Route::Idle,
            claim: None,
            settle: None,
            output: None,
            columns: Vec::new(),
            reveal: Vec::new(),
        }
    }
}

impl Gestures {
    /// Whether `placement` is drawn only because a swipe reveals it.
    pub(super) fn reveals(&self, placement: &Placement) -> bool {
        !self.reveal.is_empty()
            && !placement.visible
            && !placement.floating
            && !placement.fullscreen
            && self.output == Some(placement.output)
            && self.reveal.contains(&placement.id)
    }

    /// Whether `output`'s windows are on the gesture's clock, not the slide
    /// animation's.
    pub(super) fn owns(&self, output: OutputId) -> bool {
        self.output == Some(output)
    }

    fn offset(&self, now: Instant) -> f64 {
        if let Some(claim) = &self.claim {
            return claim.shift;
        }
        match &self.settle {
            Some(settle) => f64::from(viewport_position(
                settle.from,
                now,
                settle.start,
                settle.duration,
                settle.easing,
            )),
            None => 0.0,
        }
    }

    fn settle_done(&self, now: Instant) -> bool {
        self.settle.as_ref().is_none_or(|settle| {
            super::animation::progress(now, settle.start, settle.duration) >= 1.0
        })
    }

    /// Forgets the swipe's visuals. The route stays: a claimed swipe is still
    /// consumed to its `end`.
    fn clear_visuals(&mut self) -> bool {
        let had_reveal = self.output.is_some() || !self.reveal.is_empty();
        self.claim = None;
        self.settle = None;
        self.output = None;
        self.columns.clear();
        self.reveal.clear();
        had_reveal
    }
}

// ---- pure helpers (tested in `tests.rs`) ----

/// Stretches `raw` back toward `[lo, hi]` the further it strays: free inside,
/// half speed at the edge, never more than `limit` past it.
pub(super) fn rubber(raw: f64, lo: f64, hi: f64, limit: f64) -> f64 {
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, hi) };
    let pull = |over: f64| {
        if limit <= 0.0 {
            0.0
        } else {
            // `limit * over / (over + 2 * limit)`, spelled so that a huge
            // `over` cannot overflow into NaN.
            limit / (1.0 + 2.0 * limit / over)
        }
    };
    if raw > hi {
        hi + pull(raw - hi)
    } else if raw < lo {
        lo - pull(lo - raw)
    } else {
        raw
    }
}

/// The column a release lands on. `columns` are the left edges at rest,
/// `shift` the strip's offset at release, `anchor` the view's left edge and
/// `velocity` the strip's speed in px/s (positive: content moving right).
///
/// Slow, it is the column whose left edge is nearest the view's. At
/// [`FLICK_VELOCITY`] or faster it is the first column strictly ahead of the
/// view in the direction the content is travelling (left edges to the right
/// of the view, for content moving left) -- the next column whatever the
/// distance -- and the nearest again when nothing lies that way.
pub(super) fn settle_column(columns: &[ColumnX], shift: f64, anchor: i32, velocity: f64) -> usize {
    if columns.is_empty() {
        return 0;
    }
    let offset = |index: usize| f64::from(columns[index]) + shift - f64::from(anchor);
    let mut nearest = 0;
    for index in 1..columns.len() {
        if offset(index).abs() < offset(nearest).abs() {
            nearest = index;
        }
    }
    if !velocity.is_finite() || velocity.abs() < FLICK_VELOCITY {
        return nearest;
    }
    if velocity < 0.0 {
        (0..columns.len())
            .find(|&index| offset(index) > AHEAD_EPSILON)
            .unwrap_or(nearest)
    } else {
        (0..columns.len())
            .rev()
            .find(|&index| offset(index) < -AHEAD_EPSILON)
            .unwrap_or(nearest)
    }
}

/// Which axis a swipe that has covered `motion` belongs to, once it has
/// covered enough to tell.
fn pick_axis(motion: (f64, f64)) -> Option<Axis> {
    let (x, y) = (motion.0.abs(), motion.1.abs());
    if x.max(y) < LOCK_DISTANCE {
        None
    } else if x >= y {
        Some(Axis::Horizontal)
    } else {
        Some(Axis::Vertical)
    }
}

/// The smoothed speed after one more step of `step` pixels over `dt_ms`.
fn blend_velocity(previous: f64, step: f64, dt_ms: u32) -> f64 {
    let previous = if dt_ms > VELOCITY_STALE_MS {
        0.0
    } else {
        previous
    };
    let seconds = f64::from(dt_ms.max(MIN_DT_MS)) / 1000.0;
    0.5 * previous + 0.5 * (step / seconds)
}

/// What a swipe that ended `motion` along the strip maps to for the vertical
/// axis: the action to fire on release, if it travelled far enough.
fn vertical_action(mode: VerticalMode, motion: f64) -> Option<Action> {
    if !motion.is_finite() || motion.abs() < VERTICAL_DISTANCE {
        return None;
    }
    // Content moving down uncovers what lies above it.
    let dir = if motion > 0.0 {
        Vertical::Up
    } else {
        Vertical::Down
    };
    match mode {
        VerticalMode::FocusWorkspace => Some(Action::FocusWorkspace(dir)),
        VerticalMode::FocusWindow => Some(Action::FocusWindow(dir)),
        VerticalMode::None => None,
    }
}

// ---- the State side ----

/// What a swipe's `begin` read from the world.
struct Snapshot {
    output: OutputId,
    /// The focused column's index in [`Gestures::columns`], or `None` when
    /// the focus is not in the strip (nothing to move: the swipe is inert).
    focused: Option<usize>,
    lo: f64,
    hi: f64,
    limit: f64,
    anchor: i32,
}

impl State {
    /// A swipe began with `fingers` on the pad. `time` is the event's own
    /// millisecond clock.
    pub(super) fn gesture_swipe_begin(&mut self, fingers: u32, time: u32) {
        self.announce_activity();
        self.note_pointer_activity();
        // A begin with a swipe still open: close the old one first so no
        // client is left holding half a sequence.
        match self.gestures.route {
            Route::Idle => {}
            Route::Forwarded => self.forward_swipe_end(time, true),
            Route::Claimed => {
                self.gesture_abort();
            }
        }
        self.gestures.route = Route::Idle;
        let config = self.gestures.config;
        let claimed = fingers == config.fingers
            && config.enabled()
            && !self.session_lock.is_locked()
            && self.start_claim(config, time);
        if claimed {
            self.gestures.route = Route::Claimed;
        } else {
            self.gestures.route = Route::Forwarded;
            self.forward_swipe_begin(fingers, time);
        }
    }

    /// The fingers moved by `(dx, dy)` (libinput units) since the last event.
    pub(super) fn gesture_swipe_update(&mut self, dx: f64, dy: f64, time: u32) {
        self.announce_activity();
        self.note_pointer_activity();
        match self.gestures.route {
            Route::Idle => {}
            Route::Forwarded => self.forward_swipe_update(dx, dy, time),
            Route::Claimed => self.claimed_update(dx, dy, time),
        }
    }

    /// The fingers lifted (`cancelled` is false) or changed count.
    pub(super) fn gesture_swipe_end(&mut self, cancelled: bool, time: u32) {
        self.announce_activity();
        self.note_pointer_activity();
        let route = std::mem::replace(&mut self.gestures.route, Route::Idle);
        match route {
            Route::Idle => {}
            Route::Forwarded => self.forward_swipe_end(time, cancelled),
            Route::Claimed => self.claimed_end(cancelled, time),
        }
    }

    // -- forwarding --

    fn forward_swipe_begin(&mut self, fingers: u32, time: u32) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let event = GestureSwipeBeginEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time),
            fingers,
        };
        pointer.gesture_swipe_begin(self, &event);
        pointer.frame(self);
    }

    fn forward_swipe_update(&mut self, dx: f64, dy: f64, time: u32) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let event = smithay::input::pointer::GestureSwipeUpdateEvent {
            time: InputTime::from_millis(time),
            delta: (dx, dy).into(),
        };
        pointer.gesture_swipe_update(self, &event);
        pointer.frame(self);
    }

    fn forward_swipe_end(&mut self, time: u32, cancelled: bool) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let event = GestureSwipeEndEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time),
            cancelled,
        };
        pointer.gesture_swipe_end(self, &event);
        pointer.frame(self);
    }

    // -- claiming --

    /// Reads the world and opens a claim. `false` leaves nothing claimed:
    /// no output has the focus, a fullscreen window covers it, or the
    /// strip has a fullscreen column (its geometry is not the strip's).
    fn start_claim(&mut self, config: GestureConfig, time: u32) -> bool {
        let Some(snapshot) = self.gesture_snapshot() else {
            // A settle still in flight loses its reveal list to the failed
            // scan: land it rather than leave its columns mapped.
            self.finish_gesture();
            return false;
        };
        // Taking over a settle in flight continues from the offset on
        // screen, so catching a settling strip does not jump it.
        let now = Instant::now();
        let carried = if self.gestures.output == Some(snapshot.output) {
            self.gestures.offset(now)
        } else {
            0.0
        };
        self.gestures.settle = None;
        self.gestures.output = Some(snapshot.output);
        self.gestures.claim = Some(Claim {
            output: snapshot.output,
            config,
            carried,
            motion: (0.0, 0.0),
            axis: None,
            shift: carried,
            lo: snapshot.lo,
            hi: snapshot.hi,
            limit: snapshot.limit,
            velocity: 0.0,
            last_time: time,
        });
        // The columns scrolled out of view are mapped for the swipe's length.
        self.apply();
        true
    }

    /// The focused output's active strip: its columns' rest positions, the
    /// windows to reveal and the range the view may scroll. Refills the
    /// pooled lists.
    fn gesture_snapshot(&mut self) -> Option<Snapshot> {
        let arrangement = self.world.arrange();
        let output = arrangement.focused_output?;
        if self.world.fullscreen_on(output).is_some() {
            return None;
        }
        let active = self.world.workspaces(output)?.active;
        let area = self
            .world
            .usable_area(output)?
            .inset(self.world.config().gap);
        if area.w <= 0 {
            return None;
        }
        self.gestures.columns.clear();
        self.gestures.reveal.clear();
        let mut focused = None;
        let mut last_x = None;
        let mut right = i32::MIN;
        for placement in &arrangement.placements {
            if placement.output != output || placement.floating {
                continue;
            }
            if self.world.window_workspace(placement.id).map(|w| w.1) != Some(active) {
                continue;
            }
            if placement.fullscreen {
                self.gestures.columns.clear();
                self.gestures.reveal.clear();
                return None;
            }
            if last_x != Some(placement.rect.x) {
                self.gestures.columns.push(placement.rect.x);
                last_x = Some(placement.rect.x);
            }
            right = right.max(placement.rect.x.saturating_add(placement.rect.w));
            self.gestures.reveal.push(placement.id);
            if arrangement.focused == Some(placement.id) {
                focused = Some(self.gestures.columns.len() - 1);
            }
        }
        // The view's range, from the layout's own rule (a view in
        // `[0, strip - view]`): the strip's first column is at `view` to the
        // left of the usable edge. All in i64: strips are client-sized sums.
        let (lo, hi) = match self.gestures.columns.first() {
            Some(&first) => {
                let view = i64::from(area.x) - i64::from(first);
                let strip = i64::from(right) - i64::from(first);
                let scrollable = (strip - i64::from(area.w)).max(0);
                ((view - scrollable) as f64, view as f64)
            }
            None => (0.0, 0.0),
        };
        Some(Snapshot {
            output,
            focused,
            lo,
            hi,
            limit: OVERSCROLL * f64::from(area.w),
            anchor: area.x,
        })
    }

    fn claimed_update(&mut self, dx: f64, dy: f64, time: u32) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        let Some(output) = self.gestures.claim.as_ref().map(|claim| claim.output) else {
            return;
        };
        if self.session_lock.is_locked() || self.world.usable_area(output).is_none() {
            self.gesture_abort();
            return;
        }
        let Some(claim) = self.gestures.claim.as_mut() else {
            return;
        };
        let scale = claim.config.scale();
        let (sx, sy) = (dx * scale, dy * scale);
        claim.motion.0 = (claim.motion.0 + sx).clamp(-MAX_MOTION, MAX_MOTION);
        claim.motion.1 = (claim.motion.1 + sy).clamp(-MAX_MOTION, MAX_MOTION);
        let just_locked = claim.axis.is_none() && {
            claim.axis = pick_axis(claim.motion);
            claim.axis.is_some()
        };
        if claim.axis == Some(Axis::Horizontal) && claim.config.horizontal != HorizontalMode::None {
            let before = claim.shift;
            claim.shift = rubber(
                claim.carried + claim.motion.0,
                claim.lo,
                claim.hi,
                claim.limit,
            );
            if just_locked {
                // The motion before the lock lands in one step here; it was
                // not made in this event's time, so it says nothing about
                // the speed.
                claim.velocity = 0.0;
            } else {
                let since = time.wrapping_sub(claim.last_time);
                claim.velocity = blend_velocity(claim.velocity, claim.shift - before, since);
            }
            claim.last_time = time;
            self.request_render();
        } else {
            claim.last_time = time;
        }
    }

    fn claimed_end(&mut self, cancelled: bool, time: u32) {
        let Some(claim) = self.gestures.claim.take() else {
            // Aborted mid-swipe (lock, output gone): nothing left to settle.
            return;
        };
        let locked = self.session_lock.is_locked();
        let horizontal =
            claim.axis == Some(Axis::Horizontal) && claim.config.horizontal != HorizontalMode::None;
        if cancelled || locked {
            if horizontal && !locked {
                self.begin_settle(claim.shift.round() as i32);
            } else {
                self.finish_gesture();
            }
            return;
        }
        if horizontal {
            let age = time.wrapping_sub(claim.last_time);
            let velocity = if age > VELOCITY_STALE_MS {
                0.0
            } else {
                claim.velocity
            };
            self.release_horizontal(&claim, velocity);
        } else {
            self.finish_gesture();
            if claim.axis == Some(Axis::Vertical)
                && let Some(action) = vertical_action(claim.config.vertical, claim.motion.1)
            {
                self.act_bind(action, false);
            }
        }
    }

    /// Focuses the column the release landed on and starts easing the offset
    /// out against the layout's new targets.
    fn release_horizontal(&mut self, claim: &Claim, velocity: f64) {
        // The layout may have moved under the swipe (a window opened or
        // closed): read the columns and the focus again, so the steps below
        // are counted against what is there now.
        let Some(now) = self.gesture_snapshot() else {
            self.finish_gesture();
            return;
        };
        if now.output != claim.output {
            // The focus moved to another output under the swipe: the strip
            // that was dragged is not the one the focus actions would step.
            self.finish_gesture();
            return;
        }
        let target = settle_column(&self.gestures.columns, claim.shift, now.anchor, velocity);
        let before = self.world.arrange();
        let reference = before.focused;
        let x_before = reference.and_then(|id| before.get(id)).map(|p| p.rect.x);
        if let Some(focused) = now.focused {
            let steps = target.abs_diff(focused);
            let dir = if target > focused {
                Horizontal::Right
            } else {
                Horizontal::Left
            };
            for _ in 0..steps {
                self.act_bind(Action::FocusColumn(dir), false);
            }
        }
        let x_after = reference
            .and_then(|id| self.world.arrange().get(id).map(|p| p.rect.x))
            .or(x_before);
        // Where the strip is on screen now, against where it is now laid
        // out: the difference is what is left to ease away.
        let rebase = match (x_before, x_after) {
            (Some(before), Some(after)) => i64::from(before) - i64::from(after),
            _ => 0,
        };
        let residual = (claim.shift.round() as i64 + rebase)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.begin_settle(residual);
    }

    /// Eases `from` to zero, or lands at once when there is nothing to ease
    /// or animation is off.
    fn begin_settle(&mut self, from: i32) {
        let AnimationSettings {
            enabled,
            duration,
            easing,
        } = self.animation;
        if from == 0 || !enabled || duration.is_zero() || self.session_lock.is_locked() {
            self.finish_gesture();
            return;
        }
        self.gestures.claim = None;
        self.gestures.settle = Some(Settle {
            from,
            start: Instant::now(),
            duration,
            easing,
        });
        self.request_render();
    }

    /// The swipe is over and the strip is at its targets: unmap what was only
    /// revealed.
    fn finish_gesture(&mut self) {
        if self.gestures.clear_visuals() {
            self.apply();
        }
    }

    /// Drops a claimed swipe's visuals without acting (lock, output gone, a
    /// new `begin`). The route stays: the rest of that swipe is still ours to
    /// swallow.
    pub(super) fn gesture_abort(&mut self) {
        self.finish_gesture();
    }

    // -- the frame side --

    /// How far `output`'s tiled windows are drawn from their targets at `now`.
    /// Zero (one `Option` test) while no swipe or settle is on it.
    pub(super) fn gesture_shift(&self, output: Option<OutputId>) -> i32 {
        match (self.gestures.output, output) {
            (Some(owner), Some(asked)) if owner == asked => {
                self.gestures.offset(Instant::now()).round() as i32
            }
            _ => 0,
        }
    }

    /// Per-frame upkeep: lands a finished settle, drops a swipe whose output
    /// or session went away. Returns whether the next frame is needed.
    pub(super) fn gesture_tick(&mut self) -> bool {
        let Some(output) = self.gestures.output else {
            return false;
        };
        let now = Instant::now();
        if self.session_lock.is_locked() || self.world.usable_area(output).is_none() {
            self.gesture_abort();
            return false;
        }
        if self.gestures.claim.is_some() {
            return false;
        }
        if self.gestures.settle_done(now) {
            self.finish_gesture();
            return false;
        }
        true
    }

    /// The core's arrangement as a frame draws it: with the windows a swipe
    /// has scrolled out of view marked visible, so their rings and buffers
    /// are drawn. The same arrangement when idle (one empty-list test).
    pub(super) fn frame_arrangement(&self) -> Arrangement {
        let mut arrangement = self.world.arrange();
        if !self.gestures.reveal.is_empty() {
            for placement in &mut arrangement.placements {
                if self.gestures.reveals(placement) {
                    placement.visible = true;
                }
            }
        }
        arrangement
    }
}
