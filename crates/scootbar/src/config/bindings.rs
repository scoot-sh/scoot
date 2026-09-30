//! The interaction keys of a module's table (`on-click`, `on-right-click`,
//! `on-middle-click`, `on-scroll-up`, `on-scroll-down`), read into
//! [`Bindings`] and checked when the file is read, so a typo is a refusal
//! naming the key, not a click that does nothing.
//!
//! A value is one of:
//!
//! ```toml
//! on-click = "toggle-mute"                       # a module's action
//! on-scroll-up = "activate 3"                    # with its whole number
//! on-click = { exec = ["foot", "-e", "btop"] }   # a command, no shell
//! on-click = { scoot = "quit" }                  # a request to scoot
//! ```
//!
//! An `exec` line is an array (a string would be split by a shell, which
//! this never starts; write `["sh", "-c", "..."]` to have one), of at most
//! [`MAX_EXEC_ARGS`] arguments of at most [`MAX_EXEC_ARG`] bytes each, the
//! first not empty, none holding a NUL.

use std::path::Path;

use super::{Error, value};
use crate::action::{
    Action, Bindings, MAX_EXEC_ARG, MAX_EXEC_ARGS, ModuleAction, ScootAction, Trigger,
};
use crate::modules::{ArgKind, Spec};

/// One module's five values as the file gave them, in [`Trigger::ALL`]'s
/// order, with the dotted config key each is read at.
pub struct Raw<'a> {
    pub keys: [&'static str; 5],
    pub values: [Option<&'a toml::Value>; 5],
}

/// What the table `module` binds. `module` is a registry id.
pub fn read(path: &Path, module: &'static str, raw: &Raw<'_>) -> Result<Bindings, Error> {
    let spec = crate::modules::find(module);
    let mut bindings = Bindings::default();
    for ((trigger, key), given) in Trigger::ALL.into_iter().zip(raw.keys).zip(raw.values) {
        let Some(given) = given else {
            continue;
        };
        let action = action(module, spec, given)
            .map_err(|message| value(path, key, format_args!("{message}")))?;
        bindings.set(trigger, action);
    }
    Ok(bindings)
}

fn shape() -> &'static str {
    "takes a module action (a string such as \"next\"), \
     `{ exec = [\"command\", \"arg\"] }` or `{ scoot = \"quit\" }`"
}

fn action(module: &str, spec: Option<&Spec>, given: &toml::Value) -> Result<Action, String> {
    match given {
        toml::Value::String(text) => module_action(module, spec, text).map(Action::Module),
        toml::Value::Table(table) => {
            let mut entries = table.iter();
            let (Some((kind, inner)), None) = (entries.next(), entries.next()) else {
                return Err(format!("{}, with one key", shape()));
            };
            match kind.as_str() {
                "exec" => exec(inner).map(Action::Exec),
                "scoot" => scoot(inner).map(Action::Scoot),
                other => Err(format!(
                    "has no `{}` action kind: {}",
                    other.escape_debug(),
                    shape()
                )),
            }
        }
        _ => Err(shape().to_owned()),
    }
}

/// `"name"` or `"name N"`, against what the module defines.
fn module_action(module: &str, spec: Option<&Spec>, text: &str) -> Result<ModuleAction, String> {
    let mut words = text.split_whitespace();
    let Some(name) = words.next() else {
        return Err(format!("is empty: {}", shape()));
    };
    let arg = words.next();
    if words.next().is_some() {
        return Err(format!(
            "`{}` is an action and at most one number; to run a command write \
             `{{ exec = [...] }}`",
            text.escape_debug()
        ));
    }
    let defined = spec.map_or(&[][..], |spec| spec.actions);
    let Some(action) = spec.and_then(|spec| spec.action(name)) else {
        let mut known = String::new();
        for action in defined {
            known.push(' ');
            known.push_str(action.name);
        }
        return Err(if defined.is_empty() {
            format!(
                "the {module} module has no actions of its own (bind a command with \
                 `{{ exec = [...] }}`)"
            )
        } else {
            format!(
                "the {module} module has no action `{}` (it has:{known})",
                name.escape_debug()
            )
        });
    };
    let arg = match (action.arg, arg) {
        (ArgKind::None, None) => None,
        (ArgKind::None, Some(_)) => return Err(format!("`{name}` takes no number")),
        (ArgKind::Required, None) => {
            return Err(format!("`{name}` takes a whole number, as \"{name} 3\""));
        }
        (ArgKind::Required, Some(arg)) => Some(arg.parse::<i32>().map_err(|_| {
            format!(
                "`{name}` takes a whole number, not `{}`",
                arg.escape_debug()
            )
        })?),
    };
    Ok(ModuleAction {
        name: name.to_owned().into(),
        arg,
    })
}

fn exec(given: &toml::Value) -> Result<Vec<String>, String> {
    let toml::Value::Array(items) = given else {
        return Err("`exec` takes an array of the command and its arguments, \
             such as [\"foot\", \"-e\", \"btop\"]; nothing runs it through a shell, \
             so write [\"sh\", \"-c\", \"...\"] to use one"
            .to_owned());
    };
    if items.is_empty() || items.len() > MAX_EXEC_ARGS {
        return Err(format!(
            "`exec` takes a command and up to {} arguments, not {}",
            MAX_EXEC_ARGS - 1,
            items.len().saturating_sub(1)
        ));
    }
    let mut argv = Vec::with_capacity(items.len());
    for item in items {
        let toml::Value::String(arg) = item else {
            return Err("`exec` takes strings only".to_owned());
        };
        if arg.len() > MAX_EXEC_ARG {
            return Err(format!(
                "`exec` arguments are at most {MAX_EXEC_ARG} bytes each"
            ));
        }
        if arg.contains('\0') {
            return Err("`exec` arguments cannot hold a NUL".to_owned());
        }
        argv.push(arg.clone());
    }
    if argv[0].is_empty() {
        return Err("`exec` needs a command to run, not an empty string".to_owned());
    }
    Ok(argv)
}

fn scoot(given: &toml::Value) -> Result<ScootAction, String> {
    given
        .as_str()
        .and_then(ScootAction::parse)
        .ok_or_else(|| "`scoot` takes \"quit\" (what `scoot msg action quit` does)".to_owned())
}
