//! Writing a [`Shown`] into a module's [`View`]: the one place a payload
//! meets the view, so `payload.rs` itself needs nothing of the bar.

use std::fmt::Write;

use super::View;
use super::payload::Shown;
use crate::icon::Icon;

impl Shown {
    /// Writes this into `view` (which the caller cleared) with the
    /// module's static `icon` and `show_text`: the update's own glyph is
    /// drawn before the text instead of the static icon while set, and
    /// `false` draws only the icon, with the text moved into the tooltip
    /// where the update named none. The placeholder is text like any
    /// update, so it hides under `show-text = false` too. The text may be
    /// empty: the module then takes no space.
    pub fn write_with(&self, view: &mut View, show_text: bool, icon: Option<&Icon>) {
        if show_text {
            let _ = view.text_mut().write_str(&self.text);
        }
        if self.tooltip.is_empty() && !show_text {
            let _ = view.tooltip_mut().write_str(&self.text);
        } else {
            let _ = view.tooltip_mut().write_str(&self.tooltip);
        }
        view.set_class(self.class);
        if let Some(glyph) = self.icon {
            view.show_icon(&Icon::Glyph(glyph));
        } else if let Some(icon) = icon {
            view.show_icon(icon);
        }
    }
}
