//! The portal backend's D-Bus surface: name, object, interface, and the
//! session table.
//!
//! The backend owns `org.freedesktop.impl.portal.desktop.scoot` and serves
//! `org.freedesktop.impl.portal.GlobalShortcuts` version 2 at
//! `/org/freedesktop/portal/desktop`. Sessions are the portal's object
//! paths: `CreateSession` records the app id and the caller's unique name
//! under the session path, `BindShortcuts` registers each shortcut with
//! the compositor over scoot-ipc, `ListShortcuts` lists what the
//! compositor holds for the session's app, and `ConfigureShortcuts` is
//! answered as a no-op (assignment lives in the user's config file, per
//! `dev/design/global-shortcuts.md`: stage 1 has no in-compositor
//! configure UI).
//!
//! No `unsafe` here. Nothing here links beyond what `std` links.

use std::collections::HashMap;

use scoot_dbus::proto::{Reader, Writer};

/// The bus name this backend owns: the `.portal` file's `DBusName`.
pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.scoot";
/// The object all portal backend interfaces live at.
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
/// The backend interface served here.
pub const INTERFACE: &str = "org.freedesktop.impl.portal.GlobalShortcuts";
/// The interface version served.
pub const VERSION: u32 = 2;

/// The per-connection cap: shortcuts are small strings, so 1 MiB is far
/// past any legitimate call while still bounding a flood.
pub const MAX_MESSAGE: usize = 1024 * 1024;

/// How long one bus round trip waits: the daemon is local, and a bus
/// that never answers is a failed start, not a hang.
pub const ROUNDTRIP_SECS: u64 = 5;

/// At most this many sessions: past it `CreateSession` is refused with
/// `InvalidArgs`, never a silent drop. One app holds one session, so
/// this is thousands of apps.
pub const MAX_SESSIONS: usize = 4096;

/// At most this many shortcuts per session remembered here: the
/// compositor enforces its own caps (64 per connection, 512 total); this
/// bounds the backend's re-register list on reconnect.
pub const MAX_SHORTCUTS_PER_SESSION: usize = 512;

/// One remembered shortcut: what a reconnect re-registers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutBinding {
    pub shortcut_id: String,
    pub description: Option<String>,
    pub preferred_trigger: Option<String>,
}

/// One portal session: the app that owns it and who to kill it with.
#[derive(Debug, Clone)]
pub struct Session {
    pub app_id: String,
    /// The caller's unique bus name (`:1.42`): app death arrives as a
    /// `NameOwnerChanged` for this name.
    pub sender: String,
    pub shortcuts: Vec<ShortcutBinding>,
}

/// The session table: session-handle path to session.
#[derive(Debug, Default)]
pub struct Sessions {
    inner: HashMap<String, Session>,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn get(&self, handle: &str) -> Option<&Session> {
        self.inner.get(handle)
    }

    /// Records a session. `Err` when the table is full or the handle is
    /// already taken by another app (a replayed create).
    pub fn create(&mut self, handle: String, app_id: String, sender: String) -> Result<(), String> {
        if self.inner.len() >= MAX_SESSIONS && !self.inner.contains_key(&handle) {
            return Err(String::from("too many sessions"));
        }
        match self.inner.get(&handle) {
            Some(existing) if existing.app_id != app_id => {
                return Err(String::from("session handle is taken"));
            }
            Some(_) => return Ok(()),
            None => {}
        }
        if !valid_handle(&handle) || !valid_app_id(&app_id) {
            return Err(String::from("bad session handle or app id"));
        }
        self.inner.insert(
            handle,
            Session {
                app_id,
                sender,
                shortcuts: Vec::new(),
            },
        );
        Ok(())
    }

    /// Drops one session, returning the app whose shortcuts the caller
    /// must unregister from the compositor.
    pub fn remove(&mut self, handle: &str) -> Option<Session> {
        self.inner.remove(handle)
    }

    /// Drops every session owned by `sender`, returning their handles.
    /// Bounded: one sender holds few sessions, and the table itself is
    /// capped.
    pub fn remove_sender(&mut self, sender: &str) -> Vec<(String, Session)> {
        let handles: Vec<String> = self
            .inner
            .iter()
            .filter(|(_, session)| session.sender == sender)
            .map(|(handle, _)| handle.clone())
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| self.inner.remove(&handle).map(|s| (handle, s)))
            .collect()
    }

    /// Remembers a session's shortcuts after a successful bind (replaces
    /// the set, capped). Returns false when the session is unknown.
    pub fn set_shortcuts(&mut self, handle: &str, shortcuts: Vec<ShortcutBinding>) -> bool {
        let Some(session) = self.inner.get_mut(handle) else {
            return false;
        };
        let mut kept = shortcuts;
        kept.truncate(MAX_SHORTCUTS_PER_SESSION);
        session.shortcuts = kept;
        true
    }

    /// Every remembered (app, shortcut) pair, for reconnect re-register.
    /// Bounded by the session and shortcut caps.
    pub fn all_shortcuts(&self) -> Vec<(String, ShortcutBinding)> {
        let mut out = Vec::new();
        for session in self.inner.values() {
            for shortcut in &session.shortcuts {
                out.push((session.app_id.clone(), shortcut.clone()));
            }
        }
        out
    }

    /// All sessions of one app: what an `Activated` fans out to.
    pub fn handles_for_app(&self, app_id: &str) -> Vec<String> {
        self.inner
            .iter()
            .filter(|(_, session)| session.app_id == app_id)
            .map(|(handle, _)| handle.clone())
            .collect()
    }
}

/// A session handle or app id is usable when it is short, non-empty and
/// carries only path-safe characters. The portal mints handles as object
/// paths; this refuses an empty string or an absurd one, never a real
/// client.
fn valid_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= 512
        && handle.starts_with('/')
        && handle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-'))
}

fn valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id.len() <= 256
        && app_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// One parsed `BindShortcuts` shortcut: the id plus the hint and the
/// description, each capped (the compositor caps them again: 256 and
/// 1024 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindShortcut {
    pub shortcut_id: String,
    pub description: Option<String>,
    pub preferred_trigger: Option<String>,
}

/// Parses a `CreateSession` body (`oo sa{sv}`): the request handle, the
/// session handle, the app id, and the options (skipped unread).
pub fn parse_create_session(body: &[u8]) -> Result<(String, String, String), ()> {
    let mut reader = Reader::le(body);
    let handle = reader.str()?.to_owned();
    let session = reader.str()?.to_owned();
    let app_id = reader.str()?.to_owned();
    reader.skip("a{sv}")?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok((handle, session, app_id))
}

/// Parses a `BindShortcuts` body (`ooa(sa{sv})sa{sv}`): the request and
/// session handles, the shortcuts, the parent window, and the options
/// (skipped unread).
pub fn parse_bind_shortcuts(
    body: &[u8],
) -> Result<(String, String, Vec<BindShortcut>, String), ()> {
    let mut reader = Reader::le(body);
    let handle = reader.str()?.to_owned();
    let session = reader.str()?.to_owned();
    let shortcuts = read_shortcut_list(&mut reader)?;
    let parent = reader.str()?.to_owned();
    reader.skip("a{sv}")?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok((handle, session, shortcuts, parent))
}

/// Parses a `ListShortcuts` body (`oo`): the request and session handles.
pub fn parse_list_shortcuts(body: &[u8]) -> Result<(String, String), ()> {
    let mut reader = Reader::le(body);
    let handle = reader.str()?.to_owned();
    let session = reader.str()?.to_owned();
    if !reader.exhausted() {
        return Err(());
    }
    Ok((handle, session))
}

/// Parses a `ConfigureShortcuts` body (`osa{sv}`): the session handle,
/// the parent window, and the options (skipped unread).
pub fn parse_configure_shortcuts(body: &[u8]) -> Result<String, ()> {
    let mut reader = Reader::le(body);
    let session = reader.str()?.to_owned();
    let _parent = reader.str()?;
    reader.skip("a{sv}")?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(session)
}

/// Reads an `a(sa{sv})` shortcut list: at most 512 entries, each id
/// capped at 128 bytes and each string value at the compositor's caps.
fn read_shortcut_list(reader: &mut Reader<'_>) -> Result<Vec<BindShortcut>, ()> {
    let mut elements = reader.elements(8)?;
    let mut out = Vec::new();
    while !elements.exhausted() {
        if out.len() >= MAX_SHORTCUTS_PER_SESSION {
            return Err(());
        }
        elements.enter_struct()?;
        let shortcut_id = cut(elements.str()?, 128).to_owned();
        let mut description: Option<String> = None;
        let mut preferred_trigger: Option<String> = None;
        let mut dict = elements.elements(8)?;
        while !dict.exhausted() {
            dict.enter_struct()?;
            let key = dict.str()?.to_owned();
            // `variant` positions past the value; reading it raw would
            // leave the cursor inside the value and loop forever.
            dict.variant(|sig, value| {
                match (key.as_str(), sig) {
                    ("description", "s") => {
                        description = Some(cut(value.str()?, 1024).to_owned());
                    }
                    ("preferred_trigger", "s") => {
                        preferred_trigger = Some(cut(value.str()?, 256).to_owned());
                    }
                    _ => value.skip(sig)?,
                }
                Ok(())
            })?;
            dict.leave_struct();
        }
        elements.leave_struct();
        if shortcut_id.is_empty() {
            return Err(());
        }
        out.push(BindShortcut {
            shortcut_id,
            description,
            preferred_trigger,
        });
    }
    Ok(out)
}

/// Cuts a string at a char boundary.
fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Builds a `ua{sv}` reply body: the numeric response plus the results
/// dict the caller wrote with `dict`.
pub fn reply_response(results: &[(String, WriterValue)]) -> Option<Vec<u8>> {
    reply_response_code(0, results)
}

/// Builds a `ua{sv}` reply body with an explicit response code.
pub fn reply_response_code(code: u32, results: &[(String, WriterValue)]) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    body.u32(code);
    write_dict(&mut body, results)?;
    body.take_body()
}

/// One dict value the backend writes.
#[derive(Debug, Clone)]
pub enum WriterValue {
    /// A shortcut list: `a(sa{sv})`.
    Shortcuts(Vec<ShortcutResult>),
}

/// One shortcut in a result list: the id plus its user-visible trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutResult {
    pub shortcut_id: String,
    pub description: Option<String>,
    /// What the user pressed to fire it: the bound chord, if any.
    pub trigger_description: Option<String>,
}

/// Writes an `a{sv}` dict of the caller's values.
fn write_dict(body: &mut Writer, results: &[(String, WriterValue)]) -> Option<()> {
    let cookie = body.open_array(8)?;
    for (key, value) in results {
        body.open_struct();
        body.str(key);
        let WriterValue::Shortcuts(shortcuts) = value;
        body.variant("a(sa{sv})");
        write_shortcut_list(body, shortcuts)?;
        body.close_struct();
    }
    body.close_array(cookie);
    Some(())
}

/// Writes an `a(sa{sv})` shortcut list.
fn write_shortcut_list(body: &mut Writer, shortcuts: &[ShortcutResult]) -> Option<()> {
    let cookie = body.open_array(8)?;
    for shortcut in shortcuts {
        body.open_struct();
        body.str(&shortcut.shortcut_id);
        let inner = body.open_array(8)?;
        if let Some(description) = &shortcut.description {
            body.open_struct();
            body.str("description");
            body.variant("s");
            body.str(description);
            body.close_struct();
        }
        if let Some(trigger) = &shortcut.trigger_description {
            body.open_struct();
            body.str("trigger_description");
            body.variant("s");
            body.str(trigger);
            body.close_struct();
        }
        body.close_array(inner);
        body.close_struct();
    }
    body.close_array(cookie);
    Some(())
}

/// Builds an `Activated`/`Deactivated` signal body (`osta{sv}`): the
/// session handle, the shortcut id, the timestamp in ms, and empty
/// options.
pub fn signal_activation_body(
    session: &str,
    shortcut_id: &str,
    timestamp_ms: u64,
) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    body.str(session);
    body.str(shortcut_id);
    body.u64(timestamp_ms);
    let cookie = body.open_array(8)?;
    body.close_array(cookie);
    body.take_body()
}

/// Builds a `ShortcutsChanged` signal body (`oa(sa{sv})`).
pub fn signal_changed_body(session: &str, shortcuts: &[ShortcutResult]) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    body.str(session);
    write_shortcut_list(&mut body, shortcuts)?;
    body.take_body()
}

/// The introspection XML: the four methods, the three signals, and the
/// Peer/Properties/Introspectable halves.
pub const INTROSPECT_XML: &str = "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\" \
  \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n\
  <node>\n\
  <interface name=\"org.freedesktop.impl.portal.GlobalShortcuts\">\n\
  <property name=\"version\" type=\"u\" access=\"read\"/>\n\
  <method name=\"CreateSession\"><arg direction=\"in\" type=\"o\"/><arg direction=\"in\" type=\"o\"/>\
  <arg direction=\"in\" type=\"s\"/><arg direction=\"in\" type=\"a{sv}\"/>\
  <arg direction=\"out\" type=\"u\"/><arg direction=\"out\" type=\"a{sv}\"/></method>\n\
  <method name=\"BindShortcuts\"><arg direction=\"in\" type=\"o\"/><arg direction=\"in\" type=\"o\"/>\
  <arg direction=\"in\" type=\"a(sa{sv})\"/><arg direction=\"in\" type=\"s\"/><arg direction=\"in\" type=\"a{sv}\"/>\
  <arg direction=\"out\" type=\"u\"/><arg direction=\"out\" type=\"a{sv}\"/></method>\n\
  <method name=\"ListShortcuts\"><arg direction=\"in\" type=\"o\"/><arg direction=\"in\" type=\"o\"/>\
  <arg direction=\"out\" type=\"u\"/><arg direction=\"out\" type=\"a{sv}\"/></method>\n\
  <method name=\"ConfigureShortcuts\"><arg direction=\"in\" type=\"o\"/>\
  <arg direction=\"in\" type=\"s\"/><arg direction=\"in\" type=\"a{sv}\"/></method>\n\
  <signal name=\"Activated\"><arg type=\"o\"/><arg type=\"s\"/><arg type=\"t\"/><arg type=\"a{sv}\"/></signal>\n\
  <signal name=\"Deactivated\"><arg type=\"o\"/><arg type=\"s\"/><arg type=\"t\"/><arg type=\"a{sv}\"/></signal>\n\
  <signal name=\"ShortcutsChanged\"><arg type=\"o\"/><arg type=\"a(sa{sv})\"/></signal>\n\
  </interface>\n\
  <interface name=\"org.freedesktop.DBus.Properties\">\n\
  <method name=\"Get\"><arg direction=\"in\" type=\"s\"/><arg direction=\"in\" type=\"s\"/>\
  <arg direction=\"out\" type=\"v\"/></method>\n\
  </interface>\n\
  <interface name=\"org.freedesktop.DBus.Peer\">\n\
  <method name=\"Ping\"/>\n\
  <method name=\"GetMachineId\"><arg direction=\"out\" type=\"s\"/></method>\n\
  </interface>\n\
  <interface name=\"org.freedesktop.DBus.Introspectable\">\n\
  <method name=\"Introspect\"><arg direction=\"out\" type=\"s\"/></method>\n\
  </interface>\n\
  <interface name=\"org.freedesktop.impl.portal.Request\">\n\
  <method name=\"Close\"/>\n\
  </interface>\n\
  <interface name=\"org.freedesktop.impl.portal.Session\">\n\
  <method name=\"Close\"/>\n\
  <signal name=\"Closed\"/>\n\
  </interface>\n\
  </node>\n";

#[cfg(test)]
mod tests;
