//! The bar feed: native push to the bar's `notifications` push module.
//!
//! On every change of DND or of the unread count, and only then, the
//! daemon connects to the bar's control socket
//! (`$XDG_RUNTIME_DIR/scootbar-<display>.sock`) and sends
//! `set notifications <payload>`. The payload is the shape
//! `scoot-notify-sync` sends (`nix/modules/notifications-home.nix`):
//!
//! - DND: `{text: "DND n" | "DND", class: "muted", tooltip, icon?}`;
//! - unread: `{text: "n", class: "urgent" | "normal", tooltip, icon?}`,
//!   where `urgent` means at least one counted critical;
//! - idle: `{text: ""}`.
//!
//! The counted `n` is live cards plus unseen history: a card on screen
//! counts live; one held by DND or a rule counts unseen. Icons ride
//! per update from `bar_icon_unread` / `bar_icon_dnd` (empty sends
//! none, leaving the key out, exactly as the script does).
//!
//! A bar that is not running fails the send, and the next change
//! retries: nothing polls. A one-shot inotify watch on the runtime
//! dir exists only while a send is owed, so a bar started later is
//! caught up on the next change (until then it shows its
//! placeholder), and an idle daemon holds no watch, no timer and no
//! extra wakeups. A module that is not placed answers an error, and
//! the daemon logs the guidance once per run. No `unwrap`, `expect`
//! or panic on any path: every refusal degrades to a line and an owed
//! send.

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::model::dnd::Dnd;
use crate::model::store::Store;

/// The bar module the feed writes: the `push` module id the Nix side
/// places (`nix/modules/scootbar.nix`), unchanged by this ticket.
pub const MODULE_ID: &str = "notifications";

/// How long one push waits on a bar that accepted the connection but
/// does not answer, before giving up rather than stalling the loop's
/// expiries behind it. The bar answers in one turn when healthy.
const PUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// The longest control reply read. Replies are one line (`ok` or an
/// error); this only bounds a daemon gone wrong.
const MAX_REPLY: u64 = 64 * 1024;

/// What the feed counts: the mode plus the bar-visible numbers. The
/// daemon pushes on a change of this, and only then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    /// Do-not-disturb: changes the payload's shape, not just its count.
    pub dnd: Dnd,
    /// Live cards plus unseen history.
    pub unread: usize,
    /// Whether any counted card is critical.
    pub critical: bool,
}

impl State {
    /// Reads the bar-visible numbers from the store. No allocation:
    /// the daemon calls this every turn.
    pub fn of(store: &Store) -> Self {
        Self {
            dnd: store.dnd(),
            unread: store.feed_count(),
            critical: store.feed_has_critical(),
        }
    }
}

/// Builds the exact `scoot-notify-sync` payload for `state`: the
/// script's `jq` shape, key for key. Empty icons leave the key out, as
/// the script's `with_icon` does. No I/O; allocates only the returned
/// value (called on change, never per frame).
pub fn payload(state: State, icon_unread: &str, icon_dnd: &str) -> serde_json::Value {
    use serde_json::{Map, Value};
    let mut object = Map::new();
    if state.dnd.holds() {
        object.insert(
            "text".to_owned(),
            Value::String(if state.unread > 0 {
                format!("DND {}", state.unread)
            } else {
                String::from("DND")
            }),
        );
        object.insert("class".to_owned(), Value::String(String::from("muted")));
        object.insert(
            "tooltip".to_owned(),
            Value::String(if state.unread > 0 {
                format!(
                    "{} notifications held by do-not-disturb -- click to let them through",
                    state.unread
                )
            } else {
                String::from("do-not-disturb is on -- click to let notifications through")
            }),
        );
        if !icon_dnd.is_empty() {
            object.insert(
                "icon".to_owned(),
                Value::String(icon_dnd.to_owned()),
            );
        }
    } else if state.unread > 0 {
        object.insert(
            "text".to_owned(),
            Value::String(format!("{}", state.unread)),
        );
        object.insert(
            "class".to_owned(),
            Value::String(String::from(if state.critical { "urgent" } else { "normal" })),
        );
        object.insert(
            "tooltip".to_owned(),
            Value::String(format!(
                "{} notifications -- click to hold them with do-not-disturb",
                state.unread
            )),
        );
        if !icon_unread.is_empty() {
            object.insert(
                "icon".to_owned(),
                Value::String(icon_unread.to_owned()),
            );
        }
    } else {
        object.insert("text".to_owned(), Value::String(String::new()));
    }
    Value::Object(object)
}

/// Where the bar's socket lives: the environment's, or a fixed path
/// the tests hand over (a scratch socket, never the process
/// environment).
#[derive(Debug)]
enum Target {
    Env,
    Fixed(PathBuf),
}

/// The feed: the last state placed on the bar, whether a send is
/// owed, and the one-shot watch for the bar's socket appearing.
pub struct Feed {
    target: Target,
    icon_unread: String,
    icon_dnd: String,
    /// The last state placed (or refused as not-placed): `None` until
    /// the startup baseline, which counts as placed without sending.
    last: Option<State>,
    /// A send is owed: the last attempt found no bar. Retried on the
    /// next change, or when the watch fires.
    owed: bool,
    /// The not-placed guidance already logged this run.
    placed_logged: bool,
    /// The inotify watch on the runtime dir, armed only while owed,
    /// with the socket file name it scans for.
    watch: Option<Watch>,
}

/// One armed watch: the inotify fd plus the socket file name whose
/// appearance retires it.
struct Watch {
    fd: OwnedFd,
    /// The socket's final component, as bytes.
    name: Vec<u8>,
}

impl Feed {
    /// A feed pushing to the environment's bar socket, with `config`'s
    /// icons.
    pub fn new(config: &crate::config::Config) -> Self {
        Self {
            target: Target::Env,
            icon_unread: config.bar_icon_unread.clone(),
            icon_dnd: config.bar_icon_dnd.clone(),
            last: None,
            owed: false,
            placed_logged: false,
            watch: None,
        }
    }

    /// A feed pushing to `socket` (the unit tests' fake bar), with the
    /// given icons.
    #[cfg(test)]
    pub fn for_test(socket: PathBuf, icon_unread: &str, icon_dnd: &str) -> Self {
        Self {
            target: Target::Fixed(socket),
            icon_unread: icon_unread.to_owned(),
            icon_dnd: icon_dnd.to_owned(),
            last: None,
            owed: false,
            placed_logged: false,
            watch: None,
        }
    }

    /// Takes `config`'s icons (`msg reload`): the next change pushes
    /// with them.
    pub fn set_icons(&mut self, config: &crate::config::Config) {
        self.icon_unread.clone_from(&config.bar_icon_unread);
        self.icon_dnd.clone_from(&config.bar_icon_dnd);
    }

    /// Baselines without sending: the startup state counts as placed,
    /// so the first push waits for the first change (a bar started
    /// later shows its placeholder until then).
    pub fn baseline(&mut self, store: &Store) {
        self.last = Some(State::of(store));
    }

    /// Whether a push is owed: the last attempt found no bar.
    pub fn owed(&self) -> bool {
        self.owed
    }

    /// The watch fd for the daemon's poll set: `None` unless a send is
    /// owed. Never a timeout: the feed adds no wakeups of its own.
    pub fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
        self.watch.as_ref().map(|watch| watch.fd.as_fd())
    }

    /// Once per turn, after every store mutation: pushes when the
    /// state changed since the last placement. Returns a line for the
    /// log when the attempt failed (one line per failed change, as the
    /// script's one line per failed sync). Success is silent.
    pub fn sync(&mut self, store: &Store) -> Option<String> {
        let current = State::of(store);
        if self.last == Some(current) {
            return None;
        }
        self.attempt(current)
    }

    /// Retries when the watch fired and the socket may have appeared:
    /// pushes the current state when a send is owed. Returns a line
    /// for the log when the retry failed. Drains the watch either way;
    /// a drained watch with nothing owed is disarmed.
    pub fn retry(&mut self, store: &Store) -> Option<String> {
        if !self.socket_may_have_appeared() {
            return None;
        }
        if !self.owed {
            return None;
        }
        self.attempt(State::of(store))
    }

    /// Attempts one push of `current`: updates the placement, the owed
    /// flag and the watch. Returns a line for the log on failure.
    fn attempt(&mut self, current: State) -> Option<String> {
        match send_to(&self.socket_path()?, &payload(current, &self.icon_unread, &self.icon_dnd)) {
            Send::Sent => {
                self.last = Some(current);
                self.owed = false;
                self.disarm();
                None
            }
            Send::NotPlaced => {
                // Considered placed: retrying the same payload cannot
                // help, and the next change retries (silently). The
                // guidance goes out once per run.
                self.last = Some(current);
                self.owed = false;
                self.disarm();
                if self.placed_logged {
                    None
                } else {
                    self.placed_logged = true;
                    Some(String::from(
                        "scootnotify: the bar shows no notifications module (add \"notifications\" to a bar list in programs.scootbar.settings); leaving it",
                    ))
                }
            }
            Send::NoBar(detail) => {
                self.owed = true;
                self.arm();
                // The bar may have started between the failed dial and
                // the armed watch: one immediate retry closes that
                // race, and its outcome is the reported one.
                match send_to(
                    &self.socket_path()?,
                    &payload(current, &self.icon_unread, &self.icon_dnd),
                ) {
                    Send::Sent => {
                        self.last = Some(current);
                        self.owed = false;
                        self.disarm();
                        None
                    }
                    Send::NotPlaced => {
                        self.last = Some(current);
                        self.owed = false;
                        self.disarm();
                        if self.placed_logged {
                            None
                        } else {
                            self.placed_logged = true;
                            Some(String::from(
                                "scootnotify: the bar shows no notifications module (add \"notifications\" to a bar list in programs.scootbar.settings); leaving it",
                            ))
                        }
                    }
                    Send::NoBar(_) | Send::Refused(_) => {
                        Some(format!("scootnotify: cannot reach the bar ({detail}); leaving it"))
                    }
                }
            }
            Send::Refused(detail) => {
                // The bar answered with an error that is not a missing
                // module (unreachable with our fixed shape): owed, so
                // the next change retries, but no watch (the socket
                // exists, so no appearance will fire).
                self.owed = true;
                Some(format!("scootnotify: cannot reach the bar ({detail}); leaving it"))
            }
        }
    }

    /// The bar's socket path: `None` when the environment names none
    /// (no runtime dir, a too-long path), which reads as no bar.
    fn socket_path(&self) -> Option<PathBuf> {
        match &self.target {
            Target::Fixed(path) => Some(path.clone()),
            Target::Env => scootui::control::paths::from_env("scootbar")
                .map(|paths| paths.socket)
                .ok(),
        }
    }

    /// Arms the one-shot watch on the runtime dir for the socket's
    /// appearance. Best effort: no watch means the next change still
    /// retries, only a bar started later waits longer.
    fn arm(&mut self) {
        if self.watch.is_some() {
            return;
        }
        let Some(socket) = self.socket_path() else {
            return;
        };
        let Some(dir) = socket.parent() else {
            return;
        };
        if !dir.is_dir() {
            return;
        }
        let Some(name) = socket
            .file_name()
            .map(|name| name.as_encoded_bytes().to_vec())
        else {
            return;
        };
        use rustix::fs::inotify::{CreateFlags, WatchFlags, add_watch, init};
        let watch = init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)
            .ok()
            .and_then(|fd| {
                add_watch(&fd, dir, WatchFlags::CREATE | WatchFlags::MOVED_TO)
                    .ok()
                    .map(|_| Watch { fd, name })
            });
        self.watch = watch;
    }

    /// Disarms the watch: nothing is owed, so nothing watches.
    fn disarm(&mut self) {
        self.watch = None;
    }

    /// Drains the watch: whether the bar's socket may have appeared
    /// since. An unframed buffer answers `true` (a needless dial costs
    /// less than a missed bar), like the D-Bus link's scan.
    fn socket_may_have_appeared(&mut self) -> bool {
        let Some(watch) = self.watch.as_ref() else {
            return false;
        };
        let mut buf = [0u8; 1024];
        let mut seen = false;
        loop {
            match rustix::io::read(&watch.fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if scan_names(&buf[..n], &watch.name) {
                        seen = true;
                    }
                    // Level-triggered: keep draining while it speaks.
                    continue;
                }
                Err(_) => break,
            }
        }
        seen
    }
}

/// Whether an inotify buffer holds an event for `want`: whole-field
/// matching on the file name. An unframed buffer answers `true`: a
/// needless dial costs less than a missed bar (the D-Bus link's rule).
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
        // The name is NUL-padded in the event: compare without it.
        let name = name.split(|byte| *byte == 0).next().unwrap_or(&[]);
        if name == want {
            return true;
        }
        rest = tail;
        if rest.is_empty() {
            return false;
        }
    }
}

/// What one push attempt did.
enum Send {
    /// The bar answered `ok`.
    Sent,
    /// The bar answered an error naming the unplaced module.
    NotPlaced,
    /// Nothing listens (or no socket path resolves): owed.
    NoBar(String),
    /// The bar answered another error: owed, without a watch.
    Refused(String),
}

/// Sends one `set notifications` line and reads the one-line reply.
fn send_to(socket: &Path, payload: &serde_json::Value) -> Send {
    let mut stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error) => return Send::NoBar(short_error(&error)),
    };
    if stream.set_write_timeout(Some(PUSH_TIMEOUT)).is_err() {
        return Send::NoBar(String::from("cannot set a write timeout"));
    }
    if stream.set_read_timeout(Some(PUSH_TIMEOUT)).is_err() {
        return Send::NoBar(String::from("cannot set a read timeout"));
    }
    let line = set_line(payload);
    {
        use std::io::Write;
        if stream.write_all(&line).is_err() {
            return Send::NoBar(String::from("the bar closed the connection"));
        }
    }
    let reply = {
        use std::io::{BufRead, BufReader, Read};
        let mut reader = BufReader::new(&stream);
        let mut reply = Vec::new();
        match reader.take(MAX_REPLY).read_until(b'\n', &mut reply) {
            Ok(_) => reply,
            Err(error) => return Send::NoBar(short_error(&error)),
        }
    };
    parse_reply(&reply)
}

/// The request line: the bar's protocol version, `set`, our module.
fn set_line(payload: &serde_json::Value) -> Vec<u8> {
    let mut line = Vec::new();
    line.extend_from_slice(br#"{"protocol":1,"type":"set","id":""#);
    line.extend_from_slice(MODULE_ID.as_bytes());
    line.extend_from_slice(br#"","value":"#);
    // Serializing strings and numbers into a `Vec` cannot fail; if it
    // ever did, the empty value gets a `Malformed` refusal, not a
    // panic, and the send is owed like any other failure.
    if serde_json::to_writer(&mut line, payload).is_err() {
        line.extend_from_slice(b"null");
    }
    line.push(b'\n');
    line
}

/// Reads the one-line reply: `ok` is sent, an error naming the
/// unplaced module is not-placed, any other error is refused.
fn parse_reply(reply: &[u8]) -> Send {
    #[derive(serde::Deserialize)]
    struct Reply {
        #[serde(rename = "type")]
        kind: String,
        message: Option<String>,
    }
    match serde_json::from_slice::<Reply>(reply) {
        Ok(reply) if reply.kind == "ok" => Send::Sent,
        Ok(reply) if reply.kind == "error" => {
            let message = reply.message.unwrap_or_default();
            if message.contains("is not placed") {
                Send::NotPlaced
            } else {
                Send::Refused(message)
            }
        }
        _ => Send::Refused(String::from("unreadable reply from the bar")),
    }
}

/// One line for the log from an I/O error: the kind, never a dump.
fn short_error(error: &std::io::Error) -> String {
    format!("{error}")
}

#[cfg(test)]
mod tests;
