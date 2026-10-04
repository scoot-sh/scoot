//! The power module against a real `dbus-daemon`, with a logind peer
//! that is a second connection of this client: the `Can*` answers hiding
//! rows, a suspend reaching logind with `interactive: true`, a polkit
//! refusal surfacing, and malformed answers keeping what was shown. The
//! private daemon stands in for the system bus: the module is started on
//! its path, as on `/run/dbus/system_bus_socket`.
//!
//! (`SCOOTBAR_REQUIRE_DBUS_DAEMON` makes a machine without a daemon fail,
//! not skip.)

use std::time::{Duration, Instant};

use super::session::{IFACE, NAME, PATH};
use super::{Settings, start_on};
use crate::action::ModuleAction;
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::proto::Writer;
use crate::dbus::testdaemon::Daemon;
use crate::modules::harness::Harness;
use crate::modules::{OutputView, Update};

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };

/// logind: a connection that owns `org.freedesktop.login1`, answers the
/// `Can*` calls from its words, records the actions it is sent, and
/// refuses them on demand.
struct Peer {
    conn: Conn,
    cans: [String; 3],
    can_error: Option<String>,
    malformed_can: bool,
    actions: Vec<(String, bool)>,
    action_error: Option<String>,
}

impl Peer {
    /// Connects and owns `org.freedesktop.login1`.
    fn logind(daemon: &Daemon) -> Self {
        let mut peer = Self {
            conn: conn::connect(&daemon.path()).expect("the peer connects"),
            cans: ["yes".to_owned(), "yes".to_owned(), "yes".to_owned()],
            can_error: None,
            malformed_can: false,
            actions: Vec::new(),
            action_error: None,
        };
        let mut body = Writer::new();
        body.str(NAME);
        body.u32(0);
        peer.conn
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
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < Duration::from_secs(10), "no name");
            let (events, _) = peer.conn.pump();
            if events
                .iter()
                .any(|e| matches!(e, Event::Reply { token: 1, .. }))
            {
                return peer;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Serves what arrived: `Can*` answered (addressed to their sender,
    /// which the daemon requires), actions recorded and acked.
    fn serve(&mut self) {
        let (events, _) = self.conn.pump();
        for event in events {
            let Event::MethodCall {
                sender,
                path,
                interface,
                member,
                serial,
                body,
                ..
            } = event
            else {
                continue;
            };
            if path != PATH || interface != IFACE {
                continue;
            }
            let can = match member.as_str() {
                "CanSuspend" => Some(0),
                "CanReboot" => Some(1),
                "CanPowerOff" => Some(2),
                _ => None,
            };
            if let Some(at) = can {
                if let Some(error) = &self.can_error {
                    self.conn.reply_error(&sender, serial, error);
                } else if self.malformed_can {
                    let mut wrong = Writer::new();
                    wrong.boolean(true);
                    self.conn
                        .reply_return(&sender, serial, "b", &wrong.take_body().unwrap());
                } else {
                    let mut answer = Writer::new();
                    answer.str(&self.cans[at].clone());
                    self.conn
                        .reply_return(&sender, serial, "s", &answer.take_body().unwrap());
                }
            } else if ["Suspend", "Reboot", "PowerOff"].contains(&member.as_str()) {
                let mut reader = crate::dbus::proto::Reader::le(&body);
                let interactive = reader.boolean().unwrap_or(false);
                self.actions.push((member.clone(), interactive));
                if let Some(error) = &self.action_error {
                    self.conn.reply_error(&sender, serial, error);
                } else {
                    self.conn.reply_return(&sender, serial, "", &[]);
                }
            }
        }
    }
}

/// Turns of the module and logind until both are quiet; whether the
/// module reported a change meanwhile.
fn settle(harness: &mut Harness, peer: &mut Peer) -> bool {
    let mut changed = false;
    let mut quiet = 0;
    for _ in 0..2000 {
        peer.serve();
        let woke = harness.wait(Duration::from_millis(5));
        if woke == Some(Update::Changed) {
            changed = true;
        }
        if woke.is_none() {
            quiet += 1;
            if quiet >= 8 {
                break;
            }
        } else {
            quiet = 0;
        }
    }
    changed
}

fn action(name: &'static str) -> ModuleAction {
    ModuleAction::new(name, None)
}

/// Settings with the lock and logout rows on commands (hermetic: no
/// session dependence), the rest on the peered logind.
fn settings() -> Settings {
    Settings {
        icon: Some(crate::icon::Icon::Glyph('⏻')),
        commands: [
            vec!["lockit".to_owned()],
            vec!["bye".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ],
        ..Settings::default()
    }
}

fn up(daemon: &Daemon, settings: &Settings) -> Harness {
    Harness::new(start_on(daemon.path(), settings))
}

/// The rows `query` reports, in order.
fn rows(harness: &Harness) -> Vec<String> {
    harness.value_on(None).expect("a value")["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row.as_str().expect("a name").to_owned())
        .collect()
}

/// Asks the `Can*` calls (what opening the popup does: the first invoke
/// asks too) and turns until the answers land. No change asserted: a
/// refused or malformed answer keeps what was shown by design.
fn ask(harness: &mut Harness, peer: &mut Peer) {
    assert_eq!(
        harness.invoke(&DP1, &action("suspend"), 1),
        Ok(Update::Changed)
    );
    settle(harness, peer);
    // The ask armed; disarm through the timeout rather than performing.
    assert!(harness.wait(Duration::from_secs(2)).is_some());
}

#[test]
fn can_answers_hide_no_and_na() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = up(&daemon, &settings());
    let mut logind = Peer::logind(&daemon);
    logind.cans = ["no".to_owned(), "yes".to_owned(), "na".to_owned()];
    ask(&mut harness, &mut logind);
    assert_eq!(rows(&harness), vec!["lock", "logout", "reboot"]);
}

#[test]
fn challenge_shows_the_row() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = up(&daemon, &settings());
    let mut logind = Peer::logind(&daemon);
    logind.cans = ["challenge".to_owned(), "na".to_owned(), "na".to_owned()];
    ask(&mut harness, &mut logind);
    assert_eq!(rows(&harness), vec!["lock", "logout", "suspend"]);
}

#[test]
fn suspend_calls_logind_with_interactive_true() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = up(&daemon, &settings());
    let mut logind = Peer::logind(&daemon);
    settle(&mut harness, &mut logind);
    // Arm, then perform: the call reaches logind.
    assert_eq!(
        harness.invoke(&DP1, &action("suspend"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.invoke(&DP1, &action("suspend"), 1),
        Ok(Update::Changed)
    );
    assert!(settle(&mut harness, &mut logind));
    assert_eq!(logind.actions, vec![("Suspend".to_owned(), true)]);
    // Answered cleanly: no error kept.
    let value = harness.value_on(None).expect("a value");
    assert!(value.get("error").is_none(), "{value}");
    assert!(value.get("armed").is_none(), "{value}");
}

#[test]
fn a_refused_call_surfaces() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = up(&daemon, &settings());
    let mut logind = Peer::logind(&daemon);
    logind.action_error = Some("org.freedesktop.DBus.Error.AccessDenied".to_owned());
    settle(&mut harness, &mut logind);
    assert_eq!(
        harness.invoke(&DP1, &action("reboot"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.invoke(&DP1, &action("reboot"), 1),
        Ok(Update::Changed)
    );
    assert!(settle(&mut harness, &mut logind));
    assert_eq!(logind.actions, vec![("Reboot".to_owned(), true)]);
    // Said on stderr when it arrived (heard in the test output), and kept
    // for the tooltip and `query`, never silent.
    let value = harness.value_on(None).expect("a value");
    let error = value["error"].as_str().expect("an error");
    assert!(error.contains("AccessDenied"), "{value}");
    assert!(harness.view().tooltip().contains("AccessDenied"));
}

#[test]
fn malformed_and_errored_can_answers_keep_the_row() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = up(&daemon, &settings());
    let mut logind = Peer::logind(&daemon);
    // A wrong shape first...
    logind.malformed_can = true;
    ask(&mut harness, &mut logind);
    assert!(rows(&harness).contains(&"suspend".to_owned()));
    // ...then an error, then a word logind never sends: shown throughout.
    logind.malformed_can = false;
    logind.can_error = Some("org.freedesktop.DBus.Error.UnknownMethod".to_owned());
    ask(&mut harness, &mut logind);
    assert!(rows(&harness).contains(&"suspend".to_owned()));
    logind.can_error = None;
    logind.cans = ["maybe".to_owned(), "yes".to_owned(), "yes".to_owned()];
    ask(&mut harness, &mut logind);
    assert!(rows(&harness).contains(&"suspend".to_owned()));
}
