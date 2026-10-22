//! The daemon: owns the backend bus name, serves the portal methods,
//! and turns compositor events into portal signals.
//!
//! One `poll(2)` loop over the bus fd and the subscription thread's wake
//! eventfd: D-Bus method calls go to the session table and the
//! compositor over scoot-ipc, and the subscription thread's events arrive
//! over a channel the loop drains every turn (woken by the eventfd, so a
//! portal signal never waits for bus traffic). A dead compositor means
//! refused binds (never a dead backend): the subscription thread keeps
//! retrying, and the next turn re-registers what it remembers.
//!
//! Exit statuses: 1 failed, 3 the name is owned. The daemon runs until
//! killed or the bus goes away.

use std::collections::HashSet;
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_dbus::proto::{Reader, Writer};
use scoot_rt::print::warn;

use crate::ipc::{self, IpcEvent};
use crate::portal::{
    self, BindShortcut, MAX_MESSAGE, ROUNDTRIP_SECS, Sessions, ShortcutResult, WriterValue,
};

/// Runs the daemon: owns the name, serves until killed or the bus goes
/// away. Returns the exit status (3 when the name is owned).
pub fn serve() -> i32 {
    let address = std::env::var_os("DBUS_SESSION_BUS_ADDRESS");
    let mut conn = match conn::bus_addr_for(address.as_deref()) {
        Ok(conn::BusAddr::Path(path)) => match conn::connect(&path) {
            Ok(conn) => conn,
            Err(error) => {
                warn(format_args!(
                    "scoot-globalshortcuts-portal: no bus: {error}"
                ));
                return 1;
            }
        },
        Ok(conn::BusAddr::Abstract(name)) => match conn::connect_abstract(&name) {
            Ok(conn) => conn,
            Err(error) => {
                warn(format_args!(
                    "scoot-globalshortcuts-portal: no bus: {error}"
                ));
                return 1;
            }
        },
        Err(()) => {
            warn(format_args!(
                "scoot-globalshortcuts-portal: the session bus address names nothing this daemon dials"
            ));
            return 1;
        }
    };
    conn.set_max_message(MAX_MESSAGE);
    if let Err(report) = request_name(&mut conn) {
        warn(format_args!("scoot-globalshortcuts-portal: {report}"));
        return 3;
    }
    add_name_owner_match(&mut conn);
    let (sender, receiver) = mpsc::channel();
    let (wake, poll_wake) = match ipc::Wake::new(sender) {
        Ok(wake) => wake,
        Err(error) => {
            warn(format_args!(
                "scoot-globalshortcuts-portal: no wake eventfd: {error}"
            ));
            return 1;
        }
    };
    std::thread::spawn(move || ipc::subscribe_forever(wake));
    run_loop(&mut conn, receiver, poll_wake.as_fd());
    warn(format_args!(
        "scoot-globalshortcuts-portal: the bus went away"
    ));
    1
}

/// Requests the backend name with `DO_NOT_QUEUE`: `Ok` when primary
/// owner, `Err` naming the owner otherwise.
fn request_name(conn: &mut Conn) -> Result<(), String> {
    use scoot_dbus::proto::{request, request_reply};
    let mut body = Writer::new();
    body.str(portal::BUS_NAME);
    body.u32(request::DO_NOT_QUEUE);
    let Some(arg) = body.take_body() else {
        return Err(portal::BUS_NAME.to_owned());
    };
    if conn
        .call(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "RequestName",
            "su",
            &arg,
            0,
            1,
        )
        .is_err()
    {
        return Err(portal::BUS_NAME.to_owned());
    }
    match wait_u32(conn, 1) {
        Some(code) if code == request_reply::PRIMARY_OWNER => Ok(()),
        _ => Err(find_owner(conn)),
    }
}

fn find_owner(conn: &mut Conn) -> String {
    match bus_string(conn, "GetNameOwner", portal::BUS_NAME, 100) {
        Some(owner) => format!(
            "{} is owned by {owner}; stop that daemon first",
            portal::BUS_NAME
        ),
        None => format!("{} is owned; stop that daemon first", portal::BUS_NAME),
    }
}

fn bus_string(conn: &mut Conn, member: &str, arg: &str, token: u64) -> Option<String> {
    let mut body = Writer::new();
    body.str(arg);
    let arg = body.take_body()?;
    conn.call(
        conn::BUS_NAME,
        conn::BUS_PATH,
        conn::BUS_INTERFACE,
        member,
        "s",
        &arg,
        0,
        token,
    )
    .ok()?;
    let start = Instant::now();
    loop {
        if conn.dead() || start.elapsed() > Duration::from_secs(ROUNDTRIP_SECS) {
            return None;
        }
        let (events, _) = conn.pump();
        let mut parked = Vec::new();
        let mut answer: Option<Option<String>> = None;
        for event in events {
            match event {
                Event::Reply {
                    token: got, body, ..
                } if got == token => {
                    answer = Some(Reader::le(&body).str().ok().map(str::to_owned));
                }
                Event::CallError { token: got, .. } if got == token => {
                    answer = Some(None);
                }
                event => parked.push(event),
            }
        }
        if let Some(answer) = answer {
            conn.restash(parked);
            return answer;
        }
        conn.restash(parked);
        poll_in(conn);
    }
}

fn wait_u32(conn: &mut Conn, token: u64) -> Option<u32> {
    let start = Instant::now();
    loop {
        if conn.dead() || start.elapsed() > Duration::from_secs(ROUNDTRIP_SECS) {
            return None;
        }
        let (events, _) = conn.pump();
        let mut parked = Vec::new();
        let mut answer: Option<Option<u32>> = None;
        for event in events {
            match event {
                Event::Reply {
                    token: got, body, ..
                } if got == token => {
                    answer = Some(scoot_dbus::proto::read_request_reply("u", &body).ok());
                }
                Event::CallError { token: got, .. } if got == token => {
                    answer = Some(None);
                }
                event => parked.push(event),
            }
        }
        if let Some(answer) = answer {
            conn.restash(parked);
            return answer;
        }
        conn.restash(parked);
        poll_in(conn);
    }
}

fn poll_in(conn: &Conn) {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let fd = conn.as_fd();
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: 20_000_000,
    };
    let _ = poll(&mut fds, Some(&timeout));
}

/// Asks for `NameOwnerChanged`: app death arrives as that signal, and the
/// loop drops the dead sender's sessions.
fn add_name_owner_match(conn: &mut Conn) {
    use scoot_dbus::proto::flag;
    let mut body = Writer::new();
    body.str("type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged'");
    if let Some(arg) = body.take_body() {
        let _ = conn.call(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "AddMatch",
            "s",
            &arg,
            flag::NO_REPLY_EXPECTED,
            2,
        );
    }
}

/// Serves until the bus goes away.
fn run_loop(conn: &mut Conn, ipc: Receiver<IpcEvent>, wake: BorrowedFd<'_>) {
    let mut sessions = Sessions::new();
    let mut requests: HashSet<String> = HashSet::new();
    let mut compositor = ipc::Compositor::new();
    loop {
        let (events, capped) = conn.pump();
        if conn.dead() {
            return;
        }
        for event in events {
            match event {
                Event::MethodCall {
                    sender,
                    path,
                    interface,
                    member,
                    serial,
                    signature,
                    body,
                } => {
                    handle_call(
                        conn,
                        &mut sessions,
                        &mut requests,
                        &mut compositor,
                        &sender,
                        &path,
                        &interface,
                        &member,
                        serial,
                        &signature,
                        &body,
                    );
                    if conn.dead() {
                        return;
                    }
                }
                Event::Signal {
                    sender: _,
                    path,
                    interface,
                    member,
                    signature,
                    body,
                } if path == conn::BUS_PATH
                    && interface == conn::BUS_INTERFACE
                    && member == "NameOwnerChanged" =>
                {
                    owner_changed(conn, &mut sessions, &mut compositor, &signature, &body);
                }
                Event::Signal { .. } => {}
                _ => {}
            }
        }
        drain_ipc(conn, &sessions, &mut compositor, &ipc);
        if conn.dead() {
            return;
        }
        if capped || conn.has_staged_work() || conn.want_write() {
            continue;
        }
        if drain_ipc_now(conn, &sessions, &mut compositor, &ipc) {
            continue;
        }
        poll_loop(conn, wake);
        // A channel event that arrived while polling goes out next turn.
        drain_ipc(conn, &sessions, &mut compositor, &ipc);
        if conn.dead() {
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_call(
    conn: &mut Conn,
    sessions: &mut Sessions,
    requests: &mut HashSet<String>,
    compositor: &mut ipc::Compositor,
    sender: &str,
    path: &str,
    interface: &str,
    member: &str,
    serial: u32,
    signature: &str,
    body: &[u8],
) {
    if path == portal::OBJECT_PATH {
        match (interface, member) {
            (portal::INTERFACE, "CreateSession") if signature == "oosa{sv}" => {
                handle_create(conn, sessions, requests, sender, serial, body);
            }
            (portal::INTERFACE, "BindShortcuts") if signature == "ooa(sa{sv})sa{sv}" => {
                handle_bind(conn, sessions, requests, compositor, sender, serial, body);
            }
            (portal::INTERFACE, "ListShortcuts") if signature == "oo" => {
                handle_list(conn, sessions, compositor, sender, serial, body);
            }
            (portal::INTERFACE, "ConfigureShortcuts") if signature == "osa{sv}" => {
                handle_configure(conn, sessions, sender, serial, body);
            }
            ("org.freedesktop.DBus.Properties", "Get") if signature == "ss" => {
                handle_get(conn, sender, serial, body);
            }
            ("org.freedesktop.DBus.Peer", "Ping") if signature.is_empty() => {
                conn.reply_return(sender, serial, "", &[]);
            }
            ("org.freedesktop.DBus.Peer", "GetMachineId") if signature.is_empty() => {
                match machine_id() {
                    Some(id) => {
                        let mut reply = Writer::new();
                        reply.str(&id);
                        match reply.take_body() {
                            Some(arg) => conn.reply_return(sender, serial, "s", &arg),
                            None => conn.reply_error(
                                sender,
                                serial,
                                "org.freedesktop.DBus.Error.Failed",
                            ),
                        }
                    }
                    None => invalid(conn, sender, serial, "no machine id on this host"),
                }
            }
            ("org.freedesktop.DBus.Introspectable", "Introspect") if signature.is_empty() => {
                let mut reply = Writer::new();
                reply.str(portal::INTROSPECT_XML);
                match reply.take_body() {
                    Some(arg) => conn.reply_return(sender, serial, "s", &arg),
                    None => conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed"),
                }
            }
            _ => {
                if (interface, member) == (portal::INTERFACE, "CreateSession")
                    || (interface, member) == (portal::INTERFACE, "BindShortcuts")
                    || (interface, member) == (portal::INTERFACE, "ListShortcuts")
                    || (interface, member) == (portal::INTERFACE, "ConfigureShortcuts")
                {
                    invalid(conn, sender, serial, "bad arguments");
                } else {
                    conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownMethod");
                }
            }
        }
        return;
    }
    // Request and session objects: only `Close` lives here.
    if (interface == "org.freedesktop.impl.portal.Request"
        || interface == "org.freedesktop.impl.portal.Session")
        && member == "Close"
        && signature.is_empty()
    {
        if sessions.get(path).is_some() {
            if let Some(session) = sessions.remove(path) {
                // Best-effort single attempts: closing must not wedge
                // the loop behind a dead compositor.
                for shortcut in &session.shortcuts {
                    let _ = compositor.unregister_once(&session.app_id, &shortcut.shortcut_id);
                }
                emit_closed(conn, path);
            }
            conn.reply_return(sender, serial, "", &[]);
        } else if requests.remove(path) {
            conn.reply_return(sender, serial, "", &[]);
        } else {
            conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownObject");
        }
        return;
    }
    conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownObject");
}

fn handle_create(
    conn: &mut Conn,
    sessions: &mut Sessions,
    requests: &mut HashSet<String>,
    sender: &str,
    serial: u32,
    body: &[u8],
) {
    let Ok((handle, session_handle, app_id)) = portal::parse_create_session(body) else {
        invalid(conn, sender, serial, "CreateSession needs (oos a{sv})");
        return;
    };
    requests.insert(handle);
    match sessions.create(session_handle, app_id, sender.to_owned()) {
        Ok(()) => {
            // Request paths are small strings from the portal; cap the
            // remembered set so a flood cannot grow it without bound.
            if requests.len() > 8192 {
                requests.clear();
            }
            match portal::reply_response(&[]) {
                Some(arg) => conn.reply_return(sender, serial, "ua{sv}", &arg),
                None => conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed"),
            }
        }
        Err(message) => invalid(conn, sender, serial, &message),
    }
}

fn handle_bind(
    conn: &mut Conn,
    sessions: &mut Sessions,
    requests: &mut HashSet<String>,
    compositor: &mut ipc::Compositor,
    sender: &str,
    serial: u32,
    body: &[u8],
) {
    let Ok((handle, session_handle, shortcuts, _parent)) = portal::parse_bind_shortcuts(body)
    else {
        invalid(
            conn,
            sender,
            serial,
            "BindShortcuts needs (oo a(sa{sv}) s a{sv})",
        );
        return;
    };
    requests.insert(handle);
    let Some(session) = sessions.get(&session_handle) else {
        invalid(conn, sender, serial, "no such session");
        return;
    };
    if session.sender != sender {
        invalid(conn, sender, serial, "session belongs to another sender");
        return;
    }
    let app_id = session.app_id.clone();
    // Heal first: a redialed compositor connection forgot every
    // registration, and registering the new shortcuts onto a wiped
    // table would strand the session's older ones.
    heal(compositor, sessions);
    // Register each shortcut, stopping at the first transport failure.
    for shortcut in &shortcuts {
        if let Err(error) = compositor.register(
            &app_id,
            &shortcut.shortcut_id,
            shortcut.preferred_trigger.as_deref(),
            shortcut.description.as_deref(),
        ) {
            if error.kind() == std::io::ErrorKind::InvalidInput {
                invalid(conn, sender, serial, &format!("bad shortcut: {}", error));
            } else {
                conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
            }
            return;
        }
    }
    let remembered: Vec<crate::portal::ShortcutBinding> = shortcuts
        .iter()
        .map(|shortcut: &BindShortcut| crate::portal::ShortcutBinding {
            shortcut_id: shortcut.shortcut_id.clone(),
            description: shortcut.description.clone(),
            preferred_trigger: shortcut.preferred_trigger.clone(),
        })
        .collect();
    sessions.set_shortcuts(&session_handle, remembered);
    emit_changed_for_app(conn, sessions, compositor, &app_id);
    answer_shortcuts(conn, sessions, compositor, sender, serial, &app_id);
}

fn handle_list(
    conn: &mut Conn,
    sessions: &mut Sessions,
    compositor: &mut ipc::Compositor,
    sender: &str,
    serial: u32,
    body: &[u8],
) {
    let Ok((_handle, session_handle)) = portal::parse_list_shortcuts(body) else {
        invalid(conn, sender, serial, "ListShortcuts needs (oo)");
        return;
    };
    let Some(session) = sessions.get(&session_handle) else {
        invalid(conn, sender, serial, "no such session");
        return;
    };
    if session.sender != sender {
        invalid(conn, sender, serial, "session belongs to another sender");
        return;
    }
    let app_id = session.app_id.clone();
    heal(compositor, sessions);
    answer_shortcuts(conn, sessions, compositor, sender, serial, &app_id);
}

fn handle_configure(
    conn: &mut Conn,
    sessions: &mut Sessions,
    sender: &str,
    serial: u32,
    body: &[u8],
) {
    let Ok(session_handle) = portal::parse_configure_shortcuts(body) else {
        invalid(conn, sender, serial, "ConfigureShortcuts needs (os a{sv})");
        return;
    };
    let Some(session) = sessions.get(&session_handle) else {
        invalid(conn, sender, serial, "no such session");
        return;
    };
    if session.sender != sender {
        invalid(conn, sender, serial, "session belongs to another sender");
        return;
    }
    // Assignment lives in the user's config file: there is no
    // in-compositor configure UI to show (stage 1 has none), so this is
    // an acknowledged no-op. The client re-lists when the user edits.
    conn.reply_return(sender, serial, "", &[]);
}

fn handle_get(conn: &mut Conn, sender: &str, serial: u32, body: &[u8]) {
    let mut reader = Reader::le(body);
    let Ok(interface) = reader.str() else {
        invalid(conn, sender, serial, "Get needs (ss)");
        return;
    };
    let Ok(property) = reader.str() else {
        invalid(conn, sender, serial, "Get needs (ss)");
        return;
    };
    if !reader.exhausted() {
        invalid(conn, sender, serial, "Get needs (ss)");
        return;
    }
    if interface == portal::INTERFACE && property == "version" {
        let mut reply = Writer::new();
        reply.variant("u");
        reply.u32(portal::VERSION);
        match reply.take_body() {
            Some(arg) => conn.reply_return(sender, serial, "v", &arg),
            None => conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed"),
        }
    } else {
        conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.UnknownProperty");
    }
}

/// Answers `(0, {shortcuts: [...]})` for `app_id` from the compositor's
/// live list. A dead compositor is `Failed`, never an empty list passed
/// off as unbound.
fn answer_shortcuts(
    conn: &mut Conn,
    _sessions: &Sessions,
    compositor: &mut ipc::Compositor,
    sender: &str,
    serial: u32,
    app_id: &str,
) {
    let listed = match compositor.list() {
        Ok(listed) => listed,
        Err(_) => {
            conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed");
            return;
        }
    };
    let results = shortcuts_for_app(&listed, app_id);
    match portal::reply_response(&[(String::from("shortcuts"), WriterValue::Shortcuts(results))]) {
        Some(arg) => conn.reply_return(sender, serial, "ua{sv}", &arg),
        None => conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.Failed"),
    }
}

/// Re-registers what this backend remembers but the compositor no
/// longer lists (a redialed connection starts with a wiped table, and a
/// restart wipes it too): idempotent upserts, with bound chords coming
/// back from `[global_shortcuts]`. Returns the fresh list for the
/// caller's fan-out, or `None` when the compositor is down. Best-effort
/// with single attempts (see [`Compositor::list_once`]): a dead
/// compositor costs refused connects, never a wedged loop. Two passes:
/// the first may only notice the connection died (and drop it); the
/// second redials and lists on the live one.
fn heal(
    compositor: &mut ipc::Compositor,
    sessions: &Sessions,
) -> Option<Vec<scoot_ipc::ShortcutInfo>> {
    for _ in 0..2 {
        let listed = match compositor.list_once() {
            Ok(listed) => listed,
            Err(_) => continue,
        };
        for (app_id, binding) in sessions.all_shortcuts() {
            let known = listed
                .iter()
                .any(|info| info.app_id == app_id && info.shortcut_id == binding.shortcut_id);
            if !known {
                let _ = compositor.register_once(
                    &app_id,
                    &binding.shortcut_id,
                    binding.preferred_trigger.as_deref(),
                    binding.description.as_deref(),
                );
            }
        }
        // Re-list after registering, so the fan-out below sees the
        // healed state rather than the pre-heal one.
        match compositor.list_once() {
            Ok(fresh) => return Some(fresh),
            Err(_) => return Some(listed),
        }
    }
    None
}

fn shortcuts_for_app(listed: &[scoot_ipc::ShortcutInfo], app_id: &str) -> Vec<ShortcutResult> {
    listed
        .iter()
        .filter(|info| info.app_id == app_id)
        .map(|info| ShortcutResult {
            shortcut_id: info.shortcut_id.clone(),
            description: info.description.clone(),
            trigger_description: info.bound.clone(),
        })
        .collect()
}

/// Emits `ShortcutsChanged` to every session of `app_id` with its fresh
/// filtered list. Best-effort: a dead compositor emits nothing.
fn emit_changed_for_app(
    conn: &mut Conn,
    sessions: &Sessions,
    compositor: &mut ipc::Compositor,
    app_id: &str,
) {
    let listed = match compositor.list() {
        Ok(listed) => listed,
        Err(_) => return,
    };
    let results = shortcuts_for_app(&listed, app_id);
    for handle in sessions.handles_for_app(app_id) {
        if let Some(arg) = portal::signal_changed_body(&handle, &results) {
            conn.signal(
                portal::OBJECT_PATH,
                portal::INTERFACE,
                "ShortcutsChanged",
                "oa(sa{sv})",
                &arg,
            );
        }
    }
}

fn emit_closed(conn: &mut Conn, session: &str) {
    // `Closed` carries no body: queue an empty signal on the session path.
    conn.signal(
        session,
        "org.freedesktop.impl.portal.Session",
        "Closed",
        "",
        &[],
    );
}

/// Drops a dead sender's sessions and unregisters their shortcuts.
fn owner_changed(
    conn: &mut Conn,
    sessions: &mut Sessions,
    compositor: &mut ipc::Compositor,
    signature: &str,
    body: &[u8],
) {
    if signature != "sss" {
        return;
    }
    let mut reader = Reader::le(body);
    let Ok(name) = reader.str().map(str::to_owned) else {
        return;
    };
    let Ok(_old) = reader.str() else {
        return;
    };
    let Ok(new) = reader.str().map(str::to_owned) else {
        return;
    };
    if !new.is_empty() {
        return;
    }
    for (handle, session) in sessions.remove_sender(&name) {
        for shortcut in &session.shortcuts {
            let _ = compositor.unregister_once(&session.app_id, &shortcut.shortcut_id);
        }
        emit_closed(conn, &handle);
    }
}

/// Drains the subscription channel without blocking: activations go out
/// at once; `Changed` heals (single attempts, so a dead compositor
/// costs one refused connect) and fans out per app.
fn drain_ipc(
    conn: &mut Conn,
    sessions: &Sessions,
    compositor: &mut ipc::Compositor,
    ipc: &Receiver<IpcEvent>,
) {
    loop {
        match ipc.try_recv() {
            Ok(IpcEvent::Activated {
                app_id,
                shortcut_id,
                timestamp_ms,
            }) => {
                for handle in sessions.handles_for_app(&app_id) {
                    if let Some(arg) = portal::signal_activation_body(
                        &handle,
                        &shortcut_id,
                        u64::from(timestamp_ms),
                    ) {
                        conn.signal(
                            portal::OBJECT_PATH,
                            portal::INTERFACE,
                            "Activated",
                            "osta{sv}",
                            &arg,
                        );
                    }
                }
            }
            Ok(IpcEvent::Deactivated {
                app_id,
                shortcut_id,
                timestamp_ms,
            }) => {
                for handle in sessions.handles_for_app(&app_id) {
                    if let Some(arg) = portal::signal_activation_body(
                        &handle,
                        &shortcut_id,
                        u64::from(timestamp_ms),
                    ) {
                        conn.signal(
                            portal::OBJECT_PATH,
                            portal::INTERFACE,
                            "Deactivated",
                            "osta{sv}",
                            &arg,
                        );
                    }
                }
            }
            Ok(IpcEvent::Changed) => {
                // Nothing remembered means nothing to heal and nobody
                // to tell: skip the re-list instead of probing it.
                if sessions.is_empty() {
                    continue;
                }
                // Heal-then-fan-out: a reconnect (or a restart that
                // wiped the compositor's table) arrives as `Changed`
                // from the subscription thread. Single attempts only
                // (see `heal`): a dead compositor costs refused
                // connects, and the resubscribe that only succeeds once
                // the compositor serves again retries the heal.
                let Some(listed) = heal(compositor, sessions) else {
                    continue;
                };
                let mut apps: Vec<String> = Vec::new();
                for info in &listed {
                    if !apps.iter().any(|app| app == &info.app_id) {
                        apps.push(info.app_id.clone());
                    }
                }
                for app_id in &apps {
                    emit_changed_for_app(conn, sessions, compositor, app_id);
                }
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => return,
        }
    }
}

/// Whether the channel holds an event: the loop skips the poll when the
/// last turn staged bus work, but a pending portal signal still goes out
/// next turn rather than waiting for bus traffic.
fn drain_ipc_now(
    conn: &mut Conn,
    sessions: &Sessions,
    compositor: &mut ipc::Compositor,
    ipc: &Receiver<IpcEvent>,
) -> bool {
    let before = conn.want_write();
    drain_ipc(conn, sessions, compositor, ipc);
    before != conn.want_write()
}

/// Sleeps until the bus or the subscription thread speaks: the loop
/// owns no timer, so without the wake half a portal signal would wait
/// for bus traffic to be noticed.
fn poll_loop(conn: &Conn, wake: BorrowedFd<'_>) {
    use rustix::event::{PollFd, PollFlags, poll};
    let bus = conn.as_fd();
    let mut fds = [
        PollFd::new(&bus, PollFlags::IN),
        PollFd::new(&wake, PollFlags::IN),
    ];
    let _ = poll(&mut fds, None);
    // Drain the wake counter (best-effort: a shared counter means the
    // value does not matter, only that the fd goes quiet again).
    let mut buf = [0u8; 8];
    while rustix::io::read(wake, &mut buf).is_ok() {}
}

fn invalid(conn: &mut Conn, sender: &str, serial: u32, what: &str) {
    let mut body = Writer::new();
    body.str(what);
    match body.take_body() {
        Some(arg) => conn.reply_error_body(
            sender,
            serial,
            "org.freedesktop.DBus.Error.InvalidArgs",
            "s",
            &arg,
        ),
        None => conn.reply_error(sender, serial, "org.freedesktop.DBus.Error.InvalidArgs"),
    }
}

fn machine_id() -> Option<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            let id = text.lines().next().unwrap_or("").trim().to_owned();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}
