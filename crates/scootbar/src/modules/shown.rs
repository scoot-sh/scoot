//! Writing a [`Shown`] into a module's [`View`]: the one place a payload
//! meets the view, so `payload.rs` itself needs nothing of the bar.

use std::fmt::Write;

use super::View;
use super::payload::Shown;

impl Shown {
    /// Writes this into `view` (which the caller cleared). The text may
    /// be empty: the module then takes no space.
    pub fn write(&self, view: &mut View) {
        let _ = view.text_mut().write_str(&self.text);
        let _ = view.tooltip_mut().write_str(&self.tooltip);
        view.set_class(self.class);
    }
}
