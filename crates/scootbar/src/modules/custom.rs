//! The modules the config names: `[button.NAME]`, `[push.NAME]` and
//! `[exec.NAME]` tables, each defining one module whose id is `NAME`, which
//! the `left`, `center` and `right` lists then place like the built-in ones.
//!
//! A built-in module's id is a `&'static str` in the registry; these are
//! the user's, so a name is **interned**: made into a `&'static str` once,
//! by leaking it, and reused by every later reload, so a bar reloaded a
//! thousand times with the same config leaks nothing more, and the total
//! is capped ([`MAX_NAMES`] distinct names for the daemon's whole life, a
//! few hundred bytes) so a config rewritten with fresh names cannot grow it.
//!
//! Only a module that is placed is started, so a table that no list names
//! costs nothing.

use std::sync::Mutex;

use super::{Init, Module};

/// The longest a module name is, in bytes.
pub const MAX_NAME: usize = 32;

/// Distinct names interned over the daemon's life, at most.
pub const MAX_NAMES: usize = 256;

/// `exec` modules placed at once, at most: each polls up to two fds and
/// holds a `timerfd` and a child.
pub const MAX_EXEC: usize = 8;

/// What a table defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    #[cfg(feature = "button")]
    Button(super::button::Settings),
    #[cfg(feature = "push")]
    Push(super::push::Settings),
    #[cfg(feature = "exec")]
    Exec(super::exec::Settings),
}

impl Kind {
    /// The table's kind: `button`, `push` or `exec`.
    pub fn name(&self) -> &'static str {
        // With none of the three built the enum is empty and this is never
        // called.
        match *self {
            #[cfg(feature = "button")]
            Self::Button(_) => "button",
            #[cfg(feature = "push")]
            Self::Push(_) => "push",
            #[cfg(feature = "exec")]
            Self::Exec(_) => "exec",
        }
    }
}

/// One module the config defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Custom {
    pub id: &'static str,
    pub kind: Kind,
}

impl Custom {
    /// Starts it as the bar starts every module.
    #[allow(unreachable_code, unused_variables)]
    pub fn start(&self) -> Init {
        // With none of the three built the enum is empty: nothing is
        // defined, so nothing starts.
        let started: Result<Box<dyn Module>, String> = match self.kind {
            #[cfg(feature = "button")]
            Kind::Button(ref settings) => Ok(super::button::start(settings)),
            #[cfg(feature = "push")]
            Kind::Push(ref settings) => Ok(super::push::start(settings)),
            #[cfg(feature = "exec")]
            Kind::Exec(ref settings) => super::exec::start(self.id, settings),
        };
        match started {
            Ok(module) => Init::Available(module),
            Err(why) => Init::Unavailable(why),
        }
    }
}

/// Whether `id` has the shape of a module id (a built-in's or a name):
/// what `scootbar msg set` checks before asking the daemon.
pub fn well_formed(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= MAX_NAME
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Why a name is refused.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(format!("takes 1 to {MAX_NAME} characters"));
    }
    if !well_formed(name) {
        return Err(
            "takes letters, digits, `-` and `_`, starting with a letter or digit".to_owned(),
        );
    }
    if super::find(name).is_some() {
        return Err("is the name of a built-in module".to_owned());
    }
    Ok(())
}

static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// `name` as a `&'static str`, the same one every time it is asked for;
/// `None` once [`MAX_NAMES`] distinct names have been made.
pub fn intern(name: &str) -> Option<&'static str> {
    intern_in(&NAMES, name)
}

/// [`intern`] over a table of its own: the process-wide one is shared by
/// every test that parses a custom module, so the test that fills a table
/// to its cap fills one it owns.
fn intern_in(table: &Mutex<Vec<&'static str>>, name: &str) -> Option<&'static str> {
    let mut names = table
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(known) = names.iter().find(|known| **known == name) {
        return Some(known);
    }
    if names.len() >= MAX_NAMES {
        return None;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.push(leaked);
    Some(leaked)
}

#[cfg(test)]
mod tests;
