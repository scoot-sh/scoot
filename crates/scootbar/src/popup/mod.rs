//! Popups, the pure half: what a module asks to show in an `xdg_popup`
//! under its span, how it is laid out, where a pointer lands on it and what
//! it paints. No Wayland objects and no I/O, so every layout, hit and drag
//! is a unit test on plain byte slices (`daemon::popup` carries the
//! protocol).
//!
//! ## Declarative content
//!
//! A module never draws a popup. It fills a [`Content`] ([`Module::popup`],
//! asked when the popup opens and whenever the module's view changes while
//! it is open) with three kinds of widget:
//!
//! - **text**, a line, not interactive;
//! - **a slider**, a value from 0 to a maximum that is set by pressing or
//!   dragging, each new value an [`Activate`];
//! - **a button**, one row that is activated on release over it. A *list* is
//!   a column of buttons, one marked `selected`: there is no separate
//!   widget, because a list is nothing a button does not do.
//!
//! What an interaction does is always a **module action by name** (an
//! [`Activate`]): the daemon performs it through [`crate::action::perform`]
//! like a click binding or an agent's `invoke`, so a popup can do nothing a
//! binding could not, and what the user drags and what an agent invokes
//! cannot diverge.
//!
//! ## No allocation once open
//!
//! [`Content`] keeps its labels in one reused string and its widgets in one
//! reused vector: refilling it (every redraw of a volume drag) allocates
//! nothing once the first fill has sized them. [`Layout`] likewise keeps its
//! rows, and painting writes into the popup's pooled buffer.
//!
//! ## Shaped for extraction
//!
//! Everything under here depends on the bar's pure drawing (`paint`, `text`,
//! `theme`) and nothing else of the daemon, which is the seam
//! `docs/scootbar/backlog/extract-scootui.md` would cut along. It is not
//! extracted: that entry waits for a second binary to need it.
//!
//! [`Module::popup`]: crate::modules::Module::popup
//!
//! Only the volume code (the `volume` and `microphone` features), the
//! network list (the `network` feature) and the tray's menus (the `tray`
//! feature) fill a [`Content`] today, so a build with `popup` and none
//! of those has the widget API and nothing that calls it: allowed to be
//! dead there, which is not a reason to leave `popup` out of such a
//! build.

#![cfg_attr(
    not(any(
        feature = "volume",
        feature = "microphone",
        feature = "network",
        feature = "tray"
    )),
    allow(dead_code)
)]

use std::fmt::{self, Write};
use std::ops::Range;

mod hover;
mod interact;
mod layout;
mod paint;
mod wrap;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tip_tests;

pub use hover::{Hover, Key, Step};
pub use interact::{Interaction, Wheel};
pub use layout::{Layout, TIP_MAX_EM};
pub use paint::paint;

/// The most widgets a popup holds: past it a module's extras are dropped,
/// so a hostile or buggy module cannot grow the popup without bound.
pub const MAX_WIDGETS: usize = 16;
/// The most bytes of label text a popup holds, in all widgets together.
pub const MAX_TEXT: usize = 2048;

/// What a widget is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// `value` of `max`, the module action `action` taking the new value.
    Slider {
        value: u32,
        max: u32,
        action: &'static str,
    },
    /// The module action `action` (with `arg`) on release. `selected` marks
    /// the current choice of a list. `closes` closes the popup on release:
    /// a list row that acts and goes away (the network list's), as against
    /// a button that stays up for another press (the volume popup's mute).
    Button {
        action: &'static str,
        arg: Option<i32>,
        selected: bool,
        closes: bool,
    },
}

/// One widget: its kind, and its label's place in [`Content`]'s text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widget {
    pub kind: Kind,
    label: Range<usize>,
}

/// What a popup shows, top to bottom.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Content {
    widgets: Vec<Widget>,
    text: String,
}

impl Content {
    /// Empties it, keeping the memory.
    pub fn clear(&mut self) {
        self.widgets.clear();
        self.text.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }

    pub fn widgets(&self) -> &[Widget] {
        &self.widgets
    }

    pub fn label(&self, widget: &Widget) -> &str {
        self.text.get(widget.label.clone()).unwrap_or("")
    }

    /// Adds a widget with the label `label` writes. Dropped (and `false`)
    /// past [`MAX_WIDGETS`] or [`MAX_TEXT`], with no partial label kept.
    fn push(&mut self, kind: Kind, label: fmt::Arguments<'_>) -> bool {
        if self.widgets.len() >= MAX_WIDGETS || self.text.len() >= MAX_TEXT {
            return false;
        }
        let start = self.text.len();
        if self.text.write_fmt(label).is_err() {
            self.text.truncate(start);
            return false;
        }
        // Cut at the bound on a character boundary.
        if self.text.len() > MAX_TEXT {
            let mut end = MAX_TEXT;
            while !self.text.is_char_boundary(end) {
                end -= 1;
            }
            self.text.truncate(end);
        }
        let end = self.text.len();
        self.widgets.push(Widget {
            kind,
            label: start..end,
        });
        true
    }

    /// A line of text.
    pub fn text(&mut self, label: fmt::Arguments<'_>) -> bool {
        self.push(Kind::Text, label)
    }

    /// A slider at `value` of `max` (`value` is held to it, `max` to at
    /// least 1); moving it activates `action` with the new value. It has no
    /// label: a line of text above it says what it is.
    pub fn slider(&mut self, value: u32, max: u32, action: &'static str) -> bool {
        let max = max.max(1);
        self.push(
            Kind::Slider {
                value: value.min(max),
                max,
                action,
            },
            format_args!(""),
        )
    }

    /// A button, or a list row. `closes` closes the popup when the row
    /// is selected (a list that acts and goes away); `false` leaves it up.
    pub fn button(
        &mut self,
        label: fmt::Arguments<'_>,
        action: &'static str,
        arg: Option<i32>,
        selected: bool,
        closes: bool,
    ) -> bool {
        self.push(
            Kind::Button {
                action,
                arg,
                selected,
                closes,
            },
            label,
        )
    }
}

/// What an interaction asks of the module: its action `action`, with `arg`.
/// `closes` closes the popup after the action is carried out (the button's
/// flag above). The daemon performs it ([`crate::action::perform`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Activate {
    pub action: &'static str,
    pub arg: Option<i32>,
    pub closes: bool,
}
