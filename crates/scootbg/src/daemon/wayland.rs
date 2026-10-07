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
//! | `wp_viewporter` | optional: scales a 1x1 color buffer to the output |
//! | `wp_single_pixel_buffer_manager_v1` | optional: colors without shared memory |
//! | `wp_fractional_scale_manager_v1` | optional, with `wp_viewporter`: device-pixel sizes at fractional scales |
//! | `wl_output` (each) | bound as they appear, released as they go |
//! | `zxdg_output_manager_v1` | bound only once an output older than v4 (no `name`) appears |
//!
//! The outputs and their surfaces are `surfaces`; how colors are drawn,
//! given which of the optional globals exist, is `crate::paint`'s
//! [`Path`].
//!
//! ## Forcing a fallback path (debug builds only)
//!
//! The fallback paths must be tested against compositors that offer
//! everything (scoot and sway both do), so a **debug build** reads
//! `SCOOTBG_DEBUG_PATH` = `viewport-shm` or `full-shm` and takes that path
//! instead, if the globals allow it (`full-shm` stands for a compositor
//! with no viewporter, so it draws no fractional-scale buffers either), and
//! `SCOOTBG_DEBUG_NO_FRACTIONAL_SCALE` (any value) to leave
//! `wp_fractional_scale_manager_v1` unbound, as on a compositor without it.
//! A release build reads neither: the code is compiled out
//! (`cfg(debug_assertions)`), so the shipped binary has no hidden knob.
//! The tests run debug builds, as `cargo test` and `cargo nextest` do by
//! default, and skip the forced-path cases when built without debug
//! assertions.

use std::fmt;

use wayland_client::globals::{BindError, GlobalError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_display::WlDisplay;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{
    ConnectError, Connection, DispatchError, EventQueue, Proxy, QueueHandle, delegate_noop,
};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::presentation_time::client::wp_presentation::{
    Event as PresentationEvent, WpPresentation,
};
use wayland_protocols::wp::single_pixel_buffer::v1::client::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;

use super::images::Images;
use super::respond::Ready;
use super::surfaces::{Objects, XdgOutputs};
use super::transition::Transitions;
use crate::choices::Choices;
use crate::control::{ConnId, MAX_CONNECTIONS};
use crate::jobs::MAX_TRIALS;
use crate::outputs::Outputs;
use crate::paint::Path;
use crate::state::Saved;
use crate::waiters::Waiters;

/// Room for every waiting reply there can be (`crate::waiters`: one per
/// live connection, one per connection evicted in the current turn, and
/// one per queued image trial a newer request superseded).
const WAITERS: usize = 2 * MAX_CONNECTIONS + MAX_TRIALS;

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
    /// The connection's `wl_display`, for round trips sent where there is
    /// no `Connection` to hand (`LayerObjects::create`).
    pub display: WlDisplay,
    pub compositor: WlCompositor,
    pub layer_shell: ZwlrLayerShellV1,
    pub shm: WlShm,
    pub viewporter: Option<WpViewporter>,
    pub single_pixel: Option<WpSinglePixelBufferManagerV1>,
    /// Read through [`Globals::fractional_scale`], which says when it can
    /// be used.
    fractional_scale: Option<WpFractionalScaleManagerV1>,
    /// `wp_presentation`, for pacing transitions: where the compositor
    /// offers it, each frame asks for feedback, and presented or discarded
    /// drives the next one. Without it, frame callbacks and the timer pace
    /// instead.
    pub presentation: Option<WpPresentation>,
    /// How colors are drawn, from the globals above.
    pub path: Path,
}

impl Globals {
    /// The fractional-scale manager, where scootbg can act on what it says:
    /// a fractional-scale buffer is sized by a viewport, so only with a
    /// viewporter, which is exactly when the path uses one ([`Path::FullShm`]
    /// is the path with none; forced in a debug build, it stands for a
    /// compositor without one). See `crate::density`.
    pub fn fractional_scale(&self) -> Option<&WpFractionalScaleManagerV1> {
        let usable = self.viewporter.is_some() && self.path.uses_viewport();
        self.fractional_scale.as_ref().filter(|_| usable)
    }
}

/// The event-dispatch state.
#[derive(Debug)]
pub struct State {
    pub globals: Globals,
    pub outputs: Outputs<Objects>,
    pub xdg: XdgOutputs,
    /// What each output should show.
    pub choices: Choices,
    /// `set`s and `clear`s waiting for the compositor.
    pub waiters: Waiters<ConnId>,
    /// Images waiting for the worker thread, and the worker.
    pub images: Images,
    /// Replies ready for the loop to deliver: those whose sync came back,
    /// and image requests answered without one (refused, or superseded).
    /// Sized for every waiter and every queued image request there can
    /// be, and emptied every loop turn.
    pub ready: Vec<(ConnId, Ready)>,
    /// What is saved for the next start (`crate::state`), and where.
    pub saved: Saved,
    /// The profiles `apply-config` made the daemon leave
    /// (`daemon::config`) whose writers may still be finishing a save,
    /// waited for on the way out, as for `saved`. Only busy ones are kept
    /// (an idle one has nothing to lose), so this holds at most one per
    /// write under way.
    pub retired: Vec<Saved>,
    /// The transitions running on outputs (`daemon::transition`), and the
    /// timer pacing them: empty and disarmed while idle.
    pub transitions: Transitions,
}

pub struct Wayland {
    pub conn: Connection,
    pub queue: EventQueue<State>,
    pub qh: QueueHandle<State>,
    pub state: State,
}

impl Wayland {
    /// Connects through `WAYLAND_DISPLAY`/`WAYLAND_SOCKET`, lists the
    /// globals (one round trip), binds them, and binds each output.
    pub fn connect(
        images: Images,
        saved: Saved,
    ) -> Result<(Self, Vec<&'static str>), WaylandError> {
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
        let viewporter: Option<WpViewporter> = optional(&mut missing, list.bind(&qh, 1..=1, ()));
        let single_pixel: Option<WpSinglePixelBufferManagerV1> =
            optional(&mut missing, list.bind(&qh, 1..=1, ()));
        let presentation: Option<WpPresentation> =
            optional(&mut missing, list.bind(&qh, 1..=1, ()));
        let path = Path::choose(viewporter.is_some(), single_pixel.is_some(), forced_path());
        let globals = Globals {
            display: conn.display(),
            compositor,
            layer_shell,
            shm,
            viewporter,
            single_pixel,
            fractional_scale: if no_fractional_scale() {
                None
            } else {
                optional(&mut missing, list.bind(&qh, 1..=1, ()))
            },
            presentation,
            path,
        };

        let mut state = State {
            globals,
            outputs: Outputs::default(),
            xdg: XdgOutputs::default(),
            choices: Choices::default(),
            waiters: Waiters::with_capacity(WAITERS),
            images,
            ready: Vec::with_capacity(WAITERS + MAX_TRIALS),
            saved,
            retired: Vec::new(),
            transitions: Transitions::default(),
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

    /// Handles the events already read.
    pub fn dispatch_pending(&mut self) -> Result<usize, DispatchError> {
        self.queue.dispatch_pending(&mut self.state)
    }
}

/// `SCOOTBG_DEBUG_PATH` in a debug build (see the module docs): a
/// fallback path to take instead of the best one.
#[cfg(debug_assertions)]
fn forced_path() -> Option<Path> {
    let value = std::env::var_os("SCOOTBG_DEBUG_PATH")?;
    let path = value.to_str().and_then(Path::from_name);
    match path {
        Some(path) => crate::print::warn(format_args!(
            "scootbg: debug: SCOOTBG_DEBUG_PATH asks for the {} path",
            path.name()
        )),
        None => crate::print::warn(format_args!(
            "scootbg: debug: ignoring SCOOTBG_DEBUG_PATH={}: not single-pixel, \
             viewport-shm or full-shm",
            value.to_string_lossy().escape_debug()
        )),
    }
    path
}

/// A release build has no knob.
#[cfg(not(debug_assertions))]
fn forced_path() -> Option<Path> {
    None
}

/// `SCOOTBG_DEBUG_NO_FRACTIONAL_SCALE` in a debug build (see the module
/// docs): behave as if the compositor had no fractional-scale manager.
#[cfg(debug_assertions)]
fn no_fractional_scale() -> bool {
    let set = std::env::var_os("SCOOTBG_DEBUG_NO_FRACTIONAL_SCALE").is_some();
    if set {
        crate::print::warn(format_args!(
            "scootbg: debug: SCOOTBG_DEBUG_NO_FRACTIONAL_SCALE leaves \
             wp_fractional_scale_manager_v1 unbound"
        ));
    }
    set
}

/// A release build has no knob.
#[cfg(not(debug_assertions))]
fn no_fractional_scale() -> bool {
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
// is mandatory. The outputs', surfaces', buffers' and callbacks' events are
// handled in `surfaces`.
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: WpViewport);
delegate_noop!(State: WlCompositor);
delegate_noop!(State: ZwlrLayerShellV1);
delegate_noop!(State: WpViewporter);
// `WpPresentation` sends `clock_id` on bind, which says nothing scootbg
// needs (its transitions pace off the monotonic clock either way).
impl wayland_client::Dispatch<WpPresentation, ()> for State {
    fn event(
        _: &mut Self,
        _: &WpPresentation,
        event: PresentationEvent,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
        // `clock_id`, sent on bind: the presentation clock's domain, which
        // says nothing transitions need (they pace off the monotonic
        // clock either way). Anything else is ignored the same way.
        if let PresentationEvent::ClockId { .. } = event {
        }
    }
}
delegate_noop!(State: WpSinglePixelBufferManagerV1);
delegate_noop!(State: WpFractionalScaleManagerV1);
delegate_noop!(State: ZxdgOutputManagerV1);
