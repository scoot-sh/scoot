//! The panel's motion: a scale-and-fade entrance, a fade on dismissal, and
//! the selection highlight sliding between rows.
//!
//! Pure over the `Instant`s the caller samples: this module reads no clock,
//! owns no timer and allocates nothing, so a test drives it with a made-up
//! clock and the app asks "where is it now?" once per frame callback while
//! [`Motion::animating`] says something moves. With no motion in the look
//! ([`Timing::NONE`]) every query is at rest and the app takes the path it
//! always did.
//!
//! The curves are [`scootui::motion`]'s (the compositor's own). A spring
//! overshoots: the scale may pass 1 for a moment and the highlight may pass
//! its row (the caller clamps it to the list); the alpha never leaves its
//! range, and an exit never overshoots.

use std::time::Instant;

use scootui::motion::{Settle, Timing};

/// The scale the panel grows from when it opens.
pub const ENTER_SCALE: f32 = 0.96;
/// The scale the panel shrinks to as it fades out.
pub const EXIT_SCALE: f32 = 0.97;
/// The most a spring may scale the panel up, however it overshoots.
const MAX_SCALE: f32 = 1.08;

/// How the whole panel is drawn this frame: scaled about its center and
/// faded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// 1.0 at rest.
    pub scale: f32,
    /// 255 at rest.
    pub alpha: u8,
}

impl Pose {
    /// Drawn as it always was.
    pub const REST: Self = Self {
        scale: 1.0,
        alpha: 255,
    };

    /// Whether this is [`Pose::REST`]: the panel is drawn unscaled and
    /// opaque, the bytes a look without motion paints.
    pub fn is_rest(self) -> bool {
        self == Self::REST
    }
}

/// A dismissal in flight: when it began and the pose it left from, so an
/// Escape during the entrance fades from where the panel is.
#[derive(Debug, Clone, Copy)]
struct Leaving {
    start: Instant,
    from: Pose,
}

/// Where the selection highlight is: its target row offset, and the
/// distance it still has to travel to it.
#[derive(Debug, Clone, Copy, Default)]
struct Highlight {
    target: Option<f32>,
    settle: Option<Settle>,
}

/// The panel's animation state for one open.
#[derive(Debug, Clone, Copy)]
pub struct Motion {
    timing: Timing,
    entered: Option<Instant>,
    leaving: Option<Leaving>,
    highlight: Highlight,
}

impl Motion {
    /// Motion that follows `timing`; [`Timing::NONE`] moves nothing.
    pub fn new(timing: Timing) -> Self {
        Self {
            timing,
            entered: None,
            leaving: None,
            highlight: Highlight::default(),
        }
    }

    /// Whether the look asked for any motion.
    pub fn active(&self) -> bool {
        self.timing.active()
    }

    /// Begins the entrance at `now`, once: the first drawn frame. A second
    /// call (every later draw) changes nothing.
    pub fn start(&mut self, now: Instant) {
        if self.timing.active() && self.entered.is_none() {
            self.entered = Some(now);
        }
    }

    /// The pose at `now`.
    pub fn pose(&self, now: Instant) -> Pose {
        if let Some(leaving) = self.leaving {
            return self.exit_pose(leaving, now);
        }
        self.enter_pose(now)
    }

    fn enter_pose(&self, now: Instant) -> Pose {
        let Some(start) = self.entered else {
            return Pose::REST;
        };
        if self.timing.settled(start, now) {
            return Pose::REST;
        }
        // Past 1.0 mid-flight only for a spring with a bounce.
        let eased = self.timing.eased(start, now);
        Pose {
            scale: (ENTER_SCALE + (1.0 - ENTER_SCALE) * eased).clamp(0.5, MAX_SCALE),
            alpha: (eased.clamp(0.0, 1.0) * 255.0).round() as u8,
        }
    }

    fn exit_pose(&self, leaving: Leaving, now: Instant) -> Pose {
        let progress = self
            .timing
            .without_overshoot()
            .eased(leaving.start, now)
            .clamp(0.0, 1.0);
        Pose {
            scale: leaving.from.scale + (EXIT_SCALE - leaving.from.scale) * progress,
            alpha: (f32::from(leaving.from.alpha) * (1.0 - progress)).round() as u8,
        }
    }

    /// Begins the fade-out at `now`, from the pose on screen. `false` when
    /// the look has no motion: there is nothing to play and the caller
    /// exits at once. A second call keeps the first's start.
    pub fn begin_exit(&mut self, now: Instant) -> bool {
        if !self.timing.active() {
            return false;
        }
        if self.leaving.is_none() {
            self.leaving = Some(Leaving {
                start: now,
                from: self.pose(now),
            });
        }
        true
    }

    /// Whether the fade-out has finished at `now`.
    pub fn exit_done(&self, now: Instant) -> bool {
        self.leaving
            .is_some_and(|leaving| self.timing.without_overshoot().settled(leaving.start, now))
    }

    /// Whether anything is still moving at `now`: the app asks for another
    /// frame callback while this is true and asks for none once it is not.
    pub fn animating(&self, now: Instant) -> bool {
        if let Some(leaving) = self.leaving {
            return !self.timing.without_overshoot().settled(leaving.start, now);
        }
        let entering = self
            .entered
            .is_some_and(|start| !self.timing.settled(start, now));
        entering || self.sliding(now)
    }

    /// Whether the highlight is still travelling at `now`.
    pub fn sliding(&self, now: Instant) -> bool {
        self.highlight
            .settle
            .is_some_and(|settle| !settle.settled(&self.timing, now))
    }

    /// Points the highlight at `target` (its row's offset in device
    /// pixels; `None` when no row is selected on screen). With `animate`,
    /// a moved target slides from where the highlight is now, so a key
    /// held down chases the selection instead of restarting; without it
    /// (the rows changed, nothing was shown before, no motion) it snaps.
    /// The same target again changes nothing.
    pub fn set_highlight(&mut self, target: Option<f32>, animate: bool, now: Instant) {
        let target = target.filter(|target| target.is_finite());
        if self.highlight.target == target {
            return;
        }
        let from = self.highlight_at(now);
        self.highlight.settle = match (from, target) {
            (Some(from), Some(target)) if animate && self.timing.active() => {
                Some(Settle::new(from - target, now))
            }
            _ => None,
        };
        self.highlight.target = target;
    }

    /// The highlight's offset at `now`: its target plus what it still has
    /// to travel. `None` with no target.
    pub fn highlight_at(&self, now: Instant) -> Option<f32> {
        let target = self.highlight.target?;
        let behind = self
            .highlight
            .settle
            .map_or(0.0, |settle| settle.value(&self.timing, now));
        Some(target + behind)
    }
}

/// Where the sliding selection highlight is drawn this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slide {
    /// Its top, in device pixels below the first row's top.
    pub offset: u32,
    /// The visible slot whose row it covers most: that row is drawn in the
    /// selected ink, so the text stays legible while the highlight is
    /// between two rows.
    pub ink_slot: usize,
}

/// Points the highlight at the selected row and answers where it is drawn
/// at `now`. `selected` is the selected row's slot in the `slots` rows
/// shown (`None` when it is scrolled out of view), `row_h` a row's height
/// in device pixels, and `animate` whether the selection moved within the
/// same rows (rows that changed under it snap). `None` without motion, with
/// nothing selected on screen, or with no rows: the caller draws the
/// highlight on the selected row as it always did.
///
/// A spring overshoots, so the offset is held to the shown rows.
pub fn slide(
    motion: &mut Motion,
    selected: Option<usize>,
    slots: usize,
    row_h: u32,
    animate: bool,
    now: Instant,
) -> Option<Slide> {
    if !motion.active() {
        return None;
    }
    let selected = selected.filter(|slot| *slot < slots);
    let travel = |slot: usize| (slot as u64).saturating_mul(u64::from(row_h)) as f32;
    motion.set_highlight(selected.map(travel), animate, now);
    let at = motion.highlight_at(now)?;
    let last = slots.checked_sub(1)?;
    let offset = at.clamp(0.0, travel(last));
    let row = row_h.max(1) as f32;
    let nearest = (((offset + row / 2.0) / row) as usize).min(last);
    Some(Slide {
        offset: offset.round() as u32,
        ink_slot: nearest,
    })
}

#[cfg(test)]
mod tests;
