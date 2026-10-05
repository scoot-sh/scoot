//! Argument parsing, hand-rolled as scootbg's is (its dependency record,
//! §4: +12 KB against +299 KB for `clap`). The config file
//! (`$XDG_CONFIG_HOME/scoot/bar.toml`, `crate::config`) holds every option;
//! the flags stay and override its values, one by one.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use crate::bar::{self, Edge, Layer, MAX_HEIGHT, Margin, MarginError};
use crate::color::{Color, ColorError};
use crate::config::{Config, MAX_FONT_SIZE};
use crate::layout::{Layout, MAX_GAP, MAX_MODULES, PlacementError};
use crate::modules::{self, REGISTRY};
use crate::policy::{self, MAX_OUTPUTS, PolicyError, Select};

#[cfg(test)]
mod tests;

// The help text is put together from pieces, so a build without a module
// documents only what it has: the pieces that name the clock come in two
// versions, one per build, and the workspaces piece is empty without it.
#[cfg(all(feature = "clock", feature = "workspaces"))]
macro_rules! what_it_shows {
    () => {
        "A bar on every output, reserving its space, with workspaces and a clock."
    };
}
#[cfg(all(feature = "clock", not(feature = "workspaces")))]
macro_rules! what_it_shows {
    () => {
        "A bar on every output, reserving its space, with a clock."
    };
}
#[cfg(all(not(feature = "clock"), feature = "workspaces"))]
macro_rules! what_it_shows {
    () => {
        "A bar on every output, reserving its space, with workspaces."
    };
}
#[cfg(all(
    not(feature = "clock"),
    not(feature = "workspaces"),
    not(feature = "window-title")
))]
macro_rules! what_it_shows {
    () => {
        "A bar on every output, reserving its space (built with no modules)."
    };
}
#[cfg(all(
    not(feature = "clock"),
    not(feature = "workspaces"),
    feature = "window-title"
))]
macro_rules! what_it_shows {
    () => {
        "A bar on every output, reserving its space, with a window title."
    };
}

#[cfg(feature = "clock")]
macro_rules! idle {
    () => {
        "and only when something changed. Idle, it wakes twice a minute: the
clock's tick, and the compositor's release of the frame the tick replaced
(one per frame, for any wl_shm client); with no module placed, never."
    };
}
#[cfg(not(feature = "clock"))]
macro_rules! idle {
    () => {
        "and only when something changed: idle, it never wakes."
    };
}

// The `--left`/`--center`/`--right`, `--padding`, `--spacing` and
// `--clock-format` block of the daemon help, built at run time from
// [`REGISTRY`] so a new module needs nothing here: its `Modules:` entry
// comes from its registry line, and only a module with a help section of
// its own adds a `*_help!` arm below and one line to [`daemon_help`].
// `--help` is a cold path; the one small allocation costs nothing.
//
// The text is byte for byte what the combinatorial `modules!` arms this
// replaces produced, quirks included: a build with the clock names it as
// the default center module and documents `--clock-format`; `Modules:`
// wraps onto its own line when the list holds the window title, and stays
// inline otherwise; and the hand-written arms' 27-space continuations stay
// 27 spaces, while the generated arms' 28-space ones stay 28 (the split is
// whether the build has any of the clock, workspaces or window-title).
fn modules_section() -> String {
    /// The `--left` lines when the build has a clock: the default layout
    /// is the clock in the center.
    const WITH_CLOCK: &str =
        "    --left IDS           the modules along the left, center and right,
     --center IDS         comma-separated, in order (default: the clock in
     --right IDS          the center). Giving any of the three sets the whole
                           layout: a section not given is empty.";
    /// The `--left` lines otherwise.
    const WITHOUT_CLOCK: &str =
        "    --left IDS           the modules along the left, center and right,
     --center IDS         comma-separated, in order. Giving any of the three
     --right IDS          sets the whole layout.";
    /// The whole block when the build has no module at all.
    const NONE: &str = "    --left IDS           the modules along the left, center and right,
     --center IDS         comma-separated, in order. This build has none, so
     --right IDS          only an empty list is taken";
    /// `--padding` and `--spacing` when the build has any of the clock,
    /// workspaces or window-title (27-space continuations, as the
    /// hand-written arms had them).
    const PADDING_TRIPLE: &str =
        "     --padding N          logical pixels either side of each module, 0 to 1024
                           (default 8)
     --spacing N          logical pixels between neighbouring modules, 0 to
                           1024 (default 0)
";
    /// `--padding` and `--spacing` otherwise (28 spaces, as the generated
    /// arms had them).
    const PADDING_PLAIN: &str =
        "     --padding N          logical pixels either side of each module, 0 to 1024
                            (default 8)
     --spacing N          logical pixels between neighbouring modules, 0 to
                            1024 (default 0)
";
    /// `--clock-format`, only in a build with the clock.
    const CLOCK_FORMAT: &str =
        "     --clock-format FMT   the clock, as strftime (default '%-I:%M %P', which
                           shows 3:07 pm; '%H:%M' for 15:07). Specifiers:
                           %H %I %k %l %M %S %p %P %a %A %b %h %B %d %e %m %j
                           %y %Y %u %w %Z %z %R %T %F %D %%, and the flags -
                           (no padding), _ (spaces) and 0 (zeros). With %S or
                           %T it ticks every second, else once a minute. The
                           zone is $TZ's, else /etc/localtime's
";
    let mut ids: Vec<&str> = modules::REGISTRY.iter().map(|spec| spec.id).collect();
    ids.sort_unstable();
    let has_clock = modules::find("clock").is_some();
    let has_workspaces = modules::find("workspaces").is_some();
    let has_window_title = modules::find("window-title").is_some();
    let padding = if has_clock || has_workspaces || has_window_title {
        PADDING_TRIPLE
    } else {
        PADDING_PLAIN
    };
    let mut section = String::new();
    if has_clock {
        section.push_str(WITH_CLOCK);
        if has_window_title {
            section.push_str("\n                           Modules: ");
        } else {
            section.push_str(" Modules: ");
        }
        section.push_str(&ids.join(", "));
        section.push('\n');
        section.push_str(padding);
        section.push_str(CLOCK_FORMAT);
    } else if ids.is_empty() {
        section.push_str(NONE);
        section.push('\n');
        section.push_str(padding);
    } else {
        section.push_str(WITHOUT_CLOCK);
        section.push_str(" Modules: ");
        section.push_str(&ids.join(", "));
        section.push('\n');
        section.push_str(padding);
    }
    section
}

#[cfg(feature = "workspaces")]
macro_rules! workspaces_help {
    () => {
        "Workspaces (the workspaces module):
    one number per workspace of the output the bar is on, the active one
    marked; a click on a number shows that workspace. Nothing is shown
    where the compositor has no ext-workspace-v1, and clicks do nothing
    where it has no wl_seat
"
    };
}
#[cfg(not(feature = "workspaces"))]
macro_rules! workspaces_help {
    () => {
        ""
    };
}
#[cfg(feature = "workspaces")]
macro_rules! workspace_wakes {
    () => {
        " A workspace change redraws the workspaces module once."
    };
}
#[cfg(not(feature = "workspaces"))]
macro_rules! workspace_wakes {
    () => {
        ""
    };
}
// What the window-title module is, for the help page.
#[cfg(feature = "window-title")]
macro_rules! window_title_help {
    () => {
        "Window title (the window-title module):
    the focused window's title on the output the bar is on (its app id
    after it, with window-title.show-app-id), cut with an ellipsis past
    window-title.max-width, with a static icon before it when configured
    (window-title.icon; window-title.show-text = false draws only the
    icon); a click focuses it, a middle click closes it
    with window-title.allow-close. Nothing is shown where the compositor
    has no wlr-foreign-toplevel-management-v1, and clicks do nothing where
    it has no wl_seat
"
    };
}
#[cfg(not(feature = "window-title"))]
macro_rules! window_title_help {
    () => {
        ""
    };
}
// What the volume module (and its microphone twin) is, for the help page.
#[cfg(all(feature = "volume", feature = "popup"))]
macro_rules! volume_help {
    () => {
        "Volume (the volume module):
    the default sink's level in percent, dimmed while muted, with an icon
    for the level (muted, low, medium, high); a scroll changes it by
    volume.step percent (default 5) up to volume.max-volume (default 100,
    full scale), a click toggles mute, a right click runs the mixer
    command the config binds; bind a click to \"popup\" (on-click =
    \"popup\") for a slider under the module instead. Nothing is shown
    where no sound server runs
"
    };
}
#[cfg(all(feature = "volume", not(feature = "popup")))]
macro_rules! volume_help {
    () => {
        "Volume (the volume module):
    the default sink's level in percent, dimmed while muted, with an icon
    for the level (muted, low, medium, high); a scroll changes it by
    volume.step percent (default 5) up to volume.max-volume (default 100,
    full scale), a click toggles mute, a right click runs the mixer
    command the config binds. Nothing is shown where no sound server runs
"
    };
}
#[cfg(not(feature = "volume"))]
macro_rules! volume_help {
    () => {
        ""
    };
}
#[cfg(feature = "microphone")]
macro_rules! microphone_help {
    () => {
        "Microphone (the microphone module):
    the default source's level and mute, sharing the volume module's
    options under [microphone]: a scroll changes it, a click toggles mute.
    Nothing is shown where no sound server runs
"
    };
}
#[cfg(not(feature = "microphone"))]
macro_rules! microphone_help {
    () => {
        ""
    };
}
// What the battery module is, for the help page.
#[cfg(feature = "battery")]
macro_rules! battery_help {
    () => {
        "Battery (the battery module):
    the batteries' charge in percent (`combine` by default, the mean, or
    the first with batteries = \"first\"), warn at or below
    battery.warn-below (default 20) and urgent at or below
    battery.urgent-below (default 10), with battery.on-low run once per
    downward crossing of the urgent one, with an icon per level when
    configured (battery.icon takes one glyph or 5 for the charge levels;
    battery.icon-charging and battery.icon-full for those states;
    battery.show-text = false draws only the icon). Woken by the kernel's
    uevents, and re-read once a minute while discharging for drivers whose
    capacity steps are silent. Nothing is shown where there is no battery
"
    };
}
#[cfg(not(feature = "battery"))]
macro_rules! battery_help {
    () => {
        ""
    };
}
// What the network module is, for the help page.
#[cfg(feature = "network")]
macro_rules! network_help {
    () => {
        "Network (the network module):
    the default route's interface (or network.interface), by name when it
    is ethernet, the SSID when it is WiFi, VPN when it is
    a tunnel, and offline when it has no address, with an icon per state
    when configured (network.icon-ethernet, network.icon-wifi,
    network.icon-vpn and network.icon-offline, falling back to
    network.icon; network.icon-wifi takes one glyph or 4 for the signal
    levels; network.show-text = false draws only the icon); a click
    opens the picker
    (network.menu-command with the scan's SSIDs on stdin), or the native
    list with on-click = \"popup\" (network.connect-command with the SSID
    as its last argument). Nothing is
    shown where the machine has no network interface
"
    };
}
#[cfg(not(feature = "network"))]
macro_rules! network_help {
    () => {
        ""
    };
}
// What the brightness module is, for the help page.
#[cfg(feature = "brightness")]
macro_rules! brightness_help {
    () => {
        "Brightness (the brightness module):
    the panel backlight's level in percent (brightness.device names it
    where the machine has several); a scroll changes it by brightness.step
    percent (default 5), and `set` takes the absolute percent, never below
    the raw floor that keeps the panel lit, with an icon per level when
    configured (brightness.icon takes one glyph or 4 for the levels;
    brightness.show-text = false draws only the icon). Writes need permission (a udev
    rule for the backlight class or the video group grants it), and are refused naming that
    without it. Nothing is shown where the machine has no backlight
"
    };
}
#[cfg(not(feature = "brightness"))]
macro_rules! brightness_help {
    () => {
        ""
    };
}

// What the tray module is, for the help page.
#[cfg(feature = "tray")]
macro_rules! tray_help {
    () => {
        "Tray (the tray module):
    the StatusNotifierItem watcher and host: applications' icons at exact
    device pixels, a click activates, a middle click secondarily, a scroll
    scrolls; its actions (`activate N`, `secondary N`, `wheel-up N`,
    `wheel-down N`) take the item index. The bar owns the
    watcher name when free and hosts against whoever does otherwise.
    Item menus are not built yet (no DBusMenu client). Nothing is shown where no item
    is registered
"
    };
}
#[cfg(not(feature = "tray"))]
macro_rules! tray_help {
    () => {
        ""
    };
}

// What the media module is, for the help page.
#[cfg(feature = "media")]
macro_rules! media_help {
    () => {
        "Media (the media module):
    what the players on the session bus (MPRIS) are playing: artist - title
    with a play or pause icon (dimmed while paused), cut to media.max-width
    (default 320 logical pixels), with an icon per state when configured
    (media.icon-playing and media.icon-paused, falling back to media.icon;
    media.show-text = false draws only the icon). A click plays or pauses, a right click or
    a scroll down skips to the next track, a middle click or a scroll up to
    the previous one; its actions are `play-pause`, `next` and `previous`.
    Of several players the one named by media.player is shown if it is
    playing or paused, else the one that started playing last; a stopped
    player shows nothing. Nothing is shown with no player, and nothing is
    polled
"
    };
}
#[cfg(not(feature = "media"))]
macro_rules! media_help {
    () => {
        ""
    };
}

// What the bluetooth module is, for the help page.
#[cfg(feature = "bluetooth")]
macro_rules! bluetooth_help {
    () => {
        "Bluetooth (the bluetooth module):
    the adapter's power and the connected devices over BlueZ (the system
    bus): the first connected device's name with its charge when BlueZ
    reports one, `on` while an adapter is powered with nothing connected,
    and `off` while every adapter is off, with an icon per state when
    configured (bluetooth.icon-off, bluetooth.icon-on and
    bluetooth.icon-connected, falling back to bluetooth.icon;
    bluetooth.show-text = false draws only the icon). A click toggles the
    first adapter's power; its actions are `toggle` and `menu`, the picker
    (bluetooth.menu-command with the device list on stdin). Nothing is
    shown with no adapter or no BlueZ, and nothing is polled
"
    };
}
#[cfg(not(feature = "bluetooth"))]
macro_rules! bluetooth_help {
    () => {
        ""
    };
}

// What the power module is, for the help page.
#[cfg(feature = "power")]
macro_rules! power_help {
    () => {
        "Power (the power module):
    lock, log out, suspend, reboot and shut down from a popup menu with
    a confirm step (bind a click to \"popup\" with on-click = \"popup\"):
    the first click on a row arms it, the second performs it. Lock runs
    power.lock-command (hidden without one); log out quits scoot (or
    power.logout-command); the rest call logind over the system bus (or
    their power.*-command), hidden where logind refuses. Its actions are
    `lock`, `logout`, `suspend`, `reboot` and `poweroff`, each taking no
    number; an agent's invoke follows the same two steps. Nothing is
    shown without an icon (power.icon), and nothing is polled
"
    };
}
#[cfg(not(feature = "power"))]
macro_rules! power_help {
    () => {
        ""
    };
}

pub const USAGE: &str = concat!(
    "\
scootbar -- status bar for Wayland

",
    what_it_shows!(),
    "

USAGE:
    scootbar daemon [OPTIONS]
    scootbar daemon --check [OPTIONS]
    scootbar daemon --help
    scootbar msg COMMAND
    scootbar msg --help
    scootbar --version
    scootbar --help

COMMANDS:
    daemon     run the bar on this Wayland display's outputs (every one, by default)
    msg        ask the running daemon: query, reload, hide, show, toggle, version, kill, set
"
);

/// Everything in [`daemon_help`] before the modules section: the usage,
/// the outputs, the bar, the text and the `--left` options, up to the
/// `Modules:` header (with this build's idle lines).
const DAEMON_PRE: &str = concat!(
    "\
scootbar daemon -- run the bar

USAGE:
    scootbar daemon [OPTIONS]
    scootbar daemon --check [OPTIONS]

Connects to the compositor named by $WAYLAND_DISPLAY, which must support
wlr-layer-shell, and gives every output (or the ones --outputs names) a bar: a layer surface (top, by
default) along one edge (namespace \"scootbar\") that reserves its height,
so windows are arranged beside it, unless told to float over them. Outputs plugged in later get one too, and an output
unplugged takes its bar with it; with no outputs at all it waits for one.
It draws at each output's real device pixels, fractional scales included,
",
    idle!(),
    workspace_wakes!(),
    "

Outputs:
    --outputs LIST       all (the default) or comma-separated connector names,
                         like DP-1,eDP-1: only those outputs get a bar. One
                         plugged in later gets one if it is listed; one that
                         leaves loses its bar. The config file also sets a
                         bar's edge, layer, exclusive, height, margin and
                         module lists per output, in [output.\"NAME\"] tables
                         (see docs/scootbar/cli.md); those win over the
                         flags below for that output

The bar:
    --edge EDGE          top (the default) or bottom
    --layer LAYER        bottom (behind windows), top (the default: in front of
                         windows, hidden by a fullscreen one) or overlay (in
                         front of everything, fullscreen windows included)
    --exclusive BOOL     true (the default: reserve the bar's height, so
                         windows are arranged beside it) or false (float over
                         the windows, reserving nothing)
    --height N           the bar's height in logical pixels, 1 to 1024
                         (default 28)
    --margin M           space between the bar and the output's edges, in
                         logical pixels, as in CSS: ALL, VERTICAL,HORIZONTAL,
                         TOP,HORIZONTAL,BOTTOM or TOP,RIGHT,BOTTOM,LEFT, each
                         0 to 1024 (default 0). Windows keep clear of the bar
                         and of the margin on its edge; the margin on the
                         opposite edge does nothing
    --background COLOR   the bar's color, '#rrggbb' (default '#1e1e2e');
                         quote it, since the shell reads '#' as a comment
    --foreground COLOR   the text's color (default '#cdd6f4')

Text:
    --font PATH          a .ttf or .otf file. Without it, the first of a few
                         well-known files (DejaVu Sans, Noto Sans) found; with
                         none, it refuses to start (a bar with no module
                         placed needs none)
    --font-size N        the text's size (the em) in logical pixels, 1 to 256
                         (default 14)
    Fallback fonts (`bar.fallback-fonts`) and a clock icon (`clock.icon`, a
    glyph; `clock.icon-path`, SVG path data; `clock.icon-image`, a PNG, in a
    build with the icon-image feature) are config-file only; see
    docs/scootbar/cli.md.

Modules:
",
);

/// Everything in [`daemon_help`] after the modules section: one help
/// section per module that has one, then the config file and `--check`.
const DAEMON_POST: &str = concat!(
    battery_help!(),
    workspaces_help!(),
    window_title_help!(),
    volume_help!(),
    microphone_help!(),
    network_help!(),
    brightness_help!(),
    tray_help!(),
    media_help!(),
    bluetooth_help!(),
    power_help!(),
    "
The config file ($XDG_CONFIG_HOME/scoot/bar.toml, ~/.config/scoot/bar.toml
without it) holds every option above; `--config PATH` reads another file
instead. A flag given replaces the file's value for its own option; a
missing file is the defaults. Runs until the compositor goes away (exit
status 1, saying why) or it is killed; SIGTERM and SIGINT end it at once:
it keeps no state, and the compositor removes the bars with the connection.
An `exec` module's command ends with it.

--check validates instead of running: it reads the config file, applies the
flags over it, starts the placed modules and loads the font, exactly as a
start does, then exits 0 (printing `ok`) or 1 with the error a start would
give. It never connects to a compositor or claims a control socket, so it
runs anywhere: in a build, in CI, over a file about to be installed.
"
);

/// The daemon help page for this build's features: [`DAEMON_PRE`], the
/// [`modules_section`] for this build's modules, and [`DAEMON_POST`].
pub fn daemon_help() -> String {
    let mut help = String::with_capacity(DAEMON_PRE.len() + DAEMON_POST.len() + 512);
    help.push_str(DAEMON_PRE);
    help.push_str(&modules_section());
    help.push_str(DAEMON_POST);
    help
}

pub const MSG_HELP: &str = "\
scootbar msg -- ask the running daemon

USAGE:
    scootbar msg query [ID]
    scootbar msg layout
    scootbar msg invoke ID ACTION [NUMBER] [--output NAME]
    scootbar msg subscribe [module] [output]
    scootbar msg reload
    scootbar msg hide
    scootbar msg show
    scootbar msg toggle
    scootbar msg version
    scootbar msg kill
    scootbar msg set ID JSON
    scootbar msg --help

Asks the daemon for this Wayland display over its control socket
($XDG_RUNTIME_DIR/scootbar-DISPLAY.sock), which the daemon claims at
start-up and removes when it stops:

    query      each placed module's state as JSON, once per output that
               shows it (or only module ID's): its id, section, output,
               text and class, plus its icon, tooltip and value where it has
               them (output is null where the compositor never named it)
    layout     each output's bar and each module's rectangle on it, in the
               compositor's global logical pixels as last drawn, so a click
               can be aimed with `scoot msg pointer click X Y`
    invoke     run module ID's ACTION as a click would (a module's own
               action, with its NUMBER, or one of click, right-click,
               middle-click, scroll-up, scroll-down, which runs the
               configured binding; NUMBER is a scroll's steps), on the
               output NAME shows it on (default: the first)
    subscribe  stay connected and print one JSON line per event of the kinds
               named (both by default): `module` (a module's view changed,
               at most once a frame) and `output` (one was added or removed);
               there is no snapshot: subscribe first, then query. It ends
               with status 1 on a `{\"type\":\"dropped\"}` line (the daemon
               dropped it) or when the stream is cut mid-line (that line is
               discarded), and with status 0 when the stream just ends,
               which does NOT mean the daemon exited: a subscriber that was
               too slow or stopped (its socket full) is closed with no line.
               After any end, subscribe again and then query
    reload     re-read the config file and live-apply it; a bad file is
               refused and the running bar stands
    hide       take the bars away: every layer surface and buffer is
               destroyed and the exclusive zone released, so windows
               reclaim the space and the bar holds no memory but the process
    show       make the bars again
    toggle     hide if shown, show if hidden
    version    the daemon's version and protocol, as JSON
    kill       stop the daemon, once its reply is sent
    set        a JSON value for the `push` module ID, which the config
                defines (`[push.ID]`): a string (the text), an object
                `{\"text\": ..., \"class\": ..., \"tooltip\": ..., \"icon\": ...}` or null
                (clears it). Any other module, an id that is not placed and a
                value the module refuses are loud errors, never a silent ok

`query`, `layout`, `version`, `reload`, `hide`, `show` and `toggle` print the
reply (the last three say `{\"type\":\"bar\",\"visible\":false}`, what is now the
case); `kill`, `set` and `invoke` print nothing on success. Without a daemon,
every command fails saying so.
";

/// A help page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    Main,
    Daemon,
    Msg,
}

impl Topic {
    /// The help page: the main and msg pages are static text, while the
    /// daemon page is built for this build's modules (see
    /// [`modules_section`]).
    pub fn text(self) -> Cow<'static, str> {
        match self {
            Self::Main => Cow::Borrowed(USAGE),
            Self::Daemon => Cow::Owned(daemon_help()),
            Self::Msg => Cow::Borrowed(MSG_HELP),
        }
    }
}

/// What `scootbar msg` asks of the running daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// Each module's state, or one module's.
    Query {
        id: Option<String>,
    },
    /// Each module's rectangle, as last drawn.
    Layout,
    /// Run a module's action (or a trigger's binding) as a click would.
    Invoke {
        id: String,
        action: String,
        arg: Option<i32>,
        output: Option<String>,
    },
    /// Stay connected and print events of these kinds.
    Subscribe {
        events: Vec<crate::control::protocol::EventKind>,
    },
    Reload,
    Hide,
    Show,
    Toggle,
    Version,
    Kill,
    /// A value for module `id` (a well-formed id; whether a module of that
    /// name is placed is the daemon's to say, since a config names its own):
    /// the raw JSON, validated but otherwise unread.
    Set {
        id: String,
        value: String,
    },
}

/// What `scootbar daemon` runs with: the flags' values, the config file to
/// read them with, and what the flags alone (without a file) would run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonCommand {
    pub given: Given,
    /// `--config`'s path; `None` reads the default file.
    pub file: Option<PathBuf>,
    /// The flags over the defaults, without any file.
    pub config: Box<Config>,
    /// `--check`: validate and exit, never connect.
    pub check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help(Topic),
    Version,
    Daemon(Box<DaemonCommand>),
    Msg(Msg),
}

/// The flags `daemon` takes.
const OUTPUTS: &str = "--outputs";
const EDGE: &str = "--edge";
const LAYER: &str = "--layer";
const EXCLUSIVE: &str = "--exclusive";
const HEIGHT: &str = "--height";
const MARGIN: &str = "--margin";
const BACKGROUND: &str = "--background";
const FOREGROUND: &str = "--foreground";
const FONT: &str = "--font";
const FONT_SIZE: &str = "--font-size";
const LEFT: &str = "--left";
const CENTER: &str = "--center";
const RIGHT: &str = "--right";
const PADDING: &str = "--padding";
const SPACING: &str = "--spacing";
const CONFIG: &str = "--config";
/// A switch, not a value flag: the one `daemon` flag that takes nothing.
const CHECK: &str = "--check";
#[cfg(feature = "clock")]
const CLOCK_FORMAT: &str = "--clock-format";

const FLAGS: &[&str] = &[
    OUTPUTS,
    EDGE,
    LAYER,
    EXCLUSIVE,
    HEIGHT,
    MARGIN,
    BACKGROUND,
    FOREGROUND,
    FONT,
    FONT_SIZE,
    LEFT,
    CENTER,
    RIGHT,
    PADDING,
    SPACING,
    CONFIG,
    #[cfg(feature = "clock")]
    CLOCK_FORMAT,
];

/// Why a module list is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModulesError {
    /// No module of that name in this build.
    Unknown(String),
    /// Placed twice (in one section or across two).
    Twice(&'static str),
    TooMany,
}

impl fmt::Display for ModulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => {
                write!(
                    f,
                    "no module `{}` in this build (it has:",
                    id.escape_debug()
                )?;
                if REGISTRY.is_empty() {
                    write!(f, " none")?;
                }
                for spec in REGISTRY {
                    write!(f, " {}", spec.id)?;
                }
                write!(f, ")")
            }
            Self::Twice(id) => write!(f, "{}", PlacementError::Twice(id)),
            Self::TooMany => write!(f, "{}", PlacementError::TooMany),
        }
    }
}

/// Why an `--outputs` value is refused (or, after the overlay, why the
/// flag and the file together are).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputsError {
    /// Empty, or an empty entry (`DP-1,`).
    Empty,
    /// More than [`MAX_OUTPUTS`] names, or a name the policy refuses.
    Policy(PolicyError),
}

impl fmt::Display for OutputsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(
                f,
                "takes `all` or comma-separated connector names, like `DP-1,eDP-1`"
            ),
            Self::Policy(error) => write!(f, "{error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Missing,
    Unknown(String),
    Unexpected {
        command: &'static str,
        argument: String,
    },
    MissingValue(&'static str),
    Repeated(&'static str),
    Outputs {
        value: String,
        error: OutputsError,
    },
    /// `--outputs` and the file's `[output]` tables together.
    OutputsClash(PolicyError),
    Edge(String),
    Layer(String),
    Exclusive(String),
    Height(String),
    Margin {
        value: String,
        error: MarginError,
    },
    Color {
        flag: &'static str,
        value: String,
        error: ColorError,
    },
    FontSize(String),
    Gap {
        flag: &'static str,
        value: String,
    },
    Modules {
        flag: &'static str,
        error: ModulesError,
    },
    Msg(MsgError),
    #[cfg(feature = "clock")]
    ClockFormat {
        value: String,
        error: modules::clock::format::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "missing command (try --help)"),
            Self::Unknown(what) => write!(f, "unknown command `{what}` (try --help)"),
            Self::Unexpected { command, argument } => write!(
                f,
                "unexpected argument `{argument}` for `{command}` (try `scootbar {command} --help`)"
            ),
            Self::MissingValue(flag) => {
                write!(f, "`{flag}` needs a value (try `scootbar daemon --help`)")
            }
            Self::Repeated(flag) => {
                write!(f, "`{flag}` given twice (try `scootbar daemon --help`)")
            }
            Self::Outputs { value, error } => {
                write!(f, "`{OUTPUTS} {}`: {error}", value.escape_debug())
            }
            Self::OutputsClash(error) => {
                write!(f, "`{OUTPUTS}` with the config file: {error}")
            }
            Self::Edge(value) => write!(f, "`{EDGE}` takes top or bottom, not `{value}`"),
            Self::Layer(value) => {
                write!(f, "`{LAYER}` takes bottom, top or overlay, not `{value}`")
            }
            Self::Exclusive(value) => {
                write!(f, "`{EXCLUSIVE}` takes true or false, not `{value}`")
            }
            Self::Height(value) => write!(
                f,
                "`{HEIGHT}` takes a whole number of logical pixels from 1 to {MAX_HEIGHT}, \
                 not `{value}`"
            ),
            Self::Margin { value, error } => write!(f, "`{MARGIN} {value}`: {error}"),
            Self::Color { flag, value, error } => write!(f, "`{flag} {value}`: {error}"),
            Self::FontSize(value) => write!(
                f,
                "`{FONT_SIZE}` takes a whole number of logical pixels from 1 to \
                 {MAX_FONT_SIZE}, not `{value}`"
            ),
            Self::Gap { flag, value } => write!(
                f,
                "`{flag}` takes a whole number of logical pixels from 0 to {MAX_GAP}, \
                 not `{value}`"
            ),
            Self::Modules { flag, error } => write!(f, "`{flag}`: {error}"),
            Self::Msg(error) => write!(f, "{error}"),
            #[cfg(feature = "clock")]
            Self::ClockFormat { value, error } => {
                write!(f, "`{CLOCK_FORMAT} {}`: {error}", value.escape_debug())
            }
        }
    }
}

impl std::error::Error for Error {}

/// Why a `scootbar msg` command is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgError {
    /// No command at all.
    Missing,
    /// Not one of `query`, `layout`, `invoke`, `subscribe`, `reload`, `hide`, `show`, `toggle`, `version`, `kill` or `set`.
    Unknown(String),
    /// `set` and `invoke` without their module id, or `set` without its JSON
    /// value, or `invoke` without its action.
    NeedsId,
    NeedsValue,
    NeedsAction,
    /// `invoke`'s number is not a whole number that fits.
    BadArg(String),
    /// `--output` without a name.
    NeedsOutput,
    /// `subscribe` names a kind that does not exist.
    UnknownEvent(String),
    /// Not the shape of a module id.
    BadModuleId(String),
    /// The value is not JSON.
    BadJson(String),
}

impl fmt::Display for MsgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "missing msg command (try `scootbar msg --help`)"),
            Self::Unknown(what) => {
                write!(
                    f,
                    "unknown msg command `{what}` (try `scootbar msg --help`)"
                )
            }
            Self::NeedsId => write!(
                f,
                "`scootbar msg set` needs a module id and a JSON value \
                 (try `scootbar msg --help`)"
            ),
            Self::NeedsValue => write!(
                f,
                "`scootbar msg set ID` needs a JSON value (try `scootbar msg --help`)"
            ),
            Self::BadModuleId(id) => write!(
                f,
                "`{}` is not a module id (1 to {} letters, digits, `-` and `_`)",
                id.escape_debug(),
                crate::modules::custom::MAX_NAME
            ),
            Self::NeedsAction => write!(
                f,
                "`scootbar msg invoke ID` needs an action (try `scootbar msg --help`)"
            ),
            Self::BadArg(arg) => write!(
                f,
                "`{}` is not a whole number (`scootbar msg invoke ID ACTION [NUMBER]`)",
                arg.escape_debug()
            ),
            Self::NeedsOutput => write!(f, "`--output` needs an output name"),
            Self::UnknownEvent(kind) => write!(
                f,
                "unknown event kind `{}` (they are: module, output)",
                kind.escape_debug()
            ),
            Self::BadJson(error) => write!(f, "the value is not JSON: {error}"),
        }
    }
}

/// The `--version` line.
pub fn version_string() -> String {
    format!("scootbar {}", env!("CARGO_PKG_VERSION"))
}

fn is_help(arg: &str) -> bool {
    matches!(arg, "--help" | "-h")
}

/// An argument as UTF-8, or its lossy form for an error message.
fn text(arg: OsString) -> Result<String, String> {
    arg.into_string()
        .map_err(|raw| raw.to_string_lossy().into_owned())
}

/// Parses the arguments after the program name. Takes `OsString`s so a
/// non-UTF-8 argument is an error (or, for `--font`, a path), not a panic.
pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Command, Error> {
    let mut args = args.into_iter();
    let first = match args.next().map(text) {
        None => return Err(Error::Missing),
        Some(Err(lossy)) => return Err(Error::Unknown(lossy)),
        Some(Ok(first)) => first,
    };
    match first.as_str() {
        "--help" | "-h" | "help" => help(args.map(text)),
        "--version" | "-V" => match args.next() {
            None => Ok(Command::Version),
            Some(extra) => Err(Error::Unexpected {
                command: "--version",
                argument: text(extra).unwrap_or_else(|lossy| lossy),
            }),
        },
        "daemon" => daemon(args),
        "msg" => msg(args),
        _ => Err(Error::Unknown(first)),
    }
}

/// `help` / `--help`, optionally followed by one command name.
fn help(mut args: impl Iterator<Item = Result<String, String>>) -> Result<Command, Error> {
    let topic = match args.next() {
        None => Topic::Main,
        Some(Ok(name)) if name == "daemon" => Topic::Daemon,
        Some(Ok(name)) if name == "msg" => Topic::Msg,
        Some(other) => return Err(Error::Unknown(other.unwrap_or_else(|lossy| lossy))),
    };
    match args.next() {
        None => Ok(Command::Help(topic)),
        Some(extra) => Err(Error::Unexpected {
            command: "help",
            argument: extra.unwrap_or_else(|lossy| lossy),
        }),
    }
}

/// Every `daemon` flag's value, as given (each at most once).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Given {
    outputs: Option<Select>,
    edge: Option<Edge>,
    layer: Option<Layer>,
    exclusive: Option<bool>,
    height: Option<u32>,
    margin: Option<Margin>,
    background: Option<Color>,
    foreground: Option<Color>,
    font: Option<PathBuf>,
    font_size: Option<u32>,
    left: Option<Vec<&'static str>>,
    center: Option<Vec<&'static str>>,
    right: Option<Vec<&'static str>>,
    padding: Option<u32>,
    spacing: Option<u32>,
    /// `--config`'s path: not a bar option, so neither `into_config` nor
    /// `overlay` reads it; `main` loads the file with it.
    config: Option<PathBuf>,
    #[cfg(feature = "clock")]
    clock_format: Option<modules::clock::format::Format>,
}

/// `daemon`'s flags: each at most once, as `--flag VALUE` or
/// `--flag=VALUE`; `--help` alone asks for its page.
fn daemon(mut args: impl Iterator<Item = OsString>) -> Result<Command, Error> {
    let mut given = Given::default();
    let mut check = false;
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = text(arg).map_err(|lossy| Error::Unexpected {
            command: "daemon",
            argument: lossy,
        })?;
        if first && is_help(&arg) {
            return match args.next() {
                None => Ok(Command::Help(Topic::Daemon)),
                Some(extra) => Err(Error::Unexpected {
                    command: "daemon",
                    argument: text(extra).unwrap_or_else(|lossy| lossy),
                }),
            };
        }
        first = false;
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(OsString::from(value))),
            _ => (arg.as_str(), None),
        };
        if name == CHECK {
            if inline.is_some() {
                return Err(Error::Unexpected {
                    command: "daemon",
                    argument: arg,
                });
            }
            if check {
                return Err(Error::Repeated(CHECK));
            }
            check = true;
            continue;
        }
        let Some(&flag) = FLAGS.iter().find(|&&flag| flag == name) else {
            return Err(Error::Unexpected {
                command: "daemon",
                argument: arg,
            });
        };
        let Some(raw) = inline.or_else(|| args.next()) else {
            return Err(Error::MissingValue(flag));
        };
        given.take(flag, raw)?;
    }
    let file = given.config.clone();
    let config = given.clone().into_config()?;
    Ok(Command::Daemon(Box::new(DaemonCommand {
        given,
        file,
        config: Box::new(config),
        check,
    })))
}

/// `msg`'s commands, each at most its own arguments; `--help` alone asks
/// for its page.
fn msg(mut args: impl Iterator<Item = OsString>) -> Result<Command, Error> {
    let command = match next_arg(&mut args, "msg") {
        None => return Err(Error::Msg(MsgError::Missing)),
        Some(command) => command?,
    };
    if is_help(&command) {
        return match args.next() {
            None => Ok(Command::Help(Topic::Msg)),
            Some(extra) => Err(Error::Unexpected {
                command: "msg",
                argument: text(extra).unwrap_or_else(|lossy| lossy),
            }),
        };
    }
    let extra = |argument: OsString| Error::Unexpected {
        command: "msg",
        argument: text(argument).unwrap_or_else(|lossy| lossy),
    };
    match command.as_str() {
        "query" => {
            let id = match next_arg(&mut args, "msg") {
                None => None,
                Some(id) => {
                    let id = id?;
                    if !crate::modules::custom::well_formed(&id) {
                        return Err(Error::Msg(MsgError::BadModuleId(id)));
                    }
                    Some(id)
                }
            };
            match args.next() {
                None => Ok(Command::Msg(Msg::Query { id })),
                Some(argument) => Err(extra(argument)),
            }
        }
        "invoke" => {
            let Some(id) = next_arg(&mut args, "msg") else {
                return Err(Error::Msg(MsgError::NeedsId));
            };
            let id = id?;
            if !crate::modules::custom::well_formed(&id) {
                return Err(Error::Msg(MsgError::BadModuleId(id)));
            }
            let Some(action) = next_arg(&mut args, "msg") else {
                return Err(Error::Msg(MsgError::NeedsAction));
            };
            let action = action?;
            let (mut arg, mut output) = (None, None);
            while let Some(next) = next_arg(&mut args, "msg") {
                let next = next?;
                if next == "--output" {
                    let Some(name) = next_arg(&mut args, "msg") else {
                        return Err(Error::Msg(MsgError::NeedsOutput));
                    };
                    if output.replace(name?).is_some() {
                        return Err(Error::Repeated("--output"));
                    }
                } else if arg.is_none() {
                    arg = Some(
                        next.parse::<i32>()
                            .map_err(|_| Error::Msg(MsgError::BadArg(next.clone())))?,
                    );
                } else {
                    return Err(extra(OsString::from(next)));
                }
            }
            Ok(Command::Msg(Msg::Invoke {
                id,
                action,
                arg,
                output,
            }))
        }
        "subscribe" => {
            let mut events = Vec::new();
            while let Some(kind) = next_arg(&mut args, "msg") {
                let kind = kind?;
                let Some(parsed) = crate::control::protocol::EventKind::parse(&kind) else {
                    return Err(Error::Msg(MsgError::UnknownEvent(kind)));
                };
                if !events.contains(&parsed) {
                    events.push(parsed);
                }
            }
            // None named: every kind.
            if events.is_empty() {
                events.extend(crate::control::protocol::EventKind::ALL);
            }
            Ok(Command::Msg(Msg::Subscribe { events }))
        }
        "set" => {
            let Some(id) = next_arg(&mut args, "msg") else {
                return Err(Error::Msg(MsgError::NeedsId));
            };
            let id = id?;
            let Some(value) = next_arg(&mut args, "msg") else {
                return Err(Error::Msg(MsgError::NeedsValue));
            };
            let value = value?;
            if let Some(argument) = args.next() {
                return Err(extra(argument));
            }
            if !crate::modules::custom::well_formed(&id) {
                return Err(Error::Msg(MsgError::BadModuleId(id)));
            }
            if let Err(error) = serde_json::from_str::<serde::de::IgnoredAny>(&value) {
                return Err(Error::Msg(MsgError::BadJson(error.to_string())));
            }
            Ok(Command::Msg(Msg::Set { id, value }))
        }
        other => {
            let request = match other {
                "layout" => Msg::Layout,
                "reload" => Msg::Reload,
                "hide" => Msg::Hide,
                "show" => Msg::Show,
                "toggle" => Msg::Toggle,
                "version" => Msg::Version,
                "kill" => Msg::Kill,
                _ => return Err(Error::Msg(MsgError::Unknown(command))),
            };
            match args.next() {
                None => Ok(Command::Msg(request)),
                Some(argument) => Err(extra(argument)),
            }
        }
    }
}

/// The next argument as UTF-8, `None` when there is none: `Err` is its
/// lossy form for an error message.
fn next_arg(
    args: &mut impl Iterator<Item = OsString>,
    command: &'static str,
) -> Option<Result<String, Error>> {
    args.next().map(|arg| {
        text(arg).map_err(|lossy| Error::Unexpected {
            command,
            argument: lossy,
        })
    })
}

impl Given {
    /// Parses `raw` as `flag`'s value.
    fn take(&mut self, flag: &'static str, raw: OsString) -> Result<(), Error> {
        if flag == FONT || flag == CONFIG {
            // Paths: any bytes will do.
            let slot = if flag == FONT {
                &mut self.font
            } else {
                &mut self.config
            };
            return set(slot, flag, Ok(PathBuf::from(raw)));
        }
        // Every other flag takes text; a value that is not UTF-8 cannot be
        // any of them, and its lossy form says so in the flag's own error.
        let value = text(raw).unwrap_or_else(|lossy| lossy);
        match flag {
            OUTPUTS => set(
                &mut self.outputs,
                flag,
                output_list(&value).map_err(|error| Error::Outputs { value, error }),
            ),
            EDGE => set(
                &mut self.edge,
                flag,
                Edge::parse(&value).ok_or(Error::Edge(value)),
            ),
            LAYER => set(
                &mut self.layer,
                flag,
                Layer::parse(&value).ok_or(Error::Layer(value)),
            ),
            EXCLUSIVE => set(
                &mut self.exclusive,
                flag,
                bar::parse_bool(&value).ok_or(Error::Exclusive(value)),
            ),
            HEIGHT => set(
                &mut self.height,
                flag,
                bar::parse_height(&value).ok_or(Error::Height(value)),
            ),
            MARGIN => set(
                &mut self.margin,
                flag,
                Margin::parse(&value).map_err(|error| Error::Margin { value, error }),
            ),
            BACKGROUND | FOREGROUND => {
                let slot = if flag == BACKGROUND {
                    &mut self.background
                } else {
                    &mut self.foreground
                };
                set(
                    slot,
                    flag,
                    Color::parse(&value).map_err(|error| Error::Color { flag, value, error }),
                )
            }
            FONT_SIZE => set(
                &mut self.font_size,
                flag,
                bar::parse_whole(&value)
                    .filter(|size| (1..=MAX_FONT_SIZE).contains(size))
                    .ok_or(Error::FontSize(value)),
            ),
            PADDING | SPACING => {
                let slot = if flag == PADDING {
                    &mut self.padding
                } else {
                    &mut self.spacing
                };
                set(
                    slot,
                    flag,
                    bar::parse_whole(&value)
                        .filter(|gap| *gap <= MAX_GAP)
                        .ok_or(Error::Gap { flag, value }),
                )
            }
            #[cfg(feature = "clock")]
            CLOCK_FORMAT => set(
                &mut self.clock_format,
                flag,
                modules::clock::format::Format::parse(&value)
                    .map_err(|error| Error::ClockFormat { value, error }),
            ),
            // `--left`, `--center` and `--right`: `FLAGS` has nothing else.
            _ => {
                let slot = match flag {
                    LEFT => &mut self.left,
                    CENTER => &mut self.center,
                    _ => &mut self.right,
                };
                set(
                    slot,
                    flag,
                    module_list(&value).map_err(|error| Error::Modules { flag, error }),
                )
            }
        }
    }

    fn into_config(self) -> Result<Config, Error> {
        let defaults = Config::default();
        let padding = self.padding.unwrap_or(defaults.layout.padding);
        let spacing = self.spacing.unwrap_or(defaults.layout.spacing);
        let layout = if self.left.is_none() && self.center.is_none() && self.right.is_none() {
            Layout {
                padding,
                spacing,
                ..defaults.layout
            }
        } else {
            Layout {
                left: self.left.unwrap_or_default(),
                center: self.center.unwrap_or_default(),
                right: self.right.unwrap_or_default(),
                padding,
                spacing,
                ..defaults.layout
            }
        };
        check_layout(&layout)?;
        #[cfg_attr(not(feature = "clock"), allow(unused_mut))]
        let mut modules = defaults.modules;
        #[cfg(feature = "clock")]
        if let Some(format) = self.clock_format {
            modules.clock.format = format;
        }
        Ok(Config {
            bar: bar::Bar {
                edge: self.edge.unwrap_or(defaults.bar.edge),
                height: self.height.unwrap_or(defaults.bar.height),
                margin: self.margin.unwrap_or(defaults.bar.margin),
                layer: self.layer.unwrap_or(defaults.bar.layer),
                exclusive: self.exclusive.unwrap_or(defaults.bar.exclusive),
            },
            theme: crate::theme::Theme {
                background: self.background.unwrap_or(defaults.theme.background),
                foreground: self.foreground.unwrap_or(defaults.theme.foreground),
                ..defaults.theme
            },
            layout,
            font: self.font,
            fallback_fonts: defaults.fallback_fonts,
            font_size: self.font_size.unwrap_or(defaults.font_size),
            radius: defaults.radius,
            opacity: defaults.opacity,
            #[cfg(feature = "popup")]
            popup_radius: defaults.popup_radius,
            #[cfg(feature = "popup")]
            tooltip_delay: defaults.tooltip_delay,
            modules,
            outputs: crate::policy::Policy {
                select: self.outputs.unwrap_or_default(),
                overrides: Vec::new(),
            },
        })
    }

    /// Applies the given flags over `base` (the config file's, or the
    /// defaults'): each flag replaces its own value. Module sections
    /// replace one by one, unlike [`Given::into_config`]'s whole-layout
    /// rule, so a flag and the file together are still checked for a
    /// module placed twice.
    pub fn overlay(&self, base: &mut Config) -> Result<(), Error> {
        if let Some(select) = &self.outputs {
            base.outputs.select = select.clone();
            // The list over the file's tables: one the list leaves out
            // could never apply.
            base.outputs.check().map_err(Error::OutputsClash)?;
        }
        if let Some(edge) = self.edge {
            base.bar.edge = edge;
        }
        if let Some(layer) = self.layer {
            base.bar.layer = layer;
        }
        if let Some(exclusive) = self.exclusive {
            base.bar.exclusive = exclusive;
        }
        if let Some(height) = self.height {
            base.bar.height = height;
        }
        if let Some(margin) = self.margin {
            base.bar.margin = margin;
        }
        if let Some(background) = self.background {
            base.theme.background = background;
        }
        if let Some(foreground) = self.foreground {
            base.theme.foreground = foreground;
        }
        if let Some(font) = &self.font {
            base.font = Some(font.clone());
        }
        if let Some(font_size) = self.font_size {
            base.font_size = font_size;
        }
        if let Some(padding) = self.padding {
            base.layout.padding = padding;
        }
        if let Some(spacing) = self.spacing {
            base.layout.spacing = spacing;
        }
        #[cfg(feature = "clock")]
        if let Some(format) = &self.clock_format {
            base.modules.clock.format = format.clone();
        }
        if self.left.is_some() || self.center.is_some() || self.right.is_some() {
            if let Some(left) = &self.left {
                base.layout.left = left.clone();
            }
            if let Some(center) = &self.center {
                base.layout.center = center.clone();
            }
            if let Some(right) = &self.right {
                base.layout.right = right.clone();
            }
            check_layout(&base.layout)?;
        }
        Ok(())
    }
}

/// `--outputs`: `all`, or comma-separated connector names, each at most
/// once. Compared with the file's tables after the overlay.
fn output_list(value: &str) -> Result<Select, OutputsError> {
    if value == "all" {
        return Ok(Select::All);
    }
    let mut names = Vec::new();
    for name in value.split(',') {
        if name.is_empty() {
            return Err(OutputsError::Empty);
        }
        if names.len() >= MAX_OUTPUTS {
            return Err(OutputsError::Policy(PolicyError::TooManyOutputs));
        }
        policy::check_name(name).map_err(|e| OutputsError::Policy(PolicyError::Name(e)))?;
        if names.iter().any(|earlier| earlier == name) {
            return Err(OutputsError::Policy(PolicyError::Twice(name.to_owned())));
        }
        names.push(name.to_owned());
    }
    Ok(Select::Named(names))
}

/// A comma-separated list of module ids; empty is no modules.
fn module_list(value: &str) -> Result<Vec<&'static str>, ModulesError> {
    let mut ids = Vec::new();
    if value.is_empty() {
        return Ok(ids);
    }
    for id in value.split(',') {
        let spec = modules::find(id).ok_or_else(|| ModulesError::Unknown(id.to_owned()))?;
        if ids.len() >= MAX_MODULES {
            return Err(ModulesError::TooMany);
        }
        ids.push(spec.id);
    }
    Ok(ids)
}

/// No module twice across the sections, and at most [`MAX_MODULES`].
fn check_layout(layout: &Layout) -> Result<(), Error> {
    crate::layout::check_placement(layout).map_err(|(section, error)| Error::Modules {
        flag: section.flag(),
        error: match error {
            PlacementError::Twice(id) => ModulesError::Twice(id),
            PlacementError::TooMany => ModulesError::TooMany,
        },
    })
}

/// Stores a flag's parsed value, refusing a repeat before a bad value (so
/// `--height 1 --height x` says "given twice", the first mistake made).
fn set<T>(slot: &mut Option<T>, flag: &'static str, value: Result<T, Error>) -> Result<(), Error> {
    if slot.is_some() {
        return Err(Error::Repeated(flag));
    }
    *slot = Some(value?);
    Ok(())
}
