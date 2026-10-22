//! The compositor half: requests over one persistent connection, and
//! the event subscription over another.
//!
//! The request connection persists for the backend's life: the
//! compositor owns each registration by the connection that made it,
//! so a fresh connection per call would unregister every shortcut as
//! soon as it was registered. A transport failure drops it and the next
//! call redials (with read and write timeouts, so a half-dead
//! compositor fails the call instead of wedging the daemon loop). The
//! subscription holds its own connection for its life and forwards
//! `Activated`/`Deactivated`/`ShortcutsChanged` to the D-Bus loop over
//! a channel, waking it through an eventfd beside the channel.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::time::Duration;

use rustix::event::{EventfdFlags, eventfd};
use rustix::fd::OwnedFd;
use scoot_ipc::{Client, EventKind, Request, Response, ShortcutInfo};

/// How long one request waits for the compositor to answer before the
/// connection is dropped as half-dead: without a timeout a wedged read
/// stalls the daemon loop (and every portal call behind it) forever --
/// a dead compositor normally closes the socket promptly (EOF), but a
/// half-open one need not.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Connects with read and write timeouts, so a half-dead compositor
/// fails a call instead of wedging the daemon loop behind it forever.
fn connect_client() -> io::Result<Client> {
    let path = scoot_ipc::socket_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "neither SCOOT_SOCKET nor XDG_RUNTIME_DIR is set",
        )
    })?;
    let stream = UnixStream::connect(&path)?;
    stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    Client::from_stream(stream)
}

/// The wake-up beside the event channel: the subscription thread owns
/// one end and the D-Bus loop polls the other, so a portal signal never
/// waits for bus traffic to be noticed. An `eventfd` counter (one byte
/// per event would do; the counter coalesces bursts): the loop drains
/// it, then the channel, every turn it is readable.
pub struct Wake {
    sender: Sender<IpcEvent>,
    fd: OwnedFd,
}

impl Wake {
    /// The thread half plus the loop's poll half (a shared counter: the
    /// loop drains what the thread writes).
    pub fn new(sender: Sender<IpcEvent>) -> io::Result<(Self, OwnedFd)> {
        let fd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        let poll = fd.try_clone()?;
        Ok((Self { sender, fd }, poll))
    }

    /// Sends the event and wakes the loop. Returns whether the loop is
    /// still there: `false` tells the subscription thread to stop.
    /// Never blocks: the channel is unbounded and the counter write is
    /// best-effort (a full counter means the loop is already awake).
    pub fn send(&self, event: IpcEvent) -> bool {
        let live = self.sender.send(event).is_ok();
        let _ = rustix::io::write(&self.fd, &1u64.to_ne_bytes());
        live
    }
}

/// One compositor event the D-Bus loop turns into a portal signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcEvent {
    Activated {
        app_id: String,
        shortcut_id: String,
        timestamp_ms: u32,
    },
    Deactivated {
        app_id: String,
        shortcut_id: String,
        timestamp_ms: u32,
    },
    Changed,
}

/// How many times one call redials the compositor before giving up
/// (about 1 second): a restart in progress answers the next try, a dead
/// compositor fails fast enough to keep the portal responsive.
const RETRIES: u32 = 5;
const RETRY_PAUSE: Duration = Duration::from_millis(200);

/// One persistent compositor connection for every request this backend
/// makes. Persistence is load-bearing, not an optimization: the
/// compositor owns each registration by the connection that made it and
/// drops it when that connection closes, so a fresh connection per call
/// would unregister every shortcut as soon as it was registered. One
/// connection for the backend's life; a transport failure drops it and
/// the next call redials, and the caller re-registers what the
/// redial forgot (see the daemon's heal).
pub struct Compositor {
    client: Option<Client>,
}

impl Compositor {
    pub fn new() -> Self {
        Self { client: None }
    }

    /// Registers (upserts) one shortcut with the compositor. The user's
    /// chord needs no extra step: a fresh register applies
    /// `[global_shortcuts]` itself, and a bound chord stays bound
    /// across the upsert.
    pub fn register(
        &mut self,
        app_id: &str,
        shortcut_id: &str,
        preferred_trigger: Option<&str>,
        description: Option<&str>,
    ) -> io::Result<Vec<ShortcutInfo>> {
        let request = Request::ShortcutRegister {
            app_id: app_id.to_owned(),
            shortcut_id: shortcut_id.to_owned(),
            preferred_trigger: preferred_trigger.map(str::to_owned),
            description: description.map(str::to_owned),
        };
        self.round_trip(request)
    }

    /// One removal attempt, no redial: the cleanup half of
    /// [`Compositor::list_once`] (session close and app death must not
    /// wedge the loop behind a dead compositor, one redial per
    /// shortcut).
    pub fn unregister_once(
        &mut self,
        app_id: &str,
        shortcut_id: &str,
    ) -> io::Result<Vec<ShortcutInfo>> {
        self.round_once(Request::ShortcutUnregister {
            app_id: app_id.to_owned(),
            shortcut_id: shortcut_id.to_owned(),
        })
    }

    /// Lists every registered shortcut.
    pub fn list(&mut self) -> io::Result<Vec<ShortcutInfo>> {
        self.round_trip(Request::Shortcuts)
    }

    /// One attempt, no redial: what the background heal uses. A
    /// redialed-every-time heal would wedge the D-Bus loop behind a
    /// dead compositor; a single refused connect is instant, and the
    /// next `Changed` (the resubscribe that only succeeds once the
    /// compositor serves again) retries it.
    pub fn list_once(&mut self) -> io::Result<Vec<ShortcutInfo>> {
        self.round_once(Request::Shortcuts)
    }

    /// One registration attempt, no redial: the heal's half of
    /// [`Compositor::list_once`].
    pub fn register_once(
        &mut self,
        app_id: &str,
        shortcut_id: &str,
        preferred_trigger: Option<&str>,
        description: Option<&str>,
    ) -> io::Result<Vec<ShortcutInfo>> {
        self.round_once(Request::ShortcutRegister {
            app_id: app_id.to_owned(),
            shortcut_id: shortcut_id.to_owned(),
            preferred_trigger: preferred_trigger.map(str::to_owned),
            description: description.map(str::to_owned),
        })
    }

    fn round_once(&mut self, request: Request) -> io::Result<Vec<ShortcutInfo>> {
        if self.client.is_none() {
            self.client = connect_client().ok();
        }
        let answer = self
            .client
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no compositor"))?
            .request(&request);
        match answer {
            Ok(Response::Shortcuts { shortcuts }) => Ok(shortcuts),
            Ok(Response::Error { message }) => {
                Err(io::Error::new(io::ErrorKind::InvalidInput, message))
            }
            Ok(other) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected reply: {other:?}"),
            )),
            Err(_) => {
                self.client = None;
                Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "the compositor went away",
                ))
            }
        }
    }

    fn round_trip(&mut self, request: Request) -> io::Result<Vec<ShortcutInfo>> {
        // Two attempts past the current connection: the live one, then
        // one redial after a transport failure. A second failure is
        // the compositor being down, not a race worth more retries (the
        // daemon's heal covers the restart).
        for attempt in 0..3 {
            if self.client.is_none() {
                self.redial()?;
            }
            let answer = self.client.as_mut().expect("redialed").request(&request);
            match answer {
                Ok(Response::Shortcuts { shortcuts }) => return Ok(shortcuts),
                Ok(Response::Error { message }) => {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, message));
                }
                Ok(other) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected reply: {other:?}"),
                    ));
                }
                Err(_) if attempt < 2 => {
                    self.client = None;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(io::ErrorKind::NotConnected, "no compositor"))
    }

    fn redial(&mut self) -> io::Result<()> {
        let mut last = io::Error::new(io::ErrorKind::NotConnected, "no compositor");
        for _ in 0..RETRIES {
            match connect_client() {
                Ok(client) => {
                    self.client = Some(client);
                    return Ok(());
                }
                Err(error) => {
                    last = error;
                    std::thread::sleep(RETRY_PAUSE);
                }
            }
        }
        Err(last)
    }
}

/// Holds the `global-shortcut` subscription for the process's life,
/// forwarding events to `wake` (which wakes the D-Bus loop) and
/// reconnecting when the compositor restarts. A lost subscription sends
/// `Changed` (the D-Bus loop heals what the compositor forgot, then
/// fans out). Returns only when the loop is gone.
pub fn subscribe_forever(wake: Wake) {
    let mut pause = Duration::from_millis(200);
    loop {
        match subscribe_once(&wake) {
            // The receiver went away: the daemon is gone, so are we.
            Ok(()) => return,
            Err(()) => {
                wake.send(IpcEvent::Changed);
            }
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_secs(5));
    }
}

fn subscribe_once(wake: &Wake) -> Result<(), ()> {
    let mut client = connect_client().map_err(|_| ())?;
    let reply = client
        .request(&Request::Subscribe {
            events: vec![EventKind::GlobalShortcut],
        })
        .map_err(|_| ())?;
    if !matches!(reply, Response::Subscribed { .. }) {
        return Err(());
    }
    // The subscription is live, so the compositor is serving: tell the
    // D-Bus loop to heal (a redialed connection starts with a wiped
    // table) and re-list. Fires on every (re)subscribe; with no
    // sessions remembered it is a skipped no-op.
    wake.send(IpcEvent::Changed);
    loop {
        match client.next_message() {
            Ok(Some(Response::ShortcutActivated {
                app_id,
                shortcut_id,
                timestamp_ms,
            })) => {
                if !wake.send(IpcEvent::Activated {
                    app_id,
                    shortcut_id,
                    timestamp_ms,
                }) {
                    return Ok(());
                }
            }
            Ok(Some(Response::ShortcutDeactivated {
                app_id,
                shortcut_id,
                timestamp_ms,
            })) => {
                if !wake.send(IpcEvent::Deactivated {
                    app_id,
                    shortcut_id,
                    timestamp_ms,
                }) {
                    return Ok(());
                }
            }
            Ok(Some(Response::ShortcutsChanged)) => {
                if !wake.send(IpcEvent::Changed) {
                    return Ok(());
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => return Err(()),
            Err(_) => return Err(()),
        }
    }
}
