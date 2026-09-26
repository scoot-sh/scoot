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
//!
//! No surface is created yet; that is outputs-and-layer-surfaces.md.

use std::fmt;

use wayland_client::globals::{BindError, GlobalError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::{
    ConnectError, Connection, DispatchError, EventQueue, Proxy, QueueHandle, delegate_noop,
};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;

/// `wl_output` v4 adds `name`, which later work uses to match
/// `--output NAME`; older outputs are still bound.
const OUTPUT_VERSION: u32 = 4;

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

/// The bound singletons. Held so they stay bound for the surfaces to come.
#[allow(dead_code)] // Read by the layer-surface work (ticket 3).
#[derive(Debug)]
pub struct Globals {
    pub compositor: WlCompositor,
    pub shm: WlShm,
    pub layer_shell: ZwlrLayerShellV1,
    pub viewporter: Option<WpViewporter>,
    pub single_pixel: Option<WpSinglePixelBufferManagerV1>,
    pub fractional_scale: Option<WpFractionalScaleManagerV1>,
}

/// The event-dispatch state.
#[derive(Debug, Default)]
pub struct State {
    /// `(registry name, output)` for each `wl_output` currently advertised.
    pub outputs: Vec<(u32, WlOutput)>,
}

pub struct Wayland {
    pub conn: Connection,
    pub queue: EventQueue<State>,
    pub state: State,
    #[allow(dead_code)] // Read by the layer-surface work (ticket 3).
    pub globals: Globals,
}

impl Wayland {
    /// Connects through `WAYLAND_DISPLAY`/`WAYLAND_SOCKET`, lists the
    /// globals (one round trip) and binds them.
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
            shm,
            layer_shell,
            viewporter: optional(&mut missing, list.bind(&qh, 1..=1, ())),
            single_pixel: optional(&mut missing, list.bind(&qh, 1..=1, ())),
            fractional_scale: optional(&mut missing, list.bind(&qh, 1..=1, ())),
        };

        let mut state = State::default();
        let registry = list.registry().clone();
        list.contents().with_list(|advertised| {
            for global in advertised {
                if global.interface == WlOutput::interface().name {
                    bind_output(&registry, &qh, &mut state, global.name, global.version);
                }
            }
        });

        Ok((
            Self {
                conn,
                queue,
                state,
                globals,
            },
            missing,
        ))
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

fn bind_output(
    registry: &WlRegistry,
    qh: &QueueHandle<State>,
    state: &mut State,
    name: u32,
    version: u32,
) {
    let output = registry.bind::<WlOutput, _, _>(name, version.min(OUTPUT_VERSION), qh, ());
    state.outputs.push((name, output));
}

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == WlOutput::interface().name => {
                bind_output(registry, qh, state, name, version);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.outputs.iter().position(|(n, _)| *n == name) {
                    let (_, output) = state.outputs.swap_remove(index);
                    // `release` exists from v3; before that the object
                    // simply stays inert on our side.
                    if output.version() >= 3 {
                        output.release();
                    }
                }
            }
            _ => {}
        }
    }
}

// No events scootbg needs yet: the output's mode, scale and name arrive
// with outputs-and-layer-surfaces.md, and `wl_shm.format` is not needed
// because XRGB8888 is mandatory.
delegate_noop!(State: ignore WlOutput);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlCompositor);
delegate_noop!(State: ZwlrLayerShellV1);
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpSinglePixelBufferManagerV1);
delegate_noop!(State: WpFractionalScaleManagerV1);
