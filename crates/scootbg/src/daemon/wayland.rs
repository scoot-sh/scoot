//! The Wayland connection and the globals scootbg binds.
//!
//! Plain `wayland-client` on its pure-Rust backend. At startup this binds
//! every global the later wallpaper work needs, so a compositor that cannot
//! host scootbg is refused at once rather than on the first `set`:
//!
//! | Global | Needed? |
//! |---|---|
//! | `wl_compositor` (v4+), `wl_shm` | required: every compositor has them |
//! | `zwlr_layer_shell_v1` | required: a wallpaper is a background layer surface |
//! | `wp_viewporter` | optional: scales a 1x1 colour buffer to the output |
//! | `wp_single_pixel_buffer_manager_v1` | optional: colours without shared memory |
//! | `wp_fractional_scale_manager_v1` | optional: device-pixel sizes at fractional scales |
//! | `wl_output` (each) | bound as they appear, released as they go |
//! | `zxdg_output_manager_v1` | bound only once an output older than v4 (no `name`) appears |
//!
//! The outputs and their surfaces are `surfaces`.

use std::fmt;

use wayland_client::globals::{BindError, GlobalError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::{
    ConnectError, Connection, DispatchError, EventQueue, Proxy, QueueHandle, delegate_noop,
};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;

use super::surfaces::{Objects, XdgOutputs};
use crate::outputs::Outputs;

#[derive(Debug)]
pub enum WaylandError {
    Connect(ConnectError),
    Registry(GlobalError),
    /// A required global is missing or too old.
    Required {
        interface: &'static str,
        why: &'static str,
        error: BindError,
    },
}

impl fmt::Display for WaylandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(f, "cannot connect to the Wayland compositor: {error}"),
            Self::Registry(error) => write!(f, "cannot list the compositor's globals: {error}"),
            Self::Required {
                interface,
                why,
                error,
            } => write!(
                f,
                "the compositor cannot host scootbg: {interface} ({why}): {error}"
            ),
        }
    }
}

impl std::error::Error for WaylandError {}

/// The bound singletons.
#[derive(Debug)]
pub struct Globals {
    pub compositor: WlCompositor,
    pub layer_shell: ZwlrLayerShellV1,
    /// Held so they stay bound for the buffers to come (solid-colour.md,
    /// hidpi-fractional-scale.md).
    #[allow(dead_code)]
    pub shm: WlShm,
    #[allow(dead_code)]
    pub viewporter: Option<WpViewporter>,
    #[allow(dead_code)]
    pub single_pixel: Option<WpSinglePixelBufferManagerV1>,
    #[allow(dead_code)]
    pub fractional_scale: Option<WpFractionalScaleManagerV1>,
}

/// The event-dispatch state.
#[derive(Debug)]
pub struct State {
    pub globals: Globals,
    pub outputs: Outputs<Objects>,
    pub xdg: XdgOutputs,
}

pub struct Wayland {
    pub conn: Connection,
    pub queue: EventQueue<State>,
    pub state: State,
}

impl Wayland {
    /// Connects through `WAYLAND_DISPLAY`/`WAYLAND_SOCKET`, lists the
    /// globals (one round trip), binds them, and binds each output.
    pub fn connect() -> Result<(Self, Vec<&'static str>), WaylandError> {
        let conn = Connection::connect_to_env().map_err(WaylandError::Connect)?;
        let (list, queue) = registry_queue_init::<State>(&conn).map_err(WaylandError::Registry)?;
        let qh = queue.handle();

        let required = |interface, why, error| WaylandError::Required {
            interface,
            why,
            error,
        };
        let compositor = list
            .bind::<WlCompositor, _, _>(&qh, 4..=6, ())
            .map_err(|e| required("wl_compositor", "version 4 or later", e))?;
        let shm = list
            .bind::<WlShm, _, _>(&qh, 1..=2, ())
            .map_err(|e| required("wl_shm", "shared-memory buffers", e))?;
        let layer_shell = list
            .bind::<ZwlrLayerShellV1, _, _>(&qh, 1..=4, ())
            .map_err(|e| {
                required(
                    "zwlr_layer_shell_v1",
                    "wlr-layer-shell, which a wallpaper is drawn on",
                    e,
                )
            })?;

        let mut missing = Vec::new();
        let globals = Globals {
            compositor,
            layer_shell,
            shm,
            viewporter: optional(&mut missing, list.bind(&qh, 1..=1, ())),
            single_pixel: optional(&mut missing, list.bind(&qh, 1..=1, ())),
            fractional_scale: optional(&mut missing, list.bind(&qh, 1..=1, ())),
        };

        let mut state = State {
            globals,
            outputs: Outputs::default(),
            xdg: XdgOutputs::default(),
        };
        let registry = list.registry().clone();
        list.contents().with_list(|advertised| {
            // The manager first: an output listed before it may need it.
            for global in advertised {
                state
                    .xdg
                    .advertised(&global.interface, global.name, global.version);
            }
            for global in advertised {
                state.global(
                    &registry,
                    &conn,
                    &qh,
                    &global.interface,
                    global.name,
                    global.version,
                );
            }
        });

        Ok((Self { conn, queue, state }, missing))
    }

    /// Handles the events already read.
    pub fn dispatch_pending(&mut self) -> Result<usize, DispatchError> {
        self.queue.dispatch_pending(&mut self.state)
    }
}

/// An optional global: `None`, with its name noted, when absent or too
/// old.
fn optional<I: Proxy>(missing: &mut Vec<&'static str>, bound: Result<I, BindError>) -> Option<I> {
    match bound {
        Ok(global) => Some(global),
        Err(_) => {
            missing.push(I::interface().name);
            None
        }
    }
}

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                state.xdg.advertised(&interface, name, version);
                state.global(registry, conn, qh, &interface, name, version);
            }
            wl_registry::Event::GlobalRemove { name } => state.global_remove(name),
            _ => {}
        }
    }
}

// No events scootbg needs: `wl_shm.format` is not needed because XRGB8888
// is mandatory. The outputs', surfaces' and callbacks' events are handled
// in `surfaces`.
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlCompositor);
delegate_noop!(State: ZwlrLayerShellV1);
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpSinglePixelBufferManagerV1);
delegate_noop!(State: WpFractionalScaleManagerV1);
delegate_noop!(State: ZxdgOutputManagerV1);
