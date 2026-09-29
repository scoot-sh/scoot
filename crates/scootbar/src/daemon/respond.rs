//! What the daemon answers to each control request.
//!
//! Every request is answered at once, from the daemon's state: `query`
//! reads each placed module's view on every output, `reload` re-reads the
//! config file and live-applies it, `version` and `kill` are what they
//! sound like, and `set` is refused loudly (no module takes one yet).
//!
//! A refused `reload` changes nothing: the file is re-read and fully
//! validated before anything is touched, and the font and the modules are
//! started before the running ones are swapped out.

use std::path::PathBuf;

use wayland_client::QueueHandle;

use super::wayland::State;
use crate::cli::Given;
use crate::config::{self, Config};
use crate::control::Handler;
use crate::control::protocol::{ModuleView, PROTOCOL_VERSION, Reply, Request, write_reply};
use crate::font;
use crate::modules::{self, OutputView, Placed, Update, View};
use crate::print::warn;
use crate::render::Scene;
use crate::text::Text;

/// The request handler for one round of the poll loop.
pub struct Responder<'a> {
    /// Set by `kill`: the poll loop stops after this round.
    pub stop: bool,
    state: &'a mut State,
    qh: &'a QueueHandle<State>,
    /// The config file a `reload` re-reads.
    file: Option<&'a PathBuf>,
    /// The daemon flags, overlaid onto every reload as at start-up.
    given: &'a Given,
    /// One view, reused for every module on every output a `query` asks
    /// about, so a query allocates nothing past the reply itself.
    view: View,
}

impl<'a> Responder<'a> {
    pub fn new(
        state: &'a mut State,
        qh: &'a QueueHandle<State>,
        file: Option<&'a PathBuf>,
        given: &'a Given,
    ) -> Self {
        Self {
            stop: false,
            state,
            qh,
            file,
            given,
            view: View::default(),
        }
    }
}

impl Handler for Responder<'_> {
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
            Request::Query => write_query(
                out,
                &self.state.content.modules,
                &self.state.outputs,
                &mut self.view,
            ),
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

/// A `query` reply: every placed module's view on every output, in
/// placement order. The views are asked into one reused [`View`], so a
/// query allocates nothing beyond the connection's output buffer.
fn write_query(
    out: &mut Vec<u8>,
    placed: &[Placed],
    outputs: &crate::outputs::Outputs<super::surfaces::Objects>,
    scratch: &mut View,
) {
    let start = out.len();
    // Every element serializes (strings, numbers, a char), and writing
    // into a `Vec` cannot fail: `failed` is still checked rather than
    // assumed, so a half-written line never goes out.
    let mut failed = false;
    out.extend_from_slice(br#"{"type":"modules","modules":["#);
    let mut first = true;
    for module in placed {
        for entry in outputs.iter() {
            let name = entry.output.info().name.as_deref();
            scratch.clear();
            module.module.view(&OutputView { name }, scratch);
            if !first {
                out.push(b',');
            }
            first = false;
            let view = ModuleView {
                id: module.id,
                section: module.section.name(),
                output: name,
                text: scratch.text(),
                class: scratch.class().name(),
                icon: scratch.icon(),
            };
            if serde_json::to_writer(&mut *out, &view).is_err() {
                failed = true;
                break;
            }
        }
    }
    if failed {
        out.truncate(start);
        out.extend_from_slice(br#"{"type":"error","message":"internal: reply failed to encode"}"#);
    } else {
        out.extend_from_slice(b"]}");
    }
    out.push(b'\n');
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
        #[cfg(feature = "workspaces")] mut config: Config,
        #[cfg(not(feature = "workspaces"))] config: Config,
    ) -> Result<(), String> {
        // The workspaces link is plumbing, not configuration: the daemon's
        // Wayland dispatch and the module share this run's one, and the
        // file never sets it (`[workspaces]` is reserved and empty).
        #[cfg(feature = "workspaces")]
        {
            config.modules.workspaces.link = self.state.workspaces.clone();
        }
        // Started first, but said only once the reload is known good (see
        // below): a refused reload stays silent.
        let mut notes = Vec::new();
        let modules = modules::start(&config.layout, &config.modules, &mut |id, why| {
            notes.push(format!(
                "the {id} module is unavailable, and left out: {why}"
            ));
        });
        // The font next: a font that vanished since validation refuses
        // the reload with the running bar untouched. The notes above are
        // said only now, once the reload is known good, so a refused
        // reload stays silent.
        let text = if modules.is_empty() {
            None
        } else {
            let font = font::find(config.font.as_deref()).map_err(|error| error.to_string())?;
            Some(Text::new(font.face))
        };
        for note in notes {
            warn(format_args!("scootbar: note: {note}"));
        }
        let geometry = config.bar != self.state.bar;
        let count = modules.len();
        let resize = count != self.state.content.modules.len();
        self.state.bar = config.bar;
        self.state.content.modules = modules;
        self.state.content.text = text;
        self.state.content.style = config.style();
        // New placement, new style, new font: every output measures its
        // views again from scratch. The scenes are small (one short vector
        // per module) and a reload is rare, so they are rebuilt rather
        // than patched. Every canvas is cleared too, resized where the
        // module count changed (whose records are sized to it): the style,
        // the text and the modules are swapped wholesale, while fresh
        // scenes start at revision 0, so the old shown record could match
        // the new state and the next turn would draw nothing until the
        // next tick. The buffers are made again on demand.
        let content = &self.state.content;
        for entry in self.state.outputs.iter_mut() {
            entry.objects.scene = Scene::new(&content.modules);
            if resize {
                entry.objects.canvas.resize(count);
            } else {
                entry.objects.canvas.clear();
            }
        }
        if geometry {
            self.state.recreate_bars(self.qh);
        }
        Ok(())
    }

    /// A value for module `id`: an unknown id, and any module that takes
    /// no value (every module today), are loud errors, never a silent ok.
    fn set(&mut self, id: &str, value: &serde_json::Value) -> Result<(), String> {
        let Some(placed) = self.state.content.modules.iter_mut().find(|p| p.id == id) else {
            if modules::find(id).is_none() {
                return Err(format!("no module `{id}` in this build"));
            }
            let mut shows = String::new();
            for placed in &self.state.content.modules {
                if !shows.is_empty() {
                    shows.push(' ');
                }
                shows.push_str(placed.id);
            }
            if shows.is_empty() {
                return Err(format!("`{id}` is not placed: this bar shows nothing"));
            }
            return Err(format!(
                "`{id}` is not placed in this bar (it shows:{shows})"
            ));
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
