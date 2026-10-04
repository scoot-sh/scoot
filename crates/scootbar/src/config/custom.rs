//! The tables that define modules by name: `[button.NAME]`, `[push.NAME]`
//! and `[exec.NAME]` (`crate::modules::custom`).
//!
//! ```toml
//! left = ["workspaces", "launcher"]
//! right = ["weather", "status", "clock"]
//!
//! [button.launcher]
//! icon = "\U000f0e65"             # or icon-path / icon-image, as the clock's
//! text = ""                       # shown after the icon
//! on-click = { exec = ["scootlaunch"] }
//!
//! [exec.weather]
//! command = ["sh", "-c", "while :; do curl -s 'wttr.in?format=1'; sleep 600; done"]
//! format = "text"                 # or "json"
//! placeholder = "..."
//!
//! [push.status]
//! placeholder = ""
//! ```
//!
//! A name is 1 to 32 letters, digits, `-` or `_`, is not a built-in
//! module's id, and is unique across the three kinds. Every table also takes
//! the five interaction keys and a `margin`. A table the lists never name
//! costs nothing (only placed modules start). Every refusal names its dotted
//! key (`exec.weather.command`).

#![cfg_attr(
    not(any(feature = "button", feature = "push", feature = "exec")),
    allow(dead_code)
)]

#[cfg(any(feature = "button", feature = "push", feature = "exec"))]
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

#[cfg(any(feature = "button", feature = "push", feature = "exec"))]
use serde::Deserialize;

use super::{Error, MAX_GAP, bindings};
use crate::action::Bindings;
#[cfg(any(feature = "button", feature = "push", feature = "exec"))]
use crate::modules::custom::Kind;
use crate::modules::custom::{Custom, check_name, intern};
#[cfg(feature = "exec")]
use crate::modules::payload::Format;

/// Most modules the config defines by name, over the three kinds.
pub const MAX_DEFINED: usize = 32;

/// The longest `command` is (`exec`'s own bound, in arguments and bytes).
#[cfg(feature = "exec")]
const MAX_COMMAND: usize = crate::action::MAX_EXEC_ARGS;
#[cfg(feature = "exec")]
const MAX_COMMAND_ARG: usize = crate::action::MAX_EXEC_ARG;

/// A `[button.NAME]` table.
#[cfg(feature = "button")]
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct ButtonFile {
    text: Option<String>,
    icon: Option<String>,
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// Without the `icon-image` feature the key is still taken, so the
    /// refusal says what is missing.
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    margin: Option<u32>,
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// A `[push.NAME]` table.
#[cfg(feature = "push")]
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct PushFile {
    placeholder: Option<String>,
    icon: Option<String>,
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// Without the `icon-image` feature the key is still taken, so the
    /// refusal says what is missing.
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    #[serde(rename = "show-text")]
    show_text: Option<bool>,
    margin: Option<u32>,
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// An `[exec.NAME]` table.
#[cfg(feature = "exec")]
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct ExecFile {
    command: Option<toml::Value>,
    format: Option<String>,
    placeholder: Option<String>,
    icon: Option<String>,
    #[serde(rename = "icon-path")]
    icon_path: Option<String>,
    #[serde(rename = "icon-viewbox")]
    icon_viewbox: Option<String>,
    /// Without the `icon-image` feature the key is still taken, so the
    /// refusal says what is missing.
    #[serde(rename = "icon-image")]
    icon_image: Option<String>,
    #[serde(rename = "show-text")]
    show_text: Option<bool>,
    margin: Option<u32>,
    #[serde(rename = "on-click")]
    on_click: Option<toml::Value>,
    #[serde(rename = "on-right-click")]
    on_right_click: Option<toml::Value>,
    #[serde(rename = "on-middle-click")]
    on_middle_click: Option<toml::Value>,
    #[serde(rename = "on-scroll-up")]
    on_scroll_up: Option<toml::Value>,
    #[serde(rename = "on-scroll-down")]
    on_scroll_down: Option<toml::Value>,
}

/// Every table of a file, borrowed.
#[derive(Default)]
pub(super) struct Tables<'a> {
    #[cfg(feature = "button")]
    pub button: Option<&'a BTreeMap<String, ButtonFile>>,
    #[cfg(feature = "push")]
    pub push: Option<&'a BTreeMap<String, PushFile>>,
    #[cfg(feature = "exec")]
    pub exec: Option<&'a BTreeMap<String, ExecFile>>,
    #[cfg(not(any(feature = "button", feature = "push", feature = "exec")))]
    pub none: Option<&'a ()>,
}

/// What the tables define, with the margins and bindings each set.
#[derive(Default)]
pub(super) struct Defined {
    pub modules: Vec<Custom>,
    pub margins: Vec<(&'static str, u32)>,
    pub bindings: Vec<(&'static str, Bindings)>,
}

impl Defined {
    /// The names defined, for the layout lists to place.
    pub fn names(&self) -> Vec<&'static str> {
        self.modules.iter().map(|custom| custom.id).collect()
    }
}

/// An error at the dotted key `key`.
fn at(path: &Path, key: String, message: impl fmt::Display) -> Error {
    Error::Named {
        path: path.to_owned(),
        key,
        message: message.to_string(),
    }
}

/// Reads every table: the names checked, each kind's own keys, the margins
/// and the bindings.
#[allow(unused_mut, unused_variables)]
pub(super) fn read(path: &Path, tables: &Tables<'_>) -> Result<Defined, Error> {
    let mut defined = Defined::default();
    // (kind, name) in file order within a kind, kinds in a fixed order.
    #[cfg(feature = "button")]
    if let Some(buttons) = tables.button {
        for (name, table) in buttons {
            let id = claim(path, &defined, "button", name)?;
            let key = |what: &str| format!("button.{name}.{what}");
            let icon = icon(path, name, table)?;
            let text = table.text.as_deref().unwrap_or("");
            if text.len() > crate::modules::payload::MAX_PAYLOAD {
                return Err(at(path, key("text"), "is too long"));
            }
            let settings = crate::modules::button::Settings::new(text, icon);
            defined.modules.push(Custom {
                id,
                kind: Kind::Button(settings),
            });
            common(
                path,
                &mut defined,
                id,
                "button",
                table.margin,
                [
                    table.on_click.as_ref(),
                    table.on_right_click.as_ref(),
                    table.on_middle_click.as_ref(),
                    table.on_scroll_up.as_ref(),
                    table.on_scroll_down.as_ref(),
                ],
            )?;
        }
    }
    #[cfg(feature = "push")]
    if let Some(pushes) = tables.push {
        for (name, table) in pushes {
            let id = claim(path, &defined, "push", name)?;
            let icon = push_icon(path, name, table)?;
            let settings = crate::modules::push::Settings {
                placeholder: table.placeholder.clone().unwrap_or_default(),
                icon,
                show_text: table.show_text.unwrap_or(true),
            };
            defined.modules.push(Custom {
                id,
                kind: Kind::Push(settings),
            });
            common(
                path,
                &mut defined,
                id,
                "push",
                table.margin,
                [
                    table.on_click.as_ref(),
                    table.on_right_click.as_ref(),
                    table.on_middle_click.as_ref(),
                    table.on_scroll_up.as_ref(),
                    table.on_scroll_down.as_ref(),
                ],
            )?;
        }
    }
    #[cfg(feature = "exec")]
    if let Some(execs) = tables.exec {
        for (name, table) in execs {
            let id = claim(path, &defined, "exec", name)?;
            let key = |what: &str| format!("exec.{name}.{what}");
            let command = command(path, &key("command"), table.command.as_ref())?;
            let format = match table.format.as_deref() {
                None => Format::Text,
                Some(text) => Format::parse(text).ok_or_else(|| {
                    at(
                        path,
                        key("format"),
                        format_args!("takes text or json, not `{}`", text.escape_debug()),
                    )
                })?,
            };
            let settings = crate::modules::exec::Settings {
                command,
                format,
                placeholder: table.placeholder.clone().unwrap_or_default(),
                restart: crate::modules::exec::Restart::default(),
                icon: exec_icon(path, name, table)?,
                show_text: table.show_text.unwrap_or(true),
            };
            defined.modules.push(Custom {
                id,
                kind: Kind::Exec(settings),
            });
            common(
                path,
                &mut defined,
                id,
                "exec",
                table.margin,
                [
                    table.on_click.as_ref(),
                    table.on_right_click.as_ref(),
                    table.on_middle_click.as_ref(),
                    table.on_scroll_up.as_ref(),
                    table.on_scroll_down.as_ref(),
                ],
            )?;
        }
    }
    let _ = tables;
    Ok(defined)
}

/// The name `name` of a `kind` table, checked and interned: refused if it
/// is malformed, a built-in's, or already defined (by any kind).
fn claim(
    path: &Path,
    defined: &Defined,
    kind: &'static str,
    name: &str,
) -> Result<&'static str, Error> {
    if defined.modules.len() >= MAX_DEFINED {
        return Err(at(
            path,
            format!("{kind}.{}", name.escape_debug()),
            format_args!("is one table too many: at most {MAX_DEFINED} modules are defined"),
        ));
    }
    if let Err(message) = check_name(name) {
        // A well-formed name (a built-in's) is quoted as a key is; one that
        // is not is quoted as TOML would need to.
        let key = if crate::modules::custom::well_formed(name) {
            format!("{kind}.{name}")
        } else {
            format!("{kind}.\"{}\"", name.escape_debug())
        };
        return Err(at(path, key, format_args!("the module name {message}")));
    }
    if let Some(other) = defined.modules.iter().find(|custom| custom.id == name) {
        return Err(at(
            path,
            format!("{kind}.{name}"),
            format_args!(
                "is already defined as a {} module; a name is one module's",
                other.kind.name()
            ),
        ));
    }
    intern(name).ok_or_else(|| {
        at(
            path,
            format!("{kind}.{name}"),
            "is one name too many for this run of the bar (a bar remembers at most 256 module names \
             over its life, reloads included): restart it",
        )
    })
}

/// A table's margin and interaction keys.
fn common(
    path: &Path,
    defined: &mut Defined,
    id: &'static str,
    kind: &'static str,
    margin: Option<u32>,
    values: [Option<&toml::Value>; 5],
) -> Result<(), Error> {
    if let Some(margin) = margin {
        if margin > MAX_GAP {
            return Err(at(
                path,
                format!("{kind}.{id}.margin"),
                format_args!(
                    "takes a whole number of logical pixels from 0 to {MAX_GAP}, not `{margin}`"
                ),
            ));
        }
        if margin > 0 {
            defined.margins.push((id, margin));
        }
    }
    let read = bindings::read(id, values).map_err(|(trigger, message)| {
        at(path, format!("{kind}.{id}.{}", trigger.key()), message)
    })?;
    if !read.is_empty() {
        defined.bindings.push((id, read));
    }
    Ok(())
}

/// The button's icon from its `icon`, `icon-path`, `icon-viewbox` and
/// `icon-image` keys (the clock's rules, `config::icon`).
#[cfg(feature = "button")]
fn icon(path: &Path, name: &str, table: &ButtonFile) -> Result<Option<crate::icon::Icon>, Error> {
    super::icon::read(
        &format!("button.{name}"),
        "a button",
        &super::icon::Keys {
            icon: table.icon.as_deref(),
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: table.icon_image.as_deref(),
        },
    )
    .map_err(|(key, message)| at(path, key, message))
}

/// A push module's icon from the same four keys under its own name.
#[cfg(feature = "push")]
fn push_icon(
    path: &Path,
    name: &str,
    table: &PushFile,
) -> Result<Option<crate::icon::Icon>, Error> {
    super::icon::read(
        &format!("push.{name}"),
        "a push",
        &super::icon::Keys {
            icon: table.icon.as_deref(),
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: table.icon_image.as_deref(),
        },
    )
    .map_err(|(key, message)| at(path, key, message))
}

/// An exec module's icon from the same four keys under its own name.
#[cfg(feature = "exec")]
fn exec_icon(
    path: &Path,
    name: &str,
    table: &ExecFile,
) -> Result<Option<crate::icon::Icon>, Error> {
    super::icon::read(
        &format!("exec.{name}"),
        "an exec",
        &super::icon::Keys {
            icon: table.icon.as_deref(),
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: table.icon_image.as_deref(),
        },
    )
    .map_err(|(key, message)| at(path, key, message))
}

/// An `exec` module's `command`: a non-empty array of strings, no shell.
#[cfg(feature = "exec")]
fn command(path: &Path, key: &str, given: Option<&toml::Value>) -> Result<Vec<String>, Error> {
    let Some(given) = given else {
        return Err(at(
            path,
            key.to_owned(),
            "is required: the command and its arguments, such as [\"sh\", \"-c\", \"...\"]",
        ));
    };
    let toml::Value::Array(items) = given else {
        return Err(at(
            path,
            key.to_owned(),
            "takes an array of the command and its arguments, such as \
             [\"sh\", \"-c\", \"...\"]; nothing runs it through a shell",
        ));
    };
    if items.is_empty() || items.len() > MAX_COMMAND {
        return Err(at(
            path,
            key.to_owned(),
            format_args!("takes a command and up to {} arguments", MAX_COMMAND - 1),
        ));
    }
    let mut argv = Vec::with_capacity(items.len());
    for item in items {
        let toml::Value::String(arg) = item else {
            return Err(at(path, key.to_owned(), "takes strings only"));
        };
        if arg.len() > MAX_COMMAND_ARG || arg.contains('\0') {
            return Err(at(
                path,
                key.to_owned(),
                format_args!("arguments are at most {MAX_COMMAND_ARG} bytes and hold no NUL"),
            ));
        }
        argv.push(arg.clone());
    }
    if argv[0].is_empty() {
        return Err(at(
            path,
            key.to_owned(),
            "needs a command, not an empty string",
        ));
    }
    Ok(argv)
}
