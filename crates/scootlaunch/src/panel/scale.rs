//! What the compositor says about the scale to draw at: the surface's
//! `preferred_buffer_scale`, its `wp_fractional_scale_v1.preferred_scale`,
//! and the integer `wl_output.scale` of the outputs it is on, fed to the
//! [`Demand`](scootui::surfaces::Demand) that picks the best one
//! (fractional, else the larger of the other two).
//!
//! The panel opens on a null output, so which output it shows on is the
//! compositor's choice: the surface's own events say what it wants, and
//! `wl_surface.enter` names the output whose `wl_output.scale` backs them
//! up on a compositor that sends neither of the first two.
//!
//! A change of the scale to draw at redraws the panel, and nothing else
//! does: a compositor re-sends the same preferred scale on every enter.
//! Events for a destroyed surface or its fractional-scale object (in
//! flight when it went) are dropped by identity, as the bar's are.

use wayland_client::protocol::{wl_output, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};

use super::App;

/// The integer scale of each bound output, and which of them the surface is
/// on. Generic over the output handle so the arithmetic is tested without a
/// connection.
#[derive(Debug)]
pub(super) struct OutputScales<K> {
    known: Vec<(K, i32)>,
    entered: Vec<K>,
}

impl<K> Default for OutputScales<K> {
    fn default() -> Self {
        Self {
            known: Vec::new(),
            entered: Vec::new(),
        }
    }
}

impl<K: PartialEq + Clone> OutputScales<K> {
    /// An output was bound: scale 1 until it says otherwise.
    pub fn bound(&mut self, output: K) {
        if !self.known.iter().any(|(known, _)| *known == output) {
            self.known.push((output, 1));
        }
    }

    /// `wl_output.scale`. A factor below 1 is a broken compositor's, and
    /// ignored (the scale divides).
    pub fn set_scale(&mut self, output: &K, factor: i32) {
        if factor < 1 {
            return;
        }
        if let Some((_, scale)) = self.known.iter_mut().find(|(known, _)| known == output) {
            *scale = factor;
        }
    }

    /// `wl_surface.enter`.
    pub fn enter(&mut self, output: &K) {
        if !self.entered.contains(output) {
            self.entered.push(output.clone());
        }
    }

    /// `wl_surface.leave`.
    pub fn leave(&mut self, output: &K) {
        self.entered.retain(|entered| entered != output);
    }

    /// The surface is gone: it is on no output.
    pub fn leave_all(&mut self) {
        self.entered.clear();
    }

    /// The scale to fall back on: the largest of the outputs the surface is
    /// on (drawn larger and scaled down is sharp; the other way is soft),
    /// `None` while it is on none, so nothing is guessed.
    pub fn current(&self) -> Option<i32> {
        self.entered
            .iter()
            .map(|entered| {
                self.known
                    .iter()
                    .find(|(known, _)| known == entered)
                    .map_or(1, |(_, scale)| *scale)
            })
            .max()
    }
}

impl App {
    /// Applies `change` to the demand's scale wishes, and redraws when the
    /// scale to draw at moved.
    fn rescale(&mut self, change: impl FnOnce(&mut scootui::surfaces::Demand)) {
        let before = self.demand.scale();
        change(&mut self.demand);
        if self.demand.scale() != before {
            self.dirty = true;
        }
    }

    /// Feeds the entered outputs' integer scale ([`OutputScales::current`])
    /// to the demand; `None` changes nothing.
    fn rescale_to(&mut self, factor: Option<i32>) {
        if let Some(factor) = factor {
            self.rescale(|demand| demand.set_output_scale(factor));
        }
    }

    /// Whether `surface` is the live one.
    fn is_live_surface(&self, surface: &wl_surface::WlSurface) -> bool {
        self.live.as_ref().map(|live| live.surface()) == Some(surface)
    }
}

/// `wl_surface`'s own events (v6): the integer scale to draw at, and the
/// outputs it is on. `preferred_buffer_transform` is left alone, as in the
/// bar: an untransformed buffer is correct everywhere.
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
                app.rescale_to(app.scales.current());
            }
            wl_surface::Event::Leave { output } => {
                app.scales.leave(&output);
                app.rescale_to(app.scales.current());
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

/// `wl_output.scale`: the fallback under the surface's own events.
impl Dispatch<wl_output::WlOutput, ()> for App {
    fn event(
        app: &mut App,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let wl_output::Event::Scale { factor } = event {
            app.scales.set_scale(output, factor);
            app.rescale_to(app.scales.current());
        }
    }
}

#[cfg(test)]
mod tests;
