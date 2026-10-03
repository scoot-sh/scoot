//! What the pointer is doing on the popup, as a pure state machine: hover,
//! a press armed on a button, a slider drag. The glue (`daemon::popup`)
//! turns `wl_pointer` events into calls here and carries out what they say.
//!
//! **A button fires on release, over the same button the press armed**, as
//! the bar's own clicks do (`crate::pointer`): a press that slides off
//! before the release does nothing. **A slider acts on press and on every
//! motion that changes its value**, and only then: a pointer moving a
//! thousand times a second within one step is a thousand stores and no
//! action, so the module is asked once per value, never once per event.
//! A drag follows the pointer along the whole row, past the track's ends
//! (clamped to them), but **ends when the pointer leaves the popup**: a
//! popup's grab moves the pointer's focus off it (measured on scoot), so no
//! more motion and not the release arrive, and a drag left held would
//! resume, with no button down, when the pointer came back. [`Interaction::clear`]
//! is what the leave does, and it keeps the last value the module was asked
//! for.
//!
//! **A wheel over the popup scrolls the content a row a notch**, held to
//! what fits ([`Interaction::scroll_by`]): [`Wheel`] groups a frame's axis
//! events the way the bar's own scroll does (discrete notches winning where
//! both arrive), and a frame is bounded work (arithmetic and a store, no
//! allocation).

use super::layout::Layout;
use super::{Activate, Content, Kind};

/// One step of a continuous scroll, in `axis` units (pixels): libinput's
/// 15 per wheel notch, which is also what a wheel with no `axis_value120`
/// reports (as the bar's own scroll counts it).
const AXIS_UNITS: f64 = 15.0;
/// A step, in the 120ths the wheel counts in.
const AXIS_STEP: i32 = 120;

/// A wheel's movement over the popup, grouped by frame into whole rows.
/// Pure (integers in, an integer out), so every sequence is a unit test.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Wheel {
    /// The continuous `axis` movement of the frame being read, in 120ths.
    continuous: i32,
    /// Its discrete movement (`axis_discrete`, `axis_value120`), in 120ths.
    discrete: i32,
    has_discrete: bool,
    /// What the frames so far left short of a row.
    residual: i32,
}

impl Wheel {
    /// Continuous vertical scroll, in pixels, down positive.
    pub fn axis(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
        let units = (value * f64::from(AXIS_STEP) / AXIS_UNITS).clamp(-1.0e6, 1.0e6) as i32;
        self.continuous = self.continuous.saturating_add(units);
    }

    /// Whole wheel notches (`axis_discrete`).
    pub fn axis_discrete(&mut self, notches: i32) {
        self.axis_value120(notches.saturating_mul(AXIS_STEP));
    }

    /// A wheel's movement in 120ths of a notch (`axis_value120`).
    pub fn axis_value120(&mut self, value120: i32) {
        self.discrete = self.discrete.saturating_add(value120);
        self.has_discrete = true;
    }

    /// The scroll ended (a finger lifted): the part of a row it left is
    /// not carried into the next one.
    pub fn axis_stop(&mut self) {
        self.residual = 0;
    }

    /// The frame of events so far is whole: whole rows owed, down
    /// positive. A row a notch; a smooth scroll's fractions wait in the
    /// residual, and a direction change drops what is left.
    pub fn frame(&mut self) -> i32 {
        let units = if self.has_discrete {
            self.discrete
        } else {
            self.continuous
        };
        self.continuous = 0;
        self.discrete = 0;
        self.has_discrete = false;
        if units == 0 {
            return 0;
        }
        if self.residual != 0 && (self.residual < 0) != (units < 0) {
            self.residual = 0;
        }
        let total = self.residual.saturating_add(units);
        let steps = total / AXIS_STEP;
        self.residual = total - steps * AXIS_STEP;
        steps
    }
}

/// What one event did: whether the popup must be drawn again, and the
/// action it asks of the module.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcome {
    pub redraw: bool,
    pub activate: Option<Activate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// A press armed on the button in this row.
    Button(usize),
    /// A drag on the slider in this row, at this value.
    Slider(usize, u32),
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Interaction {
    hover: Option<usize>,
    held: Option<Held>,
    /// How far down the content has scrolled, in device pixels: zero where
    /// it all fits. A wheel over the popup moves it a row a notch, held to
    /// [`Layout::max_scroll`].
    scroll: u32,
}

impl Interaction {
    /// The button row under the pointer.
    pub fn hover(&self) -> Option<usize> {
        self.hover
    }

    /// How far down the content has scrolled, in device pixels.
    pub fn scroll(&self) -> u32 {
        self.scroll
    }

    /// The slider being dragged: its row and the value it shows, which is
    /// the pointer's, ahead of what the module last said.
    pub fn dragging(&self) -> Option<(usize, u32)> {
        match self.held {
            Some(Held::Slider(row, value)) => Some((row, value)),
            _ => None,
        }
    }

    /// Whether a button or a drag is being held.
    #[cfg(test)]
    pub fn is_held(&self) -> bool {
        self.held.is_some()
    }

    /// Forgets everything: the pointer left, or the popup is going. Whether
    /// anything was there to forget.
    pub fn clear(&mut self) -> bool {
        let changed = self.hover.is_some() || self.held.is_some() || self.scroll != 0;
        *self = Self::default();
        changed
    }

    /// The content changed under it: whatever named a row that is gone, or
    /// is no longer the kind of widget it was, is dropped, and the scroll
    /// is held to what the new content allows.
    pub fn retain(&mut self, content: &Content, layout: &Layout) {
        let kind = |row: usize| content.widgets().get(row).map(|w| w.kind);
        if self
            .hover
            .is_some_and(|row| !matches!(kind(row), Some(Kind::Button { .. })))
        {
            self.hover = None;
        }
        let keep = match self.held {
            Some(Held::Button(row)) => matches!(kind(row), Some(Kind::Button { .. })),
            Some(Held::Slider(row, _)) => matches!(kind(row), Some(Kind::Slider { .. })),
            None => true,
        };
        if !keep {
            self.held = None;
        }
        self.scroll = self.scroll.min(layout.max_scroll());
    }

    /// Scrolls by `steps` rows (down positive), held to what the content
    /// allows. Whether the view moved: the caller redraws then. Bounded
    /// work (arithmetic and a store), no allocation.
    pub fn scroll_by(&mut self, layout: &Layout, steps: i32) -> bool {
        if steps == 0 {
            return false;
        }
        let row = layout
            .rows()
            .first()
            .map(|row| row.y1.saturating_sub(row.y0))
            .unwrap_or(0);
        if row == 0 {
            return false;
        }
        let max = layout.max_scroll();
        let scrolled = if steps < 0 {
            let up = (steps as i64).saturating_neg() as u64 * u64::from(row);
            self.scroll.saturating_sub(up.min(u32::MAX as u64) as u32)
        } else {
            let down = steps as u64 * u64::from(row);
            self.scroll.saturating_add(down.min(u32::MAX as u64) as u32)
        };
        let scrolled = scrolled.min(max);
        if scrolled == self.scroll {
            return false;
        }
        self.scroll = scrolled;
        // A press armed on a row that scrolled from under the pointer is
        // dropped: the release must land over the button it armed on, and
        // the pointer did not move (the bar's own clicks work the same way
        // when the modules move instead).
        self.held = None;
        true
    }

    /// A viewport `y` (device pixels in the popup as drawn) as a content
    /// one (device pixels in the laid-out rows).
    fn content_y(&self, y: i64) -> i64 {
        y.saturating_add(self.scroll as i64)
    }

    fn button_at(content: &Content, layout: &Layout, x: i64, y: i64) -> Option<usize> {
        let row = layout.hit(x, y)?;
        matches!(content.widgets().get(row)?.kind, Kind::Button { .. }).then_some(row)
    }

    /// The pointer moved to `(x, y)`, device pixels in the popup as drawn
    /// (the scroll is added to reach the rows).
    pub fn motion(&mut self, content: &Content, layout: &Layout, x: i64, y: i64) -> Outcome {
        if let Some(Held::Slider(row, was)) = self.held {
            let Some((max, action)) = Layout::slider_max(content, row) else {
                self.held = None;
                return Outcome {
                    redraw: true,
                    activate: None,
                };
            };
            let value = layout.value_at(row, x, max);
            if value == was {
                return Outcome::default();
            }
            self.held = Some(Held::Slider(row, value));
            return Outcome {
                redraw: true,
                activate: Some(Activate {
                    action,
                    arg: i32::try_from(value).ok(),
                    closes: false,
                }),
            };
        }
        let over = Self::button_at(content, layout, x, self.content_y(y));
        let redraw = over != self.hover;
        self.hover = over;
        Outcome {
            redraw,
            activate: None,
        }
    }

    /// The left button went down at `(x, y)`.
    pub fn press(&mut self, content: &Content, layout: &Layout, x: i64, y: i64) -> Outcome {
        // A second press while one is held is not a click.
        if self.held.is_some() {
            return Outcome::default();
        }
        let y = self.content_y(y);
        let Some(row) = layout.hit(x, y) else {
            return Outcome::default();
        };
        match content.widgets().get(row).map(|w| w.kind) {
            Some(Kind::Slider { max, action, .. }) => {
                let value = layout.value_at(row, x, max);
                self.held = Some(Held::Slider(row, value));
                Outcome {
                    redraw: true,
                    activate: Some(Activate {
                        action,
                        arg: i32::try_from(value).ok(),
                        closes: false,
                    }),
                }
            }
            Some(Kind::Button { .. }) => {
                self.held = Some(Held::Button(row));
                self.hover = Some(row);
                Outcome {
                    redraw: true,
                    activate: None,
                }
            }
            _ => Outcome::default(),
        }
    }

    /// The left button came up at `(x, y)`.
    pub fn release(&mut self, content: &Content, layout: &Layout, x: i64, y: i64) -> Outcome {
        let held = self.held.take();
        let mut out = Outcome {
            redraw: held.is_some(),
            activate: None,
        };
        if let Some(Held::Button(row)) = held {
            if Self::button_at(content, layout, x, self.content_y(y)) == Some(row) {
                if let Some(Kind::Button {
                    action,
                    arg,
                    closes,
                    ..
                }) = content.widgets().get(row).map(|w| w.kind)
                {
                    out.activate = Some(Activate {
                        action,
                        arg,
                        closes,
                    });
                }
            }
        }
        out
    }
}
