//! The Wayland side of each output: its `wl_output`, and its bar, a
//! `wl_surface` with the `zwlr_layer_surface_v1` role. The decisions are the
//! pure model's (`crate::outputs`); this turns events into calls on it and
//! carries out the [`Effect`]s it returns. scootbg's `daemon/surfaces.rs`,
//! the same shape.
//!
//! Each object carries its output's [`OutputId`] as user data, and every
//! event looks the output up by it again. An output removed meanwhile is
//! not found, and the event is dropped, whatever it was.
//!
//! The surface: `top` layer, namespace `scootbar`, anchored to its edge and
//! both sides with size 0 × height, the bar's margins, an exclusive zone of
//! its height (`crate::bar` says why not height plus margin), and no
//! keyboard interactivity. All of that is set before the first commit,
//! which has no buffer, so the compositor reserves the bar's space from
//! that commit and windows move once, before the bar draws. Each
//! `configure` is acked at once; the loop draws (or commits) once the batch
//! of events is dispatched (`daemon::draw`).
//!
//! **Scale.** Where the compositor has `wp_fractional_scale_v1` (and a
//! viewporter), each surface gets a fractional-scale object and a viewport
//! before its first commit, so its `preferred_scale` arrives before its
//! first `configure`. `wl_surface.preferred_buffer_scale` (v6) is read too.

use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1,
};

use super::canvas::Canvas;
use super::wayland::{Globals, State};
use crate::bar::Bar;
use crate::density::Scale;
use crate::modules::{ClickCtx, OutputView, Update};
use crate::outputs::{Effect, Entry, OutputId, Rotation, Size};
use crate::print::warn;
use crate::render::{self, Scene};

/// The newest `wl_output` scootbar knows (v4 adds `name`). Older outputs
/// are still bound.
const OUTPUT_VERSION: u32 = 4;
/// `wl_output.release` exists from v3.
const OUTPUT_RELEASE_SINCE: u32 = 3;
/// The layer-shell namespace: what the surface is for.
pub const NAMESPACE: &str = "scootbar";

/// The Wayland objects for one output.
#[derive(Debug)]
pub struct Objects {
    output: WlOutput,
    pub(super) layer: Option<LayerObjects>,
    /// The buffers drawn on the surface; emptied with it.
    pub(super) canvas: Canvas,
    /// The modules as this output shows them.
    pub(super) scene: Scene,
}

impl Objects {
    /// Destroys everything, children first: the layer surface, the buffers,
    /// then the `wl_output` itself (`release` from v3; before that the
    /// proxy just stays inert on our side, which is all the protocol
    /// offers).
    fn destroy(mut self) {
        if let Some(layer) = self.layer.take() {
            layer.destroy();
        }
        self.canvas.clear();
        if self.output.version() >= OUTPUT_RELEASE_SINCE {
            self.output.release();
        }
    }

    /// The output's own `wl_output`, for matching workspace groups and
    /// pointer surfaces to it by proxy identity.
    fn wl_output(&self) -> &WlOutput {
        &self.output
    }
}

/// A bar surface: the role object, the surface it is on, and its
/// fractional-scale object and viewport where the compositor has them.
#[derive(Debug)]
pub struct LayerObjects {
    pub(super) surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    fractional: Option<WpFractionalScaleV1>,
    pub(super) viewport: Option<WpViewport>,
    /// What this surface's persistent, double-buffered state was last set
    /// to, so a draw sends only what changes: the buffer scale (1 on a
    /// fresh surface, as the protocol says), the viewport's destination and
    /// the opaque region (logical sizes; `None` while never set).
    pub(super) buffer_scale: u32,
    pub(super) destination: Option<Size>,
    pub(super) opaque: Option<Size>,
}

impl LayerObjects {
    /// Creates the bar for `output` and commits it with no buffer, so the
    /// compositor reserves its zone and answers with a `configure`.
    fn create(
        globals: &Globals,
        bar: &Bar,
        output: &WlOutput,
        qh: &QueueHandle<State>,
        id: OutputId,
    ) -> Self {
        let surface = globals.compositor.create_surface(qh, id);
        // Before the first commit, so the scale arrives before the first
        // `configure` and the first draw is at it. Only with a viewporter,
        // without which a fraction cannot be acted on (the manager is not
        // even bound then).
        let viewport = globals
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, qh, ()));
        let fractional = globals
            .fractional_scale
            .as_ref()
            .map(|manager| manager.get_fractional_scale(&surface, qh, id));
        let layer = globals.layer_shell.get_layer_surface(
            &surface,
            Some(output),
            Layer::Top,
            NAMESPACE.to_owned(),
            qh,
            id,
        );
        let anchors = bar.anchors();
        let mut anchor = Anchor::empty();
        for (on, edge) in [
            (anchors.top, Anchor::Top),
            (anchors.bottom, Anchor::Bottom),
            (anchors.left, Anchor::Left),
            (anchors.right, Anchor::Right),
        ] {
            if on {
                anchor |= edge;
            }
        }
        layer.set_anchor(anchor);
        let (width, height) = bar.requested_size();
        layer.set_size(width, height);
        let [top, right, bottom, left] = bar.margins();
        layer.set_margin(top, right, bottom, left);
        layer.set_exclusive_zone(bar.exclusive_zone());
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.commit();
        Self {
            surface,
            layer,
            fractional,
            viewport,
            buffer_scale: 1,
            destination: None,
            opaque: None,
        }
    }

    /// The fractional-scale object, the viewport and the role first, then
    /// the surface, as the protocols ask.
    pub(super) fn destroy(self) {
        if let Some(fractional) = self.fractional {
            fractional.destroy();
        }
        if let Some(viewport) = self.viewport {
            viewport.destroy();
        }
        self.layer.destroy();
        self.surface.destroy();
    }
}

/// What a `wl_display.sync` callback is for.
#[derive(Debug, Clone, Copy)]
pub enum RoundTrip {
    /// Sent right after binding: every event answering the bind is in.
    Settle(OutputId),
    /// Sent after a `closed`: the output survived it, so re-create.
    Retry(OutputId),
}

impl State {
    /// A global appeared (at start-up or later). Binds it if it is an
    /// output.
    pub fn global(
        &mut self,
        registry: &WlRegistry,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        interface: &str,
        name: u32,
        version: u32,
    ) {
        if interface != WlOutput::interface().name {
            return;
        }
        let version = version.min(OUTPUT_VERSION);
        let modules = &self.content.modules;
        let id = self.outputs.add(name, |id| Objects {
            output: registry.bind::<WlOutput, _, _>(name, version, qh, id),
            layer: None,
            canvas: Canvas::new(modules.len()),
            scene: Scene::new(modules),
        });
        // After the bind, so its callback comes after every event it caused.
        conn.display().sync(qh, RoundTrip::Settle(id));
    }

    /// A global went away. An output's objects are all destroyed, whatever
    /// state it was in, mid-configure or mid-draw included.
    pub fn global_remove(&mut self, name: u32) {
        if let Some(entry) = self.outputs.remove_global(name) {
            #[cfg(feature = "workspaces")]
            self.workspaces
                .0
                .borrow_mut()
                .purge_output(entry.objects.wl_output());
            entry.objects.destroy();
        }
    }

    /// Carries out `effect` for `entry`.
    fn apply(
        globals: &Globals,
        bar: &Bar,
        entry: &mut Entry<Objects>,
        effect: Effect,
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let id = entry.output.id();
        let objects = &mut entry.objects;
        match effect {
            Effect::None => {}
            Effect::Create => {
                // The model creates only from a state with no surface, so
                // this never replaces one; if it ever did, the old one is
                // destroyed rather than leaked.
                if let Some(old) = objects.layer.take() {
                    old.destroy();
                }
                objects.layer = Some(LayerObjects::create(globals, bar, &objects.output, qh, id));
            }
            Effect::Ack(serial) => {
                if let Some(layer) = &objects.layer {
                    layer.layer.ack_configure(serial);
                }
            }
            Effect::DestroyAndRetry => {
                if let Some(layer) = objects.layer.take() {
                    layer.destroy();
                }
                objects.canvas.clear();
                // Said when the retry happens, not here: a compositor
                // closes the surfaces of an output it is removing, and
                // that is no news.
                conn.display().sync(qh, RoundTrip::Retry(id));
            }
            Effect::DestroyAndGiveUp => {
                if let Some(layer) = objects.layer.take() {
                    layer.destroy();
                }
                objects.canvas.clear();
                warn(format_args!(
                    "scootbar: the compositor closed the bar on {} again; giving up on \
                     that output until it is plugged in again",
                    entry.output.label()
                ));
            }
        }
    }
}

impl Dispatch<WlOutput, OutputId> for State {
    fn event(
        state: &mut Self,
        output: &WlOutput,
        event: wl_output::Event,
        id: &OutputId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.outputs.get_mut(*id) else {
            return;
        };
        // The live output's only: a removed one's events may be in flight.
        if entry.objects.wl_output() != output {
            return;
        }
        let output = &mut entry.output;
        match event {
            wl_output::Event::Geometry {
                transform: WEnum::Value(transform),
                ..
            } => output.stage_rotation(rotation_of(transform)),
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } => output.stage_mode(flags.contains(wl_output::Mode::Current), width, height),
            wl_output::Event::Scale { factor } => output.stage_scale(factor),
            wl_output::Event::Name { name } => {
                #[cfg(feature = "workspaces")]
                state
                    .workspaces
                    .0
                    .borrow_mut()
                    .note_output_name(entry.objects.wl_output(), &name);
                output.stage_name(name);
            }
            wl_output::Event::Done => output.done(),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, OutputId> for State {
    fn event(
        state: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        id: &OutputId,
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.outputs.get_mut(*id) else {
            return;
        };
        // Only the live surface's events count: one destroyed after a
        // `closed` may still have events in flight.
        if entry.objects.layer.as_ref().map(|l| &l.layer) != Some(layer) {
            return;
        }
        let effect = match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => entry.output.configure(serial, width, height),
            zwlr_layer_surface_v1::Event::Closed => entry.output.closed(),
            _ => Effect::None,
        };
        Self::apply(&state.globals, &state.bar, entry, effect, conn, qh);
    }
}

impl Dispatch<WlBuffer, OutputId> for State {
    fn event(
        state: &mut Self,
        buffer: &WlBuffer,
        event: wl_buffer::Event,
        id: &OutputId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wl_buffer::Event::Release = event else {
            return;
        };
        // A buffer of an output removed meanwhile was destroyed with it.
        if let Some(entry) = state.outputs.get_mut(*id) {
            entry.objects.canvas.released(buffer);
        }
    }
}

impl Dispatch<WlCallback, RoundTrip> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        round_trip: &RoundTrip,
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_callback::Event::Done { .. } = event else {
            return;
        };
        let (RoundTrip::Settle(id) | RoundTrip::Retry(id)) = *round_trip;
        // Removed meanwhile: nothing to do.
        let Some(entry) = state.outputs.get_mut(id) else {
            return;
        };
        let effect = match round_trip {
            RoundTrip::Settle(_) => entry.output.settled(),
            RoundTrip::Retry(_) => {
                let effect = entry.output.retry();
                if effect == Effect::Create {
                    warn(format_args!(
                        "scootbar: the compositor closed the bar on {}; creating it again (once)",
                        entry.output.label()
                    ));
                }
                effect
            }
        };
        Self::apply(&state.globals, &state.bar, entry, effect, conn, qh);
    }
}

/// `wp_fractional_scale_v1.preferred_scale`, the best scale there is.
impl Dispatch<WpFractionalScaleV1, OutputId> for State {
    fn event(
        state: &mut Self,
        fractional: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        id: &OutputId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else {
            return;
        };
        let Some(entry) = state.outputs.get_mut(*id) else {
            return;
        };
        // The live surface's only: a destroyed one's may be in flight.
        let live = entry
            .objects
            .layer
            .as_ref()
            .and_then(|l| l.fractional.as_ref());
        if live == Some(fractional) {
            entry.output.prefer_fractional(scale);
        }
    }
}

/// `wl_surface`'s own events (v6): the integer scale to draw at, used where
/// there is no fractional one. Enter and leave say nothing a bar anchored
/// to one output needs.
impl Dispatch<WlSurface, OutputId> for State {
    fn event(
        state: &mut Self,
        surface: &WlSurface,
        event: wayland_client::protocol::wl_surface::Event,
        id: &OutputId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::protocol::wl_surface::Event;
        let Some(entry) = state.outputs.get_mut(*id) else {
            return;
        };
        if entry.objects.layer.as_ref().map(|l| &l.surface) != Some(surface) {
            return;
        }
        // `preferred_buffer_transform` is left alone, as in scootbg: an
        // untransformed buffer is correct everywhere.
        if let Event::PreferredBufferScale { factor } = event {
            entry.output.prefer_buffer_scale(factor);
        }
    }
}

delegate_noop!(State: WlRegion);

/// `BTN_LEFT` (`linux/input-event-codes.h`): the button a click is. No
/// scroll, no hover, no other button: the workspaces module's minimal hit
/// test only.
const BTN_LEFT: u32 = 0x110;

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities { capabilities } = event else {
            return;
        };
        let has_pointer = matches!(capabilities, WEnum::Value(caps) if caps.contains(wl_seat::Capability::Pointer));
        if has_pointer && state.pointer.is_none() {
            state.pointer = Some(seat.get_pointer(qh, ()));
        } else if !has_pointer && state.pointer.is_some() {
            state.pointer = None;
            state.pointer_on = None;
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        pointer: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if state.pointer.as_ref() != Some(pointer) {
            return;
        }
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                // The bar surface entered, if it is still a live one.
                state.pointer_on = state
                    .outputs
                    .iter_mut()
                    .find(|entry| {
                        entry.objects.layer.as_ref().map(|l| &l.surface) == Some(&surface)
                    })
                    .map(|entry| (entry.output.id(), surface_x, surface_y));
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                if let Some((_, x, y)) = state.pointer_on.as_mut() {
                    (*x, *y) = (surface_x, surface_y);
                }
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_on = None;
            }
            wl_pointer::Event::Button {
                button: BTN_LEFT,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => {
                click(state);
            }
            _ => {}
        }
    }
}

/// A left press: route it to whichever module's span holds the pointer, as
/// a position in that span. Only the workspaces module answers today; any
/// other module keeps the default and nothing happens.
fn click(state: &mut State) {
    let Some((id, x, y)) = state.pointer_on else {
        return;
    };
    if !(0.0..state.bar.height as f64).contains(&y) {
        return;
    }
    let Some(entry) = state
        .outputs
        .iter_mut()
        .find(|entry| entry.output.id() == id)
    else {
        return;
    };
    let scale = entry.output.scale();
    let x = to_device(x, scale);
    let scene = &entry.objects.scene;
    let Some(index) = scene
        .spans()
        .iter()
        .position(|span| span.width > 0 && x >= span.x && x < span.end())
    else {
        return;
    };
    // `view` borrows the scene, the module borrows the content: disjoint,
    // so both live together.
    let scene = &entry.objects.scene;
    let Some(view) = scene.view(index) else {
        return;
    };
    let span = scene.spans()[index];
    let content = &mut state.content;
    let (Some(text), style) = (content.text.as_ref(), &content.style) else {
        // Unreachable with a module placed (the daemon loads a font for
        // any), but a press with no font to hit-test against is nothing.
        return;
    };
    let ctx = ClickCtx {
        output: OutputView {
            name: entry.output.info().name.as_deref(),
        },
        x: x.saturating_sub(span.x),
        view,
        text,
        em: render::em(style.font_size, scale),
        padding: render::device(style.padding, scale),
    };
    if let Some(placed) = content.modules.get_mut(index) {
        if placed.module.on_click(&ctx) == Update::Changed {
            placed.revision = placed.revision.wrapping_add(1);
        }
    }
}

/// Surface-local logical pixels to device pixels at `scale`: pointer
/// coordinates arrive in the surface's (viewport destination) space.
fn to_device(x: f64, scale: Scale) -> u32 {
    if !x.is_finite() || x <= 0.0 {
        return 0;
    }
    match scale {
        Scale::Integer(factor) => (x * f64::from(factor.max(1))) as u32,
        Scale::Fractional(v120) => (x * f64::from(v120.max(1)) / 120.0) as u32,
    }
}

fn rotation_of(transform: wl_output::Transform) -> Rotation {
    use wl_output::Transform as W;
    match transform {
        W::_90 | W::_270 | W::Flipped90 | W::Flipped270 => Rotation::Sideways,
        // `Normal`, 180, the flips, and any value a later protocol version
        // adds.
        _ => Rotation::Upright,
    }
}
