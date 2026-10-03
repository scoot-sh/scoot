//! The bluetooth module against a real `dbus-daemon`, with a BlueZ that
//! is a second connection of this client: the path from the name
//! appearing to a device on the bar, a power set reaching BlueZ, a
//! restart, a crash, and what the daemon's own match filtering keeps from
//! waking the bar at all. The private daemon stands in for the system
//! bus: the module is started on its path, as on `/run/dbus/system_bus_socket`.
//!
//! (`SCOOTBAR_REQUIRE_DBUS_DAEMON` makes a machine without a daemon fail,
//! not skip.)

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{Settings, start_with};
use crate::dbus::bluez::build::{self, entry};
use crate::dbus::bluez::{ADAPTER, DEVICE, MANAGER, NAME, PROPERTIES, ROOT};
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::link::Addr;
use crate::dbus::proto::Writer;
use crate::dbus::testdaemon::Daemon;
use crate::modules::harness::Harness;
use crate::modules::{ModuleAction, OutputView, Update};

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };
const HCI0: &str = "/org/bluez/hci0";
const DEV: &str = "/org/bluez/hci0/dev_11_22_33_44_55_66";

/// BlueZ: a connection that owns `org.bluez`, answers `GetManagedObjects`
/// and `GetAll` from its state, records the power sets it is sent, and
/// emits signals on demand.
struct Peer {
    conn: Conn,
    managed: Vec<u8>,
    getalls: HashMap<(String, String), Vec<u8>>,
    sets: Vec<(String, bool)>,
    getmanageds: usize,
}

impl Peer {
    /// Connects and owns `org.bluez`, serving `managed` as its set.
    fn bluez(daemon: &Daemon, managed: Vec<u8>) -> Self {
        let mut peer = Self {
            conn: conn::connect(&daemon.path()).expect("the peer connects"),
            managed,
            getalls: HashMap::new(),
            sets: Vec::new(),
            getmanageds: 0,
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

    /// Serves what arrived: enumeration and reads answered (addressed to
    /// their sender, which the daemon requires), power sets recorded.
    fn serve(&mut self) {
        let (events, _) = self.conn.pump();
        for event in events {
            let Event::MethodCall {
                sender,
                path,
                interface,
                member,
                serial,
                signature,
                body,
            } = event
            else {
                continue;
            };
            if interface == MANAGER && member == "GetManagedObjects" {
                self.getmanageds += 1;
                let managed = self.managed.clone();
                self.conn
                    .reply_return(&sender, serial, "a{oa{sa{sv}}}", &managed);
            } else if interface == PROPERTIES && member == "GetAll" {
                let mut reader = crate::dbus::proto::Reader::le(&body);
                let iface = reader.str().unwrap_or("").to_owned();
                if let Some(props) = self.getalls.get(&(path.clone(), iface)).cloned() {
                    self.conn.reply_return(&sender, serial, "a{sv}", &props);
                } else {
                    self.conn.reply_error(
                        &sender,
                        serial,
                        "org.freedesktop.DBus.Error.UnknownMethod",
                    );
                }
                let _ = signature;
            } else if interface == PROPERTIES && member == "Set" {
                let mut reader = crate::dbus::proto::Reader::le(&body);
                if let (Ok(iface), Ok(prop)) = (reader.str(), reader.str()) {
                    if iface == ADAPTER && prop == "Powered" {
                        if let Ok(powered) = reader.variant(|sig, reader| {
                            if sig == "b" {
                                reader.boolean()
                            } else {
                                Err(())
                            }
                        }) {
                            self.sets.push((path, powered));
                        }
                    }
                }
            }
        }
    }

    /// Emits an object-manager signal from the manager's path, as BlueZ
    /// does.
    fn manager(&mut self, member: &str, sig: &str, body: &[u8]) {
        self.conn.signal(ROOT, MANAGER, member, sig, body);
        let _ = self.conn.pump();
    }

    /// Emits a `PropertiesChanged` on `path`, as BlueZ does.
    fn changed(&mut self, path: &str, body: &[u8]) {
        self.conn
            .signal(path, PROPERTIES, "PropertiesChanged", "sa{sv}as", body);
        let _ = self.conn.pump();
    }
}

/// Turns of the module and BlueZ until both are quiet; whether the
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

fn shown(harness: &Harness) -> String {
    harness.view_on(Some("DP-1")).text().to_owned()
}

fn up(daemon: &Daemon) -> Harness {
    Harness::new(start_with(&Settings::default(), Addr::Path(daemon.path())))
}

#[test]
fn a_real_daemon_carries_the_whole_conversation() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = up(&daemon);
    let mut bluez = Peer::bluez(&daemon, build::small_world());
    assert!(settle_for(&mut harness, &mut bluez, 150));
    assert_eq!(shown(&harness), "Headset 72%");

    // An external power toggle arrives as a signal.
    bluez.changed(
        HCI0,
        &build::changed(
            ADAPTER,
            &|p| entry(p, "Powered", "b", &|w| w.boolean(false)),
            &[],
        ),
    );
    assert!(settle_for(&mut harness, &mut bluez, 150));
    assert_eq!(shown(&harness), "off");

    // The click's toggle reaches BlueZ as a `Set`.
    assert_eq!(
        harness.invoke(&DP1, &action("toggle"), 1),
        Ok(Update::Unchanged)
    );
    // No view change comes of it locally; turn so the peer is served.
    settle(&mut harness, &mut bluez);
    assert_eq!(bluez.sets, vec![(HCI0.to_owned(), true)]);

    // BlueZ crashing (its connection dying without goodbye) empties the
    // module: the bus releases its names itself.
    drop(bluez);
    assert!(settle_no_peer(&mut harness));
    assert_eq!(shown(&harness), "");
}

/// As [`settle`], then keeps turning for `ms` more: for what a timer
/// delivers.
fn settle_for(harness: &mut Harness, peer: &mut Peer, ms: u64) -> bool {
    let mut changed = settle(harness, peer);
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        peer.serve();
        if harness.wait(Duration::from_millis(5)) == Some(Update::Changed) {
            changed = true;
        }
    }
    changed
}

/// Turns of the module with no peer left to serve.
fn settle_no_peer(harness: &mut Harness) -> bool {
    let mut changed = false;
    for _ in 0..2000 {
        match harness.wait(Duration::from_millis(5)) {
            Some(Update::Changed) => {
                changed = true;
            }
            Some(Update::Unchanged) => {}
            None => break,
        }
    }
    changed
}

#[test]
fn a_restart_reenumerates_from_the_new_owner() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = up(&daemon);
    let mut bluez = Peer::bluez(&daemon, build::small_world());
    assert!(settle_for(&mut harness, &mut bluez, 150));
    assert_eq!(shown(&harness), "Headset 72%");
    drop(bluez);
    assert!(settle_no_peer(&mut harness));
    assert_eq!(shown(&harness), "");
    // Another process takes the name: the set is read again, and the new
    // state (nothing connected) is what shows.
    let mut bluez = Peer::bluez(
        &daemon,
        build::managed(&|body| {
            build::adapter_object(body, HCI0, true);
            build::device_object(body, DEV, false, Some("Headset"), None, None);
        }),
    );
    assert!(settle_for(&mut harness, &mut bluez, 150));
    assert_eq!(shown(&harness), "on");
    assert!(bluez.getmanageds >= 1);
}

#[test]
fn a_strangers_signal_is_not_believed() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = up(&daemon);
    let mut bluez = Peer::bluez(
        &daemon,
        build::managed(&|body| {
            build::adapter_object(body, HCI0, false);
        }),
    );
    assert!(settle_for(&mut harness, &mut bluez, 150));
    assert_eq!(shown(&harness), "off");
    // A peer that owns nothing says the adapter is on: the daemon routes
    // it (the path matches the rule), and the module must not believe
    // it, since it is not from the tracked owner.
    let mut stranger = conn::connect(&daemon.path()).expect("the stranger connects");
    let mut body = Writer::new();
    body.str(ADAPTER);
    if let Some(cookie) = body.open_array(8) {
        entry(&mut body, "Powered", "b", &|w| w.boolean(true));
        body.close_array(cookie);
    }
    if let Some(cookie) = body.open_array(4) {
        body.close_array(cookie);
    }
    let body = body.take_body().unwrap();
    stranger.signal(HCI0, PROPERTIES, "PropertiesChanged", "sa{sv}as", &body);
    let _ = stranger.pump();
    assert!(!settle(&mut harness, &mut bluez));
    assert_eq!(shown(&harness), "off");
    // BlueZ itself saying the same is believed.
    bluez.changed(
        HCI0,
        &build::changed(
            ADAPTER,
            &|p| entry(p, "Powered", "b", &|w| w.boolean(true)),
            &[],
        ),
    );
    assert!(settle_for(&mut harness, &mut bluez, 150));
    assert_eq!(shown(&harness), "on");
}

#[test]
fn hotplug_arrives_as_manager_signals() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = up(&daemon);
    let mut bluez = Peer::bluez(&daemon, build::managed(&|_| {}));
    assert!(!settle(&mut harness, &mut bluez));
    assert_eq!(shown(&harness), "");
    bluez.manager(
        "InterfacesAdded",
        "oa{sa{sv}}",
        &build::added(HCI0, &|b| {
            build::iface(b, ADAPTER, &|p| build::adapter_props(p, Some(true)));
        }),
    );
    assert!(settle_for(&mut harness, &mut bluez, 150));
    assert_eq!(shown(&harness), "on");
    bluez.manager(
        "InterfacesAdded",
        "oa{sa{sv}}",
        &build::added(DEV, &|b| {
            build::iface(b, DEVICE, &|p| {
                build::device_props(p, Some(true), Some("Buds"), None)
            });
        }),
    );
    assert!(settle_for(&mut harness, &mut bluez, 150));
    assert_eq!(shown(&harness), "Buds");
    bluez.manager("InterfacesRemoved", "oas", &build::removed(DEV, &[DEVICE]));
    assert!(settle_for(&mut harness, &mut bluez, 150));
    assert_eq!(shown(&harness), "on");
}
