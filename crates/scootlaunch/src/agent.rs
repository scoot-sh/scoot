//! `--run-id`: resolve one entry (and one of its actions) by id and
//! launch it through the panel's spawn path, with no surface.
//!
//! The headless path mints no activation token (no surface, no serial):
//! a fresh spawn still takes focus on map, because the compositor
//! focuses every new toplevel. Everything else — `DBusActivatable`
//! first with an `Exec` fallback, the environment rules, the systemd
//! scope move, the detached fallback with no bus, frecency — is the
//! panel's [`spawn`](crate::spawn) path, not a second implementation.
//!
//! Stdout carries exactly one JSON object, nothing else:
//! a success object on exit 0, an error object on exit 1. Usage errors
//! (the CLI's exit 2) never reach here. Notes and warnings go to
//! stderr, like `--list` and `--query`.

#[cfg(test)]
mod tests;

use crate::index::{Action, Entry};
use crate::spawn::app_id_for;

/// At most this many bytes an id or action id may hold: past it the
/// request is refused before any lookup, so a megabyte of `A` never
/// becomes a scope name or a log line.
pub const MAX_ID_LEN: usize = 1024;

/// Why `--run-id` refused to launch. Each maps to a stable `code` in
/// the error object, pinned below and documented on the site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// Empty, too long, or holding control characters.
    InvalidId(String),
    /// No entry matches.
    UnknownId(String),
    /// The suffix-insensitive form matches more than one entry.
    AmbiguousId(String),
    /// Empty, too long, or holding control characters.
    InvalidAction(String),
    /// The entry has no action with that id.
    UnknownAction(String),
}

impl ResolveError {
    pub fn code(&self) -> &'static str {
        match self {
            ResolveError::InvalidId(_) => "invalid-id",
            ResolveError::UnknownId(_) => "unknown-id",
            ResolveError::AmbiguousId(_) => "ambiguous-id",
            ResolveError::InvalidAction(_) => "invalid-action",
            ResolveError::UnknownAction(_) => "unknown-action",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            ResolveError::InvalidId(message)
            | ResolveError::UnknownId(message)
            | ResolveError::AmbiguousId(message)
            | ResolveError::InvalidAction(message)
            | ResolveError::UnknownAction(message) => message,
        }
    }
}

/// Whether a raw id or action id is nameable at all: non-empty, short,
/// and free of control characters (which would otherwise ride into a
/// scope name, a log line, or the JSON itself).
fn nameable(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_ID_LEN {
        return false;
    }
    !value.chars().any(|char| char.is_control())
}

/// Resolves `id` (and `action`, if given) against the built index.
/// Hidden entries resolve: `NoDisplay` and friends are still
/// launchable by id. Masked ids (`Hidden=true`) never indexed resolve
/// as unknown, like any other missing file.
///
/// The match is exact first (`firefox.desktop`), then the
/// suffix-insensitive form (`firefox` matching one `firefox.desktop`).
/// A suffix-insensitive form matching more than one entry is
/// ambiguous and launches nothing: a launcher that guesses launches
/// the wrong app. A prefix or fuzzy match never launches.
pub fn resolve<'a>(
    entries: &'a [Entry],
    id: &str,
    action: Option<&str>,
) -> Result<(&'a Entry, Option<&'a Action>), ResolveError> {
    if !nameable(id) {
        return Err(ResolveError::InvalidId(invalid_id_message(id)));
    }
    if let Some(wanted) = action
        && !nameable(wanted)
    {
        return Err(ResolveError::InvalidAction(invalid_action_message(wanted)));
    }
    let entry = find_entry(entries, id);
    let Some(entry) = entry else {
        // Distinguish "more than one" from "none" only for the
        // suffix-insensitive shape: an exact miss is unknown.
        if is_ambiguous(entries, id) {
            return Err(ResolveError::AmbiguousId(format!(
                "ambiguous id: more than one entry matches {short}",
                short = short(id)
            )));
        }
        return Err(ResolveError::UnknownId(format!(
            "unknown id: no entry matches {short}",
            short = short(id)
        )));
    };
    match action {
        None => Ok((entry, None)),
        Some(wanted) => match entry.actions.iter().find(|got| got.id == wanted) {
            Some(found) => Ok((entry, Some(found))),
            None => Err(ResolveError::UnknownAction(format!(
                "unknown action {action} for {id}",
                action = short(wanted),
                id = short(&entry.id)
            ))),
        },
    }
}

fn find_entry<'a>(entries: &'a [Entry], id: &str) -> Option<&'a Entry> {
    if let Some(exact) = entries.iter().find(|entry| entry.id == id) {
        return Some(exact);
    }
    // The suffix-insensitive form: `firefox` for `firefox.desktop`.
    // Exactly one match launches; zero is unknown, more is ambiguous
    // (answered by the caller).
    let mut found: Option<&'a Entry> = None;
    let mut count = 0;
    for entry in entries {
        if app_id_for(&entry.id) == id
            || app_id_for(&entry.id) == stripped(id)
            || entry.id == with_suffix(id)
        {
            count += 1;
            found = Some(entry);
        }
    }
    if count == 1 { found } else { None }
}

fn is_ambiguous(entries: &[Entry], id: &str) -> bool {
    let mut count = 0;
    for entry in entries {
        if app_id_for(&entry.id) == id
            || app_id_for(&entry.id) == stripped(id)
            || entry.id == with_suffix(id)
        {
            count += 1;
            if count > 1 {
                return true;
            }
        }
    }
    false
}

fn stripped(id: &str) -> &str {
    id.strip_suffix(".desktop").unwrap_or(id)
}

fn with_suffix(id: &str) -> String {
    if id.ends_with(".desktop") {
        id.to_string()
    } else {
        format!("{id}.desktop")
    }
}

fn invalid_id_message(id: &str) -> String {
    if id.is_empty() {
        return "invalid id: the id is empty".to_string();
    }
    if id.len() > MAX_ID_LEN {
        return format!("invalid id: longer than {MAX_ID_LEN} bytes");
    }
    format!(
        "invalid id: {short} holds control characters",
        short = short(id)
    )
}

fn invalid_action_message(action: &str) -> String {
    if action.is_empty() {
        return "invalid action: the action is empty".to_string();
    }
    if action.len() > MAX_ID_LEN {
        return format!("invalid action: longer than {MAX_ID_LEN} bytes");
    }
    format!(
        "invalid action: {short} holds control characters",
        short = short(action)
    )
}

/// A hostile id in a human line: the first 128 chars, never the whole
/// input, with control characters escaped (a raw newline must not
/// break the line).
fn short(value: &str) -> String {
    let mut out = String::new();
    for char in value.chars().take(128) {
        for escaped in char.escape_debug() {
            out.push(escaped);
        }
    }
    if value.chars().count() > 128 {
        out.push('…');
    }
    out
}

/// The success object for `--run-id`: exactly one JSON object on
/// stdout, exit 0. Only the fields that happened are present: a fresh
/// spawn carries `pid` (and `scope` when the scope move landed), an
/// activation carries `activated`, a bus-less launch carries
/// `detached`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RunOk {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
}

impl RunOk {
    pub fn from_report(report: &crate::spawn::LaunchReport) -> Self {
        RunOk {
            ok: true,
            pid: report.pid,
            scope: report.scope.clone(),
            activated: report.activated.then_some(true),
            detached: report.detached.then_some(true),
        }
    }
}

/// The error object for `--run-id`: exactly one JSON object on stdout,
/// exit 1. `code` is one of `invalid-id`, `unknown-id`,
/// `ambiguous-id`, `invalid-action`, `unknown-action` or
/// `launch-failed`. Human notes stay on stderr.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RunErr {
    pub ok: bool,
    pub code: String,
    pub error: String,
}

impl RunErr {
    pub fn resolve(error: &ResolveError) -> Self {
        RunErr {
            ok: false,
            code: error.code().to_string(),
            error: error.message().to_string(),
        }
    }

    pub fn launch_failed(reason: &str) -> Self {
        RunErr {
            ok: false,
            code: "launch-failed".to_string(),
            error: sanitize(reason),
        }
    }
}

/// A spawn failure in the JSON: cut at 1 KiB and stripped of control
/// characters, so a hostile `Exec` path cannot smuggle a newline into
/// the object.
fn sanitize(reason: &str) -> String {
    let mut out = String::new();
    for char in reason.chars() {
        if out.len() >= 1024 {
            break;
        }
        if char.is_control() {
            out.push('�');
        } else {
            out.push(char);
        }
        if out.len() >= 1024 {
            break;
        }
    }
    if reason.len() > out.len() {
        out.push('…');
    }
    out
}
