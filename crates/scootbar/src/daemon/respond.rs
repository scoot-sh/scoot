//! What the daemon answers to each control request.
//!
//! Every request is answered at once, from the daemon's state: `query`
//! and `layout` read what is drawn (`agent`), `invoke` runs an action as a
//! click would, `subscribe` dedicates the connection to events (`events`),
//! `reload` re-reads the
//! config file and live-applies it, `version` and `kill` are what they
//! sound like, and `set` goes to the module (only `push` takes one).
//!
//! A refused `reload` changes nothing: the file is re-read and fully
//! validated before anything is touched, and the font and the modules are
//! started before the running ones are swapped out.

use std::path::PathBuf;

use wayland_client::{Connection, QueueHandle};

use super::Content;
use super::agent;
use super::wayland::State;
use crate::cli::Given;
use crate::config::{self, Config};
use crate::control::protocol::{PROTOCOL_VERSION, Reply, Request, write_reply};
use crate::control::{Handler, Kinds, MAX_SUBSCRIBERS};
use crate::font;
use crate::modules::{self, Placed, Update, View};
use crate::policy::Placement;
use crate::print::warn;

/// The request handler for one round of the poll loop.
pub struct Responder<'a> {
    /// Set by `kill`: the poll loop stops after this round.
    pub stop: bool,
    state: &'a mut State,
    conn: &'a Connection,
    qh: &'a QueueHandle<State>,
    /// The config file a `reload` re-reads.
    file: Option<&'a PathBuf>,
    /// The daemon flags, overlaid onto every reload as at start-up.
    given: &'a Given,
    /// One view, reused for every module on every output a `query` asks
    /// about, so a query allocates nothing past the reply itself.
    view: View,
    /// How many connections are subscribed now, for the cap.
    subscribers: usize,
    /// Set by a `subscribe` just handled: what the connection now carries.
    subscription: Option<Kinds>,
}

impl<'a> Responder<'a> {
    pub fn new(
        state: &'a mut State,
        conn: &'a Connection,
        qh: &'a QueueHandle<State>,
        file: Option<&'a PathBuf>,
        given: &'a Given,
        subscribers: usize,
    ) -> Self {
        Self {
            stop: false,
            state,
            conn,
            qh,
            file,
            given,
            view: View::default(),
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
        let request = match crate::control::protocol::parse(line) {
            Ok(request) => request,
            Err(error) => {
                write_reply(out, &Reply::Error { message: &error });
                return;
            }
        };
        match request {
            Request::Query { id } => {
                if let Err(message) = agent::write_query(
                    out,
                    &self.state.content.modules,
                    &self.state.outputs,
                    &mut self.view,
                    id.as_deref(),
                ) {
                    write_reply(out, &Reply::Error { message: &message });
                }
            }
            Request::Layout => agent::write_layout(out, self.state),
            Request::Invoke {
                id,
                action,
                arg,
                output,
            } => match agent::invoke(
                self.state,
                &id,
                &action,
                arg,
                output.as_deref(),
                #[cfg(feature = "popup")]
                self.qh,
            ) {
                Ok(()) => write_reply(out, &Reply::Ok),
                Err(message) => write_reply(out, &Reply::Error { message: &message }),
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
                    self.subscription = Some(Kinds::of(&events));
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
            Request::Reload => match self.reload() {
                Ok(()) => write_reply(out, &Reply::Ok),
                Err(message) => write_reply(out, &Reply::Error { message: &message }),
            },
            Request::Hide | Request::Show | Request::Toggle => {
                // Only the flag: the loop makes or destroys the surfaces
                // once the turn's requests are all served, so a burst of
                // toggles is one change.
                self.state.hidden = match request {
                    Request::Hide => true,
                    Request::Show => false,
                    _ => !self.state.hidden,
                };
                write_reply(
                    out,
                    &Reply::Bar {
                        visible: !self.state.hidden,
                    },
                );
            }
            Request::Kill => {
                self.stop = true;
                write_reply(out, &Reply::Ok);
            }
            Request::Set { id, value } => match self.set(&id, &value) {
                Ok(()) => write_reply(out, &Reply::Ok),
                Err(message) => write_reply(out, &Reply::Error { message: &message }),
            },
        }
    }
}

impl Responder<'_> {
    /// Re-reads the config file and live-applies it, with the daemon flags
    /// overlaid as at start-up. `Err` is the reply, and the running bar
    /// stands exactly as it was.
    fn reload(&mut self) -> Result<(), String> {
        let mut config =
            config::reload(self.file.map(PathBuf::as_path)).map_err(|e| e.to_string())?;
        // The flags over the file, as at start-up: a clash between them
        // (a module the file and a flag place together) refuses the
        // reload with the running bar untouched, like a bad file.
        self.given
            .overlay(&mut config)
            .map_err(|error| error.to_string())?;
        self.apply(config)
    }

    fn apply(
        &mut self,
        #[cfg(any(feature = "workspaces", feature = "window-title"))] mut config: Config,
        #[cfg(not(any(feature = "workspaces", feature = "window-title")))] config: Config,
    ) -> Result<(), String> {
        // The workspaces link is plumbing, not configuration: the daemon's
        // Wayland dispatch and the module share this run's one, and the
        // file never sets it (`[workspaces]` is reserved and empty).
        #[cfg(feature = "workspaces")]
        {
            config.modules.workspaces.link = self.state.workspaces.clone();
        }
        #[cfg(feature = "window-title")]
        // The same for the window-title link: the daemon's
        // `wlr-foreign-toplevel-management-v1` dispatch and the module
        // share this run's one.
        {
            config.modules.window_title.link = self.state.title.clone();
        }
        // Said only once the reload is known good (see below): a refused
        // reload stays silent.
        let mut notes = Vec::new();
        let content = Self::stage(&config, &mut self.state.content.modules, &mut |id, why| {
            notes.push(format!(
                "the {id} module is unavailable, and left out: {why}"
            ));
        })?;
        for note in notes {
            warn(format_args!("scootbar: note: {note}"));
        }
        // The swap drops what the new bar did not keep: a removed `exec`'s
        // group dies with its module here.
        self.state.content = content;
        // The placement is rebuilt whether or not a module was kept, so
        // subscribers are told every module.
        self.state.events.invalidate();
        // New placement, new style, new font: every output is placed again
        // from scratch (its bar, its modules, its scene and buffers, and
        // whether it has a bar at all; `State::replace_placement`). The
        // scenes are small (one short vector per module) and a reload is
        // rare, so they are rebuilt rather than patched. Fresh scenes start
        // at revision 0, so the old shown record could match the new state
        // and the next turn would draw nothing until the next tick: the
        // canvases are emptied and every output draws once (with no modules
        // placed `stale` is always false, and the removed modules' pixels
        // would stay as ghosts).
        self.state.replace_placement(
            Placement {
                bar: config.bar,
                font_size: config.font_size,
                layout: config.layout,
                policy: config.outputs,
            },
            self.conn,
            self.qh,
        );
        // What is placed now decides what the bar holds (`binds`).
        self.state.sync_binds(self.qh);
        Ok(())
    }

    /// What a reload swaps in. The font comes first: a font that vanished
    /// since validation refuses the reload with the running bar untouched
    /// — nothing is started, nothing is moved out of `old`, nothing is
    /// killed. Only then are the modules started, with `old` handed over
    /// so an unchanged `exec` keeps its child. `Err` leaves `old` exactly
    /// as it was; `Ok` leaves what was not kept in `old` for the caller to
    /// drop (which kills a removed `exec`'s group).
    ///
    /// Whether a font is needed is the layout's answer, not the started
    /// modules': every placed module starts unless the process is out of
    /// file descriptors, and then the bar is past refusing a reload.
    fn stage(
        config: &Config,
        old: &mut Vec<Placed>,
        warn: &mut dyn FnMut(&str, &str),
    ) -> Result<Content, String> {
        let placing = config.outputs.to_start(&config.layout);
        let text = if placing.placed().next().is_none() {
            None
        } else {
            let text = font::text(config.font.as_deref(), &config.fallback_fonts)
                .map_err(|error| error.to_string())?;
            Some(text)
        };
        let modules = modules::start(&placing, &config.modules, old, warn);
        Ok(Content {
            modules,
            text,
            style: config.style(),
            #[cfg(feature = "popup")]
            tooltip_delay: std::time::Duration::from_millis(config.tooltip_delay.into()),
        })
    }

    /// A value for module `id`: an id that is not placed, and any module
    /// that takes no value (every one but `push`), are loud errors, never a
    /// silent ok.
    fn set(&mut self, id: &str, value: &serde_json::Value) -> Result<(), String> {
        let Some(placed) = self.state.content.modules.iter_mut().find(|p| p.id == id) else {
            return Err(agent::not_placed(id, &self.state.content.modules));
        };
        match placed.module.on_set(value) {
            Ok(Update::Changed) => {
                placed.revision = placed.revision.wrapping_add(1);
                Ok(())
            }
            Ok(Update::Unchanged) => Ok(()),
            Err(error) => Err(format!("`{id}`: {error}")),
        }
    }
}

#[cfg(all(test, feature = "exec"))]
mod tests;
