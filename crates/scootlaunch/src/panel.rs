//! The panel: the overlay exclusive-keyboard surface with the entry
//! and the virtualized list, fed by the matcher.
//!
//! One process per open: the surface is created on open and destroyed on
//! close, so nothing is held at idle. The look layer is read at every
//! open; the single-instance toggle (see [`socket`](crate::socket))
//! decides whether an apps or run invocation opens or dismisses (dmenu
//! instances neither listen nor signal: each opens its own panel).
//! Picking an apps or run entry launches it ([`spawn`](crate::spawn):
//! the activation token, the environment, its own systemd scope, the
//! detached fallback); a dmenu pick prints and launches nothing.
//! Dismissing exits 1. A failed spawn draws its error for 2 s and the
//! panel stays open.
//!
//! Keys and mouse follow the design's table: typing filters, the
//! selection keys move, Enter launches (or prints) the selection,
//! Ctrl+Enter runs the typed text (apps/run) or prints it (dmenu, even
//! with `--only-match`), Escape dismisses. A click launches (or prints)
//! the row; the wheel scrolls. The token is minted from the choosing key or button
//! serial (never a focus serial, which the compositor refuses); past
//! its 200 ms bound the launch goes without one. A click outside the
//! panel dismisses it: with exclusive keyboard the compositor never
//! hands the keyboard to the desktop, so scoot sends the surface
//! `closed` instead (it recognises the `scootlaunch` namespace), and any
//! `closed` exits 1, the same as Escape. A keyboard leave with the
//! pointer outside the panel dismisses too (a compositor that moves
//! focus on a click), while a leave with the pointer still over the
//! panel (a locker taking the keyboard) keeps drawing and waits for
//! focus, as the design's failure table says.
//!
//! A look can add a shadow and motion ([`crate::stage`], [`crate::motion`]).
//! The surface is then the panel plus the shadow's room, equal on both
//! sides, and its input region the panel alone, so a press on the shadow is
//! a press outside. The apps and run panels compose on the stage; dmenu
//! never does. With a motion the panel scales and fades in, slides its
//! selection highlight, and fades out when it is dismissed (Escape, the
//! toggle) or has launched, with the keyboard released and the input
//! region emptied from the first fade frame. A `closed` from the compositor
//! and a keyboard leave with the pointer outside exit at once instead: the
//! protocol ignores everything a client does to a surface after `closed`.
//! The motion runs on frame callbacks, one outstanding at a time, and none
//! once the panel is at rest.
//!
//! Control tracking comes off the `wl_keyboard.modifiers` depressed
//! mask's Control bit (`0x4`, the XKB core bit): the seat reports a
//! release with the same keysym and empty text as a navigation press,
//! so the app tracks pressed keys itself (cleared on every focus change)
//! to tell a release from a press.

use std::os::unix::io::AsFd as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use scoot_rt::print::{print, warn};
use scootui::entry::{
    Geometry, paint_entry, paint_list, paint_list_over, paint_list_over_ink, paint_password,
};
use scootui::keyboard::{
    KeyOutput, KeyboardSeat, MIN_TEXT_BYTES, dispatch_keyboard, negotiate_version,
};
use scootui::output::Size;
use scootui::paint::Canvas;
use scootui::surfaces::{Buffers, Demand, Effect, Globals, LayerLive, LayerSpec, Plan};
use scootui::text::Text;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_region,
    wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
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
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor, KeyboardInteractivity,
};

use crate::cli::ColorOverrides;
use crate::dmenu::{DmenuUi, Outcome as DmenuOutcome, Print as DmenuPrint};
use crate::highlight::fill_round_rect;
use crate::index::{Action, Entry};
use crate::layout::{BORDER, Layout, Rect};
use crate::look::Look;
use crate::motion::Slide;
use crate::spawn::{self, Token};
use crate::stage::Effects;
use crate::ui::{Outcome, Ui};
use scootui::outputs::Frame;

/// How long the registry sweep and the first configure may take: a local
/// compositor answers in milliseconds, so this only bounds a wedge.
const PATIENCE: Duration = Duration::from_secs(10);
/// The padding between the border and the widgets, in device pixels.
const PAD: u32 = crate::layout::DMENU_PAD;
/// The gap between the entry line and the list, in device pixels.
const GAP: u32 = crate::layout::DMENU_GAP;
/// The entry's placeholder, in `dim`.
const PLACEHOLDER: &str = "Type to search";
/// evdev `BTN_LEFT`: wayland-client names no button codes.
const BTN_LEFT: u32 = 0x110;
/// The XKB core Control bit in the `wl_keyboard.modifiers` depressed
/// mask (`Shift` is `0x1`, `Lock` is `0x2`).
const MOD_CTRL: u32 = 0x4;

/// What the panel opens with: `--font PATH`, `--config PATH`, the
/// index's `--lenient`, and the mode (apps, run, or dmenu).
pub struct Options {
    pub font: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub lenient: bool,
    pub mode: Mode,
}

/// The panel's mode: the apps index, the `$PATH` scan, or the dmenu
/// lines. Each carries the per-invocation color overrides (above the
/// user's file and the look); the dmenu mode carries its input flags
/// and its lines besides.
pub enum Mode {
    Apps { colors: ColorOverrides },
    Run { colors: ColorOverrides },
    Dmenu(DmenuConfig),
}

/// What a dmenu open carries: the resolved prompt (`> ` when neither
/// `--prompt` nor `--prompt-only` names one), whether the list is
/// hidden (`--prompt-only`), the mask (`--password`), the match rules,
/// the dim hint (empty by default), the stdin lines, and the color
/// overrides. A pick prints (the line, or its 0-based stdin index with
/// `--index`) and launches nothing.
pub struct DmenuConfig {
    pub lines: Vec<String>,
    pub prompt: String,
    pub prompt_only: bool,
    pub password: bool,
    pub only_match: bool,
    pub index: bool,
    pub placeholder: String,
    pub colors: ColorOverrides,
}

/// Opens the panel and runs it to a launch, a print or a dismiss: 0
/// for a launched or printed pick, 1 for a dismiss, a runtime failure
/// or a bad file (the flags were fine, as the bar's), 2 for a bad
/// invocation.
pub fn open(opts: Options) -> i32 {
    // The toggle first (apps and run modes only: dmenu instances
    // neither listen nor signal — each opens its own panel). A live
    // panel is told to close, and this invocation exits without
    // needing a font, an index or a compositor connection of its own.
    let claim = match &opts.mode {
        Mode::Dmenu(_) => None,
        Mode::Apps { .. } | Mode::Run { .. } => {
            let socket_path = match crate::socket::live_socket_path() {
                Ok(path) => path,
                Err(error) => {
                    warn(format_args!("scootlaunch: {error}"));
                    return 1;
                }
            };
            match crate::socket::open(&socket_path) {
                Ok(crate::socket::Open::Signalled) => return 0,
                Ok(crate::socket::Open::Listen(claim)) => Some(claim),
                Err(error) => {
                    warn(format_args!("scootlaunch: {error}"));
                    return 1;
                }
            }
        }
    };
    // The user's own file: missing is the defaults; bad is a loud
    // refusal (there is no running instance a bad file could loop).
    let config_path = opts
        .config
        .clone()
        .unwrap_or_else(crate::config::default_path);
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
    // warning per fallback. The per-invocation colors override it.
    let (mut look, mut notes) = crate::look::load(None);
    // The user's `[effects]` win over the layer.
    crate::look::apply_user(&mut look, &config, &config_path, &mut notes);
    for note in &notes {
        warn(format_args!("scootlaunch: {note}"));
    }
    let colors = match &opts.mode {
        Mode::Apps { colors } | Mode::Run { colors } => colors,
        Mode::Dmenu(config) => &config.colors,
    };
    apply_colors(&mut look, colors);
    // The index and the frecency the apps list ranks with (run mode
    // scans `$PATH` instead, dmenu ranks its own lines with no
    // history): building the index anywhere else wastes a full scan.
    let env = crate::index::Env::from_env();
    let built = match &opts.mode {
        Mode::Apps { .. } => Some(crate::index::build(
            &env,
            &crate::index::BuildOpts {
                use_cache: true,
                lenient_exec: opts.lenient,
            },
        )),
        Mode::Dmenu(_) | Mode::Run { .. } => None,
    };
    if let Some(built) = &built {
        for note in &built.notes {
            warn(format_args!("scootlaunch: {note}"));
        }
    }
    let history = crate::rank::frecency::load(env.xdg_state_home.as_deref());
    let now = crate::rank::frecency::now_secs();
    let rows = config.rows as usize;
    // A prompt-only dmenu panel shows the entry alone: no gap, no list.
    let prompt_only = matches!(&opts.mode, Mode::Dmenu(config) if config.prompt_only);
    let list_rows = if prompt_only { 0 } else { config.rows };
    let prompt_color = colors.prompt.unwrap_or(look.theme.foreground);
    // The dmenu presentation and the empty-list text: borrowed out of
    // the mode before the model moves it.
    let dmenu = match &opts.mode {
        Mode::Dmenu(config) => Some(DmenuSpec {
            prompt: config.prompt.clone(),
            placeholder: config.placeholder.clone(),
            index: config.index,
            prompt_color,
            prompt_only: config.prompt_only,
        }),
        _ => None,
    };
    let empty_text: &'static str = match &opts.mode {
        Mode::Run { .. } => "No executables",
        _ => "No applications",
    };
    let model = match opts.mode {
        Mode::Apps { .. } => match built {
            Some(built) => BuiltModel::Apps(Ui::open(built.entries, history, now, rows)),
            None => {
                warn(format_args!("scootlaunch: no index to open on"));
                return 1;
            }
        },
        Mode::Run { .. } => {
            let names = crate::run::executable_names(&env.path_dirs);
            BuiltModel::Run(Ui::open(crate::run::to_entries(&names), history, now, rows))
        }
        Mode::Dmenu(config) => BuiltModel::Dmenu {
            lines: config.lines,
            only_match: config.only_match,
            prompt_only: config.prompt_only,
            password: config.password,
        },
    };
    let launch = LaunchCtx {
        state_home: env.xdg_state_home.clone(),
        home: std::env::var_os("HOME").map(PathBuf::from),
        display_set: std::env::var_os("DISPLAY").is_some(),
    };
    // The typeface: the flag, then the file, then the look's family, then
    // the well-known ones. The panel draws no bold, so no bold face loads.
    let given = opts.font.clone().or_else(|| config.font.map(PathBuf::from));
    let spec = scootui::font::Spec {
        given: given.as_deref(),
        family: &look.font_family,
        fallbacks: &[],
        bold: false,
    };
    let mut text = match scootui::font::text_with(&spec) {
        Ok((text, note)) => {
            if let Some(note) = note {
                warn(format_args!("scootlaunch: {note}"));
            }
            text
        }
        Err(error) => {
            warn(format_args!("scootlaunch: {error}"));
            return 1;
        }
    };
    let em = look.font_size_px.clamp(1.0, 256.0);
    let layout = if dmenu.is_some() {
        Layout::dmenu(em)
    } else {
        Layout::new(em, look.radius, 1.0)
    };
    let width = (config.width_em as f32 * em) as u32;
    // A prompt-only dmenu panel shows the entry alone: no gap, no list
    // (the `list_rows` above already resolved this for the same rows).
    let height = layout.height(list_rows);
    let icon_theme = look.icon_theme.clone();
    let icon_bases = crate::icons::theme_bases();
    // The shadow and the motion are the apps and run panel's: dmenu keeps
    // its fixed painter, and a script waiting on its pick never waits on a
    // fade.
    let fx = if dmenu.is_some() {
        Effects::off()
    } else {
        Effects::from_look(&look)
    };
    run_panel(
        claim,
        model,
        &mut text,
        &look,
        PanelSpec {
            width,
            height,
            layout,
            rows: list_rows as usize,
            launch,
            dmenu,
            empty_text,
            icon_theme,
            icon_bases,
            fx,
        },
    )
}

/// Applies the per-invocation `rrggbbaa` overrides above the look
/// layer: `background` replaces the panel fill (its alpha becomes the
/// panel's opacity), the rest replace their roles. Only the
/// background may be translucent (the CLI refuses any other alpha),
/// since the panel draws those roles solid.
///
/// `match` lands on `hover`: plain rows draw it directly, and the
/// selected row draws it whenever it contrasts with the fill (see
/// [`Theme::match_on_selection`](scootui::theme::Theme::match_on_selection)),
/// so the flag reads the same on both rows wherever it stays legible.
fn apply_colors(look: &mut Look, colors: &ColorOverrides) {
    if let Some((color, alpha)) = colors.background {
        look.surface = color;
        look.opacity = f32::from(alpha) / 255.0;
    }
    if let Some(color) = colors.text {
        look.theme.foreground = color;
    }
    if let Some(color) = colors.border {
        look.focus = color;
    }
    if let Some(color) = colors.selection {
        look.theme.accent = color;
    }
    if let Some(color) = colors.selection_text {
        look.theme.background = color;
    }
    if let Some(color) = colors.r#match {
        look.theme.hover = color;
    }
}

/// What a launch resolves against: read once at open, so the spawn
/// never touches the process environment (which the tests sandbox).
struct LaunchCtx {
    state_home: Option<PathBuf>,
    home: Option<PathBuf>,
    display_set: bool,
}

/// What the Wayland session opens with: the panel's size, the
/// launch context, the dmenu presentation (if any) and the empty-list
/// text, bundled so the runner takes few arguments.
struct PanelSpec {
    width: u32,
    height: u32,
    layout: Layout,
    rows: usize,
    launch: LaunchCtx,
    dmenu: Option<DmenuSpec>,
    empty_text: &'static str,
    icon_theme: String,
    icon_bases: Vec<PathBuf>,
    fx: Effects,
}

/// The dmenu presentation: resolved in [`open`] before the model moves
/// the config's lines.
struct DmenuSpec {
    /// The prompt before the entry (`> ` by default).
    prompt: String,
    /// The dim hint while the entry is empty (empty by default).
    placeholder: String,
    /// Print the pick's 0-based stdin index instead of its line.
    index: bool,
    /// The prompt ink (the `--prompt-color` override, else foreground).
    prompt_color: scootui::color::Color,
    /// An input field with no list: stdin was not read.
    prompt_only: bool,
}

/// The built state machine, before the Wayland session assembles it:
/// apps and run share [`Ui`] (over the desktop index, or over `$PATH`
/// names); dmenu carries its lines for [`run_panel`] to build the
/// [`DmenuUi`].
enum BuiltModel {
    Apps(Ui),
    Run(Ui),
    Dmenu {
        lines: Vec<String>,
        only_match: bool,
        prompt_only: bool,
        password: bool,
    },
}

/// The panel's state machine: apps and run picks launch, dmenu picks
/// print.
enum Model {
    Apps(Ui),
    Run(Ui),
    Dmenu(DmenuUi),
}

/// Runs the Wayland session to a pick, a print or a dismiss.
fn run_panel(
    claim: Option<crate::socket::Claim>,
    built: BuiltModel,
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
    let model = match built {
        BuiltModel::Apps(ui) => Model::Apps(ui),
        BuiltModel::Run(ui) => Model::Run(ui),
        BuiltModel::Dmenu {
            lines,
            only_match,
            prompt_only,
            password,
        } => Model::Dmenu(match password {
            true => DmenuUi::open_secret(lines, spec.rows, only_match, prompt_only),
            false => DmenuUi::open(lines, spec.rows, only_match, prompt_only),
        }),
    };
    let mut app = App::new(
        claim,
        model,
        spec.width,
        spec.height,
        spec.layout,
        spec.launch,
        spec.dmenu,
        spec.empty_text,
        spec.icon_theme,
        spec.icon_bases,
        spec.fx,
    );
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
    // A null output: the compositor picks the output under the pointer
    // (its primary when the pointer is on none). The panel carries no
    // output logic of its own.
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
    /// A dmenu pick: printed on the way out (the line, its index, or
    /// the typed text), never launched.
    Print(String),
    /// Dismissed by the user: Escape or the toggle. The panel fades out
    /// when the look has motion.
    Dismiss,
    /// The compositor took the panel away (an outside click, an unplugged
    /// output) or focus left it with the pointer outside: it is gone now,
    /// so there is nothing to fade and no surface to commit to.
    Closed,
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
    /// The toggle's listen claim (`None` in dmenu mode, which neither
    /// listens nor signals). Removing the file happens on drop.
    claim: Option<crate::socket::Claim>,
    model: Model,
    width: u32,
    height: u32,
    layout: Layout,
    /// The apps panel's geometry as last drawn (device pixels) and the
    /// scale it was drawn at, so a click lands on the row the pixels
    /// show at a fractional scale; `None` before the first draw and in
    /// dmenu mode (drawn at scale 1 from `layout`).
    drawn: Option<(Layout, f64)>,
    launch: LaunchCtx,
    dmenu: Option<DmenuSpec>,
    empty_text: &'static str,
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
    /// The icon theme and bases the visible rows resolve through, read
    /// once at open. The cache lives for the open only: only visible
    /// rows load, at most one file each, bounded by the module.
    icon_theme: String,
    icon_bases: Vec<PathBuf>,
    icons: crate::icons::Cache,
    /// The look's shadow room, stage and motion; all off for a plain panel.
    fx: Effects,
    /// Where the panel's top-left corner sits in the surface, in logical
    /// pixels: the shadow's room as last drawn. Pointer positions arrive in
    /// surface coordinates, so every hit test subtracts it.
    origin: (u32, u32),
    /// A `wl_surface.frame` callback is outstanding: at most one, so a
    /// slow compositor is never asked for a queue of frames.
    frame_requested: bool,
    /// The callback fired: the next frame of a motion may be drawn.
    frame_due: bool,
    /// The input region last sent.
    region: Option<Region>,
    /// The panel is fading out: the keyboard is released and the input
    /// region is empty, so it takes nothing while it plays.
    fading: bool,
}

/// An input region the panel set: the whole surface (the default), the
/// panel's rectangle inside its shadow's room, or nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    Surface,
    Panel(Rect),
    Nothing,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    fn new(
        claim: Option<crate::socket::Claim>,
        model: Model,
        width: u32,
        height: u32,
        layout: Layout,
        launch: LaunchCtx,
        dmenu: Option<DmenuSpec>,
        empty_text: &'static str,
        icon_theme: String,
        icon_bases: Vec<PathBuf>,
        fx: Effects,
    ) -> Self {
        let origin = fx.room;
        Self {
            claim,
            model,
            width,
            height,
            layout,
            drawn: None,
            launch,
            dmenu,
            empty_text,
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
            icon_theme,
            icon_bases,
            icons: crate::icons::Cache::default(),
            fx,
            origin,
            frame_requested: false,
            frame_due: false,
            region: None,
            fading: false,
        }
    }

    /// The surface's size in logical pixels: the panel plus its shadow's
    /// room on every side.
    fn surface_size(&self) -> (u32, u32) {
        let (rx, ry) = self.fx.room;
        (
            self.width.saturating_add(rx.saturating_mul(2)),
            self.height.saturating_add(ry.saturating_mul(2)),
        )
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
        let (width, height) = self.surface_size();
        let spec = LayerSpec {
            namespace: "scootlaunch",
            exclusive_keyboard: true,
            anchor: Anchor::empty(),
            width,
            height,
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
        self.panel_point().is_none()
    }

    /// The pointer in the panel's own coordinates (logical pixels from its
    /// top-left corner), or `None` when it is off the panel: outside the
    /// surface, or over the shadow's room, which is not the panel.
    fn panel_point(&self) -> Option<(u32, u32)> {
        if !self.pointer_in {
            return None;
        }
        let x = self.pointer_x.checked_sub(self.origin.0)?;
        let y = self.pointer_y.checked_sub(self.origin.1)?;
        (x < self.width && y < self.height).then_some((x, y))
    }

    /// Whether the toggle's `close` arrived (apps/run only; dmenu
    /// holds no toggle): a second invocation dismisses this one.
    fn accept_close(&self) -> bool {
        self.claim
            .as_ref()
            .is_some_and(|claim| claim.accept_close())
    }

    /// The shown list row under the pointer, if the pointer is over one.
    fn hit_row(&self) -> Option<usize> {
        let (_, pointer_y) = self.panel_point()?;
        let list = match &self.model {
            Model::Apps(ui) | Model::Run(ui) => ui.list(),
            Model::Dmenu(dui) => dui.list(),
        };
        let (layout, factor) = self.drawn.unwrap_or((self.layout, 1.0));
        // The pointer is in surface (logical) pixels, the drawn rows in
        // device pixels.
        let y = (f64::from(pointer_y) * factor) as u32;
        let list_y = layout.list_y();
        if y < list_y || layout.row_h == 0 {
            return None;
        }
        let row = (y - list_y) / layout.row_h;
        let row = row as usize;
        (row < list.visible_range().len()).then_some(row)
    }

    /// Draws the panel into a pooled buffer and commits it.
    fn draw(&mut self, text: &mut Text, look: &Look, qh: &QueueHandle<Self>) {
        let Some(size) = self.configured_size else {
            return;
        };
        // A zero axis is the client's to choose: the size it asked for.
        let (wanted_w, wanted_h) = self.surface_size();
        let size = Size {
            width: if size.0 == 0 { wanted_w } else { size.0 },
            height: if size.1 == 0 { wanted_h } else { size.1 },
        };
        // A frame of a motion changes no content, but it is a draw: the
        // plan only draws what it is told has changed.
        match self.demand.plan(Some(size), self.dirty || self.frame_due) {
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
        let error = self.spawn_error.as_ref().map(|(text, _)| text.as_str());
        let dmenu = self.dmenu.as_ref();
        let now = Instant::now();
        let mut region = Region::Surface;
        match &self.model {
            Model::Apps(ui) | Model::Run(ui) => {
                let layout = device_layout(look, frame.scale);
                let factor = frame.scale.factor().max(MIN_FACTOR);
                self.drawn = Some((layout, factor));
                let view = PanelView {
                    ui,
                    look,
                    width: self.width,
                    height: self.height,
                    error,
                    empty_text: self.empty_text,
                    scale: frame.scale,
                    icon_theme: &self.icon_theme,
                    icon_bases: &self.icon_bases,
                };
                if self.fx.stage.is_some() {
                    let origin = staged::paint_staged(
                        &mut self.fx,
                        &view,
                        text,
                        &mut self.icons,
                        slot.pixels_mut(),
                        dims,
                        frame.size,
                        self.dirty,
                        now,
                    );
                    self.origin = origin;
                    region = if self.fading {
                        Region::Nothing
                    } else if origin == (0, 0) {
                        Region::Surface
                    } else {
                        Region::Panel(Rect {
                            x: origin.0,
                            y: origin.1,
                            width: self.width,
                            height: self.height,
                        })
                    };
                } else {
                    paint_panel(slot.pixels_mut(), dims, &view, text, &mut self.icons, None);
                }
            }
            Model::Dmenu(dui) => {
                let Some(dmenu) = dmenu else {
                    return;
                };
                paint_dmenu(
                    slot.pixels_mut(),
                    dims,
                    dui,
                    text,
                    look,
                    self.width,
                    self.height,
                    self.layout.row_h,
                    dmenu,
                );
            }
        }
        let Some(live) = self.live.as_mut() else {
            return;
        };
        sync_region(
            &mut self.region,
            self.compositor.as_ref(),
            live.surface(),
            region,
            qh,
        );
        // Another frame while something moves: one callback at a time, and
        // none once it has come to rest, so a still panel asks for nothing.
        if self.fx.motion.animating(now) && !self.frame_requested {
            live.surface().frame(qh, ());
            self.frame_requested = true;
        }
        self.frame_due = false;
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
        // A fading panel takes nothing: a key queued before the keyboard
        // was released must not launch what the user already dismissed.
        if self.fading {
            return;
        }
        match &mut self.model {
            Model::Apps(ui) => match ui.key(keysym, self.ctrl, decoded) {
                Outcome::Ignored => {}
                Outcome::Changed => self.dirty = true,
                Outcome::Selected(_) => {
                    let serial = self.key_serial;
                    if let Some(entry) = ui.selected() {
                        self.exit = Some(Exit::Launch(Box::new(LaunchRequest {
                            entry: Some(entry.clone()),
                            action: ui.selected_action().cloned(),
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
            },
            // A run pick launches its basename through `PATH`
            // (frecency keys it `run:<basename>` inside the spawn); the
            // typed-text path is the apps one.
            Model::Run(ui) => match ui.key(keysym, self.ctrl, decoded) {
                Outcome::Ignored => {}
                Outcome::Changed => self.dirty = true,
                Outcome::Selected(_) => {
                    let serial = self.key_serial;
                    if let Some(entry) = ui.selected() {
                        self.exit = Some(Exit::Launch(Box::new(LaunchRequest {
                            entry: None,
                            action: None,
                            run_argv: Some(vec![entry.name.clone()]),
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
            },
            // A dmenu pick prints and launches nothing: the line (or
            // its 0-based stdin index with `--index`), or the typed
            // text. One write happens in `finish`, on the way out.
            Model::Dmenu(dui) => match dui.key(keysym, self.ctrl, decoded) {
                DmenuOutcome::Ignored => {}
                DmenuOutcome::Changed => self.dirty = true,
                DmenuOutcome::Print(print) => {
                    let index = self.dmenu.as_ref().is_some_and(|spec| spec.index);
                    let output = match print {
                        DmenuPrint::Text(text) => text,
                        DmenuPrint::Line(line) if index => format!("{line}"),
                        DmenuPrint::Line(line) => dui.line(line).unwrap_or("").to_string(),
                    };
                    self.exit = Some(Exit::Print(format!("{output}\n")));
                }
                DmenuOutcome::Cancelled => self.exit = Some(Exit::Dismiss),
            },
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

/// What one apps panel draw reads, apart from the pixels and the caches.
struct PanelView<'a> {
    ui: &'a Ui,
    look: &'a Look,
    /// The panel's logical size.
    width: u32,
    height: u32,
    /// A fresh spawn failure, drawn instead of the list.
    error: Option<&'a str>,
    empty_text: &'a str,
    scale: scootui::density::Scale,
    icon_theme: &'a str,
    icon_bases: &'a [PathBuf],
}

/// `logical` pixels at output scale `factor`, at least one for a nonzero
/// length.
fn scale_len(logical: u32, factor: f64) -> u32 {
    if logical == 0 {
        0
    } else {
        ((f64::from(logical) * factor)
            .round()
            .max(1.0)
            .min(f64::from(u32::MAX))) as u32
    }
}

/// Paints the panel: the rounded chrome (border ring in `focus`, fill in
/// `surface` at `opacity`), the entry line, and the list's visible rows.
/// `dims` is the buffer size; the logical `width`/`height` size the panel
/// when the compositor configures larger. The search field sits at the
/// top, the list below a gap; every measurement comes from [`Layout`].
/// A fresh spawn failure draws instead of the list.
///
/// The selected row's highlight is drawn first, across the whole inner
/// box, then the row text over it, then each visible row's icon (or its
/// tile) inside the highlight, one `lead` from its left edge. The search
/// line carries no icon. Only visible rows resolve or decode (one file
/// each, bounded by [`crate::icons`]); the keystroke path never touches
/// the cache.
/// `slide` places the highlight between rows during a motion; `None` draws
/// it on the selected row.
fn paint_panel(
    pixels: &mut [u8],
    dims: (u32, u32),
    view: &PanelView<'_>,
    text: &mut Text,
    icons: &mut crate::icons::Cache,
    slide: Option<Slide>,
) {
    let PanelView {
        ui,
        look,
        width,
        height,
        error,
        empty_text,
        scale,
        icon_theme,
        icon_bases,
    } = *view;
    let (buffer_w, buffer_h) = dims;
    let factor = scale.factor().max(MIN_FACTOR);
    let width_d = scale_len(width, factor).min(buffer_w).max(1);
    let height_d = scale_len(height, factor).min(buffer_h).max(1);
    let layout = device_layout(look, scale);
    let (em, radius_d) = (layout.em, scale_len(look.radius, factor));
    chrome_scaled(
        pixels,
        buffer_w,
        width_d,
        height_d,
        layout.border,
        radius_d,
        look,
        look.fill_stops(),
    );
    let Some(mut canvas) = Canvas::new(pixels, buffer_w, buffer_h) else {
        return;
    };
    let first_row = layout.row(width_d, 0);
    let list_geo = layout.list_geometry(width_d);
    // The search field: the entry line alone, with no row icon (the
    // selection's icon is on its own row).
    paint_entry(
        &mut canvas,
        text,
        &look.theme,
        ui.entry(),
        &layout.field_geometry(width_d),
        true,
        PLACEHOLDER,
    );
    // An error or an empty list draws one dim line where the first row
    // would be, aligned with the row text.
    let note = match error {
        Some(error) => Some((error, look.theme.urgent)),
        None if ui.list().count() == 0 => Some((empty_text, look.theme.dim)),
        None => None,
    };
    if let Some((note, color)) = note {
        let metrics = text.metrics(em);
        text.draw(
            &mut canvas,
            None,
            note,
            em,
            i64::from(list_geo.text_x()),
            i64::from(first_row.y) + metrics.baseline(layout.row_h),
            color,
            list_geo.clip(),
        );
        return;
    }
    // The selection highlight first, full row width and the panel's
    // corner radius, so the icon and text draw on top of it. A look with
    // motion slides it between rows (and moves the selected-row ink with
    // it); without, it sits on the selected row.
    let selected = ui.list().selected();
    let range = ui.list().visible_range();
    match slide {
        Some(slide) => {
            let rect = Rect {
                y: layout.list_y().saturating_add(slide.offset),
                ..layout.row(width_d, 0)
            };
            fill_round_rect(&mut canvas, rect, layout.radius, look.theme.accent);
            paint_list_over_ink(
                &mut canvas,
                text,
                &look.theme,
                ui.list(),
                &list_geo,
                &|row| ui.row_text(row),
                Some(range.start.saturating_add(slide.ink_slot)),
            );
        }
        None => {
            for (slot, row) in range.clone().enumerate() {
                if selected == Some(row) {
                    let rect = layout.row(width_d, slot as u32);
                    fill_round_rect(&mut canvas, rect, layout.radius, look.theme.accent);
                }
            }
            paint_list_over(
                &mut canvas,
                text,
                &look.theme,
                ui.list(),
                &list_geo,
                &|row| ui.row_text(row),
            );
        }
    }
    // The visible rows' icons, after the text (so a slow filesystem
    // never moves the text): one bounded load each, cached per open. A
    // letter tile inverts on the row the highlight covers.
    let inked = match slide {
        Some(slide) => Some(range.start.saturating_add(slide.ink_slot)),
        None => selected,
    };
    for (slot, row) in range.enumerate() {
        let rect = layout.icon(layout.row(width_d, slot as u32));
        draw_icon(
            &mut canvas,
            text,
            look,
            icons,
            icon_theme,
            icon_bases,
            ui.row_icon(row),
            ui.row_label(row),
            rect,
            inked == Some(row),
        );
    }
}

/// Sends `want` as the surface's input region when it is not what was last
/// sent. A panel inside a shadow's room takes pointer input on the panel
/// alone: a press on the shadow lands on the desktop, which the compositor
/// reads as a press outside the launcher (its hit test follows the input
/// region), so the shadow never counts as the panel.
fn sync_region(
    sent: &mut Option<Region>,
    compositor: Option<&wl_compositor::WlCompositor>,
    surface: &wl_surface::WlSurface,
    want: Region,
    qh: &QueueHandle<App>,
) {
    if *sent == Some(want) || (sent.is_none() && want == Region::Surface) {
        *sent = Some(want);
        return;
    }
    let Some(compositor) = compositor else {
        return;
    };
    match want {
        // The default region: the whole surface.
        Region::Surface => surface.set_input_region(None),
        Region::Nothing | Region::Panel(_) => {
            let region = compositor.create_region(qh, ());
            if let Region::Panel(rect) = want {
                let side = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
                region.add(
                    side(rect.x),
                    side(rect.y),
                    side(rect.width),
                    side(rect.height),
                );
            }
            surface.set_input_region(Some(&region));
            region.destroy();
        }
    }
    *sent = Some(want);
}

/// The smallest output scale the draw honors.
const MIN_FACTOR: f64 = 0.25;

/// The apps panel's geometry in device pixels at `scale`: all of it
/// derives from the device em and radius, so the shape is the same at
/// every font size and scale. The one place the draw and the pointer's
/// row hit-test take it from.
fn device_layout(look: &Look, scale: scootui::density::Scale) -> Layout {
    let factor = scale.factor().max(MIN_FACTOR);
    let em = ((f64::from(look.font_size_px) * factor) as f32).clamp(1.0, 512.0);
    let radius = (f64::from(look.radius) * factor)
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32;
    Layout::new(em, radius, factor)
}

/// Draws one row's icon into `rect` (a square, `rect.width` device pixels
/// a side): the decoded image blended over the row, or a rounded letter
/// tile in `accent` with the letter in `background` (swapped on the
/// `selected` row, whose fill is the accent already). Never panics: a
/// short buffer or a missing glyph draws nothing.
#[allow(clippy::too_many_arguments)]
fn draw_icon(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    look: &Look,
    icons: &mut crate::icons::Cache,
    theme: &str,
    bases: &[PathBuf],
    icon: Option<&str>,
    label: &str,
    rect: Rect,
    selected: bool,
) {
    let (x, y, side) = (rect.x, rect.y, rect.width);
    if side == 0 {
        return;
    }
    let loaded = icons.get(theme, bases, icon, label, side);
    match loaded {
        crate::icons::Icon::Image(decoded) => {
            let width = canvas.width();
            let clip = scootui::paint::Span { x: 0, width };
            let width = decoded.width.min(side);
            let height = decoded.height.min(side);
            for row in 0..height {
                let Some(line) = decoded.pixels.get(
                    row as usize * decoded.width as usize * 4
                        ..row as usize * decoded.width as usize * 4 + width as usize * 4,
                ) else {
                    continue;
                };
                for (col, pixel) in line.as_chunks::<4>().0.iter().enumerate() {
                    let pixel = [pixel[0], pixel[1], pixel[2], pixel[3]];
                    canvas.blend_premultiplied(
                        x as i64 + col as i64,
                        y as i64 + row as i64,
                        pixel,
                        clip,
                    );
                }
            }
        }
        crate::icons::Icon::Tile(letter) => {
            // The selected row's fill is the accent already: invert, so
            // the tile stays a visible square there.
            let (fill, ink) = if selected {
                (look.theme.background, look.theme.accent)
            } else {
                (look.theme.accent, look.theme.background)
            };
            fill_round_rect(canvas, rect, side / 5, fill);
            let mut letter_text = [0u8; 4];
            let letter_str = letter.encode_utf8(&mut letter_text);
            let em = tile_em(side);
            let metrics = text.metrics(em);
            let baseline = y as i64 + metrics.baseline(side);
            let ink_w = text.measure(None, letter_str, em);
            let at = x.saturating_add(side.saturating_sub(ink_w) / 2);
            text.draw(
                canvas,
                None,
                letter_str,
                em,
                at as i64,
                baseline,
                ink,
                scootui::paint::Span { x, width: side },
            );
        }
    }
}

/// The tile letter's em: the icon side, so the glyph fills the tile.
fn tile_em(side: u32) -> f32 {
    (side as f32 * 0.72).clamp(1.0, 512.0)
}

/// Paints the dmenu panel: the same chrome, the prompt at the entry's
/// text start (in the prompt color), the entry beside it, and the
/// list's visible rows below — or nothing below in prompt-only mode.
/// In password mode the entry draws masked (`*` per character, never
/// the secret); an error never draws here (a dmenu pick prints, and
/// printing cannot fail into the panel). Anything drawn beside the
/// entry starts at [`Geometry::text_x`], aligned with the widgets.
#[allow(clippy::too_many_arguments)]
fn paint_dmenu(
    pixels: &mut [u8],
    dims: (u32, u32),
    dui: &DmenuUi,
    text: &mut Text,
    look: &Look,
    width: u32,
    height: u32,
    row_h: u32,
    spec: &DmenuSpec,
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
    let metrics = text.metrics(em);
    let baseline = i64::from(inner) + metrics.baseline(row_h);
    // The prompt first, at the text start; the entry shifts right by
    // its measured width, so prompt and query never overlap.
    let prompt_w = if spec.prompt.is_empty() {
        0
    } else {
        text.draw(
            &mut canvas,
            None,
            spec.prompt.as_str(),
            em,
            i64::from(entry_geo.text_x()),
            baseline,
            spec.prompt_color,
            entry_geo.clip(),
        );
        text.measure(None, spec.prompt.as_str(), em)
    };
    let entry_geo = Geometry {
        x: entry_geo.x.saturating_add(prompt_w),
        width: entry_geo.width.saturating_sub(prompt_w),
        ..entry_geo
    };
    if let Some(secret) = dui.secret_entry() {
        paint_password(&mut canvas, text, &look.theme, secret, &entry_geo, true);
    } else if let Some(entry) = dui.plain_entry() {
        paint_entry(
            &mut canvas,
            text,
            &look.theme,
            entry,
            &entry_geo,
            true,
            spec.placeholder.as_str(),
        );
    }
    if spec.prompt_only || dui.list().count() == 0 {
        // No list: a prompt-only field, or nothing (yet) matching.
        // Fuzzel shows just the input either way.
        return;
    }
    let list_geo = Geometry {
        x: inner,
        y: inner.saturating_add(row_h).saturating_add(GAP),
        width: width.saturating_sub(inner.saturating_mul(2)),
        row_height: row_h,
        em,
    };
    paint_list(
        &mut canvas,
        text,
        &look.theme,
        dui.list(),
        &list_geo,
        &|row| dui.row_text(row),
        false,
    );
}

/// The chrome: a rounded rect in `focus` with the `surface` fill inset by
/// [`BORDER`], premultiplied at `opacity`. Outside the rounded rect the
/// buffer stays transparent, so the corners show the desktop. One pass
/// over the panel's pixels; redraws happen on state changes, never per
/// frame.
fn chrome(pixels: &mut [u8], buffer_w: u32, width: u32, height: u32, look: &Look) {
    chrome_scaled(
        pixels,
        buffer_w,
        width,
        height,
        BORDER,
        look.radius,
        look,
        (look.surface, look.surface),
    );
}

/// The chrome at a device border width: the fractional draw scales the
/// border with the rest, so a 1.5 output keeps a 3-device-pixel ring
/// instead of a stretched 2-pixel one. The fill runs from `stops.0` at the
/// top row to `stops.1` at the bottom; equal stops are the flat `surface`
/// fill, written as it always was.
#[allow(clippy::too_many_arguments)]
fn chrome_scaled(
    pixels: &mut [u8],
    buffer_w: u32,
    width: u32,
    height: u32,
    border: u32,
    radius: u32,
    look: &Look,
    stops: (scootui::color::Color, scootui::color::Color),
) {
    let radius = radius
        .min(width.saturating_div(2))
        .min(height.saturating_div(2)) as f32;
    let alpha = (look.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let flat = stops.0 == stops.1;
    let surface_flat = stops.0.argb8888(alpha);
    let focus = look.focus.argb8888(255);
    let stride = buffer_w as usize * 4;
    let half_w = width as f32 / 2.0;
    let half_h = height as f32 / 2.0;
    for y in 0..height {
        let Some(row) = pixels.chunks_exact_mut(stride).nth(y as usize) else {
            break;
        };
        let surface = if flat {
            surface_flat
        } else {
            scootui::paint::row_color(stops.0, stops.1, y, height).argb8888(alpha)
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
            let pixel = if distance > -(border as f32) {
                focus
            } else {
                surface
            };
            let Some(chunk) = row.as_chunks_mut::<4>().0.get_mut(x as usize) else {
                continue;
            };
            chunk.copy_from_slice(&pixel.to_le_bytes());
        }
    }
}

/// The queue pump: flush, dispatch, repeat ticks, the toggle socket
/// (apps/run only), and draws, until a pick, a print or a dismiss.
struct Pump {
    queue: wayland_client::EventQueue<App>,
    qh: QueueHandle<App>,
}

impl Pump {
    fn handle(&self) -> &QueueHandle<App> {
        &self.qh
    }

    /// Runs dispatches until `done`, or the timeout. `false` on timeout.
    /// The toggle socket is watched throughout (apps/run): a `close`
    /// arriving mid-open dismisses.
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
            if app.accept_close() {
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
            match app.claim.as_ref() {
                Some(claim) => {
                    let toggle = claim.as_fd();
                    let mut polls = [
                        rustix::event::PollFd::new(&wayland, rustix::event::PollFlags::IN),
                        rustix::event::PollFd::new(&toggle, rustix::event::PollFlags::IN),
                    ];
                    let _ = rustix::event::poll(&mut polls, Some(&timespec));
                    if !polls[0].revents().is_empty() {
                        let _ = guard.read();
                    }
                }
                // Dmenu mode holds no toggle: wait on the connection
                // alone.
                None => {
                    let mut polls = [rustix::event::PollFd::new(
                        &wayland,
                        rustix::event::PollFlags::IN,
                    )];
                    let _ = rustix::event::poll(&mut polls, Some(&timespec));
                    if !polls[0].revents().is_empty() {
                        let _ = guard.read();
                    }
                }
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
            if app.accept_close() {
                self.fade_out(&mut app, text, look);
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
            if app.exit.is_none() && (app.dirty || app.frame_due) {
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
                // Dmenu mode holds no toggle: two fds, not three.
                let wayland = guard.connection_fd();
                let repeat = app.seat.repeat_fd();
                let repeat_fd = repeat.unwrap_or(wayland);
                match app.claim.as_ref() {
                    Some(claim) => {
                        let toggle = claim.as_fd();
                        let mut polls = [
                            rustix::event::PollFd::new(&wayland, rustix::event::PollFlags::IN),
                            rustix::event::PollFd::new(&repeat_fd, rustix::event::PollFlags::IN),
                            rustix::event::PollFd::new(&toggle, rustix::event::PollFlags::IN),
                        ];
                        let _ = rustix::event::poll(&mut polls, None);
                        if !polls[0].revents().is_empty() {
                            let _ = guard.read();
                        }
                    }
                    None => {
                        let mut polls = [
                            rustix::event::PollFd::new(&wayland, rustix::event::PollFlags::IN),
                            rustix::event::PollFd::new(&repeat_fd, rustix::event::PollFlags::IN),
                        ];
                        let _ = rustix::event::poll(&mut polls, None);
                        if !polls[0].revents().is_empty() {
                            let _ = guard.read();
                        }
                    }
                }
                // A due repeat or a toggle close with a quiet connection
                // drops the guard unread (the documented `read_after_poll`
                // shape): the next turn drains and draws.
                continue;
            };
            match exit {
                Exit::Launch(request) => {
                    match self.launch_request(&mut app, &request) {
                        // The launch has run: the panel fades while the
                        // app opens, with the keyboard already released.
                        LaunchDone::Launched => {
                            self.fade_out(&mut app, text, look);
                            // Whatever closed the fading panel, the pick
                            // launched: the exit code is the launch's.
                            app.exit = None;
                            return self.finish(app, 0, None);
                        }
                        LaunchDone::Dismissed => {
                            self.fade_out(&mut app, text, look);
                            return self.finish(app, 1, Some(Exit::Dismiss));
                        }
                        // A failed spawn draws its error and stays
                        // open: no frecency, no exit.
                        LaunchDone::Failed(error) => {
                            app.spawn_error = Some((error, Instant::now()));
                            app.dirty = true;
                        }
                    }
                }
                Exit::Dismiss => {
                    self.fade_out(&mut app, text, look);
                    return self.finish(app, 1, Some(Exit::Dismiss));
                }
                Exit::Closed => return self.finish(app, 1, Some(Exit::Closed)),
                Exit::Failed => return self.finish(app, 1, Some(Exit::Failed)),
                Exit::Print(text) => return self.finish(app, 0, Some(Exit::Print(text))),
            }
        }
    }
}

/// How long past the fade's own length it may take before the panel goes
/// regardless: frame callbacks stop when the surface is hidden (a locked
/// session, an occluded output), and a fade waiting on one would never end.
const FADE_SLACK: Duration = Duration::from_millis(100);

impl Pump {
    /// Plays the fade-out of a dismissed or launched panel, then returns
    /// for the caller to tear the session down. Nothing waits on it that
    /// should not: it is the last thing before the exit and bounded by the
    /// wall clock, not by frame callbacks.
    ///
    /// The panel is a picture from the first fade frame. That frame
    /// releases the keyboard (so the window the user returns to takes keys
    /// at once, not after the fade) and empties the input region (so a
    /// click lands on the desktop), the toggle's claim goes (a second
    /// invocation opens a fresh panel rather than signalling one that is
    /// leaving), and keys and buttons already queued are ignored. A look
    /// without motion, a dmenu panel and a panel the compositor already
    /// closed skip it.
    fn fade_out(&mut self, app: &mut App, text: &mut Text, look: &Look) {
        let begun = Instant::now();
        if app.live.is_none() || app.fx.stage.is_none() || !app.fx.motion.begin_exit(begun) {
            return;
        }
        app.fading = true;
        if let Some(live) = app.live.as_ref() {
            live.role()
                .set_keyboard_interactivity(KeyboardInteractivity::None);
        }
        app.claim = None;
        app.frame_due = true;
        let qh = self.qh.clone();
        let deadline = begun + look.timing().duration() + FADE_SLACK;
        loop {
            let now = Instant::now();
            // A closed or failed surface ends it too: `exit` is only set
            // by the compositor or a failed draw while the panel fades.
            if app.exit.is_some() || app.fx.motion.exit_done(now) || now >= deadline {
                return;
            }
            // Dispatch first, as the drive loop does: it is what turns a
            // `wl_callback.done` into `frame_due`, so a draw checked
            // before it would sleep in the poll with a frame already due.
            if self.queue.dispatch_pending(app).is_err() || app.exit.is_some() {
                return;
            }
            if app.frame_due {
                app.draw(text, look, &qh);
            }
            if self.queue.flush().is_err() {
                return;
            }
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            let wait = deadline.saturating_duration_since(Instant::now());
            let timespec = rustix::time::Timespec {
                tv_sec: wait.as_secs() as _,
                tv_nsec: wait.subsec_nanos() as _,
            };
            let wayland = guard.connection_fd();
            let mut polls = [rustix::event::PollFd::new(
                &wayland,
                rustix::event::PollFlags::IN,
            )];
            let _ = rustix::event::poll(&mut polls, Some(&timespec));
            if !polls[0].revents().is_empty() {
                let _ = guard.read();
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
            if app.accept_close() {
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
            // A launch always holds the toggle (only apps and run
            // launch; dmenu prints), so the wait watches both.
            match app.claim.as_ref() {
                Some(claim) => {
                    let toggle = claim.as_fd();
                    let mut polls = [
                        rustix::event::PollFd::new(&wayland, rustix::event::PollFlags::IN),
                        rustix::event::PollFd::new(&toggle, rustix::event::PollFlags::IN),
                    ];
                    let _ = rustix::event::poll(&mut polls, Some(&timespec));
                    if !polls[0].revents().is_empty() {
                        let _ = guard.read();
                    }
                }
                None => {
                    let mut polls = [rustix::event::PollFd::new(
                        &wayland,
                        rustix::event::PollFlags::IN,
                    )];
                    let _ = rustix::event::poll(&mut polls, Some(&timespec));
                    if !polls[0].revents().is_empty() {
                        let _ = guard.read();
                    }
                }
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
    /// `code` is the fallback; a taken exit decides instead. A print
    /// writes once, on the way out: a reader that went away (`--dmenu
    /// ... | head -c0`) warns and exits 1, as the headless commands do.
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
            Some(Exit::Dismiss | Exit::Closed) => 1,
            // A dmenu pick, printed once on the way out: the line, its
            // index, or the typed text. Nothing launches.
            Some(Exit::Print(text)) => match print(&text) {
                Ok(()) => 0,
                Err(error) => {
                    warn(format_args!("scootlaunch: cannot write the pick: {error}"));
                    1
                }
            },
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
empty_dispatch!(wl_region::WlRegion);
empty_dispatch!(wl_output::WlOutput);
empty_dispatch!(WpViewport);
empty_dispatch!(WpViewporter);
empty_dispatch!(WpFractionalScaleManagerV1);
empty_dispatch!(WpFractionalScaleV1);
empty_dispatch!(ZwlrLayerShellV1);
empty_dispatch!(XdgActivationV1);

impl Dispatch<wl_callback::WlCallback, ()> for App {
    fn event(
        app: &mut App,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            app.frame_requested = false;
            app.frame_due = true;
        }
    }
}

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
                // focus, drawing as before. A panel fading out has released
                // the keyboard itself: that leave is its own doing.
                if !app.seat.has_focus() && focused_before && !app.fading && app.pointer_outside() {
                    app.exit = Some(Exit::Closed);
                    return;
                }
                app.feed(output, &text);
            }
            Ok(None) => {
                if !app.seat.has_focus() && focused_before && !app.fading && app.pointer_outside() {
                    app.exit = Some(Exit::Closed);
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
                if button != BTN_LEFT || app.fading {
                    return;
                }
                if !matches!(state, WEnum::Value(wl_pointer::ButtonState::Pressed)) {
                    return;
                }
                // A click outside never arrives here (it goes to the
                // surface below); a click on a row picks it at once:
                // apps and run launch it, dmenu prints it.
                if let Some(row) = app.hit_row() {
                    match &mut app.model {
                        Model::Apps(ui) => {
                            let row = ui.list().visible_range().start.saturating_add(row);
                            match ui.click(row) {
                                Outcome::Selected(_) => {
                                    let serial = app.button_serial;
                                    if let Some(entry) = ui.selected() {
                                        app.exit = Some(Exit::Launch(Box::new(LaunchRequest {
                                            entry: Some(entry.clone()),
                                            action: ui.selected_action().cloned(),
                                            run_argv: None,
                                            serial,
                                        })));
                                    }
                                }
                                Outcome::Cancelled => app.exit = Some(Exit::Dismiss),
                                Outcome::Ignored | Outcome::Changed | Outcome::Run(_) => {}
                            }
                        }
                        Model::Run(ui) => {
                            let row = ui.list().visible_range().start.saturating_add(row);
                            match ui.click(row) {
                                Outcome::Selected(_) => {
                                    let serial = app.button_serial;
                                    if let Some(entry) = ui.selected() {
                                        app.exit = Some(Exit::Launch(Box::new(LaunchRequest {
                                            entry: None,
                                            action: None,
                                            run_argv: Some(vec![entry.name.clone()]),
                                            serial,
                                        })));
                                    }
                                }
                                Outcome::Cancelled => app.exit = Some(Exit::Dismiss),
                                Outcome::Ignored | Outcome::Changed | Outcome::Run(_) => {}
                            }
                        }
                        Model::Dmenu(dui) => {
                            let row = dui.list().visible_range().start.saturating_add(row);
                            match dui.click(row) {
                                DmenuOutcome::Print(print) => {
                                    let index = app.dmenu.as_ref().is_some_and(|spec| spec.index);
                                    let output = match print {
                                        DmenuPrint::Text(text) => text,
                                        DmenuPrint::Line(line) if index => format!("{line}"),
                                        DmenuPrint::Line(line) => {
                                            dui.line(line).unwrap_or("").to_string()
                                        }
                                    };
                                    app.exit = Some(Exit::Print(format!("{output}\n")));
                                }
                                DmenuOutcome::Cancelled => app.exit = Some(Exit::Dismiss),
                                DmenuOutcome::Ignored | DmenuOutcome::Changed => {}
                            }
                        }
                    }
                }
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                if !matches!(axis, WEnum::Value(wl_pointer::Axis::VerticalScroll)) {
                    return;
                }
                // The wheel scrolls the window without moving the
                // selection (a prompt-only dmenu panel shows no list,
                // so its window is empty and the scroll is a no-op).
                if value > 0.0 {
                    match &mut app.model {
                        Model::Apps(ui) | Model::Run(ui) => ui.scroll(false),
                        Model::Dmenu(dui) => dui.scroll(false),
                    }
                    app.dirty = true;
                } else if value < 0.0 {
                    match &mut app.model {
                        Model::Apps(ui) | Model::Run(ui) => ui.scroll(true),
                        Model::Dmenu(dui) => dui.scroll(true),
                    }
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
        // A release only frees the slot. A draw that found none free left
        // `dirty` set and waits in the poll for this; setting it here
        // redrew every frame the compositor released, so one keystroke
        // kept the panel committing for as long as it stayed open.
        if let wl_buffer::Event::Release = event {
            app.buffers.released(buffer);
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
        _: &QueueHandle<App>,
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
                        app.exit = Some(Exit::Closed);
                    }
                }
                app.configured_size = Some((width, height));
                app.dirty = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                if !live {
                    return;
                }
                // The compositor no longer wants the panel: an unplugged
                // output, or a click outside it (scoot closes an exclusive
                // launcher that way). A transient panel is not re-created,
                // unlike a bar, which retries once. Further changes to the
                // surface are ignored after `closed`, so there is nothing to
                // fade: it exits at once.
                app.exit = Some(Exit::Closed);
            }
            _ => {}
        }
    }
}

mod staged;

#[cfg(test)]
mod bytes_tests;
#[cfg(test)]
mod staged_tests;

#[cfg(test)]
mod tests {
    //! The fractional icon side: `paint_panel` at 1.5x draws a bigger
    //! icon bitmap from the scaled em, not a stretched 1x one. No
    //! compositor needed: the draw runs into a buffer and the icon
    //! square's pixels are counted.

    use super::*;
    use crate::rank::frecency::Frecency;

    fn test_text(dir: &std::path::Path) -> Text {
        let path = dir.join("test.ttf");
        std::fs::write(&path, scootui::testfont::build()).expect("font");
        scootui::font::text(Some(&path), &[]).expect("text")
    }

    fn red_png(path: &std::path::Path) {
        let mut buf = Vec::new();
        let mut encoder = png::Encoder::new(&mut buf, 48, 48);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        let mut red = Vec::with_capacity(48 * 48 * 4);
        for _ in 0..48 * 48 {
            red.extend_from_slice(&[255, 0, 0, 255]);
        }
        writer.write_image_data(&red).expect("png data");
        drop(writer);
        std::fs::write(path, &buf).expect("png")
    }

    #[test]
    fn fractional_scale_draws_a_bigger_icon_bitmap() {
        let dir = tempfile::tempdir().expect("tmp");
        let theme = dir.path().join("icons").join("Fixture");
        let apps = theme.join("48x48").join("apps");
        std::fs::create_dir_all(&apps).expect("mkdir");
        red_png(&apps.join("red.png"));
        std::fs::write(
            theme.join("index.theme"),
            "[Icon Theme]\nName=Fixture\nDirectories=48x48/apps\n\n[48x48/apps]\nSize=48\nType=Fixed\n",
        )
        .expect("index");
        let entry = Entry {
            id: "red.desktop".to_owned(),
            name: "Red".to_owned(),
            generic_name: None,
            comment: None,
            keywords: Vec::new(),
            icon: Some("red".to_owned()),
            exec: vec!["red.desktop".to_owned()],
            terminal: false,
            dbus_activatable: false,
            actions: Vec::new(),
            path: std::path::PathBuf::from("/usr/share/applications/red.desktop"),
            workdir: None,
            hidden: None,
        };
        let ui = Ui::open(vec![entry], Frecency::empty(), 0, 10);
        let look = Look::default();
        let mut text = test_text(dir.path());
        // 1.5x through the fractional scale: 180/120.
        let scale = scootui::density::Scale::Fractional(180);
        assert_eq!(scale.factor(), 1.5);
        let width = 400u32;
        let logical = Layout::new(look.font_size_px, look.radius, 1.0);
        let height = logical.height(10);
        let width_d = (f64::from(width) * 1.5).round() as u32;
        let height_d = (f64::from(height) * 1.5).round() as u32;
        let mut pixels = vec![0u8; width_d as usize * height_d as usize * 4];
        let mut icons = crate::icons::Cache::default();
        let bases = [dir.path().join("icons")];
        let view = PanelView {
            ui: &ui,
            look: &look,
            width,
            height,
            error: None,
            empty_text: "empty",
            scale,
            icon_theme: "Fixture",
            icon_bases: &bases,
        };
        paint_panel(
            &mut pixels,
            (width_d, height_d),
            &view,
            &mut text,
            &mut icons,
            None,
        );
        // The 1.5x icon side from the scaled em, as `paint_panel`
        // derives it: above the 1x side a scale-1 draw would use.
        let em = (f64::from(look.font_size_px) * 1.5) as f32;
        let device = Layout::new(em, (f64::from(look.radius) * 1.5).round() as u32, 1.5);
        let side = device.side;
        assert!(side > logical.side, "a 1.5x side {side} above the 1x side");
        // The first row's icon square, as `paint_panel` places it.
        let icon = device.icon(device.row(width_d, 0));
        assert_eq!(icon.width, side);
        // BGRA premultiplied, as the canvas blends it: opaque red.
        let mut red = 0u32;
        for x in icon.x..icon.right() {
            for y in icon.y..icon.bottom() {
                let at = (y * width_d + x) as usize * 4;
                let pixel = &pixels[at..at + 4];
                if pixel == [0, 0, 255, 255] {
                    red += 1;
                }
            }
        }
        assert!(
            red >= side * side / 2,
            "{red} red icon pixels for one {side}px row icon at 1.5x"
        );
    }

    /// Fonts 11 to 20 at scales 1, 1.5 and 2, with an empty list, one
    /// row, a full window and a hostile-narrow panel: the draw never
    /// panics, the highlight's first and last pixels sit at the inner
    /// padding, and the pixels left of the icon are still highlight.
    #[test]
    fn the_panel_draws_at_every_font_and_scale() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut text = test_text(dir.path());
        let mut icons = crate::icons::Cache::default();
        for font in [11.0f32, 13.0, 16.0, 20.0] {
            for scale in [
                scootui::density::Scale::Integer(1),
                scootui::density::Scale::Fractional(180),
                scootui::density::Scale::Integer(2),
            ] {
                for count in [0usize, 1, 5, 40] {
                    let entries = (0..count)
                        .map(|n| Entry {
                            id: format!("app{n}.desktop"),
                            name: format!("Application number {n}"),
                            generic_name: None,
                            comment: None,
                            keywords: Vec::new(),
                            icon: None,
                            exec: vec!["x".to_owned()],
                            terminal: false,
                            dbus_activatable: false,
                            actions: Vec::new(),
                            path: std::path::PathBuf::from("/usr/share/applications/x.desktop"),
                            workdir: None,
                            hidden: None,
                        })
                        .collect();
                    let ui = Ui::open(entries, Frecency::empty(), 0, 10);
                    let look = Look {
                        font_size_px: font,
                        ..Look::default()
                    };
                    let logical = Layout::new(font, look.radius, 1.0);
                    let width = (30.0 * font) as u32;
                    let height = logical.height(10);
                    let factor = scale.factor();
                    let width_d = (f64::from(width) * factor).round() as u32;
                    let height_d = (f64::from(height) * factor).round() as u32;
                    let mut pixels = vec![0u8; width_d as usize * height_d as usize * 4];
                    let view = PanelView {
                        ui: &ui,
                        look: &look,
                        width,
                        height,
                        error: None,
                        empty_text: "empty",
                        scale,
                        icon_theme: "Fixture",
                        icon_bases: &[],
                    };
                    paint_panel(
                        &mut pixels,
                        (width_d, height_d),
                        &view,
                        &mut text,
                        &mut icons,
                        None,
                    );
                    if count == 0 {
                        continue;
                    }
                    let em = (f64::from(font) * factor) as f32;
                    let radius = (f64::from(look.radius) * factor).round() as u32;
                    let device = Layout::new(em, radius, factor);
                    let row = device.row(width_d, 0);
                    let accent = look.theme.accent.argb8888(255).to_le_bytes();
                    let at = |x: u32, y: u32| {
                        let i = (y * width_d + x) as usize * 4;
                        pixels[i..i + 4].to_vec()
                    };
                    let mid = row.y + row.height / 2;
                    // The first row is selected: highlight edge to edge,
                    // and the icon column's left margin is still fill.
                    assert_eq!(at(row.x, mid), accent, "font {font} scale {factor}");
                    assert_eq!(
                        at(row.right() - 1, mid),
                        accent,
                        "font {font} scale {factor}"
                    );
                    assert_eq!(
                        at(row.x + device.lead - 1, mid),
                        accent,
                        "font {font} scale {factor}"
                    );
                    // Nothing paints outside the inner box's rows: the
                    // padding between the border and the highlight is
                    // not accent.
                    assert_ne!(at(row.x - 1, mid), accent, "font {font} scale {factor}");
                }
            }
        }
    }
}
