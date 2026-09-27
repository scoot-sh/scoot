//! The Wayland side of each output: its `wl_output` (and `zxdg_output_v1`
//! for one older than v4), and its wallpaper surface, a `wl_surface` with
//! the `zwlr_layer_surface_v1` role. The decisions are the pure model's
//! (`crate::outputs`); this turns events into calls on it and carries out
//! the [`Effect`]s it returns.
//!
//! Each object carries its output's [`OutputId`] as user data, and every
//! event looks the output up by it again. An output removed meanwhile is
//! not found, and the event is dropped, whatever it was.
//!
//! The surface: `background` layer, namespace `wallpaper`, anchored to all
//! four edges with size 0×0 (so the compositor sizes it to the output),
//! exclusive zone -1 (it covers the whole output, under any bar, and
//! reserves nothing), no keyboard interactivity, and an empty input region
//! so pointer and touch events fall through to the desktop. It is committed
//! once with no buffer, which asks for a `configure`; each `configure` is
//! acked straight away, and the surface is then drawn (or redrawn at its
//! new size) by `change::reconcile` if it should show a color. A layer
//! surface maps only with a buffer: with nothing chosen, nothing is shown.

use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1,
};

use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use super::canvas::{Canvas, destroy_viewport};
use super::change::reconcile;
use super::wayland::{Globals, State};
use crate::outputs::{Effect, Entry, OutputId, Surface, Transform};
use crate::print::warn;

/// The newest `wl_output` scootbg knows. Older outputs are still bound.
const OUTPUT_VERSION: u32 = 4;
/// `wl_output.name`, which later work matches `--output NAME` against,
/// exists from v4; an older output is asked for an `xdg_output` instead.
const OUTPUT_NAME_SINCE: u32 = 4;
/// `wl_output.release` exists from v3.
const OUTPUT_RELEASE_SINCE: u32 = 3;
/// `zxdg_output_v1` v2 adds `name` and `description`; v3 moves atomicity to
/// `wl_output.done`. Any version is useful (v1 still has the logical size).
const XDG_OUTPUT_VERSION: u32 = 3;
/// The layer-shell namespace: what the surface is for.
const NAMESPACE: &str = "wallpaper";

/// The Wayland objects for one output.
#[derive(Debug)]
pub struct Objects {
    pub(super) output: WlOutput,
    xdg: Option<ZxdgOutputV1>,
    pub(super) layer: Option<LayerObjects>,
    /// The buffers drawn on the surface; they outlive a surface that the
    /// compositor closes and scootbg re-creates.
    pub(super) canvas: Canvas,
}

impl Objects {
    /// Destroys everything, children first: the layer surface (with its
    /// viewport and `wl_surface`), the buffers, the `xdg_output`, then the
    /// `wl_output` itself (`release` from v3; before that the proxy just
    /// stays inert on our side, which is all the protocol offers).
    fn destroy(mut self) {
        if let Some(layer) = self.layer.take() {
            layer.destroy();
        }
        self.canvas.clear();
        if let Some(xdg) = self.xdg.take() {
            xdg.destroy();
        }
        if self.output.version() >= OUTPUT_RELEASE_SINCE {
            self.output.release();
        }
    }
}

/// A wallpaper surface: the role object, the surface it is on, and the
/// surface's viewport once a color path needs one.
#[derive(Debug)]
pub struct LayerObjects {
    pub(super) surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    pub(super) viewport: Option<WpViewport>,
    /// The size (logical) and buffer scale this surface's persistent state
    /// was last sent for: viewport destination, buffer scale and opaque
    /// region are double-buffered and stay until changed, so a draw at the
    /// same size sends none of them again. `None` for a fresh surface.
    pub(super) sized: Option<(crate::outputs::Size, u32)>,
}

impl LayerObjects {
    /// Creates the surface for `output` and commits it with no buffer, so
    /// the compositor answers with a `configure`.
    pub(super) fn create(
        globals: &Globals,
        output: &WlOutput,
        qh: &QueueHandle<State>,
        id: OutputId,
    ) -> Self {
        let surface = globals.compositor.create_surface(qh, ());
        // An empty region: no input anywhere on the surface.
        let region = globals.compositor.create_region(qh, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        let layer = globals.layer_shell.get_layer_surface(
            &surface,
            Some(output),
            Layer::Background,
            NAMESPACE.to_owned(),
            qh,
            id,
        );
        layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.commit();
        Self {
            surface,
            layer,
            viewport: None,
            sized: None,
        }
    }

    /// The viewport and the role first, then the surface, as the protocols
    /// ask.
    pub(super) fn destroy(self) {
        destroy_viewport(self.viewport);
        self.layer.destroy();
        self.surface.destroy();
    }
}

/// `zxdg_output_manager_v1`: where it is advertised, and the binding once
/// an output needed it.
#[derive(Debug, Default)]
pub struct XdgOutputs {
    /// `(registry name, version)` while the compositor advertises it.
    advertised: Option<(u32, u32)>,
    bound: Option<ZxdgOutputManagerV1>,
}

impl XdgOutputs {
    /// Notes the manager's global if `interface` is it.
    pub fn advertised(&mut self, interface: &str, name: u32, version: u32) {
        if interface == ZxdgOutputManagerV1::interface().name {
            self.advertised = Some((name, version));
        }
    }

    /// The manager, bound on first use; `None` if not advertised.
    fn get(
        &mut self,
        registry: &WlRegistry,
        qh: &QueueHandle<State>,
    ) -> Option<&ZxdgOutputManagerV1> {
        if self.bound.is_none() {
            let (name, version) = self.advertised?;
            self.bound = Some(registry.bind(name, version.min(XDG_OUTPUT_VERSION), qh, ()));
        }
        self.bound.as_ref()
    }

    /// The manager's global went away: forget it. `xdg_output`s made from
    /// it stay valid (the protocol says so).
    fn removed(&mut self, name: u32) {
        if self.advertised.is_some_and(|(n, _)| n == name) {
            self.advertised = None;
            if let Some(manager) = self.bound.take() {
                // `destroy` is the manager's only request, from v1.
                manager.destroy();
            }
        }
    }
}

/// What a `wl_display.sync` callback is for.
#[derive(Debug, Clone, Copy)]
pub enum RoundTrip {
    /// Sent right after binding: every event answering the bind is in.
    Settle(OutputId),
    /// Sent after a `closed`: the output survived it, so re-create.
    Retry(OutputId),
    /// Sent after a `done` changed a configured output: redraw it then, not
    /// at once. A scale or mode change usually brings a `configure` too, and
    /// the compositor may send it after the `done`; drawing at the `done`
    /// would draw at the old surface size and the new scale, a buffer up to
    /// four times too large, only to replace it. By the time this comes
    /// back any such `configure` has been handled, so this redraws only if
    /// none came (a mode and scale doubled together keep the size).
    Redraw(OutputId),
    /// Sent after the commits the `set`s and `clear`s resolved in one loop
    /// turn waited for (numbered by `crate::waiters`): the compositor has
    /// processed them, so their connections get their replies.
    Replies(u64),
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
        let xdg_manager = if version < OUTPUT_NAME_SINCE {
            // No `wl_output.name`: the only other source of one.
            self.xdg.get(registry, qh).cloned()
        } else {
            None
        };
        let id = self.outputs.add(name, |id| {
            let output = registry.bind::<WlOutput, _, _>(name, version, qh, id);
            let xdg = xdg_manager.map(|manager| manager.get_xdg_output(&output, qh, id));
            Objects {
                output,
                xdg,
                layer: None,
                canvas: Canvas::default(),
            }
        });
        // After the bind (and `get_xdg_output`), so its callback comes
        // after every event they caused.
        conn.display().sync(qh, RoundTrip::Settle(id));
    }

    /// A global went away. An output's objects are all destroyed, whatever
    /// state it was in.
    pub fn global_remove(&mut self, name: u32) {
        match self.outputs.remove_global(name) {
            Some(entry) => entry.objects.destroy(),
            None => self.xdg.removed(name),
        }
    }

    /// Carries out `effect` for `entry`.
    fn apply(
        globals: &Globals,
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
                objects.layer = Some(LayerObjects::create(globals, &objects.output, qh, id));
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
                objects.canvas.surface_gone();
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
                    "scootbg: the compositor closed the wallpaper surface on {} again; \
                     giving up on that output (`scootbg query` shows it as gave-up)",
                    entry.output.label()
                ));
            }
        }
    }
}

impl Dispatch<WlOutput, OutputId> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        id: &OutputId,
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.outputs.get_mut(*id) else {
            return;
        };
        let output = &mut entry.output;
        match event {
            wl_output::Event::Geometry {
                transform: WEnum::Value(transform),
                ..
            } => output.stage_transform(transform_of(transform)),
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } => output.stage_mode(flags.contains(wl_output::Mode::Current), width, height),
            wl_output::Event::Scale { factor } => output.stage_scale(factor),
            wl_output::Event::Name { name } => output.stage_name(name),
            wl_output::Event::Description { description } => output.stage_description(description),
            wl_output::Event::Done => {
                output.done();
                redraw_later(entry, conn, qh);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, OutputId> for State {
    fn event(
        state: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        id: &OutputId,
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.outputs.get_mut(*id) else {
            return;
        };
        let output = &mut entry.output;
        match event {
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                output.stage_xdg_logical(width, height);
            }
            zxdg_output_v1::Event::Name { name } => output.stage_name(name),
            zxdg_output_v1::Event::Description { description } => {
                output.stage_description(description);
            }
            // v1 and v2 apply their own events here; v3 leaves it to
            // `wl_output.done`.
            zxdg_output_v1::Event::Done => {
                output.done();
                redraw_later(entry, conn, qh);
            }
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
        let (effect, configured) = match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => (entry.output.configure(serial, width, height), true),
            zwlr_layer_surface_v1::Event::Closed => (entry.output.closed(), false),
            _ => (Effect::None, false),
        };
        Self::apply(&state.globals, entry, effect, conn, qh);
        if configured {
            // Draw, or redraw at the new size. A mapped surface commits
            // after the ack even when nothing about it changed, so the ack
            // takes effect; an unmapped one has nothing to commit.
            let committed = reconcile(&state.globals, &state.choices, entry, qh);
            if !committed && entry.output.shows().is_some() {
                if let Some(layer) = &entry.objects.layer {
                    layer.surface.commit();
                }
            }
        }
    }
}

impl Dispatch<WlBuffer, OutputId> for State {
    fn event(
        state: &mut Self,
        buffer: &WlBuffer,
        event: wl_buffer::Event,
        id: &OutputId,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_buffer::Event::Release = event else {
            return;
        };
        let Some(entry) = state.outputs.get_mut(*id) else {
            return;
        };
        // Single-pixel buffers are released too; only shm slots care.
        if entry.objects.canvas.released(buffer) {
            reconcile(&state.globals, &state.choices, entry, qh);
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
        let (id, retry) = match *round_trip {
            RoundTrip::Settle(id) => (id, false),
            RoundTrip::Retry(id) => (id, true),
            RoundTrip::Redraw(id) => {
                if let Some(entry) = state.outputs.get_mut(id) {
                    reconcile(&state.globals, &state.choices, entry, qh);
                }
                return;
            }
            RoundTrip::Replies(sync) => {
                // Within the capacity `ready` was made with (the bound in
                // `crate::waiters`), and drained every loop turn.
                state.waiters.synced(sync, &mut state.ready);
                return;
            }
        };
        // Removed meanwhile: nothing to do.
        let Some(entry) = state.outputs.get_mut(id) else {
            return;
        };
        let effect = match retry {
            false => entry.output.settled(),
            true => {
                let effect = entry.output.retry();
                if effect == Effect::Create {
                    warn(format_args!(
                        "scootbg: the compositor closed the wallpaper surface on {}; \
                         creating it again (once)",
                        entry.output.label()
                    ));
                }
                effect
            }
        };
        Self::apply(&state.globals, entry, effect, conn, qh);
    }
}

// A wallpaper takes no input and needs no feedback from its surface yet:
// `preferred_buffer_scale` and friends are hidpi-fractional-scale.md's. No
// frame callbacks either: a static color is drawn once per change.
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: WlRegion);

/// After a `done`: a configured surface may need a redraw at a new scale
/// (a full-size buffer) or, configured 0x0 before any mode was known, a
/// first draw; see [`RoundTrip::Redraw`] for why one round trip later.
fn redraw_later(entry: &Entry<Objects>, conn: &Connection, qh: &QueueHandle<State>) {
    if matches!(entry.output.surface(), Surface::Configured { .. }) {
        conn.display()
            .sync(qh, RoundTrip::Redraw(entry.output.id()));
    }
}

fn transform_of(transform: wl_output::Transform) -> Transform {
    use wl_output::Transform as W;
    match transform {
        W::_90 => Transform::Rotate90,
        W::_180 => Transform::Rotate180,
        W::_270 => Transform::Rotate270,
        W::Flipped => Transform::Flipped,
        W::Flipped90 => Transform::Flipped90,
        W::Flipped180 => Transform::Flipped180,
        W::Flipped270 => Transform::Flipped270,
        // `Normal`, and any value a later protocol version adds.
        _ => Transform::Normal,
    }
}
