//! The spawning's tests: the pure pins (unit names, the bus bodies,
//! the environment rules), each mutation-proved, plus the bus calls
//! against a real `dbus-daemon` and a fake application service.

use super::*;
use scoot_dbus::proto::Reader;

// --- Pure pins ----------------------------------------------------------

#[test]
fn app_id_strips_the_desktop_suffix() {
    assert_eq!(
        app_id_for("org.mozilla.firefox.desktop"),
        "org.mozilla.firefox"
    );
    assert_eq!(app_id_for("terminal.desktop"), "terminal");
    assert_eq!(app_id_for("bare"), "bare");
    assert_eq!(app_id_for("desktop"), "desktop");
}

#[test]
fn scope_names_follow_the_systemd_scheme() {
    assert_eq!(
        scope_name("org.mozilla.firefox.desktop", "1a2b3c4d"),
        "app-scootlaunch-org.mozilla.firefox.desktop-1a2b3c4d.scope"
    );
}

#[test]
fn scope_ids_escape_hostile_characters() {
    assert_eq!(escape_unit_id("a/b:c d%e"), "a_b_c_d_e");
    assert_eq!(escape_unit_id(""), "_");
    assert_eq!(escape_unit_id("ok-Name.1_x"), "ok-Name.1_x");
}

#[test]
fn scope_ids_cut_at_a_char_boundary() {
    let long = format!("{}.desktop", "ü".repeat(200));
    let escaped = escape_unit_id(&long);
    assert!(escaped.len() <= MAX_SCOPE_ID, "{escaped}");
    assert!(escaped.is_char_boundary(escaped.len()), "{escaped}");
    assert!(scope_name(&long, "12345678").len() < 256);
}

#[test]
fn command_scope_bases_lose_their_dir_and_dashes() {
    assert_eq!(command_scope_base("/usr/bin/foot"), "foot");
    assert_eq!(command_scope_base("my-app"), "my_app");
    assert_eq!(command_scope_base(""), "_");
    assert_eq!(command_scope_base("/"), "_");
}

#[test]
fn random_suffixes_are_hex_and_unique() {
    let first = random_suffix();
    assert_eq!(first.len(), RANDOM_BYTES * 2, "{first}");
    assert!(
        first.chars().all(|char| char.is_ascii_hexdigit()),
        "{first}"
    );
    let second = random_suffix();
    assert_ne!(first, second, "two draws collide");
}

#[test]
fn bus_names_come_from_the_id() {
    assert_eq!(
        dbus_bus_name("org.mozilla.firefox.desktop").as_deref(),
        Some("org.mozilla.firefox")
    );
    // One element is no bus name (the wire wants a dot): `Exec` only.
    assert_eq!(dbus_bus_name("single.desktop"), None);
    // Not bus names: empty, and what the wire refuses.
    assert_eq!(dbus_bus_name(".desktop"), None);
    assert_eq!(dbus_bus_name("has space.desktop"), None);
    assert_eq!(dbus_bus_name(""), None);
}

#[test]
fn object_paths_lead_with_a_slash_and_map_dots_and_dashes() {
    assert_eq!(
        dbus_object_path("org.mozilla.firefox").as_deref(),
        Some("/org/mozilla/firefox")
    );
    assert_eq!(
        dbus_object_path("com.example.my-app").as_deref(),
        Some("/com/example/my_app")
    );
    assert_eq!(dbus_object_path(""), None);
    assert_eq!(dbus_object_path("has space"), None);
    assert_eq!(dbus_object_path("a..b"), None);
}

#[test]
fn run_text_splits_on_spaces() {
    let argv = split_run_argv("htop --sort-key PERCENT_CPU");
    assert_eq!(argv, vec!["htop", "--sort-key", "PERCENT_CPU"]);
    assert!(split_run_argv("   ").is_empty());
    assert!(split_run_argv("").is_empty());
}

#[test]
fn child_envs_strip_the_old_tokens_and_set_the_new() {
    // No token, no display: both carriers stripped, nothing set.
    let (remove, set) = child_env_delta(None, false);
    assert_eq!(remove, vec![ACTIVATION_TOKEN_ENV, STARTUP_ID_ENV]);
    assert!(set.is_empty());
    // A token without a display: the token rides `XDG_ACTIVATION_TOKEN`
    // only (no X startup id for a Wayland-only child).
    let (remove, set) = child_env_delta(Some("tok"), false);
    assert_eq!(remove, vec![ACTIVATION_TOKEN_ENV, STARTUP_ID_ENV]);
    assert_eq!(set, vec![(ACTIVATION_TOKEN_ENV, "tok".to_string())]);
    // A token with a display: the startup id rides along.
    let (_, set) = child_env_delta(Some("tok"), true);
    assert_eq!(
        set,
        vec![
            (ACTIVATION_TOKEN_ENV, "tok".to_string()),
            (STARTUP_ID_ENV, "tok".to_string())
        ]
    );
    // An empty token sets nothing (a refused mint leaks nothing).
    let (_, set) = child_env_delta(Some(""), true);
    assert!(set.is_empty());
}

#[test]
fn tokens_never_debug() {
    let token = Token::new("secret-token-123".to_string());
    let debug = format!("{token:?}");
    assert!(!debug.contains("secret-token-123"), "{debug}");
    assert_eq!(token.as_str(), "secret-token-123");
    assert!(!token.is_empty());
    assert!(Token::new(String::new()).is_empty());
}

#[test]
fn working_dirs_prefer_path_then_home() {
    let root = tempfile::tempdir().unwrap();
    let present = root.path().join("work");
    std::fs::create_dir(&present).unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    assert_eq!(
        resolve_cwd(Some(present.to_str().unwrap()), Some(&home)),
        Some(present.clone())
    );
    // A missing `Path=` falls back to `$HOME`, not to an error.
    assert_eq!(
        resolve_cwd(Some("/nonexistent-scootlaunch-dir"), Some(&home)),
        Some(home.clone())
    );
    // A relative `Path=` is not trusted.
    assert_eq!(
        resolve_cwd(Some("relative"), Some(&home)),
        Some(home.clone())
    );
    // No home either: inherit.
    assert_eq!(resolve_cwd(None, None), None);
    assert_eq!(
        resolve_cwd(Some("/nonexistent-scootlaunch-dir"), None),
        None
    );
}

#[test]
fn transient_bodies_carry_the_scope_slice_pids_and_collect_mode() {
    let (sig, body) =
        transient_body("app-scootlaunch-x-12345678.scope", 4242, "scootlaunch x").unwrap();
    assert_eq!(sig, "ssa(sv)a(sa(sv))");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("app-scootlaunch-x-12345678.scope"), "{text}");
    assert!(text.contains("fail"), "{text}");
    assert!(text.contains("Slice"), "{text}");
    assert!(text.contains("app.slice"), "{text}");
    assert!(text.contains("PIDs"), "{text}");
    assert!(text.contains("CollectMode"), "{text}");
    assert!(text.contains("inactive-or-failed"), "{text}");
    assert!(text.contains("scootlaunch x"), "{text}");
    assert!(
        body.windows(4)
            .any(|window| window == 4242u32.to_le_bytes()),
        "the pid rides along"
    );
}

#[test]
fn activate_bodies_carry_the_token_key() {
    let (sig, body) = activate_body(Some("tok-1")).unwrap();
    assert_eq!(sig, "a{sv}");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("activation-token"), "{text}");
    assert!(text.contains("tok-1"), "{text}");
    let (_, empty) = activate_body(None).unwrap();
    let empty_text = String::from_utf8_lossy(&empty);
    assert!(!empty_text.contains("activation-token"), "{empty_text}");
}

#[test]
fn activate_action_bodies_name_the_action_first() {
    let (sig, body) = activate_action_body("new-window", Some("tok-2")).unwrap();
    assert_eq!(sig, "sava{sv}");
    let mut reader = Reader::le(&body);
    assert_eq!(reader.str().unwrap(), "new-window");
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("activation-token"), "{text}");
    assert!(text.contains("tok-2"), "{text}");
}

#[test]
fn detach_specs_round_trip() {
    let spec = DetachSpec {
        argv: vec!["/usr/bin/app".to_string(), "a b".to_string()],
        cwd: Some("/home/u".to_string()),
        remove: vec!["XDG_ACTIVATION_TOKEN".to_string()],
        set: vec![("A".to_string(), "B".to_string())],
    };
    let json = serde_json::to_string(&spec).unwrap();
    let back: DetachSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(back.argv, spec.argv);
    assert_eq!(back.cwd, spec.cwd);
    assert_eq!(back.remove, spec.remove);
    assert_eq!(back.set, spec.set);
}

// --- Against a real bus -------------------------------------------------

fn daemon() -> Option<scoot_dbus::testdaemon::Daemon> {
    scoot_dbus::testdaemon::Daemon::spawn()
}

fn bus_addr_of(daemon: &scoot_dbus::testdaemon::Daemon) -> BusAddr {
    BusAddr::Path(daemon.path())
}

#[test]
fn a_missing_service_is_a_named_error_not_a_hang() {
    let Some(daemon) = daemon() else {
        return;
    };
    let addr = bus_addr_of(&daemon);
    let deadline = Instant::now() + Duration::from_secs(10);
    let call = Call {
        dest: "sh.scoot.Nobody",
        path: "/sh/scoot/Nobody",
        interface: "sh.scoot.Nothing",
        member: "Ping",
        sig: "",
        body: &[],
    };
    let error = dbus_call_at(&addr, &call, deadline).expect_err("nobody owns the name");
    assert!(
        error.contains("ServiceUnknown"),
        "a refusal names itself: {error}"
    );
}

#[test]
fn a_scope_move_without_a_manager_is_a_named_refusal() {
    let Some(daemon) = daemon() else {
        return;
    };
    let addr = bus_addr_of(&daemon);
    // The test bus has no systemd: the framing reaches a real daemon
    // and its refusal comes back shaped.
    let deadline = Instant::now() + Duration::from_secs(10);
    let (sig, body) =
        transient_body("app-scootlaunch-x-12345678.scope", 1, "scootlaunch x").unwrap();
    let call = Call {
        dest: "org.freedesktop.systemd1",
        path: "/org/freedesktop/systemd1",
        interface: "org.freedesktop.systemd1.Manager",
        member: "StartTransientUnit",
        sig: &sig,
        body: &body,
    };
    let error = dbus_call_at(&addr, &call, deadline).expect_err("no manager on the test bus");
    assert!(
        error.contains("ServiceUnknown"),
        "a refusal names itself: {error}"
    );
}

/// A fake single-instance application: owns a bus name on the test
/// bus and answers `Activate`, recording the platform data it got.
fn serve_activation(
    daemon: &scoot_dbus::testdaemon::Daemon,
    bus_name: &str,
) -> (std::thread::JoinHandle<Vec<u8>>, String) {
    use scoot_dbus::conn::{self, Event};
    use scoot_dbus::proto::{Writer, request};
    use scoot_dbus::testdaemon::until;
    let path = daemon.path();
    let owned = bus_name.to_string();
    let handle = std::thread::spawn(move || {
        let mut service = conn::connect(&path).expect("the daemon accepts the service");
        let mut body = Writer::new();
        body.str(&owned);
        body.u32(request::ALLOW_REPLACEMENT | request::DO_NOT_QUEUE);
        service
            .call(
                conn::BUS_NAME,
                conn::BUS_PATH,
                conn::BUS_INTERFACE,
                "RequestName",
                "su",
                &body.take_body().unwrap(),
                0,
                1,
            )
            .unwrap();
        let Event::Reply { .. } = until(&mut service, |event| {
            matches!(event, Event::Reply { token: 1, .. })
        }) else {
            unreachable!();
        };
        let Event::MethodCall {
            sender,
            serial,
            member,
            signature,
            body,
            ..
        } = until(&mut service, |event| {
            matches!(event, Event::MethodCall { .. })
        })
        else {
            unreachable!();
        };
        assert_eq!(member, "Activate");
        assert_eq!(signature, "a{sv}");
        service.reply_return(&sender, serial, "", &[]);
        let (events, _) = service.pump();
        assert!(!service.dead());
        drop(events);
        body
    });
    (handle, bus_name.to_owned())
}

/// Under a real user manager (opt-in: `SCOOTLAUNCH_MANUAL_SCOPE=1`,
/// the dev VM — never CI), a spawn lands in its own scope: the call,
/// the unit's slice, and `systemctl --user status` agree. Skipped
/// without the variable, so no runner depends on a manager.
#[test]
fn scope_moves_a_spawn_under_a_user_manager() {
    if std::env::var_os("SCOOTLAUNCH_MANUAL_SCOPE").is_none() {
        eprintln!("skipped: set SCOOTLAUNCH_MANUAL_SCOPE=1 under a user manager");
        return;
    }
    // The sleeper writes its own pid (after `exec`, the same process),
    // so the cleanup kills exactly it and nothing else's.
    let dir = std::env::temp_dir().join(format!("scootlaunch-scope-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("sleeper.pid");
    let ctx = Ctx {
        state_home: None,
        home: None,
        display_set: false,
        now: 0,
    };
    let entry = Entry {
        id: "sh.scoot.Sleep.desktop".to_string(),
        name: "Sleep".to_string(),
        generic_name: None,
        comment: None,
        keywords: Vec::new(),
        icon: None,
        exec: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!("echo $$ > '{}'; exec sleep 27", marker.display()),
        ],
        terminal: false,
        dbus_activatable: false,
        actions: Vec::new(),
        path: PathBuf::from("/nonexistent-scootlaunch/sleep.desktop"),
        workdir: None,
        hidden: None,
    };
    launch_entry(&entry, None, None, &ctx).expect("a manager moves the spawn");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let pid_text = std::fs::read_to_string(&marker).unwrap_or_default();
    let pid: u32 = pid_text.trim().parse().expect("the sleeper wrote its pid");
    // The scope is up: find it, show it, and record the raw output.
    let units = std::process::Command::new("systemctl")
        .args([
            "--user",
            "list-units",
            "--type=scope",
            "--state=running",
            "--no-legend",
            "--no-pager",
        ])
        .output()
        .expect("systemctl runs");
    let listing = String::from_utf8_lossy(&units.stdout);
    let scope = listing
        .lines()
        .map(|line| line.split_whitespace().next().unwrap_or(""))
        .find(|unit| unit.starts_with("app-scootlaunch-sh.scoot.Sleep.desktop-"))
        .expect("the scope is listed")
        .to_string();
    let status = std::process::Command::new("systemctl")
        .args(["--user", "status", &scope, "--no-pager"])
        .output()
        .expect("systemctl runs");
    let out = String::from_utf8_lossy(&status.stdout);
    eprintln!("SCOPE-STATUS:\n{out}");
    assert!(out.contains("sleep"), "the sleeper is in its scope:\n{out}");
    let show = std::process::Command::new("systemctl")
        .args([
            "--user",
            "show",
            &scope,
            "-p",
            "Slice,CollectMode,Description,NControlProcesses",
        ])
        .output()
        .expect("systemctl runs");
    let shown = String::from_utf8_lossy(&show.stdout);
    eprintln!("SCOPE-SHOW:\n{shown}");
    assert!(
        shown.contains("Slice=app.slice"),
        "the scope is in app.slice:\n{shown}"
    );
    // Cleanup: exactly our sleeper, by pid.
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .output();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn activate_reaches_the_service_with_the_token() {
    let Some(daemon) = daemon() else {
        return;
    };
    let (service, _) = serve_activation(&daemon, "sh.scoot.Stub");
    // The service needs a turn to own its name before the call.
    std::thread::sleep(Duration::from_millis(300));
    let addr = bus_addr_of(&daemon);
    let deadline = Instant::now() + Duration::from_secs(10);
    // The explicit-address twin of `app_activate` (which reads the
    // process bus address — never mutated from a test): same member,
    // path and body.
    let (sig, body) = activate_body(Some("tok-abc")).unwrap();
    let call = Call {
        dest: "sh.scoot.Stub",
        path: "/sh/scoot/Stub",
        interface: "org.freedesktop.Application",
        member: "Activate",
        sig: &sig,
        body: &body,
    };
    let (reply_sig, _) = dbus_call_at(&addr, &call, deadline).unwrap();
    assert!(reply_sig.is_empty());
    let body = service.join().unwrap();
    // The platform data holds exactly one entry: activation-token.
    let mut reader = Reader::le(&body);
    let mut entries = reader.elements(8).unwrap();
    entries.enter_struct().unwrap();
    assert_eq!(entries.str().unwrap(), "activation-token");
    let value = entries
        .variant(|sig, reader| {
            assert_eq!(sig, "s");
            reader.str().map(str::to_string)
        })
        .unwrap();
    assert_eq!(value, "tok-abc");
    entries.leave_struct();
    assert!(entries.exhausted());
}
