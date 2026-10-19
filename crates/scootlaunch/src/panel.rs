//! The panel: the overlay exclusive-keyboard surface with the entry
//! and the virtualized list, fed by the matcher.
//!
//! One process per open: the surface is created on open and destroyed on
//! close, so nothing is held at idle. The look layer is read at every
//! open; the single-instance toggle (see [`socket`](crate::socket))
//! decides whether this invocation opens or dismisses. Picking an
//! entry launches it ([`spawn`](crate::spawn): the activation token,
//! the environment, its own systemd scope, the detached fallback);
//! dismissing exits 1. A failed spawn draws its error for 2 s and the
//! panel stays open.
//!
//! Keys and mouse follow the design's table: typing filters, the
//! selection keys move, Enter launches the selection, Ctrl+Enter runs
//! the typed text, Escape dismisses. A click launches the row; the
//! wheel scrolls. The token is minted from the choosing key or button
//! serial (never a focus serial, which the compositor refuses); past
//! its 200 ms bound the launch goes without one. A click
//! on the bare desktop does not dismiss in v1: with exclusive keyboard
//! the compositor holds focus (`layer_keyboard_focus` keeps an
//! exclusive surface first), so no keyboard leave arrives and the click
//! itself goes to the surface below, never to the panel. A keyboard
//! leave with the pointer outside the panel still dismisses (whatever
//! compositor policy delivers one), while a leave with the pointer
//! still over the panel (a locker taking the keyboard) keeps drawing
//! and waits for focus, as the design's failure table says.
//!
//! Control tracking comes off the `wl_keyboard.modifiers` depressed
//! mask's Control bit (`0x4`, the XKB core bit): the seat reports a
//! release with the same keysym and empty text as a navigation press,
//! so the app tracks pressed keys itself (cleared on every focus change)
//! to tell a release from a press.

use std::os::unix::io::AsFd as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use scoot_rt::print::warn;
use scootui::entry::{Geometry, paint_entry, paint_list};
use scootui::keyboard::{
    KeyOutput, KeyboardSeat, MIN_TEXT_BYTES, dispatch_keyboard, negotiate_version,
};
use scootui::output::Size;
use scootui::paint::Canvas;
use scootui::surfaces::{Buffers, Demand, Effect, Globals, LayerLive, LayerSpec, Plan};
use scootui::text::Text;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    Event as ActivationTokenEvent, XdgActivationTokenV1,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{self, Anchor};

use crate::index::{Action, Entry};
use crate::look::Look;
use crate::spawn::{self, Token};
use crate::ui::{Outcome, Ui};
use scootui::outputs::Frame;

/// How long the registry sweep and the first configure may take: a local
/// compositor answers in milliseconds, so this only bounds a wedge.
const PATIENCE: Duration = Duration::from_secs(10);
/// The panel border's thickness, in device pixels.
const BORDER: u32 = 2;
/// The padding between the border and the widgets, in device pixels.
const PAD: u32 = 8;
/// The gap between the entry line and the list, in device pixels.
const GAP: u32 = 4;
/// The entry's placeholder, in `dim`.
const PLACEHOLDER: &str = "Type to search";
/// evdev `BTN_LEFT`: wayland-client names no button codes.
const BTN_LEFT: u32 = 0x110;
/// The XKB core Control bit in the `wl_keyboard.modifiers` depressed
/// mask (`Shift` is `0x1`, `Lock` is `0x2`).
const MOD_CTRL: u32 = 0x4;

/// What the panel opens with: `--font PATH`, `--config PATH`, and the
/// index's `--lenient`.
pub struct Options {
    pub font: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub lenient: bool,
}

/// Opens the panel and runs it to a launch or a dismiss: 0 for a
/// launched pick, 1 for a dismiss, a runtime failure or a bad file
/// (the flags were fine, as the bar's), 2 for a bad invocation.
pub fn open(opts: &Options) -> i32 {
    // The toggle first: a live panel is told to close, and this
    // invocation exits without needing a font, an index or a
    // compositor connection of its own.
    let socket_path = match crate::socket::live_socket_path() {
        Ok(path) => path,
        Err(error) => {
            warn(format_args!("scootlaunch: {error}"));
            return 1;
        }
    };
    let claim = match crate::socket::open(&socket_path) {
        Ok(crate::socket::Open::Signalled) => return 0,
        Ok(crate::socket::Open::Listen(claim)) => claim,
        Err(error) => {
            warn(format_args!("scootlaunch: {error}"));
            return 1;
        }
    };
    // The user's own file: missing is the defaults; bad is a loud
    // refusal (there is no running instance a bad file could loop).
    let config = match &opts.config {
        Some(path) => match crate::config::load(path) {
            Ok(config) => config,
            Err(error) => {
                warn(format_args!("scootlaunch: {error}"));
                return 1;
            }
        },
        None => {
            let path = crate::config::default_path();
            match std::fs::read(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    crate::config::Config::default()
                }
                Err(error) => {
                    warn(format_args!(
                        "scootlaunch: cannot read {}: {error}",
                        path.display()
                    ));
                    return 1;
                }
                Ok(bytes) => match crate::config::parse(&bytes) {
                    Ok(config) => config,
                    Err(error) => {
                        warn(format_args!("scootlaunch: {error}"));
                        return 1;
                    }
                },
            }
        }
    };
    // The look layer, read at this open: missing is the defaults, one
    // warning per fallback.
    let (look, notes) = crate::look::load(None);
    for note in &notes {
        warn(format_args!("scootlaunch: {note}"));
    }
    // The index and the frecency the list ranks with.
    let env = crate::index::Env::from_env();
    let built = crate::index::build(
        &env,
        &crate::index::BuildOpts {
            use_cache: true,
            lenient_exec: opts.lenient,
        },
    );
    for note in &built.notes {
        warn(format_args!("scootlaunch: {note}"));
    }
    let history = crate::rank::frecency::load(env.xdg_state_home.as_deref());
    let now = crate::rank::frecency::now_secs();
    let ui = Ui::open(built.entries, history, now, config.rows as usize);
    let launch = LaunchCtx {
        state_home: env.xdg_state_home.clone(),
        home: std::env::var_os("HOME").map(PathBuf::from),
        display_set: std::env::var_os("DISPLAY").is_some(),
    };
    // The typeface: the flag, then the file, then the well-known ones.
    let given = opts.font.clone().or_else(|| config.font.map(PathBuf::from));
    let mut text = match scootui::font::text(given.as_deref(), &[]) {
        Ok(text) => text,
        Err(error) => {
            warn(format_args!("scootlaunch: {error}"));
            return 1;
        }
    };
    let em = look.font_size_px.clamp(1.0, 256.0);
    let row_h = em.ceil() as u32 + 8;
    let width = (config.width_em as f32 * em) as u32;
    let height = BORDER
        .saturating_add(PAD)
        .saturating_mul(2)
        .saturating_add(row_h.saturating_mul(config.rows.saturating_add(1)))
        .saturating_add(GAP);
    run_panel(
        claim,
        ui,
        &mut text,
        &look,
        PanelSpec {
            width,
            height,
            row_h,
            launch,
        },
    )
}

/// What a launch resolves against: read once at open, so the spawn
/// never touches the process environment (which the tests sandbox).
struct LaunchCtx {
    state_home: Option<PathBuf>,
    home: Option<PathBuf>,
    display_set: bool,
}

/// What the Wayland session opens with: the panel's size and the
/// launch context, bundled so the runner takes few arguments.
struct PanelSpec {
    width: u32,
    height: u32,
    row_h: u32,
    launch: LaunchCtx,
}

/// Runs the Wayland session to a pick or a dismiss.
fn run_panel(
    claim: crate::socket::Claim,
    ui: Ui,
    text: &mut Text,
    look: &Look,
    spec: PanelSpec,
) -> i32 {
    let conn = match Connection::connect_to_env() {
        Ok(conn) => conn,
        Err(error) => {
            warn(format_args!(
                "scootlaunch: cannot reach the Wayland display: {error}"
            ));
            return 1;
        }
    };
    let queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut pump = Pump { queue, qh };
    let mut app = App::new(claim, ui, spec.width, spec.height, spec.row_h, spec.launch);
    // The registry sweep: globals, then outputs, keyboard and pointer.
    if !pump.wait_until(PATIENCE, &mut app, |app| app.ready()) {
        warn(format_args!(
            "scootlaunch: no layer-shell session: {}",
            app.missing()
        ));
        return pump.finish(app, 1, None);
    }
    if app.outputs == 0 {
        warn(format_args!("scootlaunch: no outputs: nothing to open on"));
        return pump.finish(app, 1, None);
    }
    // A null output: the compositor picks (today its primary; the
    // `layer-shell-default-output` core ticket moves it under the
    // pointer). The panel carries no output logic of its own.
    app.show(pump.handle());
    // The first configure sizes the draws; without one there is nothing
    // to draw into.
    if !pump.wait_until(PATIENCE, &mut app, |app| app.configured()) {
        warn(format_args!("scootlaunch: the surface never configured"));
        return pump.finish(app, 1, None);
    }
    if app.exit.is_some() {
        let exit = app.exit.take();
        return pump.finish(app, 1, exit);
    }
    pump.drive(app, text, look)
}

/// The exit: what the drive ended with.
enum Exit {
    /// A pick or run text: launched on the way out (the token, the
    /// spawn, the scope), not printed. Boxed: the request dwarfs the
    /// other exits.
    Launch(Box<LaunchRequest>),
    /// Dismissed (Escape, outside click, toggle, compositor close).
    Dismiss,
    /// A runtime failure, already warned about.
    Failed,
}

/// What one pick carries into the launch: the entry (or nothing for
/// run text) with the argv resolved at pick time, and the choosing
/// input's serial for the token.
struct LaunchRequest {
    entry: Option<Entry>,
    action: Option<Action>,
    /// Ad-hoc argv (`Ctrl+Enter`): the typed text split on spaces
    /// ([`spawn::split_run_argv`]). `None` for an entry pick (the
    /// entry's own argv launches).
    run_argv: Option<Vec<String>>,
    serial: Option<u32>,
}

/// Runs one pick to completion: the entry (or command) with `token`.
/// Frecency is recorded inside the spawn on any launch an app came
/// out of. `Err` is a spawn failure: nothing started, nothing
/// recorded.
fn execute_launch(
    request: &LaunchRequest,
    token: Option<&Token>,
    launch: &LaunchCtx,
    now: u64,
) -> Result<(), String> {
    let ctx = spawn::Ctx {
        state_home: launch.state_home.clone(),
        home: launch.home.clone(),
        display_set: launch.display_set,
        now,
    };
    match &request.entry {
        Some(entry) => spawn::launch_entry(entry, request.action.as_ref(), token, &ctx),
        None => match &request.run_argv {
            Some(argv) => spawn::launch_command(argv, token, &ctx),
            None => Err("nothing to launch".to_string()),
        },
    }
}

/// The Wayland state: bound globals, the on-demand surface and its
/// buffers, the seat and pointer, the panel's state machine, and what
/// the compositor last said.
struct App {
    claim: crate::socket::Claim,
    ui: Ui,
    width: u32,
    height: u32,
    row_h: u32,
    launch: LaunchCtx,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<ZwlrLayerShellV1>,
    activation: Option<XdgActivationV1>,
    viewporter: Option<WpViewporter>,
    fractional: Option<WpFractionalScaleManagerV1>,
    seat_proxy: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    outputs: u32,
    seat: KeyboardSeat,
    ctrl: bool,
    /// Pressed keysyms without their release yet: a release reports the
    /// same keysym with empty text as a navigation press, so this tells
    /// them apart. Cleared on every focus change.
    pressed: Vec<u32>,
    /// The last key press's serial, for the token of an Enter pick
    /// (the compositor only honors key and button serials delivered
    /// to this client — never a focus serial).
    key_serial: Option<u32>,
    /// The last button press's serial, for the token of a click pick.
    button_serial: Option<u32>,
    /// The token's `done`, while a launch waits for it.
    token_done: Option<String>,
    /// A spawn failure, shown in the panel until its instant passes.
    spawn_error: Option<(String, Instant)>,
    pointer_in: bool,
    pointer_x: u32,
    pointer_y: u32,
    demand: Demand,
    buffers: Buffers,
    live: Option<LayerLive>,
    configured_size: Option<(u32, u32)>,
    dirty: bool,
    exit: Option<Exit>,
    closed_retried: bool,
}

impl App {
    fn new(
        claim: crate::socket::Claim,
        ui: Ui,
        width: u32,
        height: u32,
        row_h: u32,
        launch: LaunchCtx,
    ) -> Self {
        Self {
            claim,
            ui,
            width,
            height,
            row_h,
            launch,
            compositor: None,
            shm: None,
            layer_shell: None,
            activation: None,
            viewporter: None,
            fractional: None,
            seat_proxy: None,
            keyboard: None,
            pointer: None,
            outputs: 0,
            seat: KeyboardSeat::new(),
            ctrl: false,
            pressed: Vec::new(),
            key_serial: None,
            button_serial: None,
            token_done: None,
            spawn_error: None,
            pointer_in: false,
            pointer_x: 0,
            pointer_y: 0,
            demand: Demand::new(),
            buffers: Buffers::new(c"scootlaunch"),
            live: None,
            configured_size: None,
            dirty: true,
            exit: None,
            closed_retried: false,
        }
    }

    fn ready(&self) -> bool {
        self.compositor.is_some()
            && self.shm.is_some()
            && self.layer_shell.is_some()
            && self.seat_proxy.is_some()
            && self.keyboard.is_some()
    }

    fn missing(&self) -> String {
        let mut missing = Vec::new();
        if self.compositor.is_none() {
            missing.push("wl_compositor");
        }
        if self.shm.is_none() {
            missing.push("wl_shm");
        }
        if self.layer_shell.is_none() {
            missing.push("layer-shell");
        }
        if self.keyboard.is_none() {
            missing.push("a keyboard");
        }
        missing.join(", ")
    }

    fn configured(&self) -> bool {
        self.configured_size.is_some()
    }

    fn globals(&self) -> Option<Globals<'_>> {
        Some(Globals {
            compositor: self.compositor.as_ref()?,
            layer_shell: self.layer_shell.as_ref(),
            xdg_wm_base: None,
            viewporter: self.viewporter.as_ref(),
            fractional_scale: self.fractional.as_ref(),
        })
    }

    /// Creates the surface for the current demand: a null output,
    /// centered, exclusive keyboard, no reserved space.
    fn show(&mut self, qh: &QueueHandle<Self>) {
        if self.live.is_some() {
            return;
        }
        match self.demand.show() {
            Effect::Create => {}
            _ => return,
        }
        let spec = LayerSpec {
            namespace: "scootlaunch",
            exclusive_keyboard: true,
            anchor: Anchor::empty(),
            width: self.width,
            height: self.height,
            margin: [0, 0, 0, 0],
            exclusive_zone: 0,
        };
        let Some(globals) = self.globals() else {
            warn(format_args!("scootlaunch: no compositor to open on"));
            self.exit = Some(Exit::Failed);
            return;
        };
        match LayerLive::create(&globals, None, &spec, qh, ()) {
            Ok(live) => {
                self.live = Some(live);
                self.dirty = true;
            }
            Err(error) => {
                warn(format_args!(
                    "scootlaunch: cannot open the panel: {error:?}"
                ));
                self.exit = Some(Exit::Failed);
            }
        }
    }

    fn destroy_live(&mut self) {
        self.buffers.clear();
        if let Some(live) = self.live.take() {
            live.destroy();
        }
    }

    /// Whether the pointer rests outside the panel: a keyboard leave
    /// then is an outside click, not a locker.
    fn pointer_outside(&self) -> bool {
        !self.pointer_in || self.pointer_x >= self.width || self.pointer_y >= self.height
    }

    /// The shown list row under the pointer, if the pointer is over one.
    fn hit_row(&self) -> Option<usize> {
        if !self.pointer_in || self.pointer_x >= self.width {
            return None;
        }
        let list_y = BORDER
            .saturating_add(PAD)
            .saturating_add(self.row_h)
            .saturating_add(GAP);
        if self.pointer_y < list_y || self.row_h == 0 {
            return None;
        }
        let row = (self.pointer_y - list_y) / self.row_h;
        let row = row as usize;
        (row < self.ui.list().visible_range().len()).then_some(row)
    }

    /// Draws the panel into a pooled buffer and commits it.
    fn draw(&mut self, text: &mut Text, look: &Look, qh: &QueueHandle<Self>) {
        let Some(size) = self.configured_size else {
            return;
        };
        let size = Size {
            width: if size.0 == 0 { self.width } else { size.0 },
            height: if size.1 == 0 { self.height } else { size.1 },
        };
        match self.demand.plan(Some(size), self.dirty) {
            Plan::Draw(frame) => self.draw_frame(frame, text, look, qh),
            Plan::Commit => {
                if let Some(live) = self.live.as_ref() {
                    live.surface().commit();
                    self.demand.committed();
                }
            }
            Plan::Nothing => {}
        }
    }

    fn draw_frame(&mut self, frame: Frame, text: &mut Text, look: &Look, qh: &QueueHandle<Self>) {
        let dims = match self.demand.buffer_dims(frame.size) {
            Some(dims) => dims,
            None => return,
        };
        let Some(shm) = self.shm.as_ref() else {
            return;
        };
        let slot = match self
            .buffers
            .take(shm, qh, dims, wl_shm::Format::Argb8888, ())
        {
            Ok(Some(slot)) => slot,
            Ok(None) => return,
            Err(error) => {
                warn(format_args!("scootlaunch: cannot draw the panel: {error}"));
                self.exit = Some(Exit::Failed);
                return;
            }
        };
        // A fresh spawn failure, drawn instead of the list until it
        // clears (the drive expires it after 2 s): the failed pick
        // stays visible, and the panel stays open.
        let error = self.spawn_error.as_ref().map(|(text, _)| text.clone());
        paint_panel(
            slot.pixels_mut(),
            dims,
            &self.ui,
            text,
            look,
            self.width,
            self.height,
            self.row_h,
            error.as_deref(),
        );
        let Some(live) = self.live.as_mut() else {
            return;
        };
        live.surface().attach(Some(slot.buffer()), 0, 0);
        // Full damage, every draw: the bar's shape (exact buffer rect,
        // as its canvas does). Without it the compositor keeps showing
        // the last presented frame: the mapped first frame shows
        // regardless, which reads as "opens fine, never updates".
        live.surface()
            .damage_buffer(0, 0, dims.0 as i32, dims.1 as i32);
        live.prepare(frame.size, frame.scale);
        live.surface().commit();
        slot.mark_committed();
        self.demand.drew(frame);
        self.dirty = false;
    }

    /// Feeds one decoded key output into the state machine: repeats and
    /// presses move, releases only retire the pressed tracking.
    fn feed(&mut self, output: KeyOutput, bytes: &[u8; MIN_TEXT_BYTES]) {
        let decoded = core::str::from_utf8(&bytes[..output.text_len]).unwrap_or("");
        if output.repeat {
            self.apply_press(output.keysym, decoded);
            return;
        }
        if output.text_len > 0 {
            self.remember(output.keysym);
            self.apply_press(output.keysym, decoded);
            return;
        }
        // No text: a navigation press, or any key's release. The pressed
        // set tells them apart.
        if let Some(at) = self.pressed.iter().position(|held| *held == output.keysym) {
            self.pressed.swap_remove(at);
        } else {
            self.remember(output.keysym);
            self.apply_press(output.keysym, decoded);
        }
    }

    fn remember(&mut self, keysym: u32) {
        if !self.pressed.contains(&keysym) {
            self.pressed.push(keysym);
        }
    }

    fn apply_press(&mut self, keysym: u32, decoded: &str) {
        match self.ui.key(keysym, self.ctrl, decoded) {
            Outcome::Ignored => {}
            Outcome::Changed => self.dirty = true,
            Outcome::Selected(_) => {
                let serial = self.key_serial;
                if let Some(entry) = self.ui.selected() {
                    self.exit = Some(Exit::Launch(Box::new(LaunchRequest {
                        entry: Some(entry.clone()),
                        action: self.ui.selected_action().cloned(),
                        run_argv: None,
                        serial,
                    })));
                }
            }
            Outcome::Run(run) => {
                let argv = spawn::split_run_argv(&run);
                if argv.is_empty() {
                    return;
                }
                self.exit = Some(Exit::Launch(Box::new(LaunchRequest {
                    entry: None,
                    action: None,
                    run_argv: Some(argv),
                    serial: self.key_serial,
                })));
            }
            Outcome::Cancelled => self.exit = Some(Exit::Dismiss),
        }
    }

    /// Emits due repeat ticks into the state machine, exactly what an
    /// app's daemon loop does with the timerfd in its poll set.
    fn drain_repeats(&mut self) {
        let mut text = [0u8; MIN_TEXT_BYTES];
        while let Some((_, output)) = self.seat.tick(&mut text) {
            let decoded = core::str::from_utf8(&text[..output.text_len]).unwrap_or("");
            self.apply_press(output.keysym, decoded);
            if self.exit.is_some() {
                return;
            }
        }
    }
}

/// Paints the panel: the rounded chrome (border ring in `focus`, fill in
/// `surface` at `opacity`), the entry line, and the list's visible rows.
/// `dims` is the buffer size; the logical `width`/`height` size the panel
/// when the compositor configures larger. The entry sits at the top, the
/// list below a [`GAP`]. A fresh spawn failure draws instead of the list.
#[allow(clippy::too_many_arguments)]
fn paint_panel(
    pixels: &mut [u8],
    dims: (u32, u32),
    ui: &Ui,
    text: &mut Text,
    look: &Look,
    width: u32,
    height: u32,
    row_h: u32,
    error: Option<&str>,
) {
    let (buffer_w, buffer_h) = dims;
    let width = width.min(buffer_w);
    let height = height.min(buffer_h);
    if width == 0 || height == 0 || row_h == 0 {
        return;
    }
    chrome(pixels, buffer_w, width, height, look);
    let Some(mut canvas) = Canvas::new(pixels, buffer_w, buffer_h) else {
        return;
    };
    let em = look.font_size_px.clamp(1.0, 256.0);
    let inner = BORDER.saturating_add(PAD);
    let entry_geo = Geometry {
        x: inner,
        y: inner,
        width: width.saturating_sub(inner.saturating_mul(2)),
        row_height: row_h,
        em,
    };
    paint_entry(
        &mut canvas,
        text,
        &look.theme,
        ui.entry(),
        &entry_geo,
        true,
        PLACEHOLDER,
    );
    if let Some(error) = error {
        let metrics = text.metrics(em);
        let baseline =
            i64::from(inner.saturating_add(row_h).saturating_add(GAP)) + metrics.baseline(row_h);
        text.draw(
            &mut canvas,
            None,
            error,
            em,
            i64::from(inner),
            baseline,
            look.theme.urgent,
            Span {
                x: inner,
                width: entry_geo.width,
            },
        );
        return;
    }
    if ui.list().count() == 0 {
        let metrics = text.metrics(em);
        let baseline =
            i64::from(inner.saturating_add(row_h).saturating_add(GAP)) + metrics.baseline(row_h);
        text.draw(
            &mut canvas,
            None,
            "No applications",
            em,
            i64::from(entry_geo.text_x()),
            baseline,
            look.theme.dim,
            entry_geo.clip(),
        );
        return;
    }
    let list_geo = Geometry {
        x: inner,
        y: inner.saturating_add(row_h).saturating_add(GAP),
        width: entry_geo.width,
        row_height: row_h,
        em,
    };
    paint_list(
        &mut canvas,
        text,
        &look.theme,
        ui.list(),
        &list_geo,
        &|row| ui.row_text(row),
        false,
    );
}

/// The chrome: a rounded rect in `focus` with the `surface` fill inset by
/// [`BORDER`], premultiplied at `opacity`. Outside the rounded rect the
/// buffer stays transparent, so the corners show the desktop. One pass
/// over the panel's pixels; redraws happen on state changes, never per
/// frame.
fn chrome(pixels: &mut [u8], buffer_w: u32, width: u32, height: u32, look: &Look) {
    let radius = look
        .radius
        .min(width.saturating_div(2))
        .min(height.saturating_div(2)) as f32;
    let alpha = (look.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let surface = look.surface.argb8888(alpha);
    let focus = look.focus.argb8888(255);
    let stride = buffer_w as usize * 4;
    let half_w = width as f32 / 2.0;
    let half_h = height as f32 / 2.0;
    for y in 0..height {
        let Some(row) = pixels.chunks_exact_mut(stride).nth(y as usize) else {
            break;
        };
        for x in 0..width {
            // The rounded box's signed distance: negative inside.
            let qx = (x as f32 + 0.5 - half_w).abs() - (half_w - radius);
            let qy = (y as f32 + 0.5 - half_h).abs() - (half_h - radius);
            let distance = if qx > 0.0 || qy > 0.0 {
                (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() - radius
            } else {
                qx.max(qy) - radius
            };
            if distance > 0.0 {
                continue;
            }
            let pixel = if distance > -(BORDER as f32) {
                focus
            } else {
                surface
            };
            let Some(chunk) = row.chunks_exact_mut(4).nth(x as usize) else {
                continue;
            };
            chunk.copy_from_slice(&pixel.to_le_bytes());
        }
    }
}

/// The queue pump: flush, dispatch, repeat ticks, the toggle socket, and
/// draws, until a pick or a dismiss.
struct Pump {
    queue: wayland_client::EventQueue<App>,
    qh: QueueHandle<App>,
}

impl Pump {
    fn handle(&self) -> &QueueHandle<App> {
        &self.qh
    }

    /// Runs dispatches until `done`, or the timeout. `false` on timeout.
    /// The toggle socket is watched throughout: a `close` arriving
    /// mid-open dismisses.
    fn wait_until(
        &mut self,
        timeout: Duration,
        app: &mut App,
        mut done: impl FnMut(&App) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while !done(app) {
            if Instant::now() >= deadline || app.exit.is_some() {
                return done(app);
            }
            if self.queue.flush().is_err() {
                return done(app);
            }
            match self.queue.dispatch_pending(app) {
                Ok(_) => {}
                Err(_) => {
                    return done(app);
                }
            }
            // Flush what the handlers queued (binds, acks): the poll
            // below blocks on the full remaining budget, so without
            // this the requests would sit unsent until it runs out.
            if self.queue.flush().is_err() {
                return done(app);
            }
            if app.claim.accept_close() {
                app.exit = Some(Exit::Dismiss);
                return true;
            }
            if done(app) || app.exit.is_some() {
                return true;
            }
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            let wait = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            let timespec = rustix::time::Timespec {
                tv_sec: wait.as_secs() as _,
                tv_nsec: wait.subsec_nanos() as _,
            };
            let wayland = guard.connection_fd();
            let toggle = app.claim.as_fd();
            let mut polls = [
                rustix::event::PollFd::new(&wayland, rustix::event::PollFlags::IN),
                rustix::event::PollFd::new(&toggle, rustix::event::PollFlags::IN),
            ];
            let _ = rustix::event::poll(&mut polls, Some(&timespec));
            if !polls[0].revents().is_empty() {
                let _ = guard.read();
            }
            // A toggle close with a quiet connection drops the guard
            // unread (the documented `read_after_poll` shape): the next
            // turn's `accept_close` collects it.
        }
        true
    }

    /// Drives the open panel to an exit code.
    fn drive(mut self, mut app: App, text: &mut Text, look: &Look) -> i32 {
        let qh = self.qh.clone();
        app.draw(text, look, &qh);
        loop {
            if self.queue.flush().is_err() {
                return self.finish(app, 1, None);
            }
            if self.queue.dispatch_pending(&mut app).is_err() {
                return self.finish(app, 1, None);
            }
            if self.queue.flush().is_err() {
                return self.finish(app, 1, None);
            }
            app.drain_repeats();
            if app.claim.accept_close() {
                return self.finish(app, 1, Some(Exit::Dismiss));
            }
            // A spawn failure clears after 2 s, redrawing the list
            // under it.
            if let Some((_, since)) = &app.spawn_error
                && since.elapsed() >= spawn::ERROR_HOLD
            {
                app.spawn_error = None;
                app.dirty = true;
            }
            if app.exit.is_none() && app.dirty {
                app.draw(text, look, &qh);
                // Flush the draw at once: the poll below blocks until
                // the compositor, the repeat timer or the toggle speaks,
                // and a screenshot (which wakes nobody) must find the
                // committed frame, not an out-buffer. The loop's head
                // flush covers everything else.
                if self.queue.flush().is_err() {
                    return self.finish(app, 1, None);
                }
            }
            let Some(exit) = app.exit.take() else {
                // wait below
                let Some(guard) = self.queue.prepare_read() else {
                    continue;
                };
                // The repeat timer and the toggle socket wake the loop with
                // the connection: at idle (no key held, no toggle) the poll
                // waits on the connection alone, costing no wakeups.
                let wayland = guard.connection_fd();
                let repeat = app.seat.repeat_fd();
                let toggle = app.claim.as_fd();
                let repeat_fd = repeat.unwrap_or(wayland);
                let mut polls = [
                    rustix::event::PollFd::new(&wayland, rustix::event::PollFlags::IN),
                    rustix::event::PollFd::new(&repeat_fd, rustix::event::PollFlags::IN),
                    rustix::event::PollFd::new(&toggle, rustix::event::PollFlags::IN),
                ];
                let _ = rustix::event::poll(&mut polls, None);
                if !polls[0].revents().is_empty() {
                    let _ = guard.read();
                }
                // A due repeat or a toggle close with a quiet connection
                // drops the guard unread (the documented `read_after_poll`
                // shape): the next turn drains and draws.
                continue;
            };
            match exit {
                Exit::Launch(request) => {
                    match self.launch_request(&mut app, &request) {
                        LaunchDone::Launched => return self.finish(app, 0, None),
                        LaunchDone::Dismissed => return self.finish(app, 1, Some(Exit::Dismiss)),
                        // A failed spawn draws its error and stays
                        // open: no frecency, no exit.
                        LaunchDone::Failed(error) => {
                            app.spawn_error = Some((error, Instant::now()));
                            app.dirty = true;
                        }
                    }
                }
                Exit::Dismiss => return self.finish(app, 1, Some(Exit::Dismiss)),
                Exit::Failed => return self.finish(app, 1, Some(Exit::Failed)),
            }
        }
    }
}

/// What one pick's launch did: out, dismissed mid-token, or still
/// open with an error to draw.
enum LaunchDone {
    Launched,
    Dismissed,
    Failed(String),
}

impl Pump {
    /// Runs one pick to a launch: the token (200 ms, from the
    /// choosing serial), then the spawn. A toggle `close` arriving
    /// mid-token dismisses instead of launching: a slow bus never
    /// holds the panel past its bounds.
    fn launch_request(&mut self, app: &mut App, request: &LaunchRequest) -> LaunchDone {
        let token = match request.serial {
            Some(serial) => match self.request_token(app, request, serial) {
                TokenWait::Token(token) => Some(token),
                TokenWait::None => None,
                TokenWait::Dismissed => return LaunchDone::Dismissed,
            },
            // No serial to mint from (a repeat-tick Enter with no press
            // seen): the compositor would refuse the token, so the
            // launch goes without one.
            None => None,
        };
        let now = crate::rank::frecency::now_secs();
        match execute_launch(request, token.as_ref(), &app.launch, now) {
            Ok(_) => LaunchDone::Launched,
            Err(error) => LaunchDone::Failed(error),
        }
    }

    /// Mints the activation token for a pick: `set_serial` with the
    /// choosing key or button serial, `set_surface` with the panel's
    /// surface, `set_app_id` with the id, then `commit`. Waits for
    /// `done` up to [`spawn::TOKEN_TIMEOUT`], watching the toggle
    /// throughout.
    fn request_token(&mut self, app: &mut App, request: &LaunchRequest, serial: u32) -> TokenWait {
        let (Some(activation), Some(seat), Some(live)) = (
            app.activation.as_ref(),
            app.seat_proxy.as_ref(),
            app.live.as_ref(),
        ) else {
            return TokenWait::None;
        };
        let app_id = match &request.entry {
            Some(entry) => spawn::app_id_for(&entry.id).to_string(),
            None => String::from("scootlaunch"),
        };
        app.token_done = None;
        let token = activation.get_activation_token(&self.qh, ());
        token.set_serial(serial, seat);
        token.set_app_id(app_id);
        token.set_surface(live.surface());
        token.commit();
        if self.queue.flush().is_err() {
            token.destroy();
            return TokenWait::None;
        }
        let deadline = Instant::now() + spawn::TOKEN_TIMEOUT;
        loop {
            if self.queue.dispatch_pending(app).is_err() {
                token.destroy();
                return TokenWait::None;
            }
            if let Some(done) = app.token_done.take() {
                token.destroy();
                if done.is_empty() {
                    return TokenWait::None;
                }
                return TokenWait::Token(Token::new(done));
            }
            if app.claim.accept_close() {
                token.destroy();
                return TokenWait::Dismissed;
            }
            if Instant::now() >= deadline {
                token.destroy();
                return TokenWait::None;
            }
            if self.queue.flush().is_err() {
                token.destroy();
                return TokenWait::None;
            }
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            let wait = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            let timespec = rustix::time::Timespec {
                tv_sec: wait.as_secs() as _,
                tv_nsec: wait.subsec_nanos() as _,
            };
            let wayland = guard.connection_fd();
            let toggle = app.claim.as_fd();
            let mut polls = [
                rustix::event::PollFd::new(&wayland, rustix::event::PollFlags::IN),
                rustix::event::PollFd::new(&toggle, rustix::event::PollFlags::IN),
            ];
            let _ = rustix::event::poll(&mut polls, Some(&timespec));
            if !polls[0].revents().is_empty() {
                let _ = guard.read();
            }
            // A toggle close with a quiet connection drops the guard
            // unread: the next turn's `accept_close` collects it.
        }
    }
}

/// What the token wait ended with.
enum TokenWait {
    Token(Token),
    /// No token (no serial, no globals, a timeout, an empty `done`):
    /// the launch goes without one.
    None,
    /// A toggle `close` arrived mid-token: dismiss instead.
    Dismissed,
}
impl Pump {
    /// Tears the session down: the surface and its buffers go first (no
    /// surface held past close), then the socket file with the claim.
    /// `code` is the fallback; a taken exit decides instead.
    fn finish(self, mut app: App, code: i32, exit: Option<Exit>) -> i32 {
        let exit = app.exit.take().or(exit);
        let launch = LaunchCtx {
            state_home: app.launch.state_home.clone(),
            home: app.launch.home.clone(),
            display_set: app.launch.display_set,
        };
        app.destroy_live();
        self.queue.flush().ok();
        drop(app);
        drop(self);
        match exit {
            // Unreachable in practice (a pick needs keyboard focus,
            // which needs a mapped surface, which needs the drive):
            // with no event loop left there is no token to mint, so
            // the launch goes without one; a failure warns and exits 1.
            Some(Exit::Launch(request)) => {
                let now = crate::rank::frecency::now_secs();
                match execute_launch(&request, None, &launch, now) {
                    Ok(_) => 0,
                    Err(error) => {
                        warn(format_args!("scootlaunch: {error}"));
                        1
                    }
                }
            }
            Some(Exit::Dismiss) => 1,
            Some(Exit::Failed) | None => code,
        }
    }
}

// --- Wayland dispatch ---------------------------------------------------

macro_rules! empty_dispatch {
    ($object:ty) => {
        impl Dispatch<$object, ()> for App {
            fn event(
                _: &mut App,
                _: &$object,
                _: <$object as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<App>,
            ) {
            }
        }
    };
}

empty_dispatch!(wl_compositor::WlCompositor);
empty_dispatch!(wl_surface::WlSurface);
empty_dispatch!(wl_shm::WlShm);
empty_dispatch!(wl_shm_pool::WlShmPool);
empty_dispatch!(wl_output::WlOutput);
empty_dispatch!(WpViewport);
empty_dispatch!(WpViewporter);
empty_dispatch!(WpFractionalScaleManagerV1);
empty_dispatch!(WpFractionalScaleV1);
empty_dispatch!(ZwlrLayerShellV1);
empty_dispatch!(XdgActivationV1);

impl Dispatch<XdgActivationTokenV1, ()> for App {
    fn event(
        app: &mut App,
        _: &XdgActivationTokenV1,
        event: <XdgActivationTokenV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let ActivationTokenEvent::Done { token } = event {
            app.token_done = Some(token);
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        app: &mut App,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<App>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    app.compositor = Some(registry.bind(name, 6.min(version), qh, ()));
                }
                "wl_shm" => {
                    app.shm = Some(registry.bind(name, 1.min(version), qh, ()));
                }
                "wl_output" => {
                    let _output: wl_output::WlOutput = registry.bind(name, 4.min(version), qh, ());
                    app.outputs = app.outputs.saturating_add(1);
                }
                "wl_seat" => {
                    let Some(bound) = negotiate_version(version) else {
                        return;
                    };
                    app.seat_proxy =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, bound, qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    app.layer_shell = Some(registry.bind(name, 4.min(version), qh, ()));
                }
                "xdg_activation_v1" => {
                    app.activation = Some(registry.bind(name, 1.min(version), qh, ()));
                }
                "wp_viewporter" => {
                    app.viewporter = Some(registry.bind(name, 1.min(version), qh, ()));
                }
                "wp_fractional_scale_manager_v1" => {
                    app.fractional = Some(registry.bind(name, 1.min(version), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        app: &mut App,
        _: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<App>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event
            && let WEnum::Value(caps) = capabilities
        {
            use wl_seat::Capability;
            if caps.contains(Capability::Keyboard) && app.keyboard.is_none() {
                let Some(seat) = app.seat_proxy.as_ref() else {
                    return;
                };
                app.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if caps.contains(Capability::Pointer) && app.pointer.is_none() {
                let Some(seat) = app.seat_proxy.as_ref() else {
                    return;
                };
                app.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        app: &mut App,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match &event {
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                app.ctrl = mods_depressed & MOD_CTRL != 0;
            }
            wl_keyboard::Event::Enter { .. } | wl_keyboard::Event::Leave { .. } => {
                // A focus change retires the pressed tracking: releases
                // from before it must not read as presses after.
                app.pressed.clear();
            }
            _ => {}
        }
        // The choosing serial for an Enter pick: key presses only (a
        // focus serial is what the compositor refuses). Stored on every
        // press, spent synchronously by the pick in the same event.
        if let wl_keyboard::Event::Key { serial, state, .. } = &event
            && matches!(state, WEnum::Value(wl_keyboard::KeyState::Pressed))
        {
            app.key_serial = Some(*serial);
        }
        let focused_before = app.seat.has_focus();
        let mut text = [0u8; MIN_TEXT_BYTES];
        match dispatch_keyboard(&mut app.seat, event, &mut text) {
            Ok(Some(output)) => {
                // A keyboard leave while the pointer rests outside the
                // panel is an outside click: dismiss. A leave with the
                // pointer still over the panel (a locker) waits for
                // focus, drawing as before.
                if !app.seat.has_focus() && focused_before && app.pointer_outside() {
                    app.exit = Some(Exit::Dismiss);
                    return;
                }
                app.feed(output, &text);
            }
            Ok(None) => {
                if !app.seat.has_focus() && focused_before && app.pointer_outside() {
                    app.exit = Some(Exit::Dismiss);
                }
            }
            Err(error) => {
                warn(format_args!(
                    "scootlaunch: the keymap refuses to load: {error}"
                ));
                app.exit = Some(Exit::Failed);
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        app: &mut App,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                app.pointer_in = app
                    .live
                    .as_ref()
                    .is_some_and(|live| live.surface() == &surface);
                (app.pointer_x, app.pointer_y) = (surface_x as u32, surface_y as u32);
            }
            wl_pointer::Event::Leave { .. } => {
                app.pointer_in = false;
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                (app.pointer_x, app.pointer_y) = (surface_x as u32, surface_y as u32);
            }
            wl_pointer::Event::Button {
                button,
                state,
                serial,
                ..
            } => {
                // The choosing serial for a click pick, stored like the
                // key one above.
                if matches!(state, WEnum::Value(wl_pointer::ButtonState::Pressed)) {
                    app.button_serial = Some(serial);
                }
                if button != BTN_LEFT {
                    return;
                }
                if !matches!(state, WEnum::Value(wl_pointer::ButtonState::Pressed)) {
                    return;
                }
                // A click outside never arrives here (it goes to the
                // surface below); a click on a row launches it at once.
                if let Some(row) = app.hit_row() {
                    let row = app.ui.list().visible_range().start.saturating_add(row);
                    match app.ui.click(row) {
                        Outcome::Selected(_) => {
                            let serial = app.button_serial;
                            if let Some(entry) = app.ui.selected() {
                                app.exit = Some(Exit::Launch(Box::new(LaunchRequest {
                                    entry: Some(entry.clone()),
                                    action: app.ui.selected_action().cloned(),
                                    run_argv: None,
                                    serial,
                                })));
                            }
                        }
                        Outcome::Cancelled => app.exit = Some(Exit::Dismiss),
                        Outcome::Ignored | Outcome::Changed | Outcome::Run(_) => {}
                    }
                }
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                if !matches!(axis, WEnum::Value(wl_pointer::Axis::VerticalScroll)) {
                    return;
                }
                if value > 0.0 {
                    app.ui.scroll(false);
                    app.dirty = true;
                } else if value < 0.0 {
                    app.ui.scroll(true);
                    app.dirty = true;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(
        app: &mut App,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let wl_buffer::Event::Release = event {
            app.buffers.released(buffer);
            app.dirty = true;
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for App {
    fn event(
        app: &mut App,
        role: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<App>,
    ) {
        // Stale configures for a destroyed surface are dropped by the
        // role comparison, as the surfaces contract asks.
        let live = app.live.as_ref().is_some_and(|live| live.role() == role);
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                if !live {
                    return;
                }
                // Every configure is acked at receipt, exactly once: the
                // compositor treats a second ack as a protocol error.
                if let Some(live) = app.live.as_ref() {
                    live.ack(serial);
                }
                match app.demand.configure(serial, width, height) {
                    Effect::Ack(_) | Effect::None | Effect::Create => {}
                    Effect::Destroy | Effect::DestroyAndGiveUp | Effect::DestroyAndRetry => {
                        app.exit = Some(Exit::Dismiss);
                    }
                }
                app.configured_size = Some((width, height));
                app.dirty = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                if !live {
                    return;
                }
                match app.demand.closed() {
                    Effect::DestroyAndRetry if !app.closed_retried => {
                        app.closed_retried = true;
                        app.destroy_live();
                        app.configured_size = None;
                        app.show(qh);
                    }
                    _ => app.exit = Some(Exit::Dismiss),
                }
            }
            _ => {}
        }
    }
}
