//! The portal backend against a private bus: a fake xdg-desktop-portal
//! drives the real backend binary, with no compositor behind it.
//!
//! What this pins: the backend owns its bus name, answers `Get version`
//! with 2 and `Introspect` with the interface, creates a session and
//! answers `ConfigureShortcuts` without one, refuses binds and lists
//! while the compositor is down (a `Failed` error, never a hang or a
//! fake empty list), closes sessions, and drops a dead sender's sessions
//! (app death arrives as `NameOwnerChanged`).
//!
//! Needs `dbus-daemon` on `PATH`: without one the test skips, unless
//! `SCOOT_PORTAL_REQUIRE_DBUS_DAEMON` is set, which makes that a failure
//! (CI sets it, the way the bar and notification jobs do for theirs).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use scoot_dbus::conn::{self, Conn, Event};
use scoot_dbus::proto::{Reader, Writer};

const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.scoot";
const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.impl.portal.GlobalShortcuts";

/// A private session bus: its own socket in its own directory, killed
/// and removed on drop.
struct Daemon {
    child: Child,
    dir: PathBuf,
    address: String,
}

impl Daemon {
    fn spawn() -> Option<Self> {
        let found = Command::new("dbus-daemon")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !found {
            assert!(
                std::env::var_os("SCOOT_PORTAL_REQUIRE_DBUS_DAEMON").is_none(),
                "SCOOT_PORTAL_REQUIRE_DBUS_DAEMON is set but there is no dbus-daemon on PATH"
            );
            eprintln!("skipped: no dbus-daemon on PATH");
            return None;
        }
        let dir = std::env::temp_dir().join(format!(
            "scoot-portal-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|left| left.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("a private directory for the daemon");
        let path = dir.join("bus");
        let config = dir.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                "<busconfig><type>session</type><auth>EXTERNAL</auth>\
                 <listen>unix:path={}</listen>\
                 <policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>\
                 <allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>",
                path.display(),
            ),
        )
        .expect("a config for the daemon");
        let mut child = Command::new("dbus-daemon")
            .arg("--nofork")
            .arg("--config-file")
            .arg(&config)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a private dbus-daemon");
        // Wait for the socket: the daemon binds it before serving.
        let start = Instant::now();
        while !path.exists() {
            if start.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                panic!("the private dbus-daemon never bound its socket");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let address = format!("unix:path={}", path.display());
        Some(Self {
            child,
            dir,
            address,
        })
    }

    fn connect(&self) -> Conn {
        let addr = conn::bus_addr_for(Some(std::ffi::OsStr::new(&self.address)))
            .expect("the test's address dials");
        match addr {
            conn::BusAddr::Path(path) => conn::connect(&path).expect("a client of the test bus"),
            conn::BusAddr::Abstract(name) => {
                conn::connect_abstract(&name).expect("a client of the test bus")
            }
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// One call against the backend: the reply body and signature, or the
/// error name. Anything else arriving with the answer is parked back.
fn call(
    conn: &mut Conn,
    member: &str,
    signature: &str,
    body: &[u8],
    token: u64,
) -> Result<(String, Vec<u8>), String> {
    conn.call(
        BUS_NAME,
        OBJECT_PATH,
        INTERFACE,
        member,
        signature,
        body,
        0,
        token,
    )
    .expect("the call queues");
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(15) {
            panic!("no answer to {member}");
        }
        let (events, _) = conn.pump();
        let mut parked = Vec::new();
        let mut answer = None;
        for event in events {
            match event {
                Event::Reply {
                    token: got,
                    signature,
                    body,
                } if got == token => answer = Some(Ok((signature, body))),
                Event::CallError { token: got, name } if got == token => {
                    answer = Some(Err(name));
                }
                event => parked.push(event),
            }
        }
        if let Some(answer) = answer {
            conn.restash(parked);
            return answer;
        }
        conn.restash(parked);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn empty_dict_inline(body: &mut Writer) {
    let cookie = body.open_array(8).expect("an empty dict");
    body.close_array(cookie);
}

fn create_session_body(handle: &str, session: &str, app_id: &str) -> Vec<u8> {
    let mut body = Writer::new();
    body.str(handle);
    body.str(session);
    body.str(app_id);
    empty_dict_inline(&mut body);
    body.take_body().expect("a tiny body")
}

fn get_name_owner(conn: &mut Conn) -> Option<String> {
    let mut body = Writer::new();
    body.str(BUS_NAME);
    let arg = body.take_body().expect("a tiny body");
    conn.call(
        conn::BUS_NAME,
        conn::BUS_PATH,
        conn::BUS_INTERFACE,
        "GetNameOwner",
        "s",
        &arg,
        0,
        999,
    )
    .ok()?;
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(2) {
            return None;
        }
        let (events, _) = conn.pump();
        let mut parked = Vec::new();
        let mut answer = None;
        for event in events {
            match event {
                Event::Reply {
                    token: 999, body, ..
                } => answer = Some(Reader::le(&body).str().ok().map(str::to_owned)),
                Event::CallError { token: 999, .. } => answer = Some(None),
                event => parked.push(event),
            }
        }
        if let Some(answer) = answer {
            conn.restash(parked);
            return answer;
        }
        conn.restash(parked);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The backend binary beside this test binary: integration tests run
/// from `target/<profile>/deps`, so the binary is two pops up plus its
/// name (`CARGO_BIN_EXE_*` would do the same, but the exact variable
/// spelling is toolchain-sensitive; this is not).
fn backend_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_scoot_globalshortcuts_portal") {
        return PathBuf::from(path);
    }
    let mut dir = std::env::current_exe().expect("the test's own path");
    dir.pop();
    if dir.ends_with("deps") {
        dir.pop();
    }
    dir.join("scoot-globalshortcuts-portal")
}

#[test]
fn portal_backend_serves_sessions_without_a_compositor() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let backend = backend_binary();
    let mut child = Command::new(&backend)
        .arg("daemon")
        .env("DBUS_SESSION_BUS_ADDRESS", &daemon.address)
        // No compositor here on purpose: binds and lists must refuse,
        // never hang and never fake an empty list.
        .env_remove("SCOOT_SOCKET")
        .env("XDG_RUNTIME_DIR", "/nonexistent-portal-test")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the backend starts");
    // Wait for the name: the daemon owns it before serving.
    let mut probe = daemon.connect();
    let start = Instant::now();
    loop {
        if get_name_owner(&mut probe).is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            panic!("the backend never owned {BUS_NAME}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut conn = daemon.connect();

    // Version reads 2.
    let mut body = Writer::new();
    body.str(INTERFACE);
    body.str("version");
    let arg = body.take_body().unwrap();
    conn.call(
        BUS_NAME,
        OBJECT_PATH,
        "org.freedesktop.DBus.Properties",
        "Get",
        "ss",
        &arg,
        0,
        10,
    )
    .unwrap();
    let start = Instant::now();
    let version = loop {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("no answer to Get version");
        }
        let (events, _) = conn.pump();
        let mut parked = Vec::new();
        let mut answer = None;
        for event in events {
            match event {
                Event::Reply {
                    token: 10, body, ..
                } => {
                    let mut reader = Reader::le(&body);
                    let variant = reader.variant_raw().expect("a variant");
                    assert_eq!(variant.signature(), "u");
                    let mut value = variant.read();
                    answer = Some(value.u32().expect("the version"));
                }
                Event::CallError { token: 10, name } => panic!("Get version failed: {name}"),
                event => parked.push(event),
            }
        }
        if let Some(answer) = answer {
            conn.restash(parked);
            break answer;
        }
        conn.restash(parked);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(version, 2, "the backend serves version 2");

    // Introspect names the interface.
    conn.call(
        BUS_NAME,
        OBJECT_PATH,
        "org.freedesktop.DBus.Introspectable",
        "Introspect",
        "",
        &[],
        0,
        11,
    )
    .unwrap();
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("no answer to Introspect");
        }
        let (events, _) = conn.pump();
        let mut parked = Vec::new();
        let mut done = false;
        for event in events {
            match event {
                Event::Reply {
                    token: 11, body, ..
                } => {
                    let xml = Reader::le(&body).str().unwrap().to_owned();
                    assert!(xml.contains("org.freedesktop.impl.portal.GlobalShortcuts"));
                    assert!(xml.contains("BindShortcuts"));
                    done = true;
                }
                Event::CallError { token: 11, name } => panic!("Introspect failed: {name}"),
                event => parked.push(event),
            }
        }
        if done {
            conn.restash(parked);
            break;
        }
        conn.restash(parked);
        std::thread::sleep(Duration::from_millis(10));
    }

    // CreateSession answers (0, {...}).
    let arg = create_session_body("/req/1", "/sess/1", "org.example");
    let (sig, body) =
        call(&mut conn, "CreateSession", "oosa{sv}", &arg, 20).expect("CreateSession answers");
    assert_eq!(sig, "ua{sv}");
    let mut reader = Reader::le(&body);
    assert_eq!(reader.u32().unwrap(), 0);

    // ConfigureShortcuts is an acknowledged no-op without a compositor.
    let mut arg = Writer::new();
    arg.str("/sess/1");
    arg.str("");
    empty_dict_inline(&mut arg);
    let arg = arg.take_body().unwrap();
    conn.call(
        BUS_NAME,
        OBJECT_PATH,
        INTERFACE,
        "ConfigureShortcuts",
        "osa{sv}",
        &arg,
        0,
        21,
    )
    .unwrap();
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("no answer to ConfigureShortcuts");
        }
        let (events, _) = conn.pump();
        let mut parked = Vec::new();
        let mut done = false;
        for event in events {
            match event {
                Event::Reply { token: 21, .. } => done = true,
                Event::CallError { token: 21, name } => {
                    panic!("ConfigureShortcuts failed: {name}")
                }
                event => parked.push(event),
            }
        }
        if done {
            conn.restash(parked);
            break;
        }
        conn.restash(parked);
        std::thread::sleep(Duration::from_millis(10));
    }

    // BindShortcuts with no compositor is Failed, never a hang.
    let mut arg = Writer::new();
    arg.str("/req/2");
    arg.str("/sess/1");
    let cookie = arg.open_array(8).unwrap();
    arg.open_struct();
    arg.str("toggle");
    let inner = arg.open_array(8).unwrap();
    arg.open_struct();
    arg.str("description");
    arg.variant("s");
    arg.str("Toggle");
    arg.close_struct();
    arg.close_array(inner);
    arg.close_struct();
    arg.close_array(cookie);
    arg.str("");
    empty_dict_inline(&mut arg);
    let arg = arg.take_body().unwrap();
    let refused = call(&mut conn, "BindShortcuts", "ooa(sa{sv})sa{sv}", &arg, 22);
    assert!(
        matches!(&refused, Err(name) if name == "org.freedesktop.DBus.Error.Failed"),
        "binds refuse while the compositor is down, got {refused:?}"
    );

    // ListShortcuts likewise refuses, never a fake empty list.
    let mut arg = Writer::new();
    arg.str("/req/3");
    arg.str("/sess/1");
    let arg = arg.take_body().unwrap();
    let refused = call(&mut conn, "ListShortcuts", "oo", &arg, 23);
    assert!(
        matches!(&refused, Err(name) if name == "org.freedesktop.DBus.Error.Failed"),
        "lists refuse while the compositor is down, got {refused:?}"
    );

    // App death drops the session: a new sender cannot list the old
    // handle anymore.
    {
        let mut app = daemon.connect();
        let arg = create_session_body("/req/9", "/sess/gone", "org.vanishing");
        call(&mut app, "CreateSession", "oosa{sv}", &arg, 30)
            .expect("the vanishing app creates a session");
        drop(app);
    }
    let mut arg = Writer::new();
    arg.str("/req/10");
    arg.str("/sess/gone");
    let arg = arg.take_body().unwrap();
    let start = Instant::now();
    let gone = loop {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("the dead sender's session never dropped");
        }
        match call(&mut conn, "ListShortcuts", "oo", &arg, 31) {
            Err(name) if name == "org.freedesktop.DBus.Error.InvalidArgs" => break true,
            // The backend may not have processed the death yet: a
            // `Failed` means the session is still there but the
            // compositor is down, so retry.
            Err(name) if name == "org.freedesktop.DBus.Error.Failed" => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            other => panic!("unexpected answer for a dead session: {other:?}"),
        }
    };
    assert!(gone);

    // Closing the session drops it: listing is InvalidArgs after.
    conn.call(
        BUS_NAME,
        "/sess/1",
        "org.freedesktop.impl.portal.Session",
        "Close",
        "",
        &[],
        0,
        40,
    )
    .unwrap();
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("no answer to Session.Close");
        }
        let (events, _) = conn.pump();
        let mut parked = Vec::new();
        let mut done = false;
        for event in events {
            match event {
                Event::Reply { token: 40, .. } => done = true,
                Event::CallError { token: 40, name } => panic!("Close failed: {name}"),
                event => parked.push(event),
            }
        }
        if done {
            conn.restash(parked);
            break;
        }
        conn.restash(parked);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut arg = Writer::new();
    arg.str("/req/11");
    arg.str("/sess/1");
    let arg = arg.take_body().unwrap();
    let refused = call(&mut conn, "ListShortcuts", "oo", &arg, 41);
    assert!(
        matches!(&refused, Err(name) if name == "org.freedesktop.DBus.Error.InvalidArgs"),
        "a closed session is gone, got {refused:?}"
    );

    let _ = child.kill();
    let _ = child.wait();
    // Drain stderr for debugging on failure only.
    let mut err = String::new();
    if let Some(stderr) = child.stderr.as_mut() {
        BufReader::new(stderr).read_line(&mut err).ok();
    }
    assert!(
        err.is_empty() || !err.contains("panic"),
        "backend stderr: {err}"
    );
}
