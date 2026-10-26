//! An optional horizontal and vertical slide animation for windows, off by default and free when off.
//!
//! The core (`scoot-core`) keeps returning the TARGET arrangement; this module
//! only interpolates where a window is *drawn* between two targets. No
//! arrangement semantics, focus, hit-testing or IPC geometry moves: the space
//! stays mapped at the target, `scoot msg windows` reports the target, and
//! the pointer hit-test reads the target. The slide is purely cosmetic.
//!
//! # Zero cost when disabled
//!
//! All state lives in `State::slides`, which is empty whenever the option is
//! off (cleared on disable, never filled) and empty whenever the option is on
//! but idle (finished slides are pruned). Every hot path branches once on
//! `slides.is_empty()` and otherwise runs exactly the old code. No timer, no
//! allocation, no per-frame work while empty.
//!
//! # Time source
//!
//! Monotonic frame-clock time (`Instant::now()` sampled on the frame tick and
//! on `apply()`), never a separate timer thread. A late frame jumps its
//! progress forward (frames are skipped, never queued); a frame past its
//! deadline snaps to the target immediately.
//!
//! # Easing
//!
//! Cheap closed-form ease-out cubic (`1-(1-p)^3`: two multiplies, no
//! transcendental, no table), a linear spelling, and an optional spring
//! that overshoots and settles. The spring is a small precomputed table
//! (no per-frame transcendental, no allocation): progress is clamped
//! `0..=1` before it reaches any curve, and every curve lands exactly at
//! `1.0` so a late frame still snaps and the compositor still goes idle.

use std::time::{Duration, Instant};

use scoot_core::{OutputId, Placement, WindowId};

/// Default slide length: short enough to feel instant, long enough to see.
pub const DEFAULT_DURATION_MS: u32 = 150;
/// Clamp range for `duration_ms`. Zero means "snap": a slide that finishes on
/// its first frame.
pub const MIN_DURATION_MS: u32 = 0;
pub const MAX_DURATION_MS: u32 = 500;

/// Which curve a slide follows from its start to its target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Easing {
    /// `1-(1-p)^3`. The default: fast off the mark, gentle landing.
    #[default]
    EaseOut,
    /// Straight-line interpolation. For testing and for anyone who wants it.
    Linear,
    /// An underdamped spring (damping ratio 0.6): overshoots about 9% past
    /// the target mid-flight, then settles back to an exact landing. Same
    /// duration bound as the other curves. For users who prefer niri-style
    /// motion.
    Spring,
}

impl Easing {
    /// Parses the `[animation] easing` spelling. `None` (absent key) is the
    /// default; `Err` carries the unknown name for the caller to warn about.
    pub fn parse(name: Option<&str>) -> Result<Self, String> {
        match name {
            None => Ok(Self::default()),
            Some(raw) => match raw.trim() {
                "ease-out" => Ok(Self::EaseOut),
                "linear" => Ok(Self::Linear),
                "spring" => Ok(Self::Spring),
                _ => Err(raw.to_owned()),
            },
        }
    }

    /// The config-file spelling, for reload reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::EaseOut => "ease-out",
            Self::Linear => "linear",
            Self::Spring => "spring",
        }
    }
}

/// The resolved `[animation]` table: what the live session runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationSettings {
    pub enabled: bool,
    pub duration: Duration,
    pub easing: Easing,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            duration: Duration::from_millis(u64::from(DEFAULT_DURATION_MS)),
            easing: Easing::default(),
        }
    }
}

impl AnimationSettings {
    /// Clamps a raw `duration_ms` into range. Never fails: out-of-range
    /// means the nearest bound, and the caller reports what applied.
    pub fn clamp_duration_ms(raw: u32) -> Duration {
        Duration::from_millis(u64::from(raw.clamp(MIN_DURATION_MS, MAX_DURATION_MS)))
    }
}

/// One window mid-slide: where it was seen when the slide started, where the
/// core wants it now, and when the slide started.
///
/// `from_x`/`from_y` is the *visual* position at retarget time (not the previous
/// target), so a new target arriving mid-slide continues from what is on
/// screen rather than jumping back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slide {
    pub id: WindowId,
    pub from_x: i32,
    pub to_x: i32,
    pub from_y: i32,
    pub to_y: i32,
    pub start: Instant,
    pub duration: Duration,
    pub easing: Easing,
}

/// Fraction of the slide done at `now`, clamped `0..=1`. A zero duration
/// finishes on its first frame rather than dividing by zero.
pub fn progress(now: Instant, start: Instant, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 1.0;
    }
    let elapsed = now.saturating_duration_since(start).as_secs_f32();
    (elapsed / duration.as_secs_f32()).clamp(0.0, 1.0)
}

/// The easing curve: progress in, eased progress out. Pure arithmetic, no
/// transcendental, no table — except the spring, which lerps a small baked
/// table (two loads and a multiply, still no transcendental).
pub fn easing_factor(progress: f32, easing: Easing) -> f32 {
    let clamped = progress.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => clamped,
        Easing::EaseOut => {
            let inv = 1.0 - clamped;
            1.0 - inv * inv * inv
        }
        Easing::Spring => spring_factor(clamped),
    }
}

/// The spring curve, sampled from a 65-entry table over `p` in `0..=1`.
///
/// The entries bake the step response of an underdamped spring (damping
/// ratio 0.6, normalized frequency 7.0): fast off the mark, about 9%
/// overshoot past the target mid-flight, settled back by the deadline.
/// Endpoints are pinned (`[0] == 0.0`, `[64] == 1.0`) so the slide lands
/// exactly and prunes on time. Baked once at compile time so the frame
/// path pays no `exp`/`sin` per window per frame: just a table lookup and
/// a lerp. Clamped duration still bounds the settle, and a frame past its
/// deadline still snaps (`progress` reaches `1.0` first).
const SPRING_TABLE: [f32; 65] = [
    0.000000, 0.005722, 0.021877, 0.046999, //
    0.079705, 0.118695, 0.162760, 0.210781, //
    0.261736, 0.314691, 0.368808, 0.423338, //
    0.477619, 0.531074, 0.583206, 0.633594, //
    0.681888, 0.727804, 0.771121, 0.811673, //
    0.849347, 0.884074, 0.915828, 0.944619, //
    0.970489, 0.993507, 1.013767, 1.03138, //
    1.046476, 1.059194, 1.069685, 1.078105, //
    1.084615, 1.089376, 1.092551, 1.094299, //
    1.094775, 1.094131, 1.092513, 1.090057, //
    1.086895, 1.083148, 1.078931, 1.074349, //
    1.069498, 1.064465, 1.059329, 1.05416, //
    1.04902, 1.043964, 1.039037, 1.034279, //
    1.029722, 1.025393, 1.021312, 1.017495, //
    1.01395, 1.010686, 1.007703, 1.005001, //
    1.002576, 1.00042, 0.998524, 0.996878, //
    1.000000,
];

/// The spring easing curve: progress in (already clamped `0..=1`), eased
/// progress out. Overshoots past `1.0` mid-flight by design; lands exactly.
fn spring_factor(progress: f32) -> f32 {
    const LAST: f32 = (SPRING_TABLE.len() - 1) as f32;
    let scaled = progress * LAST;
    let low = scaled.floor() as usize;
    // `progress <= 1.0`, so `low <= 64`; the `min` is belt and braces.
    let low = low.min(SPRING_TABLE.len() - 1);
    let high = (low + 1).min(SPRING_TABLE.len() - 1);
    let t = scaled - low as f32;
    SPRING_TABLE[low] + (SPRING_TABLE[high] - SPRING_TABLE[low]) * t
}

/// Where a slide is seen at `now`: the interpolation of its endpoints.
/// The spring overshoots past the target by design, so the arithmetic is
/// saturating: an `i32::MAX` window overshooting forward cannot wrap onto
/// the screen. (For the non-overshooting curves the result is identical to
/// plain `+`.)
pub fn slide_position(from_x: i32, to_x: i32, progress: f32, easing: Easing) -> i32 {
    let factor = easing_factor(progress, easing);
    let delta = i64::from(to_x) - i64::from(from_x);
    from_x.saturating_add((delta as f64 * f64::from(factor)).round() as i32)
}

/// Where `slide` is seen at `now`.
pub fn visual_x(slide: &Slide, now: Instant) -> i32 {
    slide_position(
        slide.from_x,
        slide.to_x,
        progress(now, slide.start, slide.duration),
        slide.easing,
    )
}

/// Where `slide` is seen vertically at `now`: the same interpolation on y.
pub fn visual_y(slide: &Slide, now: Instant) -> i32 {
    slide_position(
        slide.from_y,
        slide.to_y,
        progress(now, slide.start, slide.duration),
        slide.easing,
    )
}

/// Whether the slide has reached its target at `now`.
pub fn is_finished(slide: &Slide, now: Instant) -> bool {
    progress(now, slide.start, slide.duration) >= 1.0
}

/// Whether this placement slides at all: tiled windows only. Floating,
/// fullscreen and maximized windows snap (they appear, disappear or resize in
/// place; sliding them would drag the wrong pixels), and invisible windows
/// are not drawn anyway.
pub fn should_slide(placement: &Placement) -> bool {
    placement.visible && !placement.floating && !placement.fullscreen && !placement.maximized
}

/// How far a window's drawn position shifts from its target on both axes at
/// `now`: `(visual_x - target_x, visual_y - target_y)`, `(0, 0)` when there
/// is no slide for it. One predictable branch on `is_empty` covers the
/// disabled and idle cases; otherwise one linear scan of the in-flight list.
/// No allocation. This is what the frame path reads, so a diagonal move
/// costs one scan, not two.
pub fn visual_delta(
    slides: &[Slide],
    id: WindowId,
    target_x: i32,
    target_y: i32,
    now: Instant,
) -> (i32, i32) {
    if slides.is_empty() {
        return (0, 0);
    }
    for slide in slides {
        if slide.id == id {
            return (
                visual_x(slide, now).saturating_sub(target_x),
                visual_y(slide, now).saturating_sub(target_y),
            );
        }
    }
    (0, 0)
}

/// Whether any slide is still in flight at `now`. Empty (the disabled and
/// idle cases) is false without touching time.
pub fn slides_in_flight(slides: &[Slide], now: Instant) -> bool {
    if slides.is_empty() {
        return false;
    }
    slides.iter().any(|slide| !is_finished(slide, now))
}

/// One output's workspace-switch slide: a single horizontal offset the whole
/// incoming strip shares, easing from `from_x` to zero.
///
/// Unlike [`Slide`] (one per window, from the window's own old target), the
/// viewport slide exists because a workspace switch has no old target to
/// slide from: the incoming windows were unmapped (`visible = false`) and
/// the outgoing ones are gone from the arrangement. Drawing the outgoing
/// workspace would double the frame's windows and damage; instead only the
/// incoming strip slides in from the output edge over the background, so
/// every incoming window moves as one unit with one offset.
///
/// `from_x` is `±` the output's logical width (right for a higher workspace
/// index, left for a lower one); the visual at `now` is the interpolation
/// `from_x → 0`, so hit-testing and IPC (which read the target) are
/// untouched and a late frame still snaps to zero and idles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportSlide {
    pub output: OutputId,
    pub from_x: i32,
    pub start: Instant,
    pub duration: Duration,
    pub easing: Easing,
}

/// Where a viewport slide is seen at `now`: its offset from the target.
pub fn viewport_position(
    from_x: i32,
    now: Instant,
    start: Instant,
    duration: Duration,
    easing: Easing,
) -> i32 {
    slide_position(from_x, 0, progress(now, start, duration), easing)
}

/// How far an output's strip shifts from its target at `now`: the viewport
/// slide's visual, `0` when there is none. One branch on `is_empty` covers
/// the disabled and idle cases; otherwise one linear scan over at most
/// `MAX_OUTPUTS` entries. No allocation.
pub fn viewport_delta(slides: &[ViewportSlide], output: OutputId, now: Instant) -> i32 {
    if slides.is_empty() {
        return 0;
    }
    for slide in slides {
        if slide.output == output {
            return viewport_position(slide.from_x, now, slide.start, slide.duration, slide.easing);
        }
    }
    0
}

/// Whether a viewport slide is still in flight at `now`.
pub fn viewport_is_finished(slide: &ViewportSlide, now: Instant) -> bool {
    progress(now, slide.start, slide.duration) >= 1.0
}

/// Whether any viewport slide still needs frames at `now`.
pub fn viewports_in_flight(slides: &[ViewportSlide], now: Instant) -> bool {
    if slides.is_empty() {
        return false;
    }
    slides.iter().any(|slide| !viewport_is_finished(slide, now))
}

/// Drops finished viewport slides in place, keeping the allocation.
pub fn prune_finished_viewports(slides: &mut Vec<ViewportSlide>, now: Instant) {
    if slides.is_empty() {
        return;
    }
    slides.retain(|slide| !viewport_is_finished(slide, now));
}

/// Drops finished slides in place, keeping the allocation. Empty stays empty.
pub fn prune_finished(slides: &mut Vec<Slide>, now: Instant) {
    if slides.is_empty() {
        return;
    }
    slides.retain(|slide| !is_finished(slide, now));
}

use scoot_core::Arrangement;

use super::State;

impl State {
    /// Re-targets slides to a fresh arrangement, from current visuals.
    ///
    /// Call before `apply()` remaps the space, while the space still holds
    /// the old targets: a window whose new target differs from what is on
    /// screen starts (or re-starts) a slide from what is on screen, so a
    /// target arriving mid-slide never jumps back. New windows (no old
    /// position) snap; windows that stop sliding (closed, hidden, floating,
    /// fullscreen, maximized) lose their slide, so nothing goes stale and
    /// nothing leaks. Disabled or locked clears everything and snaps.
    pub(crate) fn update_slides(&mut self, arrangement: &Arrangement, now: Instant, locked: bool) {
        if !self.animation.enabled || locked {
            if !self.slides.is_empty() {
                self.slides.clear();
            }
            if !self.viewport_slides.is_empty() {
                self.viewport_slides.clear();
            }
            self.note_workspace_active();
            return;
        }
        // Drop slides for windows that are gone or no longer slide. Linear in
        // both lists, no allocation: the common case is a handful of slides.
        self.slides.retain(|slide| {
            arrangement
                .placements
                .iter()
                .any(|p| p.id == slide.id && should_slide(p))
        });
        let duration = self.animation.duration;
        let easing = self.animation.easing;
        // A zero duration snaps: no slide to keep, no frame beyond the one
        // `apply()` already asked for.
        if duration.is_zero() {
            if !self.slides.is_empty() {
                self.slides.clear();
            }
            if !self.viewport_slides.is_empty() {
                self.viewport_slides.clear();
            }
            self.note_workspace_active();
            return;
        }
        for placement in &arrangement.placements {
            if !should_slide(placement) {
                continue;
            }
            let target_x = placement.rect.x;
            let target_y = placement.rect.y;
            // What is on screen now: the in-flight visual, else the old
            // target the space still holds, else the new target (a brand-new
            // window snaps rather than sliding in from nowhere).
            let (seen_x, seen_y) = match self.slides.iter().find(|s| s.id == placement.id) {
                Some(slide) => (visual_x(slide, now), visual_y(slide, now)),
                None => match self
                    .windows
                    .get(&placement.id)
                    .and_then(|window| self.space.element_location(window))
                {
                    Some(old) => (old.x, old.y),
                    None => (target_x, target_y),
                },
            };
            if seen_x == target_x && seen_y == target_y {
                // Reached (or never left): no slide to keep.
                if let Some(index) = self.slides.iter().position(|s| s.id == placement.id) {
                    self.slides.swap_remove(index);
                }
                continue;
            }
            match self.slides.iter_mut().find(|s| s.id == placement.id) {
                Some(slide) => {
                    // A new target mid-slide: restart from what is on screen.
                    // No-op when the target did not move (same `to_x`/`to_y`).
                    if slide.to_x != target_x || slide.to_y != target_y {
                        slide.from_x = seen_x;
                        slide.to_x = target_x;
                        slide.from_y = seen_y;
                        slide.to_y = target_y;
                        slide.start = now;
                        slide.duration = duration;
                        slide.easing = easing;
                    }
                }
                None => {
                    self.slides.push(Slide {
                        id: placement.id,
                        from_x: seen_x,
                        to_x: target_x,
                        from_y: seen_y,
                        to_y: target_y,
                        start: now,
                        duration,
                        easing,
                    });
                }
            }
        }
        self.update_viewport_slides(arrangement, now);
    }

    /// Records the current active workspace per output without animating:
    /// what the disabled/locked/zero-duration early returns call so a later
    /// enable still compares against the truth, never a stale index.
    pub(crate) fn note_workspace_active(&mut self) {
        let current: Vec<(OutputId, usize)> = self
            .outputs
            .iter_with_ids()
            .filter_map(|(id, _)| self.world.workspaces(id).map(|ws| (id, ws.active)))
            .collect();
        // Update in place where possible so the steady state (no output
        // added/removed) allocates nothing.
        self.last_workspace_active
            .retain(|(id, _)| current.iter().any(|(cur, _)| cur == id));
        for (id, active) in current {
            match self
                .last_workspace_active
                .iter_mut()
                .find(|(known, _)| *known == id)
            {
                Some(slot) => slot.1 = active,
                None => self.last_workspace_active.push((id, active)),
            }
        }
        // An output that went away takes its viewport slide with it.
        if !self.viewport_slides.is_empty() {
            self.viewport_slides.retain(|slide| {
                self.last_workspace_active
                    .iter()
                    .any(|(id, _)| *id == slide.output)
            });
        }
    }

    /// Starts a viewport slide for every output whose active workspace just
    /// changed, when there is something visible to slide.
    ///
    /// Runs at the end of [`State::update_slides`], after the per-window
    /// pass: the per-window pass already snapped newly-visible windows (no
    /// old location), and this adds the shared viewport offset that makes
    /// them arrive as one strip. Empty and single-window workspaces fall out
    /// naturally: empty has no visible tiled window so no slide starts (but
    /// the index is still recorded), single-window slides its one window.
    /// One allocation-free scan per output; the slide itself is one small
    /// struct per switched output.
    fn update_viewport_slides(&mut self, arrangement: &Arrangement, now: Instant) {
        let current: Vec<(OutputId, usize)> = self
            .outputs
            .iter_with_ids()
            .filter_map(|(id, _)| self.world.workspaces(id).map(|ws| (id, ws.active)))
            .collect();
        let duration = self.animation.duration;
        let easing = self.animation.easing;
        for (id, active) in current {
            match self
                .last_workspace_active
                .iter_mut()
                .find(|(known, _)| *known == id)
            {
                Some(slot) if slot.1 == active => {}
                Some(slot) => {
                    let old = slot.1;
                    slot.1 = active;
                    // Only animate when the new workspace shows a tiled
                    // window on this output; an empty workspace has nothing
                    // to draw, so it snaps (the index update above is the
                    // whole work).
                    let has_visible = arrangement
                        .placements
                        .iter()
                        .any(|p| p.output == id && should_slide(p));
                    if !has_visible {
                        if let Some(index) =
                            self.viewport_slides.iter().position(|s| s.output == id)
                        {
                            self.viewport_slides.swap_remove(index);
                        }
                        continue;
                    }
                    let width = self.world.usable_area(id).map(|area| area.w).unwrap_or(0);
                    if width <= 0 {
                        continue;
                    }
                    let from_x = if active > old { width } else { -width };
                    match self.viewport_slides.iter_mut().find(|s| s.output == id) {
                        Some(slide) => {
                            slide.from_x = from_x;
                            slide.start = now;
                            slide.duration = duration;
                            slide.easing = easing;
                        }
                        None => self.viewport_slides.push(ViewportSlide {
                            output: id,
                            from_x,
                            start: now,
                            duration,
                            easing,
                        }),
                    }
                }
                None => self.last_workspace_active.push((id, active)),
            }
        }
        self.last_workspace_active
            .retain(|(id, _)| self.world.workspaces(*id).is_some());
        if !self.viewport_slides.is_empty() {
            self.viewport_slides.retain(|slide| {
                self.last_workspace_active
                    .iter()
                    .any(|(id, _)| *id == slide.output)
            });
        }
    }

    /// Whether any slide still needs frames at `now`.
    pub(crate) fn animation_in_flight(&self, now: Instant) -> bool {
        slides_in_flight(&self.slides, now) || viewports_in_flight(&self.viewport_slides, now)
    }

    /// Prunes slides that reached their target. Call after a frame draws.
    pub(crate) fn prune_slides(&mut self, now: Instant) {
        prune_finished(&mut self.slides, now);
        prune_finished_viewports(&mut self.viewport_slides, now);
    }
}

#[cfg(test)]
mod tests;
