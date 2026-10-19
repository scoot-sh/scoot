//! What the daemon answers to each control request.
//!
//! Every request is answered at once, from the daemon's state: `list`
//! reads the live set, `dismiss` and `invoke` close and invoke as a click
//! would (emitting the bus signals too), `subscribe` dedicates the
//! connection to events, `reload` re-reads the config file and the look
//! layer and live-applies them, and `version` is what it sounds like.
//!
//! A refused `reload` changes nothing: the file is re-read and fully
//! validated before the running config is touched, and a refused look
//! fragment never reaches the cards. The popup half of a successful
//! reload (re-theme, re-layout) is recorded in [`Responder::applied`]
//! for the daemon loop, which owns the popup.

use std::path::{Path, PathBuf};
use std::time::Instant;

use scoot_dbus::conn::Conn;

use super::protocol::{
    PROTOCOL_VERSION, ReloadScope, Reply, Request, write_reply,
};
use super::{Handler, Kinds, MAX_SUBSCRIBERS};
use crate::bus::server::Server;
use crate::config::{self, Config};
use crate::look::{self, Look};

/// What a successful reload applied: the config for the popup to lay out
/// from (`None` for a scoped reload, which never touches it), the look to
/// re-theme to, and the warnings to log. The store half is already
/// applied; the daemon loop applies this half before its next draw, so
/// cards on screen keep their ids, clocks and hover.
pub struct AppliedReload {
    pub config: Option<Config>,
    pub look: Look,
    pub notes: Vec<String>,
}

/// The request handler for one round of the poll loop.
pub struct Responder<'a> {
    /// Set by a successful `reload`: the popup half, for the loop.
    pub applied: Option<AppliedReload>,
    server: &'a mut Server,
    /// The bus connection: dismissals and invocations emit their
    /// `NotificationClosed` and `ActionInvoked` signals on it, as clicks
    /// do.
    bus: &'a mut Conn,
    /// The config file a plain `reload` re-reads.
    config_path: &'a PathBuf,
    /// The look state's directory a reload reads the layer from (`None`
    /// is no layer, as at startup).
    look_state: Option<PathBuf>,
    now: Instant,
    now_ms: u64,
    /// How many connections are subscribed now, for the cap.
    subscribers: usize,
    /// Set by a `subscribe` just handled: what the connection now carries.
    subscription: Option<Kinds>,
}

impl<'a> Responder<'a> {
    pub fn new(
        server: &'a mut Server,
        bus: &'a mut Conn,
        config_path: &'a PathBuf,
        look_state: Option<&Path>,
        now: Instant,
        now_ms: u64,
        subscribers: usize,
    ) -> Self {
        Self {
            applied: None,
            server,
            bus,
            config_path,
            look_state: look_state.map(Path::to_path_buf),
            now,
            now_ms,
            subscribers,
            subscription: None,
        }
    }
}

impl Handler for Responder<'_> {
    fn take_subscription(&mut self) -> Option<Kinds> {
        self.subscription.take()
    }

    fn handle(&mut self, line: &[u8], out: &mut Vec<u8>) {
        let reply;
        let request = match super::protocol::parse(line) {
            Ok(request) => request,
            Err(error) => {
                write_reply(out, &Reply::Error { message: &error });
                return;
            }
        };
        match request {
            Request::List => {
                let output = self.server.store().output_name();
                let notifications = self
                    .server
                    .store()
                    .ordered()
                    .iter()
                    .map(|note| super::protocol::view(note, self.now, self.now_ms, output))
                    .collect();
                reply = Reply::Notifications { notifications };
                write_reply(out, &reply);
            }
            Request::Dismiss { id, all } => match (id, all) {
                (Some(id), _) => match self.server.dismiss_and_signal(self.bus, id) {
                    true => write_reply(out, &Reply::Ok { scope: None }),
                    false => write_reply(
                        out,
                        &Reply::Error {
                            message: &format_args!("unknown notification id {id}"),
                        },
                    ),
                },
                (None, true) => {
                    self.server.dismiss_all_and_signal(self.bus);
                    write_reply(out, &Reply::Ok { scope: None });
                }
                // Unreachable: the parser refuses a `dismiss` with neither
                // an id nor `"all":true`. A loud error, not a panic, if it
                // ever happens.
                (None, false) => write_reply(
                    out,
                    &Reply::Error {
                        message: &"dismiss needs an id or `\"all\":true`",
                    },
                ),
            },
            Request::Invoke { id, action } => {
                if self
                    .server
                    .invoke_and_signal(self.bus, id, &action, None)
                {
                    write_reply(out, &Reply::Ok { scope: None });
                } else if self.server.store().get(id).is_none() {
                    write_reply(
                        out,
                        &Reply::Error {
                            message: &format_args!("unknown notification id {id}"),
                        },
                    );
                } else {
                    write_reply(
                        out,
                        &Reply::Error {
                            message: &format_args!(
                                "notification {id} has no action `{action}`"
                            ),
                        },
                    );
                }
            }
            Request::Reload { scope } => match scope {
                None => match self.reload() {
                    Ok(()) => write_reply(out, &Reply::Ok { scope: None }),
                    Err(message) => write_reply(out, &Reply::Error { message: &message }),
                },
                Some(ReloadScope::Look) => match self.reload_scoped() {
                    Ok(()) => write_reply(
                        out,
                        &Reply::Ok {
                            scope: Some(ReloadScope::Look.name()),
                        },
                    ),
                    Err(message) => write_reply(out, &Reply::Error { message: &message }),
                },
            },
            Request::Subscribe { events } => {
                if self.subscribers >= MAX_SUBSCRIBERS {
                    write_reply(
                        out,
                        &Reply::Error {
                            message: &format_args!(
                                "at most {MAX_SUBSCRIBERS} connections may be subscribed at once"
                            ),
                        },
                    );
                } else {
                    write_reply(
                        out,
                        &Reply::Subscribed {
                            events: events.iter().map(|kind| kind.name()).collect(),
                        },
                    );
                    self.subscription = Some(Kinds::of(
                        &events.iter().map(|kind| *kind as u8).collect::<Vec<_>>(),
                    ));
                    self.subscribers += 1;
                }
            }
            Request::Version => {
                reply = Reply::Version {
                    protocol: PROTOCOL_VERSION,
                    version: env!("CARGO_PKG_VERSION"),
                };
                write_reply(out, &reply);
            }
        }
    }
}

impl Responder<'_> {
    /// Re-reads the config file and the look layer, and live-applies
    /// them. `Err` is the reply, and the running daemon stands exactly as
    /// it was: the store moves only after both halves validate.
    fn reload(&mut self) -> Result<(), String> {
        let config = read_or_defaults(self.config_path)?;
        let (look, mut notes) = reload_layer_applied(self.look_state.as_deref())?;
        self.server.store_mut().set_config(config.clone());
        // Cards on screen keep their ids, clocks and hover: the loop
        // re-themes and re-lays-out under them.
        self.applied = Some(AppliedReload {
            config: Some(config),
            look,
            notes: std::mem::take(&mut notes),
        });
        Ok(())
    }

    /// Re-reads only the look layer: the look-scoped reload. The user
    /// file is never touched, so pending edits wait for a plain reload.
    /// `Err` is the reply, and the running daemon stands exactly as it
    /// was.
    fn reload_scoped(&mut self) -> Result<(), String> {
        let (look, notes) = match look::reload_layer(self.look_state.as_deref())? {
            Some((look, notes)) => (look, notes),
            None => {
                return Err(String::from(
                    "no look layer is configured, refusing the look-scoped reload",
                ));
            }
        };
        self.applied = Some(AppliedReload {
            config: None,
            look,
            notes,
        });
        Ok(())
    }
}

/// Re-reads the look layer for a plain reload: a missing layer is the
/// built-in defaults with a note (as at startup), a refused fragment
/// keeps the last good look.
fn reload_layer_applied(state: Option<&Path>) -> Result<(Look, Vec<String>), String> {
    match look::reload_layer(state)? {
        Some((look, notes)) => Ok((look, notes)),
        None => Ok((Look::default(), vec![String::from("no look layer: built-in defaults apply")])),
    }
}

/// Reads the config file, or the defaults when it is missing (as at
/// startup): `Err` only for a present-but-bad file.
fn read_or_defaults(path: &PathBuf) -> Result<Config, String> {
    match std::fs::read(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        Ok(bytes) => config::parse(&bytes),
    }
}
