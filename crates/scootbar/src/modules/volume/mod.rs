//! The volume module: the default sink's level and mute, and its
//! `microphone` twin for the default source.
//!
//! A minimal PulseAudio-protocol client on the bar's `poll` loop, with no
//! libpulse and no child process: one unix socket while the server is up,
//! one inotify watch while it is not, and nothing else. The ticket measured
//! the alternative (`pactl subscribe` plus a `wpctl get-volume` child per
//! event, about 10 ms and 10 MB transient RSS each on the Asahi machine,
//! and no usable event source where `pactl` is not installed) against this
//! one (a few dozen bytes parsed per event); the native client is the one
//! built.
//!
//! ## States
//!
//! `Waiting` owns an inotify fd on the socket's directory and shows
//! nothing; any arrival, move-in or disappearance there re-probes the
//! socket path, so a server starting, stopping or restarting is found
//! without polling. A connection dropped after its subscription is
//! answered also probes the socket once at once, since a restart that
//! keeps the path names no new event; one dropped earlier (a refused
//! cookie) waits for an event, so a standing refusal is never a connect
//! loop. `Live` owns the connected socket and sends one request
//! at a time (`AUTH`, name, server info, subscribe, the default device),
//! because replies arrive in order and one outstanding request is all the
//! matching needed. Anything the connection says that does not parse, and
//! any I/O error or hang-up, drops it back to `Waiting` with the state
//! cleared: a dead server's last volume is not shown as if live.
//!
//! ## Scroll floods
//!
//! Every change is an absolute set from the last known level (or the last
//! set target while one is unanswered), never an accumulated relative one,
//! so a touchpad flood cannot drift. At most one set is in flight; further
//! scrolls coalesce into one queued target, and every answered set is
//! followed by a re-read, so what the server clamped to is what is shown.
//!
//! ## Fixed bounds, no allocation past start-up
//!
//! Names and descriptions from the server are cut at [`proto::MAX_NAME`]
//! bytes; frames past [`proto::MAX_FRAME`] drop the connection; the read
//! buffer is fixed and reused, requests are built on the stack, and the
//! four level icons are parsed once at start-up.

use std::fmt::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::event::PollFlags;
use rustix::fs::inotify::{CreateFlags, WatchFlags};
use rustix::io::Errno;
use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with};

use super::{
    ActionSpec, ArgKind, Init, Input, InvokeError, Module, OutputView, Sources, Update, View,
};
use crate::action::{ModuleAction, Trigger};
use crate::dbus::conn::runtime_dir;
use crate::icon::path::{Vector, ViewBox};
use crate::icon::{Art, Icon};

mod icons;
pub mod proto;

// The fuzz target's check, compiled here only for the test that replays
// its corpus (`crates/scootbar/fuzz` compiles the file itself).
#[cfg(test)]
mod fuzz;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod set_tests;
#[cfg(test)]
mod tests;

use proto::{Bounded, Change, DeviceInfo, Event, Kind};

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "volume";
/// The microphone variant's id: the default source's level and mute.
pub const MIC_ID: &str = "microphone";

/// The actions a binding may name (`raise`, `lower`, `toggle-mute`, `set`,
/// `popup`), and what `scootbar msg invoke` takes. Only `set` takes a number
/// (a percent): a scroll's steps arrive through the scroll itself, and one
/// raise is one step.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "raise",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "lower",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "toggle-mute",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "set",
        arg: ArgKind::Required,
    },
    // The slider popup (`on-click = "popup"`): the bar carries it out, the
    // module only draws it (`Module::popup`).
    #[cfg(feature = "popup")]
    ActionSpec {
        name: crate::action::POPUP,
        arg: ArgKind::None,
    },
];

/// The default `step`: percent points per scroll notch and per raise.
pub const DEFAULT_STEP: u32 = 5;
/// The most `step` takes, in percent points.
pub const MAX_STEP: u32 = 50;
/// The default `max-volume`: full scale, no over-amplification.
pub const DEFAULT_MAX_VOLUME: u32 = 100;
/// The least `max-volume` takes: below full scale the module could never
/// reach it.
pub const MIN_MAX_VOLUME: u32 = 100;
/// The most `max-volume` takes, in percent: past it is distortion on every
/// device measured, not louder.
pub const MAX_MAX_VOLUME: u32 = 150;

/// The module's options. Which device it follows is fixed by which id
/// started it, not by the config.
#[derive(Debug, Clone)]
pub struct Settings {
    pub step: u32,
    pub max_volume: u32,
    /// A static icon, when the config sets the icon keys: shown for every
    /// level instead of the built-in ones.
    pub icon: Option<Icon>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            step: DEFAULT_STEP,
            max_volume: DEFAULT_MAX_VOLUME,
            icon: None,
        }
    }
}

impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        // An icon compares by what it shows: glyphs by value, art by
        // pointer, which is what `config::icon` builds.
        self.step == other.step
            && self.max_volume == other.max_volume
            && match (&self.icon, &other.icon) {
                (None, None) => true,
                (Some(a), Some(b)) => icon_eq(a, b),
                _ => false,
            }
    }
}

impl Eq for Settings {}

fn icon_eq(a: &Icon, b: &Icon) -> bool {
    match (a, b) {
        (Icon::Glyph(x), Icon::Glyph(y)) => x == y,
        (Icon::Art(x), Icon::Art(y)) => x.id() == y.id(),
        _ => false,
    }
}

#[cfg(feature = "volume")]
pub fn init(settings: &super::Settings) -> Init {
    start(&settings.volume, Kind::Sink)
}

#[cfg(feature = "microphone")]
pub fn init_microphone(settings: &super::Settings) -> Init {
    start(&settings.microphone, Kind::Source)
}

/// Starts the module for `kind`: connects now when the socket is there, or
/// waits on it when it is not. Always available: audio may appear at any
/// time, and waiting costs one inotify fd.
fn start(settings: &Settings, kind: Kind) -> Init {
    Init::Available(start_with(settings, kind, socket_path()))
}

/// The same, at an explicit socket path: the tests' fake servers.
pub(crate) fn start_with(settings: &Settings, kind: Kind, socket: PathBuf) -> Box<dyn Module> {
    let step = settings.step.clamp(1, MAX_STEP);
    let max = settings.max_volume.clamp(MIN_MAX_VOLUME, MAX_MAX_VOLUME);
    let mut volume = Volume {
        kind,
        step_raw: proto::from_percent(step),
        max_raw: proto::from_percent(max),
        icons: [None, None, None, None],
        icon_override: settings.icon.clone(),
        socket,
        watch_dir: PathBuf::new(),
        notify: None,
        live: None,
        said_connect: false,
    };
    for (slot, path) in volume.icons.iter_mut().zip(icons::PATHS) {
        match Vector::parse(path, ViewBox::default()) {
            Ok(vector) => {
                *slot = Some(Icon::Art(Art::Vector(std::sync::Arc::new(vector))));
            }
            // Built-in paths parsed in the module's own tests; if one ever
            // stops parsing, the level shows no icon rather than nothing
            // starting.
            Err(_) => *slot = None,
        }
    }
    if !volume.connect() {
        volume.watch();
    }
    Box::new(volume)
}

/// Where the server listens: `$PULSE_SERVER` when it names a unix socket
/// (`unix:PATH` or a bare path), else `$XDG_RUNTIME_DIR/pulse/native`.
pub(crate) fn socket_path() -> PathBuf {
    socket_path_for(std::env::var_os("PULSE_SERVER").as_deref())
}

fn socket_path_for(server: Option<&std::ffi::OsStr>) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    if let Some(server) = server {
        let bytes = server.as_encoded_bytes();
        let path = bytes.strip_prefix(b"unix:").unwrap_or(bytes);
        if !path.is_empty() {
            return PathBuf::from(std::ffi::OsStr::from_bytes(path));
        }
    }
    runtime_dir().join("pulse").join("native")
}

/// The directory to watch for the socket's appearance: its own directory
/// when there is one, else the runtime directory.
fn best_watch_dir(socket: &Path) -> PathBuf {
    socket
        .parent()
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(runtime_dir)
}

/// The cookie `AUTH` sends: the user's 256 bytes, or zeros when no cookie
/// file is readable (pipewire-pulse takes any cookie from the same user;
/// a real server refuses a wrong one with an error, and the module waits).
fn cookie() -> [u8; 256] {
    let mut cookie = [0u8; 256];
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")));
    if let Some(path) = base.map(|base| base.join("pulse").join("cookie")) {
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes.len() == 256 {
                cookie.copy_from_slice(&bytes);
            }
        }
    }
    cookie
}

/// The module: what it shows, and the one connection or watch it owns.
struct Volume {
    kind: Kind,
    /// One scroll notch, in raw volume units.
    step_raw: u32,
    /// The `max-volume` cap, in raw units.
    max_raw: u32,
    /// The built-in level icons, parsed once (`None` where one did not
    /// parse, which the tests forbid).
    icons: [Option<Icon>; 4],
    icon_override: Option<Icon>,
    socket: PathBuf,
    /// The directory watched while disconnected.
    watch_dir: PathBuf,
    notify: Option<Notify>,
    live: Option<Live>,
    /// Whether the missing server was said already: once, not per wake.
    said_connect: bool,
}

/// The inotify watch on the socket's directory, while disconnected.
struct Notify {
    fd: OwnedFd,
}

/// A connection: the socket, its buffers and what it still owes.
struct Live {
    fd: OwnedFd,
    /// Bytes read and not yet framed: parsed in place, compacted after.
    read: [u8; proto::MAX_FRAME],
    start: usize,
    end: usize,
    /// One frame being written, and how much of it went.
    out: [u8; 512],
    out_len: usize,
    out_sent: usize,
    tag: u32,
    /// The request still owed its reply, if any.
    outstanding: Option<(u32, Expect)>,
    /// One coalescing slot: a refresh, or a set that overwrote it.
    queued: Option<Queued>,
    /// Whether the subscription was answered: only the first server info
    /// subscribes; later ones only re-read.
    subscribed: bool,
    /// The last set target still unanswered: the base the next scroll
    /// moves from, so a flood sets absolute values, never accumulations.
    sent_volume: Option<u32>,
    sent_mute: Option<bool>,
    /// The default device's name, and what was last read of it: neither
    /// until the handshake brings them.
    default: Bounded,
    device: Option<DeviceInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Auth,
    Name,
    Server,
    Device,
    Subscribed,
    SetVolume,
    SetMute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Queued {
    Refresh,
    SetVolume(u32),
    SetMute(bool),
}

impl Volume {
    fn name(&self) -> &'static str {
        match self.kind {
            Kind::Sink => ID,
            Kind::Source => MIC_ID,
        }
    }

    /// The query's value key for the device name: `sink` or `source`.
    fn device_key(&self) -> &'static str {
        match self.kind {
            Kind::Sink => "sink",
            Kind::Source => "source",
        }
    }

    /// Connects and starts the handshake: `true` when the socket is open
    /// (the handshake still has to succeed). A refusal is said once on
    /// stderr, not per inotify wake.
    fn connect(&mut self) -> bool {
        let fd = match socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        )
        .and_then(|fd| {
            let addr = SocketAddrUnix::new(&self.socket).map_err(|_| Errno::NOENT)?;
            match connect(&fd, &addr) {
                Ok(()) | Err(Errno::AGAIN) | Err(Errno::INPROGRESS) => Ok(fd),
                Err(errno) => Err(errno),
            }
        }) {
            Ok(fd) => fd,
            Err(_) => {
                if !self.said_connect {
                    self.said_connect = true;
                    crate::print::warn(format_args!(
                        "scootbar: {}: no sound server at {}",
                        self.name(),
                        self.socket.display()
                    ));
                }
                return false;
            }
        };
        self.said_connect = false;
        self.notify = None;
        let mut live = Live {
            fd,
            read: [0; proto::MAX_FRAME],
            start: 0,
            end: 0,
            out: [0; 512],
            out_len: 0,
            out_sent: 0,
            tag: 0,
            outstanding: None,
            queued: None,
            subscribed: false,
            sent_volume: None,
            sent_mute: None,
            default: Bounded::empty(),
            device: None,
        };
        // The handshake's first word: AUTH with the protocol version and
        // the cookie. A socket that never answers is found by the hang-up
        // when the server goes away; nothing here blocks.
        let mut writer = proto::Writer::new();
        if proto::auth_payload(&cookie(), &mut writer).is_none() {
            return false;
        }
        let payload = writer.done().to_vec();
        live.request(proto::CMD_AUTH, &payload, Expect::Auth);
        live.flush();
        self.live = Some(live);
        true
    }

    /// Watches the socket's directory for its appearance: the socket's
    /// own directory when there is one, else the runtime directory
    /// itself. A watch that cannot be armed is nothing polled: the socket
    /// is re-probed on every inotify wake that does arrive, and a reload
    /// starts over.
    fn watch(&mut self) {
        self.live = None;
        let dir = best_watch_dir(&self.socket);
        let fd = match rustix::fs::inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)
            .and_then(|fd| {
                rustix::fs::inotify::add_watch(
                    &fd,
                    &dir,
                    WatchFlags::CREATE
                        | WatchFlags::MOVED_TO
                        | WatchFlags::DELETE_SELF
                        | WatchFlags::MOVE_SELF,
                )?;
                Ok(fd)
            }) {
            Ok(fd) => fd,
            Err(_) => return,
        };
        self.watch_dir = dir;
        self.notify = Some(Notify { fd });
    }

    /// Drops the connection and waits again, with nothing shown: a dead
    /// server's last level is not a level.
    ///
    /// A connection that got as far as the subscription had a server that
    /// was up and agreed to talk, so its drop may be a restart that kept
    /// the socket path: no `CREATE` will name it, and the watch alone would
    /// strand the module beside a live server. The socket is probed once,
    /// after the watch is armed (probing first could miss an appearance in
    /// between). A connection dropped *before* that (refused cookie, closed
    /// during the handshake) is not probed: connecting again would only be
    /// refused again, once per drop, so a standing refusal waits for a real
    /// event on the directory instead of looping.
    fn drop_live(&mut self) -> Update {
        let had = self.live.as_ref().is_some_and(|live| live.device.is_some());
        let reached_subscription = self.live.as_ref().is_some_and(|live| live.subscribed);
        self.watch();
        if reached_subscription {
            self.connect();
        }
        if had {
            Update::Changed
        } else {
            Update::Unchanged
        }
    }

    /// The level a scroll or raise moves from: the last set target while
    /// one is unanswered, else what is shown.
    fn base_volume(&self) -> Option<u32> {
        let live = self.live.as_ref()?;
        if let Some(sent) = live.sent_volume {
            return Some(sent);
        }
        live.device.as_ref().map(DeviceInfo::level)
    }

    fn base_mute(&self) -> Option<bool> {
        let live = self.live.as_ref()?;
        if let Some(sent) = live.sent_mute {
            return Some(sent);
        }
        live.device.as_ref().map(|device| device.muted)
    }

    /// Asks for the server info and then the default device again: after a
    /// server change, after a set was answered, or after the tracked
    /// device went away. Behind the request in flight, if any (a set
    /// there overwrites it: its answer re-reads anyway).
    fn refresh(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.outstanding.is_some() {
            if live.queued.is_none() {
                live.queued = Some(Queued::Refresh);
            }
            return;
        }
        live.request(proto::CMD_GET_SERVER_INFO, &[], Expect::Server);
        live.flush();
    }

    /// Subscribes to the device's changes and the server's own: once per
    /// connection, after the first server info.
    fn send_subscribe(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.outstanding.is_some() {
            if live.queued.is_none() {
                live.queued = Some(Queued::Refresh);
            }
            return;
        }
        let mut writer = proto::Writer::new();
        if writer.put_u32(self.kind.subscribe_mask()).is_none() {
            return;
        }
        let payload = writer.done().to_vec();
        live.request(proto::CMD_SUBSCRIBE, &payload, Expect::Subscribed);
        live.flush();
    }

    /// Sets an absolute volume: behind the request in flight, if any,
    /// overwriting whatever waited there. Without a device (a set queued
    /// before it went away) it re-reads instead of sending nowhere.
    fn set_volume(&mut self, target: u32) {
        if self.live.is_none() {
            return;
        }
        if self
            .live
            .as_ref()
            .is_some_and(|live| live.outstanding.is_some())
        {
            let Some(live) = self.live.as_mut() else {
                return;
            };
            live.queued = Some(Queued::SetVolume(target));
            live.sent_volume = Some(target);
            return;
        }
        if self.live.as_ref().is_some_and(|live| live.device.is_none()) {
            self.refresh();
            return;
        }
        let Some(live) = self.live.as_mut() else {
            return;
        };
        // By name, with an invalid index: measured against pipewire-pulse,
        // which refuses an index with a name (the queries read the same
        // way). The index the device reply carried is only matched
        // against subscription events.
        let mut writer = proto::Writer::new();
        let channels = live
            .device
            .as_ref()
            .map(|device| device.channels)
            .unwrap_or(0);
        let channels = channels.clamp(1, proto::MAX_CHANNELS);
        let volumes = [target; proto::MAX_CHANNELS];
        let Some(name) = live
            .device
            .as_ref()
            .map(|device| device.name.as_str().to_owned())
        else {
            return;
        };
        let built = writer.put_u32(proto::INVALID_INDEX).is_some()
            && writer.put_str(&name).is_some()
            && writer.put_cvolume(&volumes[..channels]).is_some();
        if !built {
            return;
        }
        let payload = writer.done().to_vec();
        live.request(self.kind.set_volume(), &payload, Expect::SetVolume);
        live.sent_volume = Some(target);
        live.flush();
    }

    /// Toggles mute absolutely, like [`Volume::set_volume`].
    fn set_mute(&mut self, target: bool) {
        if self.live.is_none() {
            return;
        }
        if self
            .live
            .as_ref()
            .is_some_and(|live| live.outstanding.is_some())
        {
            let Some(live) = self.live.as_mut() else {
                return;
            };
            live.queued = Some(Queued::SetMute(target));
            live.sent_mute = Some(target);
            return;
        }
        if self.live.as_ref().is_some_and(|live| live.device.is_none()) {
            self.refresh();
            return;
        }
        let Some(live) = self.live.as_mut() else {
            return;
        };
        // By name, with an invalid index: as the volume set above.
        let Some(name) = live
            .device
            .as_ref()
            .map(|device| device.name.as_str().to_owned())
        else {
            return;
        };
        let mut writer = proto::Writer::new();
        let built = writer.put_u32(proto::INVALID_INDEX).is_some()
            && writer.put_str(&name).is_some()
            && writer.put_bool(target).is_some();
        if !built {
            return;
        }
        let payload = writer.done().to_vec();
        live.request(self.kind.set_mute(), &payload, Expect::SetMute);
        live.sent_mute = Some(target);
        live.flush();
    }

    /// Handles one ready source: the socket, or the directory watch.
    /// At most [`FRAMES_PER_TURN`] frames a turn, so a flooding server is
    /// a bounded number of redraws, not an unbounded one.
    fn on_socket(&mut self, events: PollFlags) -> Update {
        if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            return self.drop_live();
        }
        if events.intersects(PollFlags::OUT) {
            let Some(live) = self.live.as_mut() else {
                return Update::Unchanged;
            };
            if !live.flush() {
                return self.drop_live();
            }
        }
        if !events.intersects(PollFlags::IN) {
            return Update::Unchanged;
        }
        let mut changed = Update::Unchanged;
        for _ in 0..FRAMES_PER_TURN {
            let fill = self.live.as_mut().map(Live::fill).unwrap_or(Fill::Waiting);
            match fill {
                Fill::Dropped => return self.drop_live(),
                Fill::Waiting => break,
                Fill::Frame => {
                    if self.handle_frame() == Update::Changed {
                        changed = Update::Changed;
                    }
                    if self.live.is_none() {
                        break;
                    }
                }
            }
        }
        self.compact();
        changed
    }

    /// Handles the directory watch: drains it (else it stays ready),
    /// follows the socket's directory if it appeared after the watch
    /// (inotify is not recursive, so a created `pulse/` moves the watch
    /// into it), and probes the socket when its name arrived. Events for
    /// other names cost one scan of the read bytes, never a connect.
    fn on_notify(&mut self, events: PollFlags) -> Update {
        if events.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            // A watch armed afresh may have missed the socket's arrival.
            self.watch();
            self.connect();
            return Update::Unchanged;
        }
        let mut native = false;
        if let Some(notify) = self.notify.as_ref() {
            let fd = notify.fd.as_fd();
            let mut buf = [0u8; 4096];
            loop {
                match rustix::io::read(fd, &mut buf) {
                    Ok(0) | Err(Errno::AGAIN) => break,
                    Ok(n) => {
                        if scan_names(&buf[..n], b"native") {
                            native = true;
                        }
                    }
                    Err(_) => {
                        self.watch();
                        self.connect();
                        return Update::Unchanged;
                    }
                }
            }
        }
        // A watch that moved into a directory that just appeared may have
        // missed the socket being created in it: probe, as for a name.
        if best_watch_dir(&self.socket) != self.watch_dir {
            self.watch();
            native = true;
        }
        if native {
            self.connect();
        }
        // The handshake brings the first state; nothing is shown yet.
        Update::Unchanged
    }

    /// Handles the oldest complete frame in the read buffer: matches the
    /// reply to its request, or answers the event, and says whether the
    /// view moved. Anything that does not parse drops the connection: the
    /// bytes are the server's, and a half-understood volume is worse than
    /// none.
    ///
    /// In phases, so the read buffer is never borrowed across the state
    /// change: locate, advance past the frame, parse to owned values, then
    /// apply. A frame's payload always starts 30 bytes into it (the
    /// descriptor, the command and the tag), so no pointer is kept.
    fn handle_frame(&mut self) -> Update {
        let (cmd, tag, begin, len) = {
            let Some(live) = self.live.as_ref() else {
                return Update::Unchanged;
            };
            let start = live.start;
            match proto::frame_at(&live.read[start..live.end]) {
                Ok(Some((frame, consumed))) => (frame.cmd, frame.tag, start + 30, consumed - 30),
                Ok(None) => return Update::Unchanged,
                Err(()) => return self.drop_live(),
            }
        };
        if let Some(live) = self.live.as_mut() {
            live.start += 30 + len;
        }
        let parsed = {
            let Some(live) = self.live.as_ref() else {
                return Update::Unchanged;
            };
            parse_frame(cmd, tag, &live.read[begin..begin + len], begin)
        };
        match parsed {
            Parsed::Malformed => self.drop_live(),
            Parsed::Reply { tag, begin, len } => self.on_reply(tag, begin, len),
            Parsed::Error { tag, errno } => self.on_error(tag, errno),
            Parsed::Event(event) => self.on_event(&event),
        }
    }

    /// A request the server refused: matched like a reply, then handled
    /// by what it was.
    fn on_error(&mut self, tag: u32, errno: u32) -> Update {
        let _ = errno;
        let expect = {
            let Some(live) = self.live.as_ref() else {
                return Update::Unchanged;
            };
            match live.outstanding {
                Some((pending, expect)) if pending == tag => expect,
                _ => return self.drop_live(),
            }
        };
        if let Some(live) = self.live.as_mut() {
            live.outstanding = None;
        }
        match expect {
            // The default vanished between the server info and the read:
            // nothing is shown until an event brings news, and nothing is
            // asked again unprompted, so this cannot loop.
            Expect::Device => {
                let changed = if self
                    .live
                    .as_mut()
                    .is_some_and(|live| live.device.take().is_some())
                {
                    Update::Changed
                } else {
                    Update::Unchanged
                };
                self.after_reply(changed, Follow::Nothing)
            }
            // A refused set is answered by a re-read: the server's truth,
            // whatever the set did.
            Expect::SetVolume => {
                if let Some(live) = self.live.as_mut() {
                    live.sent_volume = None;
                }
                self.after_reply(Update::Unchanged, Follow::ServerInfo)
            }
            Expect::SetMute => {
                if let Some(live) = self.live.as_mut() {
                    live.sent_mute = None;
                }
                self.after_reply(Update::Unchanged, Follow::ServerInfo)
            }
            _ => self.drop_live(),
        }
    }

    /// Matches a reply to its request, then sends what is owed next:
    /// first whatever waited behind the request in flight, else what the
    /// reply implies. A reply to no outstanding request drops the
    /// connection: the server is not answering what was asked.
    ///
    /// Queued work always wins over the follow-up, and every chain
    /// converges: a set's answer re-reads anyway, and a dropped device
    /// query is re-asked by the server info that follows a refresh.
    fn on_reply(&mut self, tag: u32, begin: usize, len: usize) -> Update {
        let expect = {
            let Some(live) = self.live.as_ref() else {
                return Update::Unchanged;
            };
            match live.outstanding {
                Some((pending, expect)) if pending == tag => expect,
                _ => return self.drop_live(),
            }
        };
        // Parsed to owned values before anything is mutated, like the
        // frame itself was.
        let body = {
            let Some(live) = self.live.as_ref() else {
                return Update::Unchanged;
            };
            let payload = &live.read[begin..begin + len];
            match parse_reply(expect, payload) {
                Some(body) => body,
                None => return self.drop_live(),
            }
        };
        if let Some(live) = self.live.as_mut() {
            live.outstanding = None;
        }
        let follow = match (expect, body) {
            (Expect::Auth, ReplyBody::Auth(version)) => {
                if version < 8 {
                    return self.drop_live();
                }
                Follow::Name
            }
            (Expect::Name, ReplyBody::Name) => Follow::ServerInfo,
            (Expect::Server, ReplyBody::Server(defaults)) => {
                let changed = self.set_defaults(&defaults);
                if changed == Update::Changed {
                    // The default vanished: nothing is shown, and no
                    // device is asked for.
                    return changed;
                }
                if self.live.as_ref().is_some_and(|live| live.subscribed) {
                    Follow::DeviceQuery
                } else {
                    Follow::Subscribe
                }
            }
            (Expect::Device, ReplyBody::Device(device)) => {
                let changed = self.set_device(device);
                return self.after_reply(changed, Follow::Nothing);
            }
            (Expect::Subscribed, ReplyBody::Ack) => {
                if let Some(live) = self.live.as_mut() {
                    live.subscribed = true;
                }
                // The defaults are already known (just read before
                // subscribing); a change meanwhile arrives as an event now
                // that the subscription is up, so ask for the device.
                Follow::DeviceQuery
            }
            (Expect::SetVolume, ReplyBody::Ack) => {
                if let Some(live) = self.live.as_mut() {
                    live.sent_volume = None;
                }
                Follow::ServerInfo
            }
            (Expect::SetMute, ReplyBody::Ack) => {
                if let Some(live) = self.live.as_mut() {
                    live.sent_mute = None;
                }
                Follow::ServerInfo
            }
            _ => return self.drop_live(),
        };
        self.after_reply(Update::Unchanged, follow)
    }

    /// Sends what is owed: the queued set or refresh first, else the
    /// reply's follow-up. `changed` is what handling the reply already
    /// decided the view did.
    fn after_reply(&mut self, changed: Update, follow: Follow) -> Update {
        let queued = self.live.as_mut().and_then(|live| live.queued.take());
        match queued {
            Some(Queued::SetVolume(target)) => self.set_volume(target),
            Some(Queued::SetMute(target)) => self.set_mute(target),
            Some(Queued::Refresh) => self.refresh(),
            None => match follow {
                Follow::Nothing => {}
                Follow::Name => {
                    self.send_name();
                }
                Follow::ServerInfo => self.refresh(),
                Follow::Subscribe => self.send_subscribe(),
                Follow::DeviceQuery => self.send_device(),
            },
        }
        changed
    }

    /// Answers a subscription event: a server change re-reads the
    /// defaults, a change or removal of the tracked device re-reads it,
    /// and anything else is not ours.
    fn on_event(&mut self, event: &Event) -> Update {
        if !self.kind.owns(event.facility) {
            return Update::Unchanged;
        }
        if event.facility == proto::FACILITY_SERVER {
            self.refresh();
            return Update::Unchanged;
        }
        let tracked = self
            .live
            .as_ref()
            .and_then(|live| live.device.as_ref())
            .is_some_and(|device| device.index == event.index);
        match event.change {
            Change::Removed if tracked => {
                if let Some(live) = self.live.as_mut() {
                    live.device = None;
                }
                self.refresh();
                Update::Changed
            }
            Change::Changed if tracked => {
                self.refresh();
                Update::Unchanged
            }
            _ => Update::Unchanged,
        }
    }

    /// Slides parsed bytes off the read buffer's front, in place.
    fn compact(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.start > 0 {
            live.read.copy_within(live.start..live.end, 0);
            live.end -= live.start;
            live.start = 0;
        }
    }

    /// Sends the client's name, after `AUTH` was answered.
    fn send_name(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let mut writer = proto::Writer::new();
        if proto::name_payload(&mut writer).is_none() {
            let _ = self.drop_live();
            return;
        }
        let payload = writer.done().to_vec();
        live.request(proto::CMD_SET_CLIENT_NAME, &payload, Expect::Name);
        live.flush();
    }

    /// Asks for the server info: the defaults.
    /// Asks for the default device by the name the server info gave, or
    /// clears what is shown when there is no default.
    fn send_device(&mut self) {
        let name = self
            .live
            .as_ref()
            .map(|live| live.default.as_str().to_owned())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let mut writer = proto::Writer::new();
        if writer.put_u32(proto::INVALID_INDEX).is_none() || writer.put_str(&name).is_none() {
            return;
        }
        let payload = writer.done().to_vec();
        live.request(self.kind.get_info(), &payload, Expect::Device);
        live.flush();
    }

    /// Keeps the new defaults. Only clearing an emptied default moves the
    /// view now; anything else is re-read after (`send_device`), and the
    /// device reply says whether it moved.
    fn set_defaults(&mut self, defaults: &proto::ServerDefaults) -> Update {
        let want = match self.kind {
            Kind::Sink => defaults.sink.as_str(),
            Kind::Source => defaults.source.as_str(),
        };
        let Some(live) = self.live.as_mut() else {
            return Update::Unchanged;
        };
        live.default.set(want);
        if want.is_empty() && live.device.take().is_some() {
            return Update::Changed;
        }
        Update::Unchanged
    }

    /// Keeps the new device state: `Changed` when anything shown moved.
    fn set_device(&mut self, device: DeviceInfo) -> Update {
        let Some(live) = self.live.as_mut() else {
            return Update::Unchanged;
        };
        if live.device.as_ref() == Some(&device) {
            return Update::Unchanged;
        }
        live.device = Some(device);
        Update::Changed
    }
}

impl Module for Volume {
    /// Its view carries a tooltip.
    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        true
    }

    /// The one fd owned: the socket while connected, the directory watch
    /// while waiting. Zero sources is never: one of the two always is.
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        if let Some(live) = self.live.as_ref() {
            let mut flags = PollFlags::IN;
            if live.needs_write() {
                flags |= PollFlags::OUT;
            }
            sources.add(live.fd.as_fd(), flags);
        } else if let Some(notify) = self.notify.as_ref() {
            sources.add(notify.fd.as_fd(), PollFlags::IN);
        }
    }

    fn on_ready(&mut self, _source: usize, events: PollFlags) -> Update {
        if self.live.is_some() {
            self.on_socket(events)
        } else {
            self.on_notify(events)
        }
    }

    /// `{percent}%` with the level's icon, dimmed when muted; nothing
    /// while disconnected or before the first read.
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let Some(device) = self.live.as_ref().and_then(|live| live.device.as_ref()) else {
            return;
        };
        let percent = proto::to_percent(device.level());
        let _ = write!(view.text_mut(), "{percent}%");
        let name = if device.description.as_str().is_empty() {
            device.name.as_str()
        } else {
            device.description.as_str()
        };
        let _ = write!(view.tooltip_mut(), "{name}: {percent}%");
        if device.muted {
            let _ = write!(view.tooltip_mut(), " (muted)");
        }
        if device.muted {
            view.set_class(crate::modules::Class::Muted);
        }
        let icon = match self.icon_override.as_ref() {
            Some(icon) => Some(icon),
            None => self.icons[icons::index(device.muted, percent)].as_ref(),
        };
        if let Some(icon) = icon {
            view.show_icon(icon);
        }
    }

    /// What `query` reports: the percent, the mute and the device name, or
    /// nothing while there is nothing shown.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        let device = self.live.as_ref().and_then(|live| live.device.as_ref())?;
        let percent = proto::to_percent(device.level());
        let mut value = serde_json::json!({
            "volume": percent,
            "muted": device.muted,
        });
        value[self.device_key()] = serde_json::Value::String(device.name.as_str().to_owned());
        Some(value)
    }

    /// A click toggles mute, a scroll raises or lowers; anything else means
    /// nothing.
    fn on_input(&self, input: &Input<'_>) -> Option<crate::action::Action> {
        let name = match input.trigger {
            Trigger::Click => "toggle-mute",
            Trigger::ScrollUp => "raise",
            Trigger::ScrollDown => "lower",
            _ => return None,
        };
        Some(crate::action::Action::Module(ModuleAction::new(name, None)))
    }

    /// Carries out `raise`, `lower` and `toggle-mute` as absolute sets from
    /// the last known level, capped at `max-volume`. Never `Changed`: what
    /// the server answers (an event, or the set's own re-read) moves the
    /// view within the turn.
    fn invoke(
        &mut self,
        _output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        match (&*action.name, action.arg) {
            ("raise" | "lower" | "toggle-mute", None) | ("set", Some(_)) => {}
            ("raise" | "lower" | "toggle-mute", Some(_)) => return Err(InvokeError::NoArg),
            ("set", None) => return Err(InvokeError::NeedsArg),
            _ => return Err(InvokeError::Unknown),
        }
        if self.live.is_none() {
            return Err(InvokeError::Refused("no sound server is running"));
        }
        if self.live.as_ref().is_some_and(|live| live.device.is_none()) {
            return Err(InvokeError::Refused(match self.kind {
                Kind::Sink => "no default sink to change",
                Kind::Source => "no default source to change",
            }));
        }
        match &*action.name {
            "raise" => {
                let base = self.base_volume().unwrap_or(0);
                let target = base
                    .saturating_add(steps.saturating_mul(self.step_raw))
                    .min(self.max_raw);
                if Some(target) == self.live.as_ref().and_then(|live| live.sent_volume)
                    || (target == base
                        && self
                            .live
                            .as_ref()
                            .is_some_and(|live| live.sent_volume.is_none()))
                {
                    return Ok(Update::Unchanged);
                }
                self.set_volume(target);
                Ok(Update::Unchanged)
            }
            "set" => {
                // A whole percent, held to `0..=max-volume`.
                let percent = u32::try_from(action.arg.unwrap_or(0)).unwrap_or(0);
                let base = self.base_volume().unwrap_or(0);
                let target = proto::from_percent(percent).min(self.max_raw);
                if Some(target) == self.live.as_ref().and_then(|live| live.sent_volume)
                    || (target == base
                        && self
                            .live
                            .as_ref()
                            .is_some_and(|live| live.sent_volume.is_none()))
                {
                    return Ok(Update::Unchanged);
                }
                self.set_volume(target);
                Ok(Update::Unchanged)
            }
            "lower" => {
                let base = self.base_volume().unwrap_or(0);
                let target = base.saturating_sub(steps.saturating_mul(self.step_raw));
                if Some(target) == self.live.as_ref().and_then(|live| live.sent_volume)
                    || (target == base
                        && self
                            .live
                            .as_ref()
                            .is_some_and(|live| live.sent_volume.is_none()))
                {
                    return Ok(Update::Unchanged);
                }
                self.set_volume(target);
                Ok(Update::Unchanged)
            }
            _ => {
                let muted = self.base_mute().unwrap_or(false);
                if self.live.as_ref().and_then(|live| live.sent_mute) == Some(!muted) {
                    return Ok(Update::Unchanged);
                }
                self.set_mute(!muted);
                Ok(Update::Unchanged)
            }
        }
    }

    /// A click mutes with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }

    /// The popup: what the device is and its level as a slider (to
    /// `max-volume`), and a button for the mute. Nothing while there is no
    /// device to change, which also closes an open popup (the server went).
    #[cfg(feature = "popup")]
    fn popup(&self, _output: &OutputView<'_>, content: &mut crate::popup::Content) -> bool {
        let Some(device) = self.live.as_ref().and_then(|live| live.device.as_ref()) else {
            return false;
        };
        let percent = proto::to_percent(device.level());
        let name = if device.description.as_str().is_empty() {
            device.name.as_str()
        } else {
            device.description.as_str()
        };
        content.text(format_args!("{name}  {percent}%"));
        content.slider(percent, proto::to_percent(self.max_raw), "set");
        content.button(
            format_args!("{}", if device.muted { "Unmute" } else { "Mute" }),
            "toggle-mute",
            None,
            false,
        );
        true
    }
}

/// What a reply's follow-up sends once the queued work is out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Follow {
    Nothing,
    Name,
    ServerInfo,
    Subscribe,
    DeviceQuery,
}

/// A frame classified, with a reply's payload located but not yet
/// shaped: its shape is known by what was asked, parsed in
/// [`Volume::on_reply`].
enum Parsed {
    Malformed,
    Reply { tag: u32, begin: usize, len: usize },
    Error { tag: u32, errno: u32 },
    Event(Event),
}

/// A reply's content, shaped by what was asked.
enum ReplyBody {
    Auth(u32),
    Name,
    Server(proto::ServerDefaults),
    Device(DeviceInfo),
    Ack,
}

/// Shapes a reply's payload by what it answers: `None` is malformed. An
/// acking reply must be empty: anything else answers nothing asked.
fn parse_reply(expect: Expect, payload: &[u8]) -> Option<ReplyBody> {
    match expect {
        Expect::Auth => {
            let mut reader = proto::Reader::new(payload);
            reader.get_u32().map(ReplyBody::Auth)
        }
        // SET_CLIENT_NAME answers its client index (a u32, ignored):
        // measured against pipewire-pulse, which does not send the empty
        // ack the other commands do.
        Expect::Name => {
            let mut reader = proto::Reader::new(payload);
            reader.get_u32().map(|_| ReplyBody::Name)
        }
        Expect::Server => proto::parse_server_info(payload).map(ReplyBody::Server),
        Expect::Device => proto::parse_device_info(payload).map(ReplyBody::Device),
        // An acking reply is empty: anything else answers nothing asked.
        // (SET_CLIENT_NAME is the exception: it answers a u32, above.
        // Only AUTH carries a value besides it, the protocol version.)
        Expect::Subscribed | Expect::SetVolume | Expect::SetMute => {
            if payload.is_empty() {
                Some(ReplyBody::Ack)
            } else {
                None
            }
        }
    }
}

fn parse_frame(cmd: u32, tag: u32, payload: &[u8], begin: usize) -> Parsed {
    if cmd == proto::CMD_SUBSCRIBE_EVENT {
        if tag != proto::INVALID_INDEX {
            return Parsed::Malformed;
        }
        return match proto::parse_event(payload) {
            Some(event) => Parsed::Event(event),
            None => Parsed::Malformed,
        };
    }
    if cmd == proto::CMD_ERROR {
        let mut reader = proto::Reader::new(payload);
        return match reader.get_u32() {
            Some(errno) => Parsed::Error { tag, errno },
            None => Parsed::Malformed,
        };
    }
    if cmd != proto::CMD_REPLY {
        return Parsed::Malformed;
    }
    // A reply's shape is known by what was asked; `on_reply` parses it.
    // Well-formedness shared by every shape here: an empty reply is an
    // ack, and anything else starts a tagged value.
    if !payload.is_empty() && !is_tagged(payload[0]) {
        return Parsed::Malformed;
    }
    Parsed::Reply {
        tag,
        begin,
        len: payload.len(),
    }
}

/// Whether the inotify bytes hold an event named `want`: records are a
/// native-endian header (watch, mask, cookie, name length) and the name.
/// Any malformed tail counts as interesting: re-probing a present socket
/// is one failed connect, while missing its appearance strands the module.
fn scan_names(buf: &[u8], want: &[u8]) -> bool {
    let mut rest = buf;
    if rest.is_empty() {
        return false;
    }
    loop {
        if rest.len() < 16 {
            return true;
        }
        let len = u32::from_ne_bytes([rest[12], rest[13], rest[14], rest[15]]) as usize;
        rest = &rest[16..];
        if rest.len() < len {
            return true;
        }
        let (name, tail) = rest.split_at(len);
        let end = name
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name.len());
        if &name[..end] == want {
            return true;
        }
        rest = tail;
        if rest.is_empty() {
            return false;
        }
    }
}

/// Whether `byte` starts a tagged value: every marker, since a bare
/// boolean's reply is always empty and never reaches here.
fn is_tagged(byte: u8) -> bool {
    matches!(
        byte,
        b't' | b'N'
            | b'L'
            | b'B'
            | b'R'
            | b'r'
            | b'a'
            | b'x'
            | b'1'
            | b'0'
            | b'T'
            | b'U'
            | b'm'
            | b'v'
            | b'P'
            | b'V'
            | b'f'
    )
}

/// Frames handled per ready turn at most: a flooding server is sixteen
/// redraws, not an unbounded storm.
const FRAMES_PER_TURN: usize = 16;

/// What one read of the socket found.
enum Fill {
    /// The connection is over: closed, errored or refused.
    Dropped,
    /// Nothing complete to handle yet.
    Waiting,
    /// A whole frame waits in the buffer.
    Frame,
}

impl Live {
    /// Sends a request: frames it into the outbox and flushes what goes.
    /// One request is outstanding at a time; the callers queue the rest.
    fn request(&mut self, cmd: u32, payload: &[u8], expect: Expect) {
        let tag = self.tag;
        self.tag = self.tag.wrapping_add(1);
        let Some(len) = proto::encode_into(cmd, tag, payload, &mut self.out) else {
            return;
        };
        self.out_len = len;
        self.out_sent = 0;
        self.outstanding = Some((tag, expect));
        self.flush();
    }

    /// Writes what the outbox holds: `false` when the connection is over.
    /// A partial write stays queued and is finished on the next `OUT`.
    fn flush(&mut self) -> bool {
        while self.out_sent < self.out_len {
            match rustix::io::write(&self.fd, &self.out[self.out_sent..self.out_len]) {
                Ok(0) => return false,
                Ok(n) => self.out_sent += n,
                Err(Errno::AGAIN) => break,
                Err(_) => return false,
            }
        }
        if self.out_sent >= self.out_len {
            self.out_len = 0;
            self.out_sent = 0;
        }
        true
    }

    /// Whether a partial write waits: the loop then polls `OUT` too.
    fn needs_write(&self) -> bool {
        self.out_len > self.out_sent
    }

    /// Reads what the socket has into the buffer's free tail: `Frame` when
    /// a whole frame waits (read or already there), `Waiting` when more is
    /// needed, `Dropped` when the connection is over.
    fn fill(&mut self) -> Fill {
        if let Ok(Some(_)) = proto::frame_at(&self.read[self.start..self.end]) {
            return Fill::Frame;
        }
        if self.end >= self.read.len() {
            // Full of an unframable prefix: the server is not framing.
            return Fill::Dropped;
        }
        match rustix::io::read(&self.fd, &mut self.read[self.end..]) {
            Ok(0) => Fill::Dropped,
            Ok(n) => {
                self.end += n;
                match proto::frame_at(&self.read[self.start..self.end]) {
                    Ok(Some(_)) => Fill::Frame,
                    Ok(None) => Fill::Waiting,
                    Err(()) => Fill::Dropped,
                }
            }
            Err(Errno::AGAIN) => Fill::Waiting,
            Err(_) => Fill::Dropped,
        }
    }
}
