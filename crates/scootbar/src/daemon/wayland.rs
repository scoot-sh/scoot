//! The Wayland connection and the globals scootbar binds.
//!
//! Plain `wayland-client` on its pure-Rust backend, as scootbg:
//!
//! | Global | Needed? |
//! |---|---|
//! | `wl_compositor` (v4+), `wl_shm` | required: every compositor has them |
//! | `zwlr_layer_shell_v1` | required: a bar is a layer surface |
//! | `wp_viewporter` | optional: sizes a buffer drawn at a fractional scale |
//! | `wp_fractional_scale_manager_v1` | optional, with `wp_viewporter`: device-pixel sizes at fractional scales |
//! | `wl_output` (each) | bound as they appear, released as they go |
//!
//! Without the two optional ones the bar is drawn at the integer scale
//! (`wl_output.scale`, the fraction rounded up) and the compositor scales it
//! down: sharp, only not device-exact.
//!
//! ## Forcing the fallback (debug builds only)
//!
//! scoot and sway both have a viewporter, so the fallback is tested by
//! pretending otherwise: a **debug build** reads
//! `SCOOTBAR_DEBUG_NO_VIEWPORTER` (any value) and leaves `wp_viewporter`
//! and `wp_fractional_scale_manager_v1` unbound. A release build reads
//! nothing: the code is compiled out, so the shipped binary has no hidden
//! knob (scootbg's `SCOOTBG_DEBUG_PATH` is the precedent).

use std::fmt;

use wayland_client::globals::{BindError, GlobalError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{ConnectError, Connection, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;

use super::surfaces::Objects;
use crate::bar::Bar;
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
                "the compositor cannot host scootbar: {interface} ({why}): {error}"
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
    pub shm: WlShm,
    pub viewporter: Option<WpViewporter>,
    /// Bound only with a viewporter, the one way to act on a fraction.
    pub fractional_scale: Option<WpFractionalScaleManagerV1>,
}

/// The event-dispatch state.
#[derive(Debug)]
pub struct State {
    pub globals: Globals,
    pub outputs: Outputs<Objects>,
    pub bar: Bar,
}

pub struct Wayland {
    pub conn: Connection,
    pub queue: EventQueue<State>,
    pub qh: QueueHandle<State>,
    pub state: State,
}

impl Wayland {
    /// Connects through `WAYLAND_DISPLAY`/`WAYLAND_SOCKET`, lists the
    /// globals (one round trip), binds them, and binds each output. Also
    /// returns the optional globals the compositor lacks, to say so.
    pub fn connect(bar: Bar) -> Result<(Self, Vec<&'static str>), WaylandError> {
        let conn = Connection::connect_to_env().map_err(WaylandError::Connect)?;
        let (list, queue) = registry_queue_init::<State>(&conn).map_err(WaylandError::Registry)?;
        let qh = queue.handle();

        let required = |interface, why, error| WaylandError::Required {
            interface,
            why,
            error,
        };
        // v4 for `damage_buffer`; v6 adds `preferred_buffer_scale`.
        let compositor = list
            .bind::<WlCompositor, _, _>(&qh, 4..=6, ())
            .map_err(|e| required("wl_compositor", "version 4 or later", e))?;
        let shm = list
            .bind::<WlShm, _, _>(&qh, 1..=1, ())
            .map_err(|e| required("wl_shm", "shared-memory buffers", e))?;
        // v1 is enough: anchors, size, margins, exclusive zone and
        // keyboard interactivity are all there from the start.
        let layer_shell = list
            .bind::<ZwlrLayerShellV1, _, _>(&qh, 1..=4, ())
            .map_err(|e| {
                required(
                    "zwlr_layer_shell_v1",
                    "wlr-layer-shell, which a bar is drawn on",
                    e,
                )
            })?;

        let mut missing = Vec::new();
        let viewporter: Option<WpViewporter> = if no_viewporter() {
            missing.push(WpViewporter::interface().name);
            None
        } else {
            optional(&mut missing, list.bind(&qh, 1..=1, ()))
        };
        let fractional_scale = if viewporter.is_some() {
            optional(&mut missing, list.bind(&qh, 1..=1, ()))
        } else {
            None
        };
        let mut state = State {
            globals: Globals {
                compositor,
                layer_shell,
                shm,
                viewporter,
                fractional_scale,
            },
            outputs: Outputs::default(),
            bar,
        };
        let registry = list.registry().clone();
        list.contents().with_list(|advertised| {
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
        Ok((
            Self {
                conn,
                queue,
                qh,
                state,
            },
            missing,
        ))
    }
}

/// `SCOOTBAR_DEBUG_NO_VIEWPORTER` in a debug build (see the module docs).
#[cfg(debug_assertions)]
fn no_viewporter() -> bool {
    let set = std::env::var_os("SCOOTBAR_DEBUG_NO_VIEWPORTER").is_some();
    if set {
        crate::print::warn(format_args!(
            "scootbar: debug: SCOOTBAR_DEBUG_NO_VIEWPORTER leaves wp_viewporter unbound"
        ));
    }
    set
}

/// A release build has no knob.
#[cfg(not(debug_assertions))]
fn no_viewporter() -> bool {
    false
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
            } => state.global(registry, conn, qh, &interface, name, version),
            wl_registry::Event::GlobalRemove { name } => state.global_remove(name),
            _ => {}
        }
    }
}

// No events scootbar needs: `wl_shm.format` is not needed because
// XRGB8888 is mandatory. Outputs', surfaces', buffers' and callbacks'
// events are handled in `surfaces`.
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: WpViewport);
delegate_noop!(State: WlCompositor);
delegate_noop!(State: ZwlrLayerShellV1);
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpFractionalScaleManagerV1);
