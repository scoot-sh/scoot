//! What the compositor says about the scale to draw the cards at: the
//! surface's `preferred_buffer_scale`, its `wp_fractional_scale_v1`
//! `preferred_scale`, and the integer `wl_output.scale` of the outputs it
//! is on, fed to the [`Demand`](scootui::surfaces::Demand) that picks the
//! best one (fractional, else the larger of the other two).
//!
//! The stack opens on a null output unless `output = "NAME"` pins one, so
//! which output it shows on is the compositor's choice: the surface's own
//! events say what it wants, and `wl_surface.enter` names the output whose
//! `wl_output.scale` backs them up on a compositor that sends neither of
//! the first two.
//!
//! A change of the scale to draw at redraws the stack and re-lays it out
//! (the icons decode at a size that follows the scale), and nothing else
//! does: a compositor re-sends the same preferred scale on every enter.
//! Events for a destroyed surface or its fractional-scale object (in
//! flight when it went) are dropped by identity.

use wayland_client::protocol::wl_surface;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};

use super::App;

impl App {
    /// Applies `change` to the demand's scale wishes, and redraws when the
    /// scale to draw at moved.
    fn rescale(&mut self, change: impl FnOnce(&mut scootui::surfaces::Demand)) {
        let before = self.demand.scale();
        change(&mut self.demand);
        if self.demand.scale() != before {
            self.dirty = true;
            // The icons decode at a size that follows the scale, and the
            // next refresh does it.
            self.refresh_due = true;
        }
    }

    /// Feeds the entered outputs' integer scale
    /// ([`OutputScales::current`](scootui::density::outputs::OutputScales::current))
    /// to the demand; none while the surface is on no output changes
    /// nothing.
    pub(super) fn rescale_to_outputs(&mut self) {
        if let Some(factor) = self.scales.current() {
            self.rescale(|demand| demand.set_output_scale(factor));
        }
    }

    /// Whether `surface` is the live one.
    fn is_live_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.live.as_ref().map(|live| live.surface()) == Some(surface)
    }
}

/// `wl_surface`'s own events (v6): the integer scale to draw at, and the
/// outputs it is on. `preferred_buffer_transform` is left alone: an
/// untransformed buffer is correct everywhere.
impl Dispatch<wl_surface::WlSurface, ()> for App {
    fn event(
        app: &mut App,
        surface: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if !app.is_live_surface(surface) {
            return;
        }
        match event {
            wl_surface::Event::PreferredBufferScale { factor } => {
                app.rescale(|demand| demand.prefer_buffer_scale(factor));
            }
            wl_surface::Event::Enter { output } => {
                app.scales.enter(&output);
                app.rescale_to_outputs();
            }
            wl_surface::Event::Leave { output } => {
                app.scales.leave(&output);
                app.rescale_to_outputs();
            }
            _ => {}
        }
    }
}

/// `wp_fractional_scale_v1.preferred_scale`, the best scale there is.
impl Dispatch<WpFractionalScaleV1, ()> for App {
    fn event(
        app: &mut App,
        fractional: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else {
            return;
        };
        let live = app.live.as_ref().and_then(|live| live.fractional());
        if live == Some(fractional) {
            app.rescale(|demand| demand.prefer_fractional(scale));
        }
    }
}

#[cfg(test)]
mod tests;
