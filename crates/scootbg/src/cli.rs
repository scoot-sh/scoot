//! Argument parsing, hand-rolled (dependencies-done.md §4: +12 KB against
//! +299 KB for `clap`). One binary: `daemon` runs the Wayland client, and
//! every other command is a client of its control socket.
//!
//! `apply-config` is scoot's one command
//! (docs/scootbg/backlog/resolved/scoot-integration-done.md); its `--serve`
//! flag is internal, the detached daemon it starts (`crate::apply`), and is
//! not advertised.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt;

use crate::color::{Color, ColorError};
use crate::image::{Filter, Mode};
use crate::protocol::{
    DEFAULT_FILL, ImageRequest, PROTOCOL_VERSION, Request, Show, SlideshowRequest, Source,
};
use crate::rotation::EveryError;
use crate::section::{Section, SectionError};
use crate::state::{Profile, ProfileError};

#[cfg(test)]
mod tests;

/// Everything in [`usage`] before the SEE ALSO tail: the commands, the
/// socket, one example per shape, the exit codes and the environment.
const USAGE_BODY: &str = "\
scootbg -- wallpaper daemon for Wayland

Early days: colors and images (PNG, JPEG, GIF, WebP; an animated GIF,
APNG or animated WebP shows its first frame) work, and the daemon
shows the last ones again when it next starts.

USAGE:
    scootbg COMMAND
    scootbg COMMAND --help
    scootbg --version
    scootbg --help [--json]
    scootbg help [COMMAND|--json]

COMMANDS:
    daemon     run the daemon for this Wayland display
    set        show a color or an image on every output, or on one
    clear      back to the compositor's own background
    query      print what each output shows, as JSON
    version    print the running daemon's version and protocol, as JSON
    kill       stop the running daemon
    apply-config
               apply scoot's [wallpaper] section, starting the daemon if
               none runs (what scoot runs; see `scootbg apply-config --help`)

The daemon listens on $XDG_RUNTIME_DIR/scootbg-NAME.sock, where NAME is the
last component of $WAYLAND_DISPLAY (wayland-0 when unset); every other
command talks to it there.

EXAMPLES:
    scootbg daemon
    scootbg set '#1e1e2e'
    scootbg query
    scootbg kill

EXIT CODES:
    0  success: the reply is on stdout (help and --version count)
    1  the run failed: no daemon, a refused value, drawing failed
    2  usage error: an unknown command, flag or value (the error names it)

ENVIRONMENT:
    WAYLAND_DISPLAY  the compositor to show the wallpaper on
    XDG_RUNTIME_DIR  where the control socket lives (scootbg-NAME.sock)
    XDG_STATE_HOME   where profiles are saved (~/.local/state/scootbg)

SEE ALSO:
    `scootbg help set`, `scootbg set --help`, `scootbg --help --json`
";

/// The main help page: [`USAGE_BODY`] plus the SEE ALSO tail, whose URLs
/// render from [`scoot_ipc::DOCS_URL`] -- the one copy of the docs domain,
/// so a move is one edit. A function rather than a `const` so the text can
/// name that constant; a cold path (one process per `--help`), so the one
/// small allocation costs nothing.
pub fn usage() -> String {
    let mut text = String::with_capacity(USAGE_BODY.len() + 128);
    text.push_str(USAGE_BODY);
    text.push_str(&scoot_ipc::docs_tail("scootbg/cli.md"));
    text
}

/// Everything in [`daemon_help`] before the SEE ALSO tail: the usage, the
/// profiles, the flags, the example, the exit codes.
const DAEMON_BODY: &str = "\
scootbg daemon -- run the wallpaper daemon

USAGE:
    scootbg daemon [--profile NAME] [--no-restore]

Connects to the compositor named by $WAYLAND_DISPLAY, which must support
wlr-layer-shell, and serves requests on $XDG_RUNTIME_DIR/scootbg-NAME.sock.
Each output gets one background-layer surface, kept across outputs coming
and going; with no outputs at all the daemon waits for one.

Every `set` and `clear` is saved, per output, in the profile's state file,
$XDG_STATE_HOME/scootbg/PROFILE (~/.local/state/scootbg/PROFILE when
XDG_STATE_HOME is unset, empty or relative), and the daemon shows it again
when it next starts. If there is no state directory to use, or the file
cannot be used (unreadable, or written by a newer scootbg), saving is off
until the daemon restarts, stderr says how to recover, and `scootbg query`
reports \"saving\":false. A saved image that is gone (moved, deleted) is
skipped with a warning on stderr, and that output shows the compositor's
own background; the daemon starts all the same, and the entry stays saved
until a `set` or `clear` replaces it. Choices for outputs that are not
plugged in stay saved too.

    --profile NAME  which state to restore and save (default: default).
                    Sessions with different profiles never restore each
                    other's wallpaper; two sessions sharing one share it,
                    the last change winning. NAME is 1 to 64 of A-Z, a-z,
                    0-9, '.', '_' and '-', not starting with '.' and
                    without '..'
    --no-restore    start with nothing shown; the state is still read, and
                    a `set` or `clear` then updates it as usual

Runs until `scootbg kill` (exit status 0) or until the compositor goes
away (exit status 1); either way the socket is removed. SIGTERM, SIGINT and
SIGHUP end it at once and leave the socket file behind; that is harmless:
other commands then report no daemon running, and the next `scootbg daemon`
replaces the file.

One daemon per display: a second one exits with an error while the first
is alive. A socket left behind by a daemon that crashed is replaced.

EXAMPLE:
    scootbg daemon --profile work

EXIT CODES:
    0  `scootbg kill` stopped it (help counts too)
    1  the compositor went away, or no second daemon while one runs
    2  usage error: an unknown flag or profile (the error names it)

SEE ALSO:
    `scootbg help daemon` prints this page; `scootbg --help --json` is the
    machine-readable form
";

/// The daemon help page: [`DAEMON_BODY`] plus the SEE ALSO tail, whose URLs
/// render from [`scoot_ipc::DOCS_URL`] -- the one copy of the docs domain,
/// so a move is one edit. A function rather than a `const` so the text can
/// name that constant; a cold path (one process per `--help`), so the one
/// small allocation costs nothing.
pub fn daemon_help() -> String {
    let mut text = String::with_capacity(DAEMON_BODY.len() + 128);
    text.push_str(DAEMON_BODY);
    text.push_str(&scoot_ipc::docs_tail("scootbg/cli.md"));
    text
}

pub const SET_HELP: &str = "\
scootbg set -- show a color, an image or a rotating directory, on every output, or on one

USAGE:
    scootbg set '#rrggbb' [--output NAME] [--workspace NAME] [--transition KIND ...]
    scootbg set PATH [--output NAME] [--workspace NAME] [--mode MODE] [--fill '#rrggbb']
                     [--filter FILTER] [--no-animate] [--transition KIND ...]
    scootbg set URL [--output NAME] [--workspace NAME] [--mode MODE] [--fill '#rrggbb']
                    [--filter FILTER] [--sha256 HEX] [--no-animate] [--transition KIND ...]
    scootbg set DIR --every DURATION [--shuffle] [--output NAME] [--mode MODE]
                    [--fill '#rrggbb'] [--filter FILTER] [--transition KIND ...]

    KIND ... is [--duration-ms MS] [--easing EASING] [--angle DEGREES]
               [--position X,Y]

An argument starting with '#' is a color: '#' and six hex digits, either
case, such as '#1e1e2e'; quote it, since the shell reads '#' as a comment.
Wallpapers are opaque, so there is no alpha.

A directory (with --every) is a slideshow: its regular files in turn, one
every DURATION, on one timer, without polling the directory. DURATION is a
number and `s`, `m`, `h` or `d`, such as `30m`: at least `1m`, whole
minutes, at most `7d`. The files are tried sorted by name, or shuffled
once with --shuffle; a file that is not an image fails to draw when its
turn comes (as a `set` of it would) until the next rotation. An animated
file shows its first frame, checked per step like one `set` (past 64
frames or 64 MiB of frames that step fails); `--no-animate` with
`--every` is a usage error: stilling is per image. At most
10,000 files are listed: a larger directory is refused, naming the cap,
rather than stalling the daemon's loop to list it. The first
file shows before `set` returns, as an image does. A new `set`, a `clear`
or a changed `apply-config` stops the slideshow, as does the directory
going away (at the next step, said on stderr: restarting the show takes a
fresh `set`); restarting the daemon
shows the last image, without resuming it. One slideshow runs at a time.

An `http://` or `https://` URL is downloaded once and cached
(`$XDG_CACHE_HOME/scootbg/`, else `~/.cache/scootbg/`): a file already
there is shown, otherwise `curl` fetches it on a worker thread (off the
event loop), the download verified against `--sha256 HEX` (64 hex digits,
as `sha256sum` prints) when given, written atomically, then shown. Until
then — and when the fetch fails — the compositor's background shows and
the error says why; nothing is retried in a loop (a new `set`, a changed
section, a reconfigured output, or a restart tries again). `file://` and
other schemes are not fetched. Prefer `https`, and pin `--sha256`.

Anything else is the path of an image: PNG, JPEG, GIF or WebP, told
apart by content, not by name. An animated GIF, APNG or animated WebP
shows its first frame (up to 64 frames and 64 MiB of frames; past that
the `set` is refused and `--no-animate` shows the first frame instead;
playing frame by frame is a follow-up). It is made
absolute here, so a relative path means from this directory. A file whose
name starts with '#' is given as './#name.png'. The file is read when the
daemon gets the request (and again for an output plugged in later), not
kept in memory. EXIF orientation is applied (a JPEG's, a WebP's, or a
PNG's eXIf chunk).

    --mode MODE      how the image fits the output:
                       fill     cover it, cropping what overflows, centred
                                (the default)
                       fit      all of it, as large as fits, centred, the
                                rest in the fill color
                       stretch  the output's size, whatever the aspect
                       center   unscaled, centred: cropped if larger, the
                                rest in the fill color
                       tile     unscaled, repeated from the top-left corner
    --fill '#rrggbb' the color around a fitted or centred image, and under
                     a transparent one (default '#000000')
    --filter FILTER  the scaling filter: lanczos3 (the default), catmull-rom,
                     bilinear or nearest (hard pixels, for pixel art)
    --no-animate     show the first frame only: a still with zero idle
                     cost, for an animation past the size cap or where
                     stillness is wanted (one image only: a color or a
                     slideshow takes none)
    --sha256 HEX     pin a URL's bytes (64 hex digits, as `sha256sum`
                     prints): anything else fails instead of showing
    --every DURATION rotate through a directory: its files in turn, one
                     every DURATION, such as `30m` (a number and `s`, `m`,
                     `h` or `d`; at least `1m`, whole minutes, at most `7d`)
    --shuffle        cycle the directory in a shuffled order, shuffled once
                     when set (needs --every)
    --transition KIND
                     how the new wallpaper arrives: none (the default: it
                     lands at once), fade, wipe or grow. With a kind, the
                     change animates from what is on screen now; a newer
                     `set` mid-transition starts from the frame showing
                     then, never queued behind it
    --duration-ms MS how long the transition takes, in milliseconds
                     (default 500; 0 lands at once; at most 60000)
    --easing EASING  how it moves through the time: linear, ease-in,
                     ease-out (the default), ease-in-out or smooth
    --angle DEGREES  a wipe's direction in degrees, clockwise from the
                     positive x-axis: 0 (the default) wipes in from the
                     left edge, 90 from the top, 180 from the right, 270
                     from the bottom
    --position X,Y   where a grow starts, as fractions of the width and
                     height (default 0.5,0.5, the center)

Without --output, every output shows it, including outputs plugged in
later, and any choice made for a single output is replaced. With --output
NAME (a connector name, as `scootbg query` lists them), only that output
does, and it keeps it when it is unplugged and plugged back in. A name that
no output has now is an error, and nothing is changed.

With --workspace NAME, the wallpaper is for one workspace instead: while
the workspace NAME (the name the compositor announces, \"1\", \"2\", ... on
scoot, as `scootbg query` lists them per output) is active, the targeted
outputs show it, arriving through the transition given here when the
workspace turns active; otherwise they show their own wallpaper. With
--output as well, only that output does. The daemon follows the
compositor's ext-workspace-v1 protocol to learn which workspace is
active, so this works on any compositor with it, not only scoot; without
it the mapping waits (and is still saved) until one does. A newer `set`
covers an older `set --workspace`, and a newer `set --workspace` covers
an older `set`: whichever you changed last wins. `scootbg clear
--workspace NAME` takes the mapping back off.

Returns once every targeted output shows it and the compositor has
processed it, so a screenshot taken straight after shows it. With a
transition, that means once the animation has finished: the reply waits
for the last frame, not the first. An image that
cannot be shown (no such file, not an image, too large, truncated or
corrupt) is an error, and every output keeps what it showed. A directory
with no files in it is an error too, as is one past the 10,000-file
listing cap or with an entry that cannot be read, changing nothing;
with files, the
first shows before `set` returns, and the rest follow on the timer, each
step animating through `--transition` like one image's `set`. When the
first file itself cannot be shown, the slideshow still starts (the reply
says so, and the rotation changes): that file fails its turns like any
file that is not an image, until the next rotation. An output
unplugged meanwhile is left out of that wait; an output whose surface is
not configured yet is waited for, until a round trip after scootbg made
it (one the compositor is slower than that to configure is drawn when it
is, and does not hold up the reply); an output scootbg gave up on (`gave-up`
in `scootbg query`, said on stderr) is left out, shows nothing, and does
not change the exit status. When a newer `set` or `clear` replaces the
choice before this image is shown, this one changes nothing (it may never
be decoded) and returns with status 0 once the newer one is shown, as a
replaced color does. Prints nothing on success.

The choice is saved and shown again when the daemon next starts (see
`scootbg daemon --help`): a color at once, an image once it has decoded.
A `--no-animate` still is live-only: after a restart the animation caps
are checked again, so an over-cap animation is refused then and the
output loses its wallpaper until the next `set`.

Exit status: 0 once shown; 1 when no daemon is running, the output is
unknown, the image cannot be shown, the directory holds no files, holds
more than 10,000 files, or has an unreadable entry, or
drawing failed (the daemon's stderr
says why); 2 for a usage error, such as a malformed color, an unknown
mode, a directory without --every, an --every without a directory, or a
--no-animate with --every.

EXAMPLES:
    scootbg set '#1e1e2e'
    scootbg set ~/wallpapers/sunset.jpg --mode fill
    scootbg set ~/wallpapers/grid.png --mode tile --output DP-1
    scootbg set ~/wallpapers --every 30m --shuffle
    scootbg set '#101014' --transition fade --duration-ms 800
    scootbg set ~/wallpapers/city.png --transition wipe --angle 90

SEE ALSO:
    `scootbg help set` prints this page; `scootbg --help --json` is the
    machine-readable form
";

pub const CLEAR_HELP: &str = "\
scootbg clear -- back to the compositor's own background

USAGE:
    scootbg clear
    scootbg clear --output NAME
    scootbg clear --workspace NAME [--output NAME]

Takes the wallpaper off every output (without --output, including any
choice made for a single output), or off the output named NAME, so the
compositor's own background shows. With --workspace NAME, takes only the
mapping for that workspace off (that output's own wallpaper shows while
it is active); without --output, for every output. The daemon keeps running. A `clear`
lands at once and takes no transition: there is no wallpaper to blend
from or to. An output
plugged in later shows nothing until the next `scootbg set`, unless a
color was set for it by name.

Returns once the compositor has processed it, like `set`, and prints
nothing on success. Saved like a `set`, so the daemon starts with it
cleared. Exit status as for `set`.

SEE ALSO:
    `scootbg help clear` prints this page; `scootbg --help --json` is the
    machine-readable form
";

pub const QUERY_HELP: &str = "\
scootbg query -- print what each output shows

USAGE:
    scootbg query

Prints the daemon's reply, one line of JSON with one entry per output:
{\"type\":\"outputs\",\"outputs\":[...],\"workspaces\":[...],
\"saving\":true,\"profile\":\"default\"}
Each entry has the output's name, description, mode, scale, transform and
logical size, its surface's state: waiting, pending, configured (with its
size), closed or gave-up (until the output is replugged), whether drawing
what it should show failed (draw_failed) and why (draw_error, as the
daemon's stderr says it), and what it shows: {\"color\":\"#rrggbb\"},
{\"image\":\"/path\",\"mode\":\"fill\",\"fill\":\"#rrggbb\",
\"filter\":\"lanczos3\"}, or null for nothing. `workspace` names the
workspace active on the output now (\"1\", \"2\", ... on scoot), or null
while unknown. After the outputs, `workspaces` lists the live
per-workspace mappings (`scootbg set --workspace`): each with its output
(null for every output), its workspace, and what it shows. After the
lists, \"saving\"
says whether changes are saved for the next start (see `scootbg daemon
--help`), and \"profile\" whose state is restored and saved: the
daemon's --profile, or the last one an `apply-config` made it adopt.

SEE ALSO:
    `scootbg help query` prints this page; `scootbg --help --json` is the
    machine-readable form
";

pub const VERSION_HELP: &str = "\
scootbg version -- ask the running daemon for its version

USAGE:
    scootbg version

Prints the daemon's reply, one line of JSON:
{\"type\":\"version\",\"protocol\":N,\"version\":\"X.Y.Z\"}
`scootbg --version` prints this binary's own version without a daemon.

SEE ALSO:
    `scootbg help version` prints this page; `scootbg --help --json` is the
    machine-readable form
";

pub const KILL_HELP: &str = "\
scootbg kill -- stop the running daemon

USAGE:
    scootbg kill

Returns once the daemon has removed its socket and closed the connection,
so a new `scootbg daemon` can start straight after.

Exit status: 0 once the daemon has stopped; 1 with \"no scootbg daemon is
running\" when none answers (none started, or it died and left its socket
file behind); 1 for any other error.

SEE ALSO:
    `scootbg help kill` prints this page; `scootbg --help --json` is the
    machine-readable form
";

pub const APPLY_CONFIG_HELP: &str = "\
scootbg apply-config -- apply scoot's [wallpaper] section

USAGE:
    scootbg apply-config [--profile NAME] JSON

What scoot runs at start-up and on every reload while its config has a
[wallpaper] section; you rarely need it yourself. JSON is the section, as
one object ('{}' when the section is gone):

    {\"image\": \"/abs/path.jpg\", \"mode\": \"fill\",
     \"output\": {\"DP-2\": {\"color\": \"#101014\"}},
     \"command\": \"scootbg\"}

At the top, the wallpaper for every output: \"image\" (an absolute path)
or \"color\" (\"#rrggbb\"), never both, and for an image \"mode\", \"fill\"
and \"filter\", as `scootbg set` takes them; neither is nothing (the
compositor's own background). \"output\" holds per-output tables by
connector name, each the same keys; an empty one means nothing there. Each
table stands alone: an output's image does not take the top level's mode.
\"command\" is scoot's, and ignored. Anything else is refused: an unknown
key, a key given twice, null, a relative path, a malformed color, an
unknown mode or filter, more than 256 outputs, over 63 KiB.

Whichever you changed last wins: the section is applied only if it changed
since the last apply-config for this profile (a fingerprint of it is kept
in the profile's state, see `scootbg daemon --help`). Unchanged, what
shows stays, so a `scootbg set` made since survives restarts and reloads
until you next change the section itself. \"command\" is not part of the
fingerprint.

When a daemon answers, it is sent the section and adopts --profile NAME
(default: default): from then on it restores and saves that profile's
state, whatever profile it started with. When none does, apply-config
starts one, detached (its own session, its stderr this command's), with
the section as its starting point, then sends it the section as above.
Two started at once settle it by the daemon's lock: one daemon runs, and
both sections reach it. With no daemon and an empty section ('{}', or only
\"command\"), it starts none: it records the clear in the profile's state
file, so a later `scootbg daemon --profile NAME` shows nothing.

A daemon from another scootbg build is reported on stderr: a different
version is a warning (the section is still sent); a different protocol, or
a daemon too old to know apply-config, is an error. A caller that reads
this command's stderr to its end waits for a daemon it started too, which
writes there: send stderr to a file or a log, not a pipe read to the end.

Returns once every output shows what it should and the compositor has
processed it, as `set` does; prints nothing on success.

Exit status: 0 applied, or unchanged, and shown; 1 no daemon could be
started or reached within 5 s, no reply within 30 s, the daemon closed the
connection before answering, a daemon from another protocol or too old,
an image in the section that is not a file (the rest is applied; every
run says so until the file is back, and the first run after shows it,
unless a `scootbg set` has replaced it there since), drawing failed, or
the state file could not be written; 2 for a usage error, the section
refused above included.

SEE ALSO:
    `scootbg help apply-config` prints this page; `scootbg --help --json`
    is the machine-readable form
";

/// Which help text to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    Main,
    Daemon,
    Set,
    Clear,
    Query,
    Version,
    Kill,
    ApplyConfig,
    /// `--help --json` (or `help --json`): the same content as JSON,
    /// versioned (see [`crate::help::SCHEMA_VERSION`]).
    Json,
}

impl Topic {
    pub fn text(self) -> Cow<'static, str> {
        match self {
            Self::Main => Cow::Owned(usage()),
            Self::Daemon => Cow::Owned(daemon_help()),
            Self::Set => Cow::Borrowed(SET_HELP),
            Self::Clear => Cow::Borrowed(CLEAR_HELP),
            Self::Query => Cow::Borrowed(QUERY_HELP),
            Self::Version => Cow::Borrowed(VERSION_HELP),
            Self::Kill => Cow::Borrowed(KILL_HELP),
            Self::ApplyConfig => Cow::Borrowed(APPLY_CONFIG_HELP),
            Self::Json => Cow::Owned(crate::help::json()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help(Topic),
    /// `--version`: this binary, answered locally.
    Version,
    Daemon(DaemonOptions),
    /// A request for the running daemon.
    Client(Request<'static>),
    /// `apply-config` (`crate::apply`).
    ApplyConfig(ApplyOptions),
}

/// `scootbg apply-config`'s arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyOptions {
    pub profile: Profile,
    /// Boxed: the section is by far the largest variant.
    pub section: Box<Section>,
    /// `--serve` (internal): this process is the detached daemon an
    /// `apply-config` started.
    pub serve: bool,
}

/// `scootbg daemon`'s flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonOptions {
    /// Whose state to restore and save (`crate::state`).
    pub profile: Profile,
    /// `--no-restore` makes it false.
    pub restore: bool,
}

impl Default for DaemonOptions {
    /// Plain `scootbg daemon`: the default profile, restored.
    fn default() -> Self {
        Self {
            profile: Profile::default(),
            restore: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Missing,
    Unknown(String),
    /// An unknown word with a guess attached: what command it was meant
    /// for, the closest valid choice, and the help topic that lists them.
    /// Used at parse sites only; garbage keeps the bare [`Error::Unknown`]
    /// shape.
    Hint {
        command: &'static str,
        what: String,
        suggestion: String,
        topic: &'static str,
    },
    Unexpected {
        command: &'static str,
        argument: String,
    },
    /// `set` with neither a color nor a path.
    MissingTarget,
    /// `set` with something that is not a color.
    Color {
        argument: String,
        error: ColorError,
    },
    /// `--mode` or `--filter` with a value it does not take.
    BadValue {
        flag: &'static str,
        value: String,
    },
    /// A transition flag without `--transition`.
    TransitionOnly(&'static str),
    /// `--mode`, `--fill` or `--filter` with a color.
    ImageOnly(&'static str),
    /// `--sha256` that is not 64 hex digits.
    BadSha(String),
    /// `--sha256` with a color or a file: it pins a download.
    ShaImageOnly,
    /// `--every` that is not a rotation pace.
    BadEvery(EveryError),
    /// `--every` with a color, a file or a link: it paces a slideshow
    /// through a directory.
    EveryNeedsDirectory(String),
    /// A directory without `--every`: it is only ever a slideshow.
    DirectoryNeedsEvery(String),
    /// `--shuffle` without `--every`: it shuffles a slideshow.
    ShuffleNeedsEvery,
    /// `--workspace` with a directory slideshow: slideshows run on every
    /// output (or one `--output`), not per workspace; map images or colors
    /// per workspace instead.
    SlideshowWithWorkspace(String),
    /// `--no-animate` with `--every`: stilling is per image, and a
    /// slideshow steps through many (each checked like one `set`).
    NoAnimateWithEvery,
    /// A URL with a NUL byte.
    UrlNul,
    /// `--workspace` empty, too long, or with a NUL byte.
    BadWorkspace {
        value: String,
        reason: String,
    },
    /// A path that the control protocol (JSON) cannot carry.
    NotUtf8(String),
    /// The path could not be made absolute (no working directory).
    Path {
        argument: String,
        error: String,
    },
    /// A flag given without its value.
    MissingValue {
        command: &'static str,
        flag: &'static str,
    },
    /// A flag given twice.
    Repeated {
        command: &'static str,
        flag: &'static str,
    },
    /// `--profile` with a name that cannot be one.
    Profile(ProfileError),
    /// `apply-config` without its JSON.
    MissingSection,
    /// `apply-config` with JSON that is not UTF-8.
    SectionNotUtf8,
    /// `apply-config` with a section that is refused.
    Section(SectionError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "missing command (try --help)"),
            Self::Unknown(what) => write!(f, "unknown command `{what}` (try --help)"),
            Self::Hint {
                command,
                what,
                suggestion,
                topic,
            } => write!(
                f,
                "unexpected `{what}` for `{command}` (did you mean `{suggestion}`? see `{topic}`)"
            ),
            Self::Unexpected { command, argument } => write!(
                f,
                "unexpected argument `{argument}` for `{command}` (try `scootbg {command} --help`)"
            ),
            Self::MissingTarget => write!(
                f,
                "`set` needs a color, such as '#1e1e2e', or an image path \
                 (try `scootbg set --help`)"
            ),
            Self::Color { argument, error } => {
                write!(f, "`{argument}`: {error} (try `scootbg set --help`)")
            }
            Self::BadValue { flag, value } => {
                let takes = if *flag == MODE {
                    "fill, fit, stretch, center or tile"
                } else if *flag == FILTER {
                    "lanczos3, catmull-rom, bilinear or nearest"
                } else if *flag == TRANSITION {
                    "none, fade, wipe or grow"
                } else if *flag == DURATION_MS {
                    "milliseconds as digits, 0 to 60000"
                } else if *flag == EASING {
                    "linear, ease-in, ease-out, ease-in-out or smooth"
                } else if *flag == ANGLE {
                    "degrees as a number"
                } else {
                    "two fractions of the width and height as `X,Y`"
                };
                write!(
                    f,
                    "`{flag}` takes {takes}, not `{value}` (try `scootbg set --help`)"
                )
            }
            Self::TransitionOnly(flag) => write!(
                f,
                "`{flag}` needs `--transition` (try `scootbg set --help`)"
            ),
            Self::ImageOnly(flag) => write!(
                f,
                "`{flag}` applies to an image, not a color (try `scootbg set --help`)"
            ),
            Self::BadSha(value) => write!(
                f,
                "`--sha256` takes 64 hex digits (as `sha256sum` prints), not `{value}` \
                 (try `scootbg set --help`)"
            ),
            Self::ShaImageOnly => write!(
                f,
                "`--sha256` pins a downloaded image, and this one is not a URL \
                 (try `scootbg set --help`)"
            ),
            Self::BadEvery(error) => write!(f, "`--every`: {error} (try `scootbg set --help`)"),
            Self::EveryNeedsDirectory(target) => write!(
                f,
                "`--every` rotates through a directory, and `{target}` is not one \
                 (try `scootbg set --help`)"
            ),
            Self::DirectoryNeedsEvery(target) => write!(
                f,
                "`{target}` is a directory: add `--every` to rotate through it, such as \
                 `--every 30m` (try `scootbg set --help`)"
            ),
            Self::ShuffleNeedsEvery => write!(
                f,
                "`--shuffle` shuffles a slideshow, and there is none: add `--every` \
                 (try `scootbg set --help`)"
            ),
            Self::SlideshowWithWorkspace(target) => write!(
                f,
                "`{target}` is a directory: a slideshow runs on every output (or one \
                 `--output`), not per workspace; map an image or a color with \
                 `--workspace` instead (try `scootbg set --help`)"
            ),
            Self::NoAnimateWithEvery => write!(
                f,
                "`--no-animate` stills one image, and a slideshow steps through a \
                 directory (each step checked like one `set`): drop `--every` to \
                 still a file (try `scootbg set --help`)"
            ),
            ),
            Self::UrlNul => write!(f, "the image URL has a NUL byte"),
            Self::BadWorkspace { value, reason } => write!(
                f,
                "`--workspace {value:?}`: {reason} (try `scootbg set --help`)"
            ),
            Self::NotUtf8(lossy) => write!(
                f,
                "`{lossy}`: the path is not valid UTF-8, which the control protocol cannot \
                 carry; rename the file or link to it from a UTF-8 path"
            ),
            Self::Path { argument, error } => {
                write!(f, "`{argument}`: cannot make the path absolute: {error}")
            }
            Self::MissingValue { command, flag } => {
                write!(f, "`{flag}` needs a value (try `scootbg {command} --help`)")
            }
            Self::Repeated { command, flag } => {
                write!(f, "`{flag}` given twice (try `scootbg {command} --help`)")
            }
            Self::Profile(error) => write!(f, "`{PROFILE}`: {error}"),
            Self::MissingSection => write!(
                f,
                "`apply-config` needs the [wallpaper] section as JSON, '{{}}' for none \
                 (try `scootbg apply-config --help`)"
            ),
            Self::SectionNotUtf8 => write!(f, "`apply-config`: the JSON is not valid UTF-8"),
            Self::Section(error) => write!(
                f,
                "apply-config: {error} (try `scootbg apply-config --help`)"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// The `--version` line: this build and the protocol it speaks.
pub fn version_string() -> String {
    format!(
        "scootbg {} (protocol {PROTOCOL_VERSION})",
        env!("CARGO_PKG_VERSION")
    )
}

fn is_help(arg: &str) -> bool {
    // Bare `help` asks like `--help` does, wherever a help flag is taken:
    // `scootbg set help`, `scootbg daemon help`. No flag is named `help`,
    // so nothing else can mean it there; a file literally named `help` is
    // given as `./help`.
    matches!(arg, "--help" | "-h" | "help")
}

/// An unknown word with a guess attached: [`Error::Hint`] naming the
/// closest candidate, or the bare error the site used before (`or`) when
/// nothing is close enough to be a typo rather than a guess.
fn hinted(
    command: &'static str,
    what: String,
    candidates: &[&str],
    topic: &'static str,
    or: impl FnOnce(String) -> Error,
) -> Error {
    match scoot_ipc::suggest(&what, candidates.iter().copied()) {
        Some(suggestion) => Error::Hint {
            command,
            what,
            suggestion: suggestion.to_owned(),
            topic,
        },
        None => or(what),
    }
}

/// Every command name, for `did you mean` over commands.
const COMMAND_NAMES: &[&str] = &[
    "daemon",
    "set",
    "clear",
    "query",
    "version",
    "kill",
    "apply-config",
];

/// `COMMAND --help --json`: the full document, or the refusal when more
/// follows. Every command's `--help` spells this the same way.
fn help_or_json(
    command: &'static str,
    topic: Topic,
    mut args: impl Iterator<Item = Result<String, String>>,
) -> Result<Command, Error> {
    match args.next() {
        None => Ok(Command::Help(topic)),
        Some(Ok(next)) if next == "--json" => match args.next() {
            None => Ok(Command::Help(Topic::Json)),
            Some(extra) => Err(Error::Unexpected {
                command,
                argument: extra.unwrap_or_else(|lossy| lossy),
            }),
        },
        Some(extra) => Err(Error::Unexpected {
            command,
            argument: extra.unwrap_or_else(|lossy| lossy),
        }),
    }
}

/// Parses the arguments after the program name. Takes `OsString`s so a
/// non-UTF-8 argument is an error, not a panic.
pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Command, Error> {
    let mut args = args.into_iter().map(|arg| {
        arg.into_string()
            .map_err(|raw| raw.to_string_lossy().into_owned())
    });
    let first = match args.next() {
        None => return Err(Error::Missing),
        Some(Err(lossy)) => return Err(Error::Unknown(lossy)),
        Some(Ok(first)) => first,
    };
    let (command, topic): (Command, Topic) = match first.as_str() {
        "--help" | "-h" | "help" => return help(args),
        "--version" | "-V" => (Command::Version, Topic::Main),
        "daemon" => return daemon(args),
        "set" => return change("set", Topic::Set, args),
        "clear" => return change("clear", Topic::Clear, args),
        "query" => (Command::Client(Request::Query), Topic::Query),
        "version" => (Command::Client(Request::Version), Topic::Version),
        "kill" => (Command::Client(Request::Kill), Topic::Kill),
        "apply-config" => return apply_config(args),
        _ => {
            return Err(hinted(
                "scootbg",
                first,
                &[
                    "daemon",
                    "set",
                    "clear",
                    "query",
                    "version",
                    "kill",
                    "apply-config",
                    "--version",
                    "-V",
                    "--help",
                    "-h",
                    "help",
                ],
                "scootbg --help",
                Error::Unknown,
            ));
        }
    };
    let name = match &command {
        Command::Version => "--version",
        Command::Daemon(_) => "daemon",
        Command::Client(request) => request.name(),
        Command::Help(_) => "help",
        Command::ApplyConfig(_) => "apply-config",
    };
    match args.next() {
        None => Ok(command),
        Some(Ok(arg)) if is_help(&arg) && command != Command::Version => {
            help_or_json(name, topic, args)
        }
        Some(other) => Err(Error::Unexpected {
            command: name,
            argument: other.unwrap_or_else(|lossy| lossy),
        }),
    }
}

/// `help` / `--help`, optionally followed by one command name or `--json`.
fn help<I: Iterator<Item = Result<String, String>>>(mut args: I) -> Result<Command, Error> {
    let topic = match args.next() {
        None => return Ok(Command::Help(Topic::Main)),
        Some(Ok(name)) if name == "help" => return Ok(Command::Help(Topic::Main)),
        Some(Ok(name)) if name == "--json" => match args.next() {
            None => return Ok(Command::Help(Topic::Json)),
            Some(extra) => {
                return Err(Error::Unexpected {
                    command: "help",
                    argument: extra.unwrap_or_else(|lossy| lossy),
                });
            }
        },
        Some(Ok(name)) => match name.as_str() {
            "daemon" => Topic::Daemon,
            "set" => Topic::Set,
            "clear" => Topic::Clear,
            "query" => Topic::Query,
            "version" => Topic::Version,
            "kill" => Topic::Kill,
            "apply-config" => Topic::ApplyConfig,
            _ => {
                return Err(hinted(
                    "scootbg help",
                    name,
                    COMMAND_NAMES,
                    "scootbg --help",
                    Error::Unknown,
                ));
            }
        },
        Some(Err(lossy)) => return Err(Error::Unknown(lossy)),
    };
    match args.next() {
        None => Ok(Command::Help(topic)),
        Some(extra) => Err(Error::Unexpected {
            command: "help",
            argument: extra.unwrap_or_else(|lossy| lossy),
        }),
    }
}

const PROFILE: &str = "--profile";
const NO_RESTORE: &str = "--no-restore";

/// `daemon [--profile NAME] [--no-restore]`, flags in any order, `--profile`
/// also as `--profile=NAME`; `--help` alone asks for help.
fn daemon<I: Iterator<Item = Result<String, String>>>(mut args: I) -> Result<Command, Error> {
    const COMMAND: &str = "daemon";
    let unexpected = |argument: String| {
        hinted(
            COMMAND,
            argument,
            &[PROFILE, NO_RESTORE],
            "scootbg daemon --help",
            |what| Error::Unexpected {
                command: COMMAND,
                argument: what,
            },
        )
    };
    let mut profile: Option<String> = None;
    let mut no_restore = false;
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = arg.map_err(unexpected)?;
        if is_help(&arg) && first {
            return help_or_json(COMMAND, Topic::Daemon, args);
        }
        first = false;
        if arg == NO_RESTORE {
            if std::mem::replace(&mut no_restore, true) {
                return Err(Error::Repeated {
                    command: COMMAND,
                    flag: NO_RESTORE,
                });
            }
            continue;
        }
        let value = if arg == PROFILE {
            args.next()
                .ok_or(Error::MissingValue {
                    command: COMMAND,
                    flag: PROFILE,
                })?
                .map_err(unexpected)?
        } else if let Some(value) = arg.strip_prefix(PROFILE).and_then(|v| v.strip_prefix('=')) {
            value.to_owned()
        } else {
            return Err(unexpected(arg));
        };
        if profile.replace(value).is_some() {
            return Err(Error::Repeated {
                command: COMMAND,
                flag: PROFILE,
            });
        }
    }
    let profile = match profile {
        None => Profile::default(),
        Some(name) => Profile::parse(&name).map_err(Error::Profile)?,
    };
    Ok(Command::Daemon(DaemonOptions {
        profile,
        restore: !no_restore,
    }))
}

/// `apply-config`'s internal flag: this process is the detached daemon
/// (`crate::apply`).
pub const SERVE: &str = "--serve";

/// `apply-config [--profile NAME] JSON`, flags in any order, `--profile`
/// also as `--profile=NAME`; `--help` alone asks for help. `--serve` is
/// internal (see [`ApplyOptions::serve`]).
fn apply_config<I: Iterator<Item = Result<String, String>>>(mut args: I) -> Result<Command, Error> {
    const COMMAND: &str = "apply-config";
    let unexpected = |argument: String| {
        hinted(
            COMMAND,
            argument,
            &[PROFILE],
            "scootbg apply-config --help",
            |what| Error::Unexpected {
                command: COMMAND,
                argument: what,
            },
        )
    };
    let mut profile: Option<String> = None;
    let mut json: Option<String> = None;
    let mut serve = false;
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = match arg {
            Ok(arg) => arg,
            Err(lossy) if json.is_none() && !lossy.starts_with('-') => {
                return Err(Error::SectionNotUtf8);
            }
            Err(lossy) => return Err(unexpected(lossy)),
        };
        if is_help(&arg) && first {
            return help_or_json(COMMAND, Topic::ApplyConfig, args);
        }
        first = false;
        if arg == SERVE {
            if std::mem::replace(&mut serve, true) {
                return Err(Error::Repeated {
                    command: COMMAND,
                    flag: SERVE,
                });
            }
            continue;
        }
        let value = if arg == PROFILE {
            args.next()
                .ok_or(Error::MissingValue {
                    command: COMMAND,
                    flag: PROFILE,
                })?
                .map_err(unexpected)?
        } else if let Some(value) = arg.strip_prefix(PROFILE).and_then(|v| v.strip_prefix('=')) {
            value.to_owned()
        } else if json.is_none() && !arg.starts_with('-') {
            json = Some(arg);
            continue;
        } else {
            return Err(unexpected(arg));
        };
        if profile.replace(value).is_some() {
            return Err(Error::Repeated {
                command: COMMAND,
                flag: PROFILE,
            });
        }
    }
    let profile = match profile {
        None => Profile::default(),
        Some(name) => Profile::parse(&name).map_err(Error::Profile)?,
    };
    let json = json.ok_or(Error::MissingSection)?;
    let section = Section::parse(json.as_bytes()).map_err(Error::Section)?;
    Ok(Command::ApplyConfig(ApplyOptions {
        profile,
        section: Box::new(section),
        serve,
    }))
}

const OUTPUT: &str = "--output";
const WORKSPACE: &str = "--workspace";
const MODE: &str = "--mode";
const FILL: &str = "--fill";
const FILTER: &str = "--filter";
const SHA256: &str = "--sha256";
const EVERY: &str = "--every";
const SHUFFLE: &str = "--shuffle";
const TRANSITION: &str = "--transition";
const DURATION_MS: &str = "--duration-ms";
const EASING: &str = "--easing";
const ANGLE: &str = "--angle";
const POSITION: &str = "--position";
const NO_ANIMATE: &str = "--no-animate";

/// `set COLOR|PATH|URL|DIR [--output NAME] [--mode M] [--fill C] [--filter F]
/// [--sha256 HEX] [--every DURATION] [--shuffle]` and `clear [--output NAME]`,
/// flags in any order after the command, each also as `--flag=VALUE`
/// (`--shuffle` takes none). `--help` alone asks for help, as for every
/// command.
fn change<I: Iterator<Item = Result<String, String>>>(
    command: &'static str,
    topic: Topic,
    mut args: I,
) -> Result<Command, Error> {
    let flags: &[&'static str] = if command == "set" {
        &[
            OUTPUT,
            WORKSPACE,
            MODE,
            FILL,
            FILTER,
            SHA256,
            EVERY,
            TRANSITION,
            DURATION_MS,
            EASING,
            ANGLE,
            POSITION,
            NO_ANIMATE,
        ]
    } else {
        &[OUTPUT, WORKSPACE]
    };
    let help_topic = if command == "set" {
        "scootbg set --help"
    } else {
        "scootbg clear --help"
    };
    let unexpected = |argument: String| {
        hinted(command, argument, flags, help_topic, |what| {
            Error::Unexpected {
                command,
                argument: what,
            }
        })
    };
    let mut target: Option<String> = None;
    // Indexed as `flags`; the last, `--no-animate`, is a boolean handled
    // above and never lands here (its slot stays `None`).
    let mut values: [Option<String>; 13] = Default::default();
    // `--shuffle` takes no value, so it is kept out of `flags` (whose
    // machinery reads one) and handled here, bare only.
    let mut shuffle = false;
    let mut no_animate = false;
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = match arg {
            Ok(arg) => arg,
            // Not UTF-8: as the path, say why it cannot be sent.
            Err(lossy) if command == "set" && target.is_none() && !lossy.starts_with('-') => {
                return Err(Error::NotUtf8(lossy));
            }
            Err(lossy) => return Err(unexpected(lossy)),
        };
        if command == "set" && (arg == SHUFFLE || arg.starts_with("--shuffle=")) {
            if arg != SHUFFLE {
                return Err(unexpected(arg));
            }
            if std::mem::replace(&mut shuffle, true) {
                return Err(Error::Repeated {
                    command,
                    flag: SHUFFLE,
                });
            }
            first = false;
            continue;
        }
        let flag = flags.iter().enumerate().find_map(|(index, &flag)| {
            if arg == flag {
                Some((index, flag, None))
            } else {
                let value = arg.strip_prefix(flag)?.strip_prefix('=')?;
                Some((index, flag, Some(value.to_owned())))
            }
        });
        if let Some((index, flag, value)) = flag {
            // `--no-animate` is a boolean: no value, never repeated.
            if flag == NO_ANIMATE {
                if value.is_some() {
                    return Err(unexpected(arg));
                }
                if std::mem::replace(&mut no_animate, true) {
                    return Err(Error::Repeated { command, flag });
                }
                first = false;
                continue;
            }
            let value = match value {
                Some(value) => value,
                None => args
                    .next()
                    .ok_or(Error::MissingValue { command, flag })?
                    .map_err(unexpected)?,
            };
            let Some(slot) = values.get_mut(index) else {
                return Err(unexpected(arg));
            };
            if slot.replace(value).is_some() {
                return Err(Error::Repeated { command, flag });
            }
        } else if is_help(&arg) && first {
            return help_or_json(command, topic, args);
        } else if command == "set" && target.is_none() && !arg.starts_with('-') {
            target = Some(arg);
        } else {
            return Err(unexpected(arg));
        }
        first = false;
    }
    let [
        output,
        workspace,
        mode,
        fill,
        filter,
        sha256,
        every,
        transition,
        duration_ms,
        easing,
        angle,
        position,
        no_animate_value,
    ] = values;
    // `--no-animate` never lands in `values` (handled above as a boolean);
    // a value there would mean the matching changed.
    debug_assert!(no_animate_value.is_none());
    let _ = no_animate_value;
    let output = output.map(Cow::Owned);
    let workspace = workspace.map(check_workspace).transpose()?.map(Cow::Owned);
    if command == "clear" {
        return Ok(Command::Client(match workspace {
            Some(workspace) => Request::ClearWorkspace { output, workspace },
            None => Request::Clear { output },
        }));
    }
    let argument = target.ok_or(Error::MissingTarget)?;
    if shuffle && every.is_none() {
        return Err(Error::ShuffleNeedsEvery);
    }
    let transition = parse_transition(transition, duration_ms, easing, angle, position)?;
    if argument.starts_with('#') {
        for (flag, given) in [
            (MODE, &mode),
            (FILL, &fill),
            (FILTER, &filter),
            (SHA256, &sha256),
        ] {
            if given.is_some() {
                return Err(Error::ImageOnly(flag));
            }
        }
        if no_animate {
            return Err(Error::ImageOnly(NO_ANIMATE));
        }
        if every.is_some() {
            return Err(Error::EveryNeedsDirectory(argument));
        }
        return match Color::parse(&argument) {
            Ok(color) => Ok(Command::Client(match workspace {
                Some(workspace) => Request::SetWorkspace {
                    show: Show::Color(color),
                    output,
                    workspace,
                    transition,
                },
                None => Request::Set {
                    show: Show::Color(color),
                    output,
                    transition,
                },
            })),
            Err(error) => Err(Error::Color { argument, error }),
        };
    }
    let mode = match mode {
        None => Mode::default(),
        Some(value) => Mode::from_name(&value).ok_or(Error::BadValue { flag: MODE, value })?,
    };
    let filter = match filter {
        None => Filter::default(),
        Some(value) => Filter::from_name(&value).ok_or(Error::BadValue {
            flag: FILTER,
            value,
        })?,
    };
    let fill = match fill {
        None => DEFAULT_FILL,
        Some(argument) => match Color::parse(&argument) {
            Ok(color) => color,
            Err(error) => return Err(Error::Color { argument, error }),
        },
    };
    let source = if crate::fetch::is_url(&argument) {
        if argument.contains('\0') {
            return Err(Error::UrlNul);
        }
        if every.is_some() {
            return Err(Error::EveryNeedsDirectory(argument));
        }
        let sha256 = sha256
            .map(|value| crate::fetch::parse_sha256(&value).map_err(|_| Error::BadSha(value)))
            .transpose()?;
        Source::Url {
            url: Cow::Owned(argument),
            sha256,
        }
    } else if std::fs::metadata(&argument).is_ok_and(|meta| meta.is_dir()) {
        if sha256.is_some() {
            return Err(Error::ShaImageOnly);
        }
        if workspace.is_some() {
            return Err(Error::SlideshowWithWorkspace(argument));
        }
        let every = every.ok_or_else(|| Error::DirectoryNeedsEvery(argument.clone()))?;
        let every_secs = crate::rotation::parse_every(&every).map_err(Error::BadEvery)?;
        if no_animate {
            return Err(Error::NoAnimateWithEvery);
        }
        let path = absolute(&argument)?;
        return Ok(Command::Client(Request::Set {
            show: Show::Slideshow(SlideshowRequest {
                dir: Cow::Owned(path),
                every_secs,
                shuffle,
                mode,
                fill,
                filter,
            }),
            output,
            transition,
        }));
    } else {
        if every.is_some() {
            return Err(Error::EveryNeedsDirectory(argument));
        }
        if sha256.is_some() {
            return Err(Error::ShaImageOnly);
        }
        let path = absolute(&argument)?;
        Source::Path(Cow::Owned(path))
    };
    Ok(Command::Client(match workspace {
        Some(workspace) => Request::SetWorkspace {
            show: Show::Image(ImageRequest {
                source,
                mode,
                fill,
                filter,
                animate: !no_animate,
            }),
            output,
            workspace,
            transition,
        },
        None => Request::Set {
            show: Show::Image(ImageRequest {
                source,
                mode,
                fill,
                filter,
                animate: !no_animate,
            }),
            output,
            transition,
        },
    }))
}

/// A `--workspace` value checked: non-empty, short, and without a NUL
/// byte (which the control protocol's JSON could carry only escaped, and
/// nothing downstream wants).
fn check_workspace(value: String) -> Result<String, Error> {
    if value.is_empty() {
        return Err(Error::BadWorkspace {
            value,
            reason: "it is empty".to_owned(),
        });
    }
    if value.len() > crate::choices::MAX_WORKSPACE_NAME {
        return Err(Error::BadWorkspace {
            value,
            reason: format!("it is past {} bytes", crate::choices::MAX_WORKSPACE_NAME),
        });
    }
    if value.contains('\0') {
        return Err(Error::BadWorkspace {
            value,
            reason: "it has a NUL byte".to_owned(),
        });
    }
    Ok(value)
}

/// The five transition flags as one [`Spec`](crate::transition::Spec):
/// parsed strictly and together (`crate::transition::assemble`), so a
/// stray `--duration-ms` teaches rather than being ignored.
fn parse_transition(
    transition: Option<String>,
    duration_ms: Option<String>,
    easing: Option<String>,
    angle: Option<String>,
    position: Option<String>,
) -> Result<crate::transition::Spec, Error> {
    use crate::transition::{self, Kind};
    let kind = transition
        .map(|value| {
            Kind::parse(&value).map_err(|_| Error::BadValue {
                flag: TRANSITION,
                value,
            })
        })
        .transpose()?;
    transition::assemble(
        kind,
        duration_ms.as_deref(),
        easing.as_deref(),
        angle.as_deref(),
        position.as_deref(),
        |key| format!("--{key}"),
    )
    .map_err(|error| {
        use crate::transition::ParseError;
        match error {
            ParseError::Orphan(flag) => {
                let flag = match flag.as_str() {
                    "--duration-ms" => DURATION_MS,
                    "--easing" => EASING,
                    "--angle" => ANGLE,
                    _ => POSITION,
                };
                Error::TransitionOnly(flag)
            }
            ParseError::UnknownKind(value) => Error::BadValue {
                flag: TRANSITION,
                value,
            },
            ParseError::UnknownEasing(value) => Error::BadValue {
                flag: EASING,
                value,
            },
            ParseError::BadDuration(value) => Error::BadValue {
                flag: DURATION_MS,
                value,
            },
            ParseError::BadAngle(value) => Error::BadValue { flag: ANGLE, value },
            ParseError::BadPosition(value) => Error::BadValue {
                flag: POSITION,
                value,
            },
        }
    })
}

/// `path` made absolute against the working directory, without touching
/// the file system beyond that (symbolic links stay as given; the daemon
/// reports a missing file).
fn absolute(path: &str) -> Result<String, Error> {
    let absolute = std::path::absolute(path).map_err(|error| Error::Path {
        argument: path.to_owned(),
        error: error.to_string(),
    })?;
    absolute
        .into_os_string()
        .into_string()
        .map_err(|lossy| Error::NotUtf8(lossy.to_string_lossy().into_owned()))
}
