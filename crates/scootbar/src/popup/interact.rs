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

use super::layout::Layout;
use super::{Activate, Content, Kind};

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
}

impl Interaction {
    /// The button row under the pointer.
    pub fn hover(&self) -> Option<usize> {
        self.hover
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
        let changed = self.hover.is_some() || self.held.is_some();
        *self = Self::default();
        changed
    }

    /// The content changed under it: whatever named a row that is gone, or
    /// is no longer the kind of widget it was, is dropped.
    pub fn retain(&mut self, content: &Content) {
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
    }

    fn button_at(content: &Content, layout: &Layout, x: i64, y: i64) -> Option<usize> {
        let row = layout.hit(x, y)?;
        matches!(content.widgets().get(row)?.kind, Kind::Button { .. }).then_some(row)
    }

    /// The pointer moved to `(x, y)`, device pixels in the popup.
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
                }),
            };
        }
        let over = Self::button_at(content, layout, x, y);
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
            if Self::button_at(content, layout, x, y) == Some(row) {
                if let Some(Kind::Button { action, arg, .. }) =
                    content.widgets().get(row).map(|w| w.kind)
                {
                    out.activate = Some(Activate { action, arg });
                }
            }
        }
        out
    }
}
