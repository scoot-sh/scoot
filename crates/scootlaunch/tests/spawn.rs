//! Launching against the world: the detached fallback with no bus,
//! and a mouse pick re-activating a running single-instance stub over
//! `org.freedesktop.Application`.
//!
//! Each test starts its own headless scoot with a fixture applications
//! tree and drives the real `scootlaunch` binary (see `harness.rs`).
//! Skipped without a `scoot` binary beside the test binary;
//! `SCOOTLAUNCH_REQUIRE_SCOOT` makes that a failure. The bus legs skip
//! without `dbus-daemon` on `PATH`.

#[path = "harness.rs"]
mod harness;

use std::fs;
use std::time::Duration;

use harness::{Panel, rgb, session_with_apps, session_with_stubs, wait_marker};

/// With no session bus at all, a pick still launches: the detached
/// fallback, out of the launcher's session, with no scope.
#[test]
fn without_a_bus_a_pick_still_launches_detached() {
    let Some((session, data_home, markers)) = session_with_stubs("no-bus", &["alpha"]) else {
        return;
    };
    let Some(mut panel) = Panel::start_on(
        session,
        data_home,
        &[],
        &[(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent-scootlaunch-bus",
        )],
    ) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Return");
    let status = panel.wait_exit();
    assert!(
        status.success(),
        "the fallback launches and exits 0: {}",
        panel.stderr()
    );
    assert_eq!(panel.stdout(), "", "a launch prints nothing");
    let line = wait_marker(&markers[0]);
    assert!(line.contains("alpha.sh"), "the stub ran detached: {line}");
    panel.clean();
}

/// A mouse-picked `DBusActivatable` entry re-activates its running
/// instance: the fake service owning the name gets `Activate` with
/// the click's token, and the `Exec` stub never runs.
#[test]
fn a_click_reactivates_the_running_instance() {
    let Some(daemon) = scoot_dbus::testdaemon::Daemon::spawn() else {
        return;
    };
    // The running instance: owns `sh.scoot.Stub` before the panel
    // opens, and answers one `Activate`.
    let service = std::thread::spawn({
        let path = daemon.path();
        move || {
            use scoot_dbus::conn::{self, Event};
            use scoot_dbus::proto::{Writer, request};
            use scoot_dbus::testdaemon::until;
            let mut service = conn::connect(&path).expect("the daemon accepts the service");
            let mut body = Writer::new();
            body.str("sh.scoot.Stub");
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
                path,
                interface,
                ..
            } = until(&mut service, |event| {
                matches!(event, Event::MethodCall { .. })
            })
            else {
                unreachable!();
            };
            assert_eq!(member, "Activate");
            assert_eq!(interface, "org.freedesktop.Application");
            assert_eq!(path, "/sh/scoot/Stub");
            assert_eq!(signature, "a{sv}");
            service.reply_return(&sender, serial, "", &[]);
            let (events, _) = service.pump();
            drop(events);
            body
        }
    });
    // The fixture entry: activatable, with an `Exec` stub that must
    // not run while the instance owns its name.
    let bodies = [(
        "sh.scoot.Stub.desktop",
        "[Desktop Entry]\nType=Application\nName=Stub\nExec=/nonexistent-scootlaunch-stub\nDBusActivatable=true\n",
    )];
    let refs: Vec<(&str, &str)> = bodies.iter().map(|(file, body)| (*file, *body)).collect();
    let Some((session, data_home)) = session_with_apps("reactivate", &refs) else {
        return;
    };
    let address = format!("unix:path={}", daemon.path().display());
    let Some(mut panel) = Panel::start_on(
        session,
        data_home,
        &[],
        &[("DBUS_SESSION_BUS_ADDRESS", &address)],
    ) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    // The only row's middle, from the border box (the mouse path, with
    // the button serial for the token — the fuzzel failure shape).
    let (x0, y0, x1, _) = panel.find_panel(rgb("#f9e2af"));
    panel.click((x0 + x1) / 2, y0 + 35 + 10);
    let status = panel.wait_exit();
    assert!(status.success(), "activation exits 0: {}", panel.stderr());
    assert_eq!(panel.stdout(), "", "a launch prints nothing");
    let body = service.join().unwrap();
    // The platform data carries the click's token, and only it.
    let mut reader = scoot_dbus::proto::Reader::le(&body);
    let mut entries = reader.elements(8).unwrap();
    entries.enter_struct().unwrap();
    assert_eq!(entries.str().unwrap(), "activation-token");
    let token = entries
        .variant(|sig, reader| {
            assert_eq!(sig, "s");
            reader.str().map(str::to_string)
        })
        .unwrap();
    assert!(!token.is_empty(), "the click minted a token");
    entries.leave_struct();
    assert!(entries.exhausted());
    panel.clean();
}

/// A `DBusActivatable` entry with no owner falls back to `Exec`: the
/// service is gone, so the stub runs instead.
#[test]
fn without_an_owner_an_activatable_entry_falls_back_to_exec() {
    let Some(daemon) = scoot_dbus::testdaemon::Daemon::spawn() else {
        return;
    };
    let scratch = daemon.path().parent().unwrap().to_path_buf();
    let script = scratch.join("fallback.sh");
    let marker = scratch.join("fallback.launched");
    fs::write(
        &script,
        format!("#!/bin/sh\necho ran >> '{}'\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let bodies = [(
        "sh.scoot.Gone.desktop",
        format!(
            "[Desktop Entry]\nType=Application\nName=Gone\nExec=\"{}\"\nDBusActivatable=true\n",
            script.display()
        ),
    )];
    let refs: Vec<(&str, &str)> = bodies
        .iter()
        .map(|(file, body)| (*file, body.as_str()))
        .collect();
    let Some((session, data_home)) = session_with_apps("activate-fallback", &refs) else {
        return;
    };
    let address = format!("unix:path={}", daemon.path().display());
    let Some(mut panel) = Panel::start_on(
        session,
        data_home,
        &[],
        &[("DBUS_SESSION_BUS_ADDRESS", &address)],
    ) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Return");
    let status = panel.wait_exit();
    assert!(
        status.success(),
        "the fallback launches and exits 0: {}",
        panel.stderr()
    );
    let ran = wait_marker(&marker);
    assert!(ran.contains("ran"), "the Exec stub ran: {ran}");
    panel.clean();
}
