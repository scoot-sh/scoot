//! What the compositor owns: Smithay's protocol state, the Wayland windows, and
//! the [`World`] that decides where they go.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use scoot_core::{Action, Config, OutputId, WindowId, World};
use smithay::desktop::{LayerSurface, PopupManager, Space, Window, WindowSurfaceType};
use smithay::input::keyboard::Keycode;
use smithay::input::{Seat, SeatState};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::backend::{
    ClientData, ClientId, DisconnectReason, ObjectId,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{BindError, Display, DisplayHandle, Resource};
use smithay::utils::{Logical, Point};
use smithay::wayland::alpha_modifier::AlphaModifierState;
use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::wayland::content_type::ContentTypeState;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::idle_inhibit::IdleInhibitManagerState;
use smithay::wayland::idle_notify::IdleNotifierState;
use smithay::wayland::input_method::InputMethodManagerState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::presentation::PresentationState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::ext_data_control::DataControlState as ExtDataControlState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::DataControlState as WlrDataControlState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::dialog::XdgDialogState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::tablet_manager::TabletManagerState;
use smithay::wayland::text_input::TextInputManagerState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::xdg_activation::XdgActivationState;
use smithay::wayland::xdg_toplevel_icon::XdgToplevelIconManager;

use crate::cli::RendererKind;

use super::bind_budget::BindBudget;
use super::cursor::Cursor;
use super::decorations::{Appearance, Decorations};
use super::ext_workspace::ExtWorkspaceState;
use super::foreign_toplevel::ForeignToplevels;
use super::foreign_toplevel_management::ForeignToplevelManagement;
use super::gamma_control::GammaControlState;
use super::idle;
use super::input;
use super::ipc::PendingIdle;
use super::keybindings::Keybindings;
use super::layer_shell;
use super::nested::Host;
use super::output_management::OutputManagement;
use super::outputs::Outputs;
use super::popup::ActivePopupGrab;
use super::render::Backend;
use super::screencopy::Screencopy;
use super::screenshot::{Encoder, PendingShot, ShotSink};
use super::session_env;
use super::session_lock::SessionLock;
use super::shm_pools::ShmPools;
use super::tty::Tty;
use super::wayland_accept::WaylandListener;
use super::wl_buffers::WlBuffers;
#[cfg(feature = "xwayland")]
use super::xwayland;

#[cfg(test)]
mod tests;

pub struct State {
    pub start_time: Instant,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, State>,
    pub loop_signal: LoopSignal,
    pub socket_name: OsString,
    pub ipc_path: Option<PathBuf>,
    /// The config file this session started from: the explicit `--config`
    /// path when one was given, else the resolved XDG default path (whether
    /// or not a file existed there at startup). What `Request::Reload`
    /// re-reads -- see `reload.rs`. `None` only when no path could be
    /// resolved at all (neither `XDG_CONFIG_HOME` nor `HOME` set), in which
    /// case a reload answers an error rather than guessing.
    pub config_path: Option<PathBuf>,
    /// The config values a reload diffs against snapshots for: `[tty] gpu`
    /// as the file named it (`None` when unset) and `[autostart] commands`
    /// as last decided. The gpu value has no live state to compare a
    /// reloaded file with (the device is already driven), so the startup
    /// value is what decides "changed, and refused" versus "agreed,
    /// silent" -- set once in `run`, never written after. The autostart
    /// list is the spawn-delta snapshot instead: seeded once in `run` with
    /// what startup drained, then advanced past the whole fresh list by
    /// every *unlocked* reload (a *locked* reload freezes it, deferring new
    /// entries to the first unlocked reload) -- see `reload.rs`.
    pub startup_gpu: Option<PathBuf>,
    pub startup_autostart: Vec<Action>,
    /// Whether the session asked for XWayland (`--xwayland` or `[xwayland]
    /// enabled` -- see `xwayland::resolve`), seeded once in `run`, never
    /// written after. What a reload diffs `[xwayland] enabled` against: the
    /// server starts once at startup, so a change refuses with "takes
    /// effect on restart" (see `reload.rs`) -- like `[tty] gpu` above, this
    /// is a request snapshot, not liveness (`xdisplay` says whether the
    /// server is actually up).
    pub startup_xwayland: bool,

    /// The layout. Everything else here exists to serve it.
    pub world: World,
    pub windows: HashMap<WindowId, Window>,
    pub next_id: u64,
    /// The focused *window*, so activation and the focus ring are only moved
    /// when they change. Not necessarily what holds the keyboard: a layer
    /// surface can (see `clicked_layer` and `layer_shell.rs`), and this stays
    /// pointing at the window focus will come back to when it doesn't.
    pub focus: Option<WindowId>,
    /// Which window covered each output, by output index, as of the last
    /// `apply()` -- `World::fullscreen_on` for every output, remembered only
    /// so `apply()` can tell when it changed (see
    /// `State::refresh_fullscreen_cover`). Updated in place, so it allocates
    /// only when an output is added.
    pub(super) fullscreen_covers: Vec<Option<WindowId>>,
    /// A fingerprint of every visible floating window's id, stacking order
    /// and rect as of the last `apply()`, so `apply()` can tell when what
    /// floats under the pointer may have changed (see
    /// `State::refresh_floating_cover`). A number, not a list: allocation-free.
    pub(super) floating_cover: u64,
    /// Toplevels created but not yet committed: the ones whose first commit
    /// is still to decide whether they float (see `floating.rs`). Almost
    /// always empty -- a client creates a toplevel and commits it in the same
    /// flush -- so the commit path's check is an empty-`Vec` test.
    pub(super) awaiting_map: Vec<WindowId>,
    /// `[floating] auto` and the `[[window_rule]]`s the session runs with:
    /// what decides at a window's first commit whether it floats. Set from
    /// the config after `State::new` and swapped whole by a reload.
    pub floating_rules: super::window_rules::FloatingRules,
    /// `[floating] modifier`: held with the left button to move a floating
    /// window, with the right to resize it (see `floating/grab.rs`). Set
    /// from the config after `State::new` and by a reload.
    pub floating_modifier: scoot_ipc::Modifier,
    /// Whether a floating window's pointer grab left the arrangement
    /// needing a full `apply()`: set from inside the grab's callbacks, which
    /// run under Smithay's pointer lock where `apply()` must not run (it can
    /// reach the pointer) -- by a motion that carried the window to another
    /// output, and by the grab's `unset` however it ended (its own release,
    /// a lock, a close, a VT pause, a replacing grab). Drained by
    /// `State::settle_floating_grab` after the pointer call (the input
    /// paths, the lock transition, the VT pause, the nested pointer leave),
    /// and cleared by any `apply()`, which does what it asks for.
    pub(super) floating_grab_resync: bool,
    /// What `dmabuf::advertise` put on the `zwp_linux_dmabuf_v1` global --
    /// the default feedback, and the builder and table behind it -- or
    /// `None` when nothing was advertised (a renderer-less harness, a
    /// renderer that imports nothing). Written once, by
    /// `headless::init_named`; read only by the GPU scanout tier, whose
    /// per-surface scanout feedback extends it and reverts to it
    /// (`dmabuf/scanout.rs`).
    #[cfg(feature = "gpu-scanout")]
    pub(super) dmabuf_default: Option<super::dmabuf::DefaultFeedback>,
    /// Per output, which surface the GPU scanout tier is steering with a
    /// scanout tranche and what that tranche is (`dmabuf/scanout.rs`).
    /// Written by `render::draw_frame_scanout` once per frame, read by
    /// `DmabufHandler::new_surface_feedback`. Empty on every other tier.
    #[cfg(feature = "gpu-scanout")]
    pub(super) scanout_feedback: super::dmabuf::scanout::ScanoutFeedbacks,
    /// The layer surface a click gave keyboard focus to, if any -- the one
    /// piece of the layer-shell focus policy that cannot be re-derived from
    /// the layer map, because nothing else records that a click happened.
    ///
    /// Holding a `LayerSurface` here holds an `Arc` to a client's surface, so
    /// it is cleared as soon as it stops meaning anything: on that surface's
    /// destruction (`layer_destroyed`), on the next click elsewhere, on the
    /// commit where that surface stops wanting the keyboard at all
    /// (`commit_layer_surface` -- a click is spent once the thing it focused
    /// asks for `none` or unmaps itself), and defensively on every focus
    /// refresh if the client vanished without any of those
    /// (`forget_dead_clicked_layer`).
    pub clicked_layer: Option<LayerSurface>,
    /// Whether the keyboard focus `refresh_keyboard_focus` last *derived*
    /// went to a layer surface rather than a window's toplevel.
    ///
    /// "Derived", not "handed out", and the distinction is real since popup
    /// grabs landed: an active `xdg_popup.grab` swallows the `set_focus`
    /// that refresh ends in (see `popup.rs`), so the keyboard may be on a
    /// menu while this says "layer". Its one reader is
    /// `commit_layer_surface`'s gate, which only decides whether to
    /// re-derive focus at all -- re-deriving once too often is a wasted
    /// layer-map walk, never a wrong answer -- so the looser meaning is
    /// safe, but it is the meaning, and this field must not be read as
    /// "a layer surface currently holds the keyboard".
    ///
    /// Written *only* there, and it means exactly that -- not "a layer
    /// surface wants the keyboard" and not "`clicked_layer` is set". It
    /// exists so a bar with
    /// `keyboard_interactivity: none` (i.e. nearly every layer surface that
    /// will ever run) skips focus resolution entirely on each of its
    /// redraws, while a surface that *stops* wanting the keyboard still gets
    /// it taken away.
    pub keyboard_on_layer: bool,
    /// `wl_surface`s whose `zwlr_layer_surface_v1` role object has just been
    /// destroyed while the surface itself is still alive.
    ///
    /// Smithay's own destruction handler resets the surface's layer state to
    /// its default (no anchors, zero size) *after* calling back into
    /// [`WlrLayerShellHandler::layer_destroyed`](super::layer_shell) -- so a
    /// client that commits afterwards (destroying the role, attaching a null
    /// buffer and committing is exactly how a launcher dismisses itself)
    /// would trip the role's commit-time size validation on that default and
    /// be killed with `invalid_size`. [`State::neutralize_destroyed_layers`]
    /// rewrites those surfaces' pending anchors once Smithay's reset has run
    /// (from `dispatch.rs`'s post-destruction hook, which is the only thing
    /// that runs after it), so the commit is the harmless no-op wlroots
    /// treats it as. Drained on every layer-surface destruction, so it never
    /// holds more than the destructions of one dispatch.
    pub layers_awaiting_neutralize: Vec<WlSurface>,
    /// Layer surfaces that have committed a buffer (`last_acked` is `Some`,
    /// the same "mapped" `layer_focus` in `layer_shell.rs` means -- *not*
    /// LayerMap membership, which a null-unmapped surface keeps).
    ///
    /// The only reader is `commit_layer_surface`'s unmap transition: a
    /// surface that was mapped and now isn't gets its pending anchors
    /// neutralized for the *next* commit (see `neutralize_destroyed_layers`
    /// for what that write is), because Smithay's unmap reset leaves the
    /// default behind and a second null commit would otherwise trip the
    /// role's size validation exactly like a post-destroy one does. A
    /// surface that was never mapped is left alone, so committing with no
    /// description at all still fails loudly with `invalid_size`.
    /// Removed on `layer_destroyed`, and swept for dead entries in
    /// `CompositorHandler::destroyed` for the teardown order where the
    /// `wl_surface` dies before its role object and `layer_destroyed` never
    /// runs, so this never outlives the surface.
    pub mapped_layers: HashSet<WlSurface>,

    /// `xdg_toplevel_icon_v1` objects that have been handed to a toplevel and
    /// are therefore immutable from now on.
    ///
    /// Held to keep a client from panicking the compositor: see
    /// `dispatch.rs`'s fourth guard and `toplevel_icon.rs`'s three methods
    /// that own this set. Written only by those, and bounded by how many icon
    /// objects a client has alive (entries are dropped on the object's
    /// destruction, from `dispatch.rs`'s `destroyed`).
    pub frozen_icons: HashSet<ObjectId>,

    pub space: Space<Window>,
    pub popups: PopupManager,
    /// The explicit `xdg_popup.grab` currently routing input into a menu,
    /// if one is. See `popup.rs` for the precedence this sits at.
    ///
    /// Kept beside the seat's own grab rather than instead of it: the seat
    /// hands back only a `&dyn KeyboardGrab`, so "has this grab ended" and
    /// "dismiss it" are questions only the [`PopupGrab`] itself can answer.
    /// Holding one holds an `Arc` to the grab's root surface, so it is
    /// dropped the moment the grab is over --
    /// [`State::settle_popup_grab`](super::popup) reaps it from the wayland
    /// display source and after every pointer button.
    ///
    /// [`PopupGrab`]: smithay::desktop::PopupGrab
    pub popup_grab: Option<ActivePopupGrab>,
    /// The serial half of the grab gate (`interaction_serials`) answers
    /// "did this client recently interact", which a menu session outlasts:
    /// a menu read for a minute, then hovered deeper, reuses a serial older
    /// than the interaction window.
    /// Toolkits also destroy the old popup before grabbing its replacement
    /// in the same input handler, so the new grab never nests inside the
    /// old one. This answers the other half -- "was the keyboard this
    /// client's moments ago" -- so those continuations are not refused.
    /// Written in exactly one place -- `handlers.rs`'s `destroyed()`, gated
    /// on the dying surface belonging to the grabbing client -- and cleared
    /// on every grant; read only by the grab gate. Dismissals (click
    /// outside, lock, `exclusive` layer) and reaps deliberately file
    /// nothing, so an ended-by-others session cannot lend its serial to a
    /// reopen. See `popup.rs`.
    pub last_popup_grab: Option<(ClientId, Instant)>,
    /// Every output this compositor drives, and the core id each is known by
    /// -- see `outputs.rs` for why the collection lives here rather than in
    /// [`scoot_core`], and for what a site reaching for
    /// [`Outputs::primary`](super::outputs::Outputs::primary) is still
    /// assuming.
    pub outputs: Outputs,
    /// The output scale resolved from `[output] scale` (see
    /// `output_scale.rs`), set at startup and re-applied live by a config
    /// reload (see `reload.rs`) -- except under `--nested`, where it stays
    /// the forced 1.0. Read by `headless`'s `set_mode` (which applies it to
    /// the `Output`), by the `wp_fractional_scale_v1` handler (which
    /// advertises it per surface), and by `ipc.rs` (which reports it to an
    /// agent that must convert between logical rects and physical screenshot
    /// pixels). It is *not* the source the input clamp reads: that goes
    /// through `output_scale::logical_size`, which derives the logical
    /// extent from the `Output` itself, so a site handling input can never
    /// disagree with what the `Space` laid out. `compositor::run` forces
    /// this to 1.0 under `--nested`, where the host compositor owns the
    /// scale.
    pub output_scale: f64,
    /// The integer form of [`Self::output_scale`] -- `ceil(output_scale)`, the
    /// value sent on `wl_surface.preferred_buffer_scale` and the same one
    /// Smithay advertises on `wl_output.scale` (see `output_scale.rs`'s
    /// `integer_scale`). Recomputed alongside `output_scale` wherever that
    /// moves (construction, config reload), never anywhere else, so the two
    /// can never disagree: it is read on every surface commit
    /// (`CompositorHandler::commit`) and on every fractional-scale bind
    /// without recomputing the `ceil` on those hot paths.
    pub integer_scale: i32,
    /// Which renderer [`Self::backends`]' entries composite with, resolved once from
    /// `--renderer`/`[renderer] backend` (see `render::resolve`) and fixed
    /// for the process's lifetime. Kept here rather than read back off the
    /// live `Backend` because `resize_output` can *replace* that backend (a
    /// pixman resize always does; a GLES one only when its in-place
    /// reallocation failed) and has to rebuild the pipeline the session was
    /// started with -- including on the path where there is no backend to
    /// ask.
    pub renderer: RendererKind,
    /// One render target per output, keyed by the id the core knows it by.
    ///
    /// A `HashMap` rather than a second `Vec` beside [`Outputs`](super::outputs::Outputs):
    /// outputs are added at startup or by a hotplug and rarely removed, looked up by id on every
    /// capture path, and number at most eight -- so the map stays tiny and
    /// the per-frame render loop walks [`Outputs`](super::outputs::Outputs)
    /// by index (creation order, which is what makes the primary first)
    /// rather than this.
    ///
    /// Every output created by `headless::init_named` or `headless::add_output`
    /// gets exactly one entry, built with the session's renderer; the entry
    /// lives as long as the output does. A capture, a gamma ramp or a
    /// screenshot resolves its *own* output's entry here -- never another
    /// output's -- which is what keeps one screen's pixels from being served
    /// as another's.
    pub backends: HashMap<OutputId, Backend>,
    /// Set only under `--nested`: the connection presenting the primary
    /// framebuffer as a window in a host compositor, and forwarding that
    /// window's input back into this seat. `None` under `--headless`.
    pub host: Option<Host>,
    /// Set only under `--tty`: the session, DRM device/surface and dumb
    /// buffers presenting the primary framebuffer on a real display, and
    /// the libinput context feeding this seat from real input devices.
    /// `None` under `--headless`/`--nested`.
    pub tty: Option<Tty>,

    /// The ring/background palette, the fallback cursor's size/color/theme
    /// and the `prefer_no_csd` policy, resolved from `[appearance]` (or its
    /// defaults) once at startup -- see `config.rs` and `decorations.rs`'s
    /// module docs.
    ///
    /// Read live wherever it is used -- the render path for the ring and
    /// background, `handlers.rs`'s `XdgDecorationHandler` for
    /// `prefer_no_csd`, `Cursor` (rebuilt in place) for the cursor fields --
    /// and written only by a config reload (see `reload.rs`), which is what
    /// keeps a second reload diffing against the first one's results.
    pub appearance: Appearance,
    /// Per-window persistent ring buffers -- see `decorations.rs`'s module
    /// doc for why these live here rather than being rebuilt every frame.
    pub decorations: Decorations,
    /// The pointer's last-requested image -- a client-supplied cursor
    /// surface, a named shape, or hidden -- plus the render buffer behind
    /// the fallback shape. Drawn on screen only under `--tty` (see
    /// `cursor.rs`'s module doc), and into a capture wherever the capture
    /// asks for the pointer, on every backend (see
    /// `render/capture_cursor.rs`).
    pub cursor: Cursor,
    /// Bumped whenever the pointer moves or the cursor's image changes, on
    /// every backend. The capture path's counterpart of `frame_serial` for
    /// the one thing a capture can show that no frame has to draw: under
    /// `--headless`/`--nested` the cursor is never in a frame, so moving it
    /// renders nothing and leaves `frame_serial` where it was -- yet a
    /// capture session that asked for the pointer (`paint_cursors`) sees
    /// its content change. `screencopy.rs` keys such a session's "has the
    /// source changed" test on both counters. Wraps, like `frame_serial`,
    /// and for the same reason it cannot matter.
    pub cursor_serial: u64,
    /// Test-only override of [`State::frame_draws_cursor`]: `Some(true)`
    /// makes a harness's frames composite the cursor the way every `--tty`
    /// frame on the dumb tier does, which is the shape the capture path's
    /// "remove the cursor" half needs to be driven against. `None` (the
    /// default) is the production rule.
    #[cfg(test)]
    pub(crate) frame_cursor_for_test: Option<bool>,
    /// Test-only failure injection: while set, the next `render::draw_frame`
    /// takes the flag and reports a frame that did not draw -- the shape a
    /// failed bind or render leaves (both only log). No harness can make a
    /// real renderer fail on demand.
    #[cfg(test)]
    pub(crate) fail_next_draw_for_test: bool,
    /// Test-only: the listening socket's and the display's loop
    /// registrations, so a test harness can remove both on teardown even
    /// when its event loop outlives it (see `test_support`'s
    /// `release_listener_sources`).
    #[cfg(test)]
    pub(crate) listener_tokens: Vec<smithay::reexports::calloop::RegistrationToken>,
    /// Test-only: every XWayland server source `xwayland::start` inserted,
    /// with its display number, for the same teardown (see
    /// `release_listener_sources`).
    #[cfg(test)]
    pub(crate) xwayland_tokens_for_test: Vec<(smithay::reexports::calloop::RegistrationToken, u32)>,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    /// `xwayland_shell_v1`: the association half of opt-in XWayland (see
    /// `xwayland/mod.rs`). Always constructed, like
    /// `xdg_shell_state` above -- and harmless when the session never asked:
    /// Smithay's `can_view` gate admits only XWayland's own client, which
    /// exists solely between a successful `XWayland::spawn` and session end,
    /// so no regular client ever sees the global and no dispatch of ours is
    /// reachable without a running server. What only exists on demand is the
    /// window manager below (at `READY`) and the keyboard-grab manager
    /// (alongside a successful spawn).
    #[cfg(feature = "xwayland")]
    pub xwayland_shell_state: xwayland::XWaylandShellState,
    /// The running X11 window manager, if the session asked for XWayland
    /// and its server reached `READY` (see `xwayland::start`). `None`
    /// otherwise -- never asked, binary absent, server died pre-`READY`, or
    /// the WM attach failed. Read by `XwmHandler::xwm_state` (see
    /// `xwayland/wm.rs`), which is how every X window reaches the layout.
    #[cfg(feature = "xwayland")]
    pub xwm: Option<xwayland::X11Wm>,
    /// The X display number while our server is believed live: set from the
    /// synchronous lock at spawn, cleared on a pre-`READY` death or a failed
    /// window-manager attach (see `xwayland/mod.rs` -- a WM-less server is not
    /// live for our purposes). `State::spawn` and `run`'s process export read
    /// exactly this -- `Some` sets `DISPLAY`, `None` leaves it untouched
    /// (no clobber of a host `DISPLAY` under `--nested`). Unconditional
    /// (a plain `u32`, no Smithay type), so the plumbing compiles and is
    /// tested in every build flavour; without the feature it stays `None`.
    pub xdisplay: Option<u32>,
    /// `zwp_xwayland_keyboard_grab_manager_v1`: created alongside a
    /// successful spawn, never in `new` (see `xwayland::start` for why the
    /// timing matters and why a never-asked session stays
    /// byte-identical). Answers no grab for any surface (see
    /// `xwayland/wm.rs`): an X client's keyboard grab stays inside the X
    /// server.
    #[cfg(feature = "xwayland")]
    pub xwayland_grab: Option<xwayland::XWaylandKeyboardGrabState>,
    /// The override-redirect X windows currently mapped -- menus, tooltips,
    /// drop-downs -- in mapping order, newest last (on top). Never in the
    /// core or the `Space`: they place themselves, and are drawn and
    /// hit-tested from here (see `xwayland/unmanaged.rs`). Written only by
    /// that module (on map, unmap/destroy, and the server's death); read by
    /// the hit test, the frame gathering, and the frame-callback and
    /// presentation passes -- each behind its lock branch. Empty in every
    /// session without an X client, so each reader costs an empty-`Vec` test.
    #[cfg(feature = "xwayland")]
    pub x11_unmanaged: Vec<smithay::xwayland::X11Surface>,
    /// Every X window, mapped or not, that currently carries a
    /// `_NET_STARTUP_ID`, by X window id -- so the focus gate can read a
    /// toolkit's startup id off its *client leader* (GTK puts it
    /// there, on an unmapped window, not on the toplevel they map; see
    /// `xwayland/focus.rs`). Written only from the XWM callbacks (a window
    /// created, its startup id changing, its destruction, the server's
    /// death); read once per redemption. Holds a handful of windows per X
    /// application, and none in a session without X clients.
    #[cfg(feature = "xwayland")]
    pub x11_startup_carriers: HashMap<u32, smithay::xwayland::X11Surface>,
    /// The window manager's ownership count for each selection at the
    /// moment the gate let it onto the Wayland side (`xwayland/selection.rs`):
    /// a Wayland paste is served only while it has not changed hands since.
    /// Written when an X selection crosses or stops being the Wayland one;
    /// read per paste.
    #[cfg(feature = "xwayland")]
    pub x11_selection_owners: xwayland::selection::CrossedOwners,
    /// `zwlr_layer_shell_v1`: bars, docks, wallpapers and notification
    /// daemons. Unlike the two `#[allow(dead_code)]` states below this one is
    /// read again -- `WlrLayerShellHandler::shell_state` (see
    /// `layer_shell.rs`) routes every layer-shell request through it. The
    /// surfaces themselves live in Smithay's per-output `LayerMap`, not here.
    pub layer_shell_state: WlrLayerShellState,
    /// `ext_workspace_manager_v1`: what a bar reads workspaces from and
    /// switches them through. Unlike the two `#[allow(dead_code)]` states
    /// below, this is read on every `apply` -- see `ext_workspace.rs`, which
    /// owns both the protocol objects and the "what have clients been told"
    /// snapshot behind them.
    pub ext_workspace: ExtWorkspaceState,
    /// `ext_foreign_toplevel_list_v1`: what a taskbar, dock or alt-tab
    /// switcher reads the window list from. Read on every window opening,
    /// closing and retitling -- see `foreign_toplevel.rs`, which owns both
    /// the protocol objects and the one handle per window behind them.
    pub foreign_toplevels: ForeignToplevels,
    /// `zwlr_foreign_toplevel_manager_v1` (version 3): the *older* window-list
    /// protocol, published alongside the `ext-` one above rather than instead
    /// of it -- the clients that exist today (every Quickshell-based shell)
    /// bind this one and ignore that one. Unlike the `ext-` list this half has
    /// a control side: `activate` and `close` are answered, and a window's
    /// `activated` bit follows [`Self::focus`]. Read on every window opening,
    /// closing, retitling and focus change -- see
    /// `foreign_toplevel_management.rs`.
    pub foreign_toplevel_management: ForeignToplevelManagement,
    /// `ext_image_copy_capture_v1` + `ext_image_capture_source_v1`: the screen
    /// capture a shell's window thumbnails, a workspace-overview preview,
    /// `grim` or a screen-share reads. Read on every capture session, every
    /// frame request and every frame tick that has one parked -- see
    /// `screencopy.rs`, which owns both the protocol objects and the parked
    /// frames behind them. Output capture only; scoot's own agent
    /// screenshots go over IPC (`screenshot.rs`) and are unaffected.
    pub screencopy: Screencopy,
    /// `ext_session_lock_manager_v1`: the compositor-enforced screen lock.
    /// Unlike the two `#[allow(dead_code)]` states below, this is read on
    /// every render, every focus refresh and every pointer hit test -- see
    /// `session_lock.rs`, which owns both the protocol objects and the one
    /// field that says whether the session is locked at all.
    pub session_lock: SessionLock,
    /// `wp_fractional_scale_v1`: the fractional value `wl_output.scale` can
    /// only round. Held only to keep the global alive; the handler itself
    /// (`FractionalScaleHandler`) lives in `output_scale.rs`.
    #[allow(dead_code)]
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    /// `wp_cursor_shape_v1` (version 2): lets a client name a cursor shape
    /// (`text`, `ew-resize`, `not-allowed`) instead of uploading a cursor
    /// surface of its own, and have the compositor draw it. Held only to keep
    /// the global alive -- Smithay routes `set_shape` straight into
    /// `SeatHandler::cursor_image` as a `CursorImageStatus::Named`, which is
    /// the same path `wl_pointer.set_cursor` with no surface already took, so
    /// there is no handler of scoot's own between the two. What each name is
    /// drawn as lives in `cursor/shapes.rs`.
    #[allow(dead_code)]
    pub cursor_shape_manager_state: CursorShapeManagerState,
    /// `wp_viewporter`: the global a client needs to render a fractionally
    /// scaled buffer (it sets a logical destination size and lets the
    /// compositor scale the buffer into it). Held only to keep the global
    /// alive -- the render path reads each surface's `ViewportCachedState`
    /// through `on_commit_buffer_handler` and needs nothing from this field.
    #[allow(dead_code)]
    pub viewporter_state: ViewporterState,
    /// Held only to keep the `zxdg_decoration_manager_v1` global alive --
    /// like `output_manager_state`, `XdgDecorationHandler` (see
    /// `handlers.rs`) has no `&mut XdgDecorationState` accessor to route
    /// dispatch through, so nothing reads this field again after `new`.
    #[allow(dead_code)]
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    /// Held only to keep the xdg-output global alive.
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,
    /// `wp_alpha_modifier_v1` (version 1): a client-controlled whole-surface
    /// opacity factor. Held only to keep the global alive -- Smithay's
    /// `from_surface` multiplies the factor into every surface-tree element
    /// it builds (see `alpha_modifier.rs`), so there is no scoot-side
    /// render work and nothing reads this field again after `new`.
    #[allow(dead_code)]
    pub alpha_modifier_state: AlphaModifierState,
    /// `wp_content_type_manager_v1` (version 1): a client labeling what kind
    /// of pixels a surface holds. Held only to keep the global alive --
    /// nothing on a CPU/pixman renderer consumes the hint (see
    /// `content_type.rs`), so nothing reads this field again after `new`.
    #[allow(dead_code)]
    pub content_type_state: ContentTypeState,
    /// `wp_single_pixel_buffer_manager_v1` (version 1): solid-color 1x1
    /// buffers with no shm behind them, for cheap toolkit fills. Held only
    /// to keep the global alive -- Smithay owns the buffers (see
    /// `single_pixel_buffer.rs`), and the render path draws them as solid
    /// fills with no scoot-side import step.
    #[allow(dead_code)]
    pub single_pixel_buffer_state: SinglePixelBufferState,
    /// `zwp_tablet_manager_v2` (version 1): drawing-tablet input. Held
    /// only to keep the global alive -- the seat's tools live in Smithay's
    /// `TabletSeat` behind `seat.tablet_seat()`, and the event plumbing is
    /// `tablet.rs`. See that module for what a pen does and what stays
    /// deferred (pads: Smithay carries none at the pinned rev).
    #[allow(dead_code)]
    pub tablet_manager_state: TabletManagerState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    /// `zwlr_data_control_manager_v1` (version 2): clipboard managers
    /// (`cliphist`, `clipman`). Held only to keep the global alive -- the
    /// handler (`DataControlHandler`, see `handlers.rs`) routes through it.
    /// No client filter: like the session-lock global, an allow-list would be
    /// theatre without security-context support (see `docs/protocols.md`'s
    /// trust note). Constructed after `primary_selection_state` because it
    /// borrows it, so data-control clients can also touch the primary
    /// selection.
    #[allow(dead_code)]
    pub wlr_data_control_state: WlrDataControlState,
    /// `ext_data_control_manager_v1` (version 1): the successor to the wlr
    /// clipboard protocol, exposed alongside it the way current compositors
    /// do. Same allow-list rationale and same borrow of
    /// `primary_selection_state` as above.
    #[allow(dead_code)]
    pub ext_data_control_state: ExtDataControlState,
    /// `zwp_primary_selection_device_manager_v1` (version 1): middle-click
    /// paste. Held only to keep the global alive, same rationale as above.
    #[allow(dead_code)]
    pub primary_selection_state: PrimarySelectionState,
    /// `zwp_text_input_manager_v3` (version 1): what an application binds to
    /// say "there is a text field here". Held only to keep the global alive
    /// -- which text field is focused follows keyboard focus inside Smithay's
    /// own seat (see `input_method.rs`), so nothing reads this field again.
    #[allow(dead_code)]
    pub text_input_manager_state: TextInputManagerState,
    /// `zwp_input_method_manager_v2` (version 1): what an input method --
    /// fcitx5, ibus, an on-screen keyboard -- binds to receive that text
    /// field and send composed text back to it. Held only to keep the global
    /// alive; `InputMethodHandler` (see `input_method.rs`) owns the one part
    /// this compositor has to do, which is the IME's popup.
    ///
    /// No client filter, for the same reason the session-lock and
    /// data-control globals have none: an allow-list would be theatre
    /// without security-context support (see `docs/protocols.md`'s trust
    /// note). Worth naming here because an input method is more privileged
    /// than those two -- it can grab the keyboard and inject text into the
    /// focused client -- so this is a deliberate consistency with the existing
    /// trust model, not an oversight about what the protocol can do.
    #[allow(dead_code)]
    pub input_method_manager_state: InputMethodManagerState,
    /// `xdg_toplevel_icon_manager_v1`: the icon a client wants shown for its
    /// window. Held only to keep the global alive -- scoot draws no icons
    /// itself, and what a bar or an agent reads comes off the surface's own
    /// cached state (see `toplevel_icon.rs`), not from here.
    ///
    /// Deliberately advertising *no* preferred icon sizes: the manager sends
    /// its `icon_size` list at bind time, and this compositor has no size to
    /// prefer, since nothing in it draws an icon. An empty list is the
    /// protocol's own way of saying exactly that ("the compositor has no
    /// preference"), whereas inventing 16/24/32 here would be telling every
    /// client to rasterize at sizes no consumer asked for.
    #[allow(dead_code)]
    pub xdg_toplevel_icon_manager: XdgToplevelIconManager,
    /// `xdg_activation_v1`: how one client asks that another be focused --
    /// a launcher handing focus to the app it just started, a notification
    /// daemon focusing the app its popup came from. Unlike the states above
    /// this is read again on every token and every activation, to sweep
    /// expired tokens and to bound how many can exist at once; see
    /// `activation.rs` for the policy those two bounds implement.
    pub xdg_activation: XdgActivationState,
    /// `zwlr_gamma_control_manager_v1`: night-light tools. Unlike the three
    /// above this is read again -- every `get_gamma_control`/`set_gamma`
    /// goes through it (see `gamma_control.rs`).
    pub gamma_control: GammaControlState,
    /// `zwlr_output_manager_v1` (version 4): what a shell's Settings ->
    /// Display page and `wlr-randr` read the output's modes, position, scale
    /// and transform from. Read on every bind and on the two sites that change
    /// the output's state -- see `output_management.rs`, which owns both the
    /// protocol objects and the snapshot of what clients have been told.
    /// Read-only: every configuration a client builds is refused.
    pub output_management: OutputManagement,
    /// `zwp_pointer_constraints_v1` (version 1): pointer lock and confinement,
    /// the half of the games/3D-app pair `relative_pointer.rs` documents.
    /// Held only to keep the global alive -- the constraints themselves live
    /// in Smithay's per-pointer map, activated from
    /// [`PointerConstraintsHandler::new_constraint`](super::relative_pointer)
    /// when the surface already has pointer focus, and honoured by the motion
    /// core in `input.rs` (a locked pointer moves nothing absolute; a
    /// confined one is clamped to its region).
    #[allow(dead_code)]
    pub pointer_constraints_state: PointerConstraintsState,
    /// `zwp_relative_pointer_manager_v1` (version 1): raw unaccelerated
    /// pointer deltas for constrained-pointer clients. Held only to keep the
    /// global alive -- Smithay owns the objects (see `relative_pointer.rs`),
    /// and the motion core in `input.rs` feeds them on every focused motion.
    #[allow(dead_code)]
    pub relative_pointer_manager_state: RelativePointerManagerState,
    /// `wp_presentation` (version 2): frame-timing feedback for smooth
    /// video/animation clients. Held only to keep the global alive --
    /// Smithay owns the feedback objects (see `presentation_time.rs`), and
    /// the frame handoff in `State::render` takes and marks them
    /// presented with this backend's timestamp semantics.
    #[allow(dead_code)]
    pub presentation_state: PresentationState,
    /// The shared per-client budget for binding the manager/list globals
    /// above (`ext_workspace`, `foreign_toplevels`,
    /// `foreign_toplevel_management`, `output_management`). Counted in each
    /// global's `bind`, released on its `stop` and `destroyed` -- see
    /// `bind_budget.rs`, which owns the policy and the number.
    pub bind_budget: BindBudget,
    /// How many live `wl_shm` pools each Wayland client holds. Counted at
    /// `create_pool` before delegation, released in `dispatch.rs`'s pool
    /// destruction hook (which also drains disconnects and kills) -- see
    /// `shm_pools.rs`, which owns the policy and the number.
    pub shm_pools: ShmPools,
    /// How many live `wl_buffer`s each Wayland client holds, whatever
    /// created them. Counted at each buffer creation before delegation,
    /// released in `dispatch.rs`'s buffer destruction hook (which also
    /// drains disconnects and kills) -- see `wl_buffers.rs`, which owns the
    /// policy and the number. This bounds buffer *objects*; the fds and
    /// mappings they keep, including after the object is destroyed, are
    /// `client_fds`' below.
    pub wl_buffers: WlBuffers,
    /// How many dma-buf plane fds each Wayland client has this compositor
    /// hold in `zwp_linux_buffer_params_v1` objects it has not created a
    /// buffer from. Counted at `add` before delegation, released when the
    /// params object is consumed or destroyed -- see
    /// `dmabuf/pending_planes.rs`, which owns the policy and the number.
    pub(super) pending_planes: super::dmabuf::pending_planes::PendingPlanes,
    /// Every fd each Wayland client has handed this compositor that it
    /// keeps (shm pools, dma-buf planes, syncobj timelines), recorded by
    /// number on arrival and forgotten once it really closes -- which is
    /// after its object is destroyed whenever a surface still has the
    /// buffer committed. The per-client fd bound, the timeline cap and fd
    /// pressure's attribution all read it. See `client_fds.rs`, which owns
    /// the policy and the numbers.
    pub(super) client_fds: super::client_fds::ClientFds,
    /// Explicit sync (`wp_linux_drm_syncobj_manager_v1`): the protocol state
    /// (and so the global) where it is offered -- the `--tty` GPU scanout
    /// tier on a device that passes the syncobj-eventfd probe, set by
    /// `tty::init` -- plus the per-client acquire-wait counts and the
    /// explicit-buffer classification the scanout tier's release hold reads
    /// (the timeline fds are `client_fds`' above). Inert everywhere else.
    /// See `drm_syncobj.rs`.
    pub(super) drm_syncobj: super::drm_syncobj::DrmSyncobj,
    /// Whether any client has ever handed this session a dmabuf the renderer
    /// accepted.
    ///
    /// One writer (`dmabuf.rs`'s `dmabuf_imported`, on a successful import)
    /// and one reader (`handlers.rs`'s `commit`, which calls
    /// `dmabuf::sync_committed_dmabufs` only when it is set). Latching, never
    /// cleared: it is not "a dmabuf is mapped right now" but "this session is
    /// one where dmabufs happen", which is the question the commit path is
    /// actually asking -- re-deriving liveness per commit would cost more
    /// than the walk it would save, and a stale `true` costs one extra ioctl
    /// pair per commit while a stale `false` would cost torn frames.
    ///
    /// What it buys: a `wl_shm`-only session -- every `--headless` test, and
    /// every session with no GL client -- keeps exactly the commit path it had
    /// before dmabuf import existed, a bool test rather than a surface-tree
    /// walk on the hottest handler this compositor has.
    pub imports_dmabufs: bool,
    /// Whether a dmabuf-cache drain is already sitting on the loop's idle
    /// queue.
    ///
    /// Written only by `dmabuf.rs` -- set in `schedule_cache_drain` (from the
    /// `wl_buffer` destruction hook), cleared by `drain_cache` when the idle
    /// runs. It is a *queued-ness* flag, not "the cache is dirty": it exists
    /// so a client destroying 512 buffers in one dispatch queues one scan
    /// rather than 512, the same batching `bind_budget.rs` does for deferred
    /// refusals. A stale `true` could only happen if an idle were dropped
    /// without running, which calloop does not do.
    pub dmabuf_drain_queued: bool,
    /// How many fds this session's GLES renderer keeps of its own for each
    /// dma-buf plane it imports, per GLES backend: `None` until the first
    /// import into one, then what `dmabuf/renderer_copies.rs` measured then
    /// (1 on Mesa's llvmpipe, 0 on a renderer that keeps none). One writer
    /// and reader, that module, which charges the copies to the client in the
    /// fd ledger. Never cleared: the GLES device is pinned for the session.
    pub(super) renderer_plane_copies: Option<u8>,
    /// `ext_idle_notifier_v1` (version 2): what a `swayidle`-style daemon
    /// binds to learn the seat has been quiet N milliseconds. Read on
    /// every input event (`announce_activity`, see `idle.rs`) and written
    /// by the inhibit bookkeeping in the same module.
    pub idle_notifier: IdleNotifierState<State>,
    /// The surfaces holding `idle-inhibit-unstable-v1` inhibition. The
    /// aggregate, not the protocol objects -- Smithay owns those; this is
    /// what `refresh_idle_inhibit` derives the notifier flag from (see
    /// `idle.rs`).
    pub idle_inhibitors: idle::Inhibitors,
    /// `zwp_idle_inhibit_manager_v1` (version 1): lets a video player or
    /// presentation app hold the session awake. Held only to keep the
    /// global alive -- `IdleInhibitHandler` (see `idle.rs`) routes through
    /// `idle_inhibitors` above.
    #[allow(dead_code)]
    pub idle_inhibit_manager_state: IdleInhibitManagerState,
    pub seat: Seat<State>,
    /// The recent input events that count as the user asking for something --
    /// key and button presses and releases, plus the pointer and keyboard
    /// `enter` serials those deliveries produced -- each with the client it
    /// was delivered to.
    ///
    /// Written by `input.rs` (`key`, `pointer_button`, and the focus-change
    /// halves of `pointer_move_quietly` and `shell.rs`'s
    /// `refresh_keyboard_focus`; plain motion serials are deliberately not
    /// among them) and read by `activation.rs` (key and button events only)
    /// and `popup.rs` (any of them). A token or grab is refused unless its
    /// claimed serial is one of these *and* was delivered to the client that
    /// asked. See `input/interaction.rs` for why this is a short history
    /// rather than a single "last serial", why the client identity is part
    /// of it, and why the two readers spend different halves.
    pub interaction_serials: input::interaction::Recent,

    pub keybindings: Keybindings,
    /// Pids of children [`State::spawn`] started and the reaper has not
    /// collected yet -- the *only* pids the `SIGCHLD` drain ever `waitpid`s
    /// (see `child_reaper.rs` for why `-1` would steal the unit-test binary's
    /// own forked children under `cargo test`).
    ///
    /// Written only here in `spawn` (on success -- a child that never started
    /// has nothing to reap) and drained only by `reap_children` on the loop
    /// thread, so entries cannot be lost or double-reaped: a pid stays until
    /// its zombie is collected, a zombie holds its pid against reuse, and an
    /// `ECHILD` (reaped elsewhere) forgets the entry rather than leaking it.
    pub spawned_children: HashSet<u32>,
    /// Keycodes currently held that a keybinding intercepted on press, so
    /// their matching release is intercepted too instead of forwarded to
    /// whatever the focused client becomes in between. See `input::key`.
    ///
    /// Invariant: only `key()` inserts or removes entries here. If any
    /// future code path ever forwards or intercepts a key release *without*
    /// going through `key()` -- e.g. Smithay's `KeyboardHandle::release_source`,
    /// used to tear down a virtual keyboard or a departing libinput device
    /// by forwarding its releases directly -- it must also remove that
    /// keycode from here, or the next real press+release of the same
    /// keycode will have its legitimate release wrongly intercepted as a
    /// stale stuck entry.
    pub suppressed_keys: HashSet<Keycode>,
    /// Keycodes currently held, mirroring the set Smithay keeps inside the
    /// seat keyboard so `input.rs` can tell a key that will be *delivered*
    /// from one it will absorb as a non-transition -- a distinction
    /// `KeyboardHandle::input`'s return value does not make. Only
    /// `State::note_held` writes it; see the invariant on
    /// [`State::suppressed_keys`] above, which applies here for the same
    /// reason.
    pub held_keys: HashSet<Keycode>,

    /// Something changed that the framebuffer doesn't show yet.
    pub needs_render: bool,
    /// Whether the pending render was asked for by anything other than the
    /// cursor: set by `request_render`, *not* by `request_cursor_render`
    /// (the cursor's own path, from `State::cursor_changed`), and taken by
    /// the next render that draws. What decides whether that render's
    /// damage moves [`State::frame_serial`].
    pub scene_dirty: bool,
    /// How many frames [`State::render`](super::State) has drawn whose damage
    /// tracker reported at least one changed region *and* that something
    /// other than the cursor asked for -- i.e. how many times the scene the
    /// framebuffer shows, cursor aside, may have changed since startup. A
    /// frame drawn only because the cursor moved or changed image (under
    /// `--tty`, the one backend whose frames draw it) does not count: that
    /// is [`State::cursor_serial`]'s, so a capture session that did not ask
    /// for the pointer is not handed the same picture again every time the
    /// pointer moves.
    ///
    /// **"May have changed", specifically**, and the distinction matters at
    /// both ends. It is *not* "how many times `render()` ran": `render()`
    /// early-returns on a clean screen, and a run that failed to bind or
    /// failed to draw leaves the previous frame in place. It is also not "how
    /// many times the pixels really differ": under `--headless`/`--nested` the
    /// damage tracker is handed a buffer age of `0`, so it reports full damage
    /// for every frame it draws whether or not anything moved. Only `--tty`
    /// passes a real age and so only there does this skip a redundant redraw.
    ///
    /// Its one reader is `screencopy.rs`, which uses it for exactly the
    /// question it answers: may the framebuffer differ from the one a capture
    /// session was last handed? A false "yes" costs one extra copy; a false
    /// "no" would leave a client's live preview frozen, which is why the
    /// conservative direction is the one taken.
    ///
    /// Shared across outputs: a redraw with damage on any output moves it,
    /// so a session on a static output may be handed one redundant copy when
    /// another output animates. Correct, just occasionally wasteful -- and
    /// the waste is one shm copy on a tick that already paid a read-back,
    /// never a wrong pixel.
    ///
    /// Wraps rather than saturates (`wrapping_add`), which is unreachable in
    /// practice -- 2^64 frames at 60 Hz is ~9.7 billion years -- and is a
    /// comparison against a stored copy in any case, never an ordering.
    pub frame_serial: u64,
    /// Whether the frame timer (see `headless::ensure_ticking`) is currently
    /// running. It drops itself when there's nothing to do rather than
    /// polling forever, so this is how callers know whether to re-arm it.
    pub timer_armed: bool,
    /// When a client last committed, which is what `wait-idle` waits on.
    pub last_commit: Instant,
    pub pending_idle: Vec<PendingIdle>,
    /// The screenshot encode worker, if one has been needed yet. `None`
    /// until the first capture spawns it lazily (see `screenshot.rs`):
    /// a session that never screenshots pays for no thread and no event
    /// source. Dropped and respawned if the worker ever goes away.
    pub screenshot_encoder: Option<Encoder>,
    /// The completion channel screenshot workers answer through. Created
    /// once, alongside its event source, and kept across worker respawns --
    /// so a restart reuses the one registered source rather than abandoning
    /// one per generation. `None` until the first capture, like the worker.
    pub screenshot_sink: Option<ShotSink>,
    /// Captures accepted but not yet fully written out -- either still
    /// encoding on the worker or draining a reply the socket would not take
    /// in one write. Serviced by the completion channel's callback and, for
    /// the draining half, by the frame tick (see `screenshot.rs`).
    pub pending_shots: Vec<PendingShot>,
}

impl State {
    /// Builds the compositor's state and opens its wayland socket.
    ///
    /// Fallible because that socket is: `$XDG_RUNTIME_DIR` may be unset (the
    /// common case -- a bare `ssh` or `su` shell has none), unwritable, or
    /// already full of other compositors' sockets. None of those is a bug to
    /// panic on, they are an operator's environment to report back to them
    /// (see [`Self::listen`]).
    pub fn new(
        event_loop: &mut EventLoop<'static, State>,
        display: Display<State>,
        config: Config,
        keybindings: Keybindings,
        appearance: Appearance,
        scale: f64,
        renderer: RendererKind,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // A no-op in a session (`run` raised it first); here so that every
        // `State`, a test harness's included, runs with the fd limit a
        // session runs with, before any client connects to it (see
        // `nofile.rs`).
        super::nofile::raise();
        let dh = display.handle();
        let compositor_state = CompositorState::new_v6::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        // Always constructed, even for Wayland-only sessions: the global it
        // registers admits only XWayland's own client (see the field doc),
        // so this is invisible protocol surface, not a behaviour change.
        #[cfg(feature = "xwayland")]
        let xwayland_shell_state = xwayland::XWaylandShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        // `xdg_wm_dialog_v1` (`xdg-dialog-v1`): how a toolkit marks a
        // toplevel as a dialog, which floats it when it maps (see
        // `floating.rs`). Nothing reads the returned state -- it holds only
        // the global's id, for a caller that would remove the global, and the
        // global lives in the display either way -- so it is not kept.
        XdgDialogState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let ext_workspace = ExtWorkspaceState::new(&dh);
        let foreign_toplevels = ForeignToplevels::new(&dh);
        let foreign_toplevel_management = ForeignToplevelManagement::new(&dh);
        let screencopy = Screencopy::new(&dh);
        let session_lock = SessionLock::new(&dh);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let cursor_shape_manager_state = CursorShapeManagerState::new::<Self>(&dh);
        let viewporter_state = super::output_scale::viewporter(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<Self>(&dh);
        let alpha_modifier_state = AlphaModifierState::new::<Self>(&dh);
        let content_type_state = ContentTypeState::new::<Self>(&dh);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        // Construction order is load-bearing: both data-control states borrow
        // the primary-selection state, so it must exist first.
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let wlr_data_control_state =
            WlrDataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let ext_data_control_state =
            ExtDataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let xdg_activation = XdgActivationState::new::<Self>(&dh);
        let xdg_toplevel_icon_manager = XdgToplevelIconManager::new::<Self>(&dh);
        let text_input_manager_state = TextInputManagerState::new::<Self>(&dh);
        let input_method_manager_state = InputMethodManagerState::new::<Self, _>(&dh, |_| true);
        let gamma_control = GammaControlState::new(&dh);
        let output_management = OutputManagement::new(&dh);
        let pointer_constraints_state = PointerConstraintsState::new::<Self>(&dh);
        let relative_pointer_manager_state = RelativePointerManagerState::new::<Self>(&dh);
        let tablet_manager_state = TabletManagerState::new::<Self>(&dh);
        // `CLOCK_MONOTONIC`: the one clock whose readings the frame handoff
        // stamps feedback with (see `presentation_time.rs`), so it is the id
        // the bind handshake must report. `Clock::new` allocates nothing --
        // it is a marker for `clock_gettime` -- so no field is kept for it.
        let presentation_state = PresentationState::new::<Self>(
            &dh,
            smithay::utils::Clock::<smithay::utils::Monotonic>::new().id() as u32,
        );
        let idle_notifier = IdleNotifierState::new(&dh, event_loop.handle());
        let idle_inhibit_manager_state = IdleInhibitManagerState::new::<Self>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "scoot");
        seat.add_keyboard(Default::default(), 200, 25)
            .expect("a keymap for the default layout");
        seat.add_pointer();

        let (socket_name, listener_tokens) = Self::listen(display, event_loop)?;
        #[cfg(not(test))]
        let _ = listener_tokens;

        // Built here rather than in the struct literal below, which moves
        // `appearance` before `cursor`'s own field initializer could read it.
        // The fallback bitmap is built exactly once, from the config this
        // process started with -- see `Cursor::new`.
        let cursor = Cursor::new(
            appearance.cursor_size,
            appearance.cursor_color,
            appearance.cursor_theme.as_deref(),
        );

        Ok(Self {
            start_time: Instant::now(),
            display_handle: dh,
            loop_handle: event_loop.handle(),
            loop_signal: event_loop.get_signal(),
            socket_name,
            ipc_path: None,
            config_path: None,
            startup_gpu: None,
            startup_autostart: Vec::new(),
            startup_xwayland: false,
            world: World::new(config),
            windows: HashMap::new(),
            next_id: 0,
            focus: None,
            fullscreen_covers: Vec::new(),
            floating_cover: 0,
            awaiting_map: Vec::new(),
            floating_rules: super::window_rules::FloatingRules::default(),
            floating_modifier: super::window_rules::DEFAULT_DRAG_MODIFIER,
            floating_grab_resync: false,
            #[cfg(feature = "gpu-scanout")]
            dmabuf_default: None,
            #[cfg(feature = "gpu-scanout")]
            scanout_feedback: Default::default(),
            clicked_layer: None,
            keyboard_on_layer: false,
            layers_awaiting_neutralize: Vec::new(),
            mapped_layers: HashSet::new(),
            frozen_icons: HashSet::new(),
            space: Space::default(),
            popups: PopupManager::default(),
            popup_grab: None,
            last_popup_grab: None,
            outputs: Outputs::default(),
            output_scale: scale,
            integer_scale: super::output_scale::integer_scale(scale),
            renderer,
            backends: HashMap::new(),
            host: None,
            tty: None,
            appearance,
            decorations: Decorations::default(),
            cursor,
            compositor_state,
            xdg_shell_state,
            #[cfg(feature = "xwayland")]
            xwayland_shell_state,
            #[cfg(feature = "xwayland")]
            xwm: None,
            xdisplay: None,
            #[cfg(feature = "xwayland")]
            xwayland_grab: None,
            #[cfg(feature = "xwayland")]
            x11_unmanaged: Vec::new(),
            #[cfg(feature = "xwayland")]
            x11_startup_carriers: HashMap::new(),
            #[cfg(feature = "xwayland")]
            x11_selection_owners: xwayland::selection::CrossedOwners::default(),
            layer_shell_state,
            ext_workspace,
            foreign_toplevels,
            foreign_toplevel_management,
            screencopy,
            session_lock,
            fractional_scale_manager_state,
            cursor_shape_manager_state,
            viewporter_state,
            xdg_decoration_state,
            shm_state,
            single_pixel_buffer_state,
            alpha_modifier_state,
            content_type_state,
            output_manager_state,
            seat_state,
            data_device_state,
            wlr_data_control_state,
            ext_data_control_state,
            primary_selection_state,
            text_input_manager_state,
            input_method_manager_state,
            xdg_toplevel_icon_manager,
            xdg_activation,
            gamma_control,
            output_management,
            pointer_constraints_state,
            relative_pointer_manager_state,
            tablet_manager_state,
            presentation_state,
            bind_budget: BindBudget::default(),
            shm_pools: ShmPools::default(),
            wl_buffers: WlBuffers::default(),
            pending_planes: Default::default(),
            client_fds: Default::default(),
            drm_syncobj: Default::default(),
            imports_dmabufs: false,
            dmabuf_drain_queued: false,
            renderer_plane_copies: None,
            idle_notifier,
            idle_inhibitors: idle::Inhibitors::default(),
            idle_inhibit_manager_state,
            seat,
            interaction_serials: input::interaction::Recent::default(),
            keybindings,
            spawned_children: HashSet::new(),
            suppressed_keys: HashSet::new(),
            held_keys: HashSet::new(),
            // true without going through request_render(), so nothing has
            // armed the frame timer yet. That's only safe because
            // headless::init() unconditionally and synchronously calls
            // state.apply() -- which does call request_render() -- before
            // the event loop ever runs. If init ever stopped doing that
            // unconditionally, this would silently reintroduce "first frame
            // never draws, timer never arms" with no test failure pointing
            // here.
            needs_render: true,
            frame_serial: 0,
            scene_dirty: true,
            cursor_serial: 0,
            #[cfg(test)]
            frame_cursor_for_test: None,
            #[cfg(test)]
            fail_next_draw_for_test: false,
            #[cfg(test)]
            listener_tokens: listener_tokens.to_vec(),
            #[cfg(test)]
            xwayland_tokens_for_test: Vec::new(),
            timer_armed: false,
            last_commit: Instant::now(),
            pending_idle: Vec::new(),
            screenshot_encoder: None,
            screenshot_sink: None,
            pending_shots: Vec::new(),
        })
    }

    /// Opens the Wayland socket and wires it into the event loop.
    ///
    /// Every failure here is a startup error the caller reports and exits on
    /// (`main` prints it and returns `FAILURE`), never a panic: this used to
    /// be `.expect("a free wayland socket")`, which turned the most ordinary
    /// misconfiguration there is -- no `$XDG_RUNTIME_DIR`, i.e.
    /// [`BindError::RuntimeDirNotSet`] -- into a panic and, under this
    /// workspace's `panic = "abort"` release profile, a core dump, with
    /// nothing in it to tell an operator what to fix.
    ///
    /// Also hands back the two loop registrations (the socket's and the
    /// display's); only a test harness keeps them.
    fn listen(
        display: Display<State>,
        event_loop: &mut EventLoop<'static, State>,
    ) -> Result<
        (
            OsString,
            [smithay::reexports::calloop::RegistrationToken; 2],
        ),
        Box<dyn std::error::Error>,
    > {
        // First, before `display` moves into the `Generic` below: this is the
        // failure that actually happens, and nothing else should have been
        // set up by the time it is reported.
        let socket = WaylandListener::bind_auto().map_err(socket_error)?;
        let name = socket.socket_name();
        let handle = event_loop.handle();
        let listener = handle
            .insert_source(socket, |stream, _, state: &mut State| {
                super::wayland_accept::admit(state, stream, super::fd_pressure::table());
            })
            // `InsertError`'s own payload is the source that could not be
            // inserted, which is of no use to an operator and would force
            // this function's error type to carry a `WaylandListener`;
            // only the reason is kept.
            .map_err(|error| error.error)?;
        let dispatcher = handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state: &mut State| {
                    // Safety: the display outlives the event loop.
                    let dispatched = unsafe { display.get_mut().dispatch_clients(state) };
                    if let Err(error) = dispatched {
                        // wayland-backend catches a single client's protocol
                        // errors internally and drops just that client;
                        // this only surfaces when the underlying epoll/kevent
                        // wait itself fails, which is process-fatal (there is
                        // nothing left to dispatch to), so stop cleanly
                        // rather than limp on or panic.
                        tracing::error!(%error, "wayland client dispatch failed; stopping");
                        state.loop_signal.stop();
                        return Ok(PostAction::Continue);
                    }
                    // A disconnecting client -- or one destroying its lock
                    // object without disconnecting -- is seen *here* and
                    // nowhere else, which matters for exactly one thing: it
                    // may have been holding the session lock, and nothing
                    // about a destroyed protocol object marks the screen
                    // dirty, drops the surfaces it left behind or moves the
                    // focus off them. See `session_lock.rs`.
                    state.refresh_lock_state();
                    // Same reasoning, one object down: a client destroying
                    // the popup it was grabbing with -- which is how a menu
                    // closes -- is seen here and nowhere else, and nothing
                    // about a destroyed role object hands the keyboard back
                    // or marks the screen dirty. See `popup.rs`. Costs one
                    // `Option` check when no menu is open, which is always,
                    // except when one is.
                    state.settle_popup_grab();
                    // Replies (e.g. the initial registry globals) should reach
                    // the socket now rather than wait for `mod.rs`'s
                    // `post_dispatch` (which also flushes every client, once
                    // per dispatch cycle, strictly after this source runs) --
                    // a client with no surface yet, like `wayland-info` or
                    // `foot` before its first frame, has nothing else that
                    // would prompt a flush before its own next read, and this
                    // one costs nothing extra when there's nothing queued (see
                    // `post_dispatch`'s doc).
                    let _ = state.display_handle.flush_clients();
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|error| error.error)?;
        Ok((name, [listener, dispatcher]))
    }

    /// Milliseconds since start, which is what Wayland input events carry.
    pub fn millis(&self) -> u32 {
        self.start_time.elapsed().as_millis() as u32
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    /// The window whose root surface `surface` is: an xdg toplevel's, or --
    /// in an `xwayland` build -- the surface XWayland associated with a
    /// managed X window. On the commit path, so allocation-free: the X arm
    /// takes that window's state lock and a reference-count bump to compare,
    /// and only X windows pay it.
    pub fn id_of(&self, surface: &WlSurface) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, window)| {
                if let Some(toplevel) = window.toplevel() {
                    return toplevel.wl_surface() == surface;
                }
                #[cfg(feature = "xwayland")]
                if let Some(x11) = window.x11_surface() {
                    return x11.wl_surface().as_ref() == Some(surface);
                }
                false
            })
            .map(|(&id, _)| id)
    }

    /// Who owns `surface` -- `None` if the surface or its client is already
    /// gone, which a client disconnecting mid-event makes possible.
    ///
    /// The same lookup `focus_changed` (see `handlers.rs`) does for the
    /// selection, named here because `input.rs` needs it per key and button
    /// event to record *whose* interaction a serial was (see
    /// `input/interaction.rs`). It costs a backend lookup, no allocation.
    pub(super) fn client_of(&self, surface: &WlSurface) -> Option<ClientId> {
        self.display_handle
            .get_client(surface.id())
            .ok()
            .map(|client| client.id())
    }

    /// What the pointer is over: the top-most surface at `pos` and where that
    /// surface sits, in the same global coordinates the pointer moves in.
    ///
    /// The search order is the render order read from the front: a bar or an
    /// overlay wins over any window, and a wallpaper loses to every one of
    /// them. A layer surface that declines `pos` -- an unmapped one, or one
    /// whose client set an input region that excludes the point -- falls
    /// through to whatever is behind it rather than swallowing the pointer,
    /// because `layer_surface_under` asks the surface tree (which honours
    /// `wl_surface.set_input_region`) rather than the layer's bounding box.
    /// Note this is the *input* region, not opacity: a bar drawn fully
    /// transparent but leaving its input region at the default takes the
    /// pointer, which is the protocol's answer and what a click-through bar
    /// has to opt out of explicitly.
    ///
    /// One caveat on "falls through": it falls through to the next *layer*
    /// (overlay, then top), not to another surface on the same layer --
    /// `LayerMap::layer_under` hands back a single layer surface rather than
    /// an iterator, so if the front-most one on a layer declines the point,
    /// a second surface overlapping it on that same layer is not asked.
    /// Overlapping surfaces on one layer are already drawn on top of each
    /// other, so this costs nothing in practice; anvil has the same
    /// limitation.
    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        // Before every other candidate, and returning whatever it finds --
        // including `None`. While the session is locked nothing but a lock
        // surface may be pointed at, so this is a replacement for the search
        // below, never a first entry in it (see `session_lock.rs`).
        if self.session_lock.is_locked() {
            return self.lock_surface_under(pos);
        }
        let above = self.layer_surface_under(&layer_shell::ABOVE_WINDOWS, pos);
        // Override-redirect X windows (menus, tooltips) sit between the top
        // layers and the windows, as they are drawn (see
        // `xwayland/unmanaged.rs`).
        #[cfg(feature = "xwayland")]
        let above = above.or_else(|| self.x11_unmanaged_under(pos));
        above
            .or_else(|| self.window_under(pos))
            .or_else(|| self.layer_surface_under(&layer_shell::BELOW_WINDOWS, pos))
    }

    /// The window surface at `pos`, ignoring layer surfaces entirely --
    /// among the windows placed on the output `pos` is on, and no other (see
    /// `output_clip.rs`), so the pointer follows what is drawn there.
    pub(super) fn window_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.window_element_under(pos)
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(surface, point)| (surface, (point + location).to_f64()))
            })
    }

    /// Runs a command inside this session.
    ///
    /// The child inherits `WAYLAND_DISPLAY` and the IPC socket path, the
    /// session-identity environment (`XDG_CURRENT_DESKTOP`,
    /// `XDG_SESSION_TYPE`, `XDG_SESSION_DESKTOP` -- see `session_env` for
    /// which are unconditional and which fill a vacuum), the live cursor
    /// theme (`XCURSOR_THEME`/`XCURSOR_SIZE`, read off the rebuilt `Cursor`
    /// so a reloaded theme reaches future children), and -- while the
    /// session's XWayland server is believed live -- `DISPLAY` for X11
    /// clients (see `xwayland/mod.rs`; unset there means inherit, so a
    /// host-provided `DISPLAY` under `--nested` survives when XWayland is
    /// off), and -- unless the
    /// token table is full -- a fresh activation token in
    /// `XDG_ACTIVATION_TOKEN`, so it can activate its own window when it maps
    /// one (see [`State::mint_spawn_token`] for which bounds apply and what a
    /// full table means). Takes `&mut` for the token table; both callers
    /// (`act`, `run`) already hold it mutably.
    ///
    /// Reports whether the entry is decided: `true` when the child started
    /// (tracked for the reaper below), or when there was nothing to start
    /// (an empty command is a silent no-op, startup's shape -- unreachable
    /// through any parser, which refuses a missing command at load). A
    /// failed `spawn` (a missing program) warns and reports `false`, so the
    /// config-reload delta can keep the entry pending for the next reload
    /// instead of marking it seen. The OS error itself lives only in that
    /// `warn!`: threading it out through `act`'s effect loop would widen
    /// both signatures to duplicate what the log already says.
    pub fn spawn(&mut self, command: &[String]) -> bool {
        let Some((program, args)) = command.split_first() else {
            return true;
        };
        let mut child = Command::new(program);
        child.args(args).env("WAYLAND_DISPLAY", &self.socket_name);
        // The soft fd limit this process was started with, not the raised
        // one: a child using `select()` cannot watch an fd past 1023 (see
        // `nofile.rs`).
        super::nofile::restore_for_child(&mut child);
        if let Some(path) = &self.ipc_path {
            child.env(scoot_ipc::SOCKET_ENV, path);
        }
        // The X display while our server is believed live, and nothing
        // otherwise: `None` inherits, so enabling nothing clobbers nothing
        // (a host `DISPLAY` under `--nested` survives). Set explicitly --
        // like `WAYLAND_DISPLAY` above -- rather than relying on the
        // process environment, so the contract reads at this site and a
        // test-harness `State` (whose process environment was never
        // settled) still gets it right. Unconditional: without the Cargo
        // feature `xdisplay` stays `None` and this is a no-op.
        if let Some(display) = self.xdisplay {
            child.env(
                super::xwayland::DISPLAY_ENV,
                super::xwayland::display_value(display),
            );
        }
        // The same `resolve` `run` applied to the process environment (see
        // `session_env`): re-resolving here is idempotent there, and keeps a
        // child correct even where the process environment was never settled.
        // Set explicitly rather than inherited so the contract reads at this
        // site, the way `WAYLAND_DISPLAY` does above.
        let current_desktop = std::env::var(session_env::CURRENT_DESKTOP).ok();
        let session_type = std::env::var(session_env::SESSION_TYPE).ok();
        let session_desktop = std::env::var(session_env::SESSION_DESKTOP).ok();
        let session_env = session_env::resolve(
            current_desktop.as_deref(),
            session_type.as_deref(),
            session_desktop.as_deref(),
        );
        child.env(session_env::CURRENT_DESKTOP, session_env.current_desktop);
        child.env(session_env::SESSION_TYPE, session_env.session_type);
        child.env(session_env::SESSION_DESKTOP, session_env.session_desktop);
        // The cursor theme this reload cycle resolved, read live rather than
        // inherited: a `cursor_theme`/`cursor_size` reload rebuilds `Cursor`
        // after `run`'s process-wide export ran, and every compositor child
        // is spawned here, so this is what keeps a future child drawing the
        // same theme the compositor draws. Set explicitly (like
        // `WAYLAND_DISPLAY` above) rather than by mutating the process
        // environment on the loop thread, which no other thread can be
        // proven not to read.
        child.env("XCURSOR_THEME", self.cursor.theme().name());
        child.env("XCURSOR_SIZE", self.cursor.theme().size().to_string());
        // Removed rather than overwritten: the compositor itself may have been
        // started with one (a launcher client, a nested session), and that
        // token is a receipt for someone else's user action -- handing it to
        // this child would let it spend an interaction it was never given.
        child.env_remove(State::ACTIVATION_TOKEN_ENV);
        let token = self.mint_spawn_token(program);
        if let Some(token) = &token {
            child.env(State::ACTIVATION_TOKEN_ENV, token.as_str());
        }
        // While XWayland is live, the same token again as the X toolkits'
        // startup id -- the chain the X focus gate redeems (see
        // `xwayland/focus.rs`). Removed first for the same reason as above:
        // an inherited one is a receipt for someone else's action. Only
        // while live, so a session without XWayland spawns exactly as it
        // always has.
        if self.xdisplay.is_some() {
            child.env_remove(State::STARTUP_ID_ENV);
            if let Some(token) = &token {
                child.env(State::STARTUP_ID_ENV, token.as_str());
            }
        }
        match child.spawn() {
            Ok(child) => {
                // The other half of that chain, for X clients that set no
                // startup id at all: which process this token was minted
                // for, so an X window whose client is this process (as the
                // X server reports it) can be matched to it.
                #[cfg(feature = "xwayland")]
                if self.xdisplay.is_some()
                    && let Some(data) = token
                        .as_ref()
                        .and_then(|token| self.xdg_activation.data_for_token(token))
                {
                    let pid = child.id();
                    data.user_data
                        .insert_if_missing_threadsafe(|| super::xwayland::SpawnedPid(pid));
                }
                // Tracked for the SIGCHLD drain, which reaps exactly these
                // pids and nothing else (see `child_reaper.rs`). Inserted
                // synchronously here, before the child can possibly exit and
                // before any drain can run -- both this and the drain live on
                // the loop thread -- so no reap is lost and none is doubled.
                self.spawned_children.insert(child.id());
                tracing::info!(?command, "spawned");
                true
            }
            Err(error) => {
                // The child never started, so nothing will ever redeem this:
                // pull it back out rather than occupying a slot until the
                // sweep finds it.
                if let Some(token) = token {
                    self.xdg_activation.remove_token(&token);
                }
                tracing::warn!(?command, %error, "could not spawn");
                false
            }
        }
    }
}

/// What to tell the operator when the wayland socket cannot be created.
///
/// `BindError`'s own `Display` names the cause; each message here adds what to
/// do about it, in the shape this project's other startup errors already have
/// (`ipc::init`'s "no socket path: set SCOOT_SOCKET or XDG_RUNTIME_DIR",
/// `config.rs`'s `ConfigFileError`). Printed by `main` as
/// `scoot: <this message>`.
///
/// Pure, and a function rather than inline `match` arms, because that is what
/// makes it testable: the variant that matters is
/// [`BindError::RuntimeDirNotSet`], and provoking it for real means unsetting
/// a process-global environment variable that every other test in this binary
/// needs (`State::new` binds a real socket) -- the same hazard item 9's umask
/// attempt hit and abandoned. The end-to-end behavior is verified against a
/// real release binary instead; see
/// `docs/roadmap/12-four-bounds.md` for that verification.
fn socket_error(error: BindError) -> String {
    match error {
        BindError::RuntimeDirNotSet => {
            "no wayland socket: $XDG_RUNTIME_DIR is not set or invalid; \
             set it to a writable directory (a login session normally provides one)"
                .to_string()
        }
        BindError::PermissionDenied => {
            "no wayland socket: $XDG_RUNTIME_DIR is not writable".to_string()
        }
        // `WaylandListener::bind_auto` tries `wayland-1` through
        // `wayland-32` (1..33, verified in the pinned rev's
        // `wayland/socket.rs`), so reaching this means all 32 are taken.
        BindError::AlreadyInUse => {
            "no wayland socket: wayland-1 through wayland-32 are all in use \
             in $XDG_RUNTIME_DIR; stop another compositor, or remove a stale socket and its .lock"
                .to_string()
        }
        BindError::Io(source) => format!("no wayland socket: {source}"),
    }
}

/// Per-client data Smithay hands back on every request.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _id: ClientId) {}
    /// A client is gone -- cleanly, crashed, or killed by a protocol error.
    ///
    /// Logging-only, deliberately: `dispatch.rs`'s module doc records that
    /// this runs while wayland-backend still holds its internal state mutex,
    /// so anything here that touched the `DisplayHandle` (directly or through
    /// `State`) would deadlock the compositor from inside every `post_error`
    /// call. A protocol error used to leave no trace at all in scoot's log,
    /// which is how a compositor-side kill of a layer-shell client stayed
    /// undiagnosed; the `ProtocolError` reason names the code, the object and
    /// the message.
    fn disconnected(&self, id: ClientId, reason: DisconnectReason) {
        match &reason {
            DisconnectReason::ConnectionClosed => {
                // `debug!`, not `info!`: a client closing its own connection
                // is the uninteresting half of this, and it is the only half
                // that repeats. Under Selkies (every webtop deployment) the
                // clipboard monitor shells out to `wl-paste --list-types`
                // every 500 ms, and each poll is a whole fresh connection --
                // so an *idle* session logged two lines a second forever,
                // burying everything else. See
                // `docs/backlog/resolved/clean-disconnect-log-flood-done.md`.
                tracing::debug!(?id, "wayland client disconnected");
            }
            DisconnectReason::ProtocolError(error) => {
                tracing::warn!(?id, ?error, "wayland client killed by a protocol error");
            }
        }
    }
}

impl State {
    /// Takes output `id`'s render target out of the map, so the render path
    /// can hold `&mut State` and `&mut` the renderer at the same time.
    ///
    /// The take-and-put-back pair is the same shape the single-backend code
    /// used (`backend.take()` around `draw_frame`); per output it also means
    /// a frame for output A never holds output B's target.
    pub(super) fn take_backend(&mut self, id: OutputId) -> Option<Backend> {
        self.backends.remove(&id)
    }

    /// Puts a taken render target back. Unconditional insert: ids are never
    /// reused and nothing else writes this map mid-frame, so the slot is
    /// always empty here.
    pub(super) fn put_backend(&mut self, id: OutputId, backend: Backend) {
        self.backends.insert(id, backend);
    }

    /// Takes the primary output's render target, for the suites that draw
    /// one-output sessions by hand. `None` before any output exists -- the
    /// same shape `take_backend` has, so a test that needs the missing case
    /// can assert on it rather than on a panic.
    #[cfg(test)]
    pub(super) fn take_primary_backend(&mut self) -> Option<Backend> {
        let id = self.outputs.primary_id()?;
        self.take_backend(id)
    }

    /// Puts the primary output's render target back. The counterpart the
    /// suites above pair with [`State::take_primary_backend`]: a no-op
    /// without a primary output, so a test that took `None` drops nothing
    /// anywhere.
    #[cfg(test)]
    pub(super) fn put_primary_backend(&mut self, backend: Backend) {
        if let Some(id) = self.outputs.primary_id() {
            self.put_backend(id, backend);
        }
    }
}
