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
//! A cheap closed-form ease-out cubic (`1-(1-p)^3`: two multiplies, no
//! transcendental, no table) plus a linear spelling. Progress is clamped
//! `0..=1` before it reaches either.

use std::time::{Duration, Instant};

use scoot_core::{Placement, WindowId};

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
                _ => Err(raw.to_owned()),
            },
        }
    }

    /// The config-file spelling, for reload reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::EaseOut => "ease-out",
            Self::Linear => "linear",
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
/// transcendental, no table.
pub fn easing_factor(progress: f32, easing: Easing) -> f32 {
    let clamped = progress.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => clamped,
        Easing::EaseOut => {
            let inv = 1.0 - clamped;
            1.0 - inv * inv * inv
        }
    }
}

/// Where a slide is seen at `now`: the interpolation of its endpoints.
pub fn slide_position(from_x: i32, to_x: i32, progress: f32, easing: Easing) -> i32 {
    let factor = easing_factor(progress, easing);
    from_x + ((f64::from(to_x - from_x) * f64::from(factor)).round() as i32)
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
    }

    /// Whether any slide still needs frames at `now`.
    pub(crate) fn animation_in_flight(&self, now: Instant) -> bool {
        slides_in_flight(&self.slides, now)
    }

    /// Prunes slides that reached their target. Call after a frame draws.
    pub(crate) fn prune_slides(&mut self, now: Instant) {
        prune_finished(&mut self.slides, now);
    }
}

#[cfg(test)]
mod tests;
