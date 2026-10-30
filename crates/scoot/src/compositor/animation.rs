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

pub use scoot_rt::easing::{
    DEFAULT_DURATION_MS, Easing, MAX_DURATION_MS, MIN_DURATION_MS, easing_factor, progress,
};

use scoot_core::{OutputId, Placement, WindowId};

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

/// One window mid-fade: a newly opened window going opaque `0 → 1`.
///
/// Open-only, by live measurement: a close fade would have to draw the
/// closed window's last buffers after `remove_window` unmaps it, but a real
/// client (foot) destroys its `wl_surface` on close, so the retained window
/// yields no elements and the fade draws nothing (see
/// `dev/design/slide-animation.md`). The close snaps instead; only opens
/// fade. The render path reads the alpha off this list and multiplies it
/// into the window's own elements; the live map and space still hold the
/// window, so no retain is needed.
///
/// Reuses the slide's timing (`duration`/`easing` from the live
/// `[animation]` table at creation; in-flight fades keep the timing they
/// started with) and the slide's frame scheduling (frames only while in
/// flight, idle when done). Damage is the fading rects only: Smithay's
/// damage tracker keys alpha (`last_alpha`), so an alpha change damages
/// exactly the window's rects, never the full output.
#[derive(Clone, Debug)]
pub struct Fade {
    pub id: WindowId,
    pub start: Instant,
    pub duration: Duration,
    pub easing: Easing,
}

/// Whether a fade has reached its end at `now`.
pub fn is_fade_finished(fade: &Fade, now: Instant) -> bool {
    progress(now, fade.start, fade.duration) >= 1.0
}

/// The alpha an opening window draws at: `0 → 1` over the fade. Pure
/// arithmetic over the eased progress, so tests pin values without time.
pub fn open_alpha(fade: &Fade, now: Instant) -> f32 {
    easing_factor(progress(now, fade.start, fade.duration), fade.easing)
}

/// How opaque a live window draws at `now`: its open fade's alpha, or `1.0`
/// when it has none (the disabled, idle and settled cases). One branch on
/// empty covers disabled/idle; otherwise one linear scan. No allocation.
pub fn fade_alpha(fades: &[Fade], id: WindowId, now: Instant) -> f32 {
    if fades.is_empty() {
        return 1.0;
    }
    for fade in fades {
        if fade.id == id {
            return open_alpha(fade, now);
        }
    }
    1.0
}

/// Whether any fade still needs frames at `now`. Empty (disabled/idle) is
/// false without touching time.
pub fn fades_in_flight(fades: &[Fade], now: Instant) -> bool {
    if fades.is_empty() {
        return false;
    }
    fades.iter().any(|fade| !is_fade_finished(fade, now))
}

/// Drops finished fades in place, keeping the allocation. Empty stays empty.
pub fn prune_finished_fades(fades: &mut Vec<Fade>, now: Instant) {
    if fades.is_empty() {
        return;
    }
    fades.retain(|fade| !is_fade_finished(fade, now));
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
            if !self.fades.is_empty() {
                self.fades.clear();
            }
            self.note_workspace_active();
            self.sync_fade_known(arrangement);
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
            if !self.fades.is_empty() {
                self.fades.clear();
            }
            self.note_workspace_active();
            self.sync_fade_known(arrangement);
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
        self.update_fades(arrangement, now);
    }

    /// Starts an open fade for every genuinely new tiled window, from
    /// transparent. Runs after [`State::update_viewport_slides`], so a
    /// workspace switch's incoming strip (which already rides the shared
    /// viewport offset) does not also fade per-window: double animation
    /// would cost both the offset and the blend every frame for the same
    /// strip.
    ///
    /// A window is "new" when the compositor has never arranged it before
    /// (tracked in [`State::fade_known`]), not when the space merely holds
    /// no old location for it: the space unmaps invisible windows, so a
    /// window scrolling back into view after a sibling closes (its `visible`
    /// flipping false->true) has no old location either, but it is a
    /// returning window, not an opening one, and it snaps instead of fading.
    /// An in-flight open fade is never restarted by a move (the slide
    /// retargets from the visual; the fade keeps its own clock, so nothing
    /// jumps). A retarget mid-fade therefore just continues. Finished fades
    /// prune on the frame tick. Closes never fade: the close snaps (see the
    /// [`Fade`] docs for why retaining the closed window draws nothing
    /// against a real client).
    fn update_fades(&mut self, arrangement: &Arrangement, now: Instant) {
        // Drop fades for windows that are gone or no longer fade (hidden,
        // floated, fullscreened, maximized). Linear in both lists, no
        // allocation.
        self.fades.retain(|fade| {
            arrangement
                .placements
                .iter()
                .any(|p| p.id == fade.id && should_slide(p))
        });
        prune_finished_fades(&mut self.fades, now);
        let duration = self.animation.duration;
        let easing = self.animation.easing;
        for placement in &arrangement.placements {
            if !should_slide(placement) {
                continue;
            }
            // A viewport-covered strip arrives as one unit; no per-window
            // fade on top of it.
            if self
                .viewport_slides
                .iter()
                .any(|s| s.output == placement.output)
            {
                continue;
            }
            // An existing fade or slide means this window was already on
            // screen: a move retargets the slide, the fade rides along.
            if self.fades.iter().any(|f| f.id == placement.id) {
                continue;
            }
            if self.slides.iter().any(|s| s.id == placement.id) {
                continue;
            }
            // The space still holds the old targets (this runs before
            // `apply()` remaps): `Some` means the window was already drawn
            // somewhere, so a stay needs no fade. `None` means brand-new or
            // newly-visible with no old location to slide from -- but only
            // the brand-new ones fade (see `fade_known`): a returning window
            // snaps instead of fading in like a fresh open.
            let is_new = !self.fade_known.contains(&placement.id)
                && match self.windows.get(&placement.id) {
                    Some(window) => self.space.element_location(window).is_none(),
                    None => true,
                };
            if !is_new {
                continue;
            }
            self.fades.push(Fade {
                id: placement.id,
                start: now,
                duration,
                easing,
            });
        }
        self.sync_fade_known(arrangement);
    }

    /// Records every arranged window id as known, so the next
    /// [`State::update_fades`] tells a genuinely new window (fade it in)
    /// from a returning one (snap it). Called on every `update_slides`
    /// path -- including the disabled/locked/zero-duration snaps, so
    /// enabling later does not fade in the windows that were already there.
    /// Bounded by the live arrangement (cleared and re-filled, capacity
    /// retained): a closed window leaves the set on the next pass, and ids
    /// are never reused, so nothing leaks and nothing collides.
    fn sync_fade_known(&mut self, arrangement: &Arrangement) {
        self.fade_known.clear();
        self.fade_known
            .extend(arrangement.placements.iter().map(|p| p.id));
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
    /// A switch arriving mid-flight retargets from the current visual
    /// (never back to the full edge); a landed one starts fresh.
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
                            // A switch mid-flight continues from what is on
                            // screen, like a per-window retarget: the current
                            // visual becomes the new start, so rapid A→B→C
                            // switches and direction reversals never jump
                            // back to the full edge. A landed slide (kept
                            // only until the next tick prunes it) starts
                            // fresh from the edge instead: its visual is the
                            // target already, which would be no slide at all.
                            if viewport_is_finished(slide, now) {
                                slide.from_x = from_x;
                            } else {
                                slide.from_x = viewport_position(
                                    slide.from_x,
                                    now,
                                    slide.start,
                                    slide.duration,
                                    slide.easing,
                                );
                            }
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

    /// Whether any slide or fade still needs frames at `now`.
    pub(crate) fn animation_in_flight(&self, now: Instant) -> bool {
        slides_in_flight(&self.slides, now)
            || viewports_in_flight(&self.viewport_slides, now)
            || fades_in_flight(&self.fades, now)
    }

    /// Prunes slides and fades that reached their target. Call after a frame draws.
    pub(crate) fn prune_slides(&mut self, now: Instant) {
        prune_finished(&mut self.slides, now);
        prune_finished_viewports(&mut self.viewport_slides, now);
        prune_finished_fades(&mut self.fades, now);
    }
}

#[cfg(test)]
mod fade_close_tests;
#[cfg(test)]
mod tests;
