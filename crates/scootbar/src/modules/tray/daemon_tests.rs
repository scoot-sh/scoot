//! The tray against a real `dbus-daemon`, with an item that is not the
//! fake bus's script but a second connection of this client, answering
//! `GetAll` and recording what the bar calls on it: the path from an
//! app's registration to an icon, a click and a crash, over the real
//! daemon's routing.

use std::time::{Duration, Instant};

use super::fake::{item_body, solid};
use super::{BusAddr, start_with};
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::proto::Writer;
use crate::dbus::testdaemon::Daemon;
use crate::modules::harness::Harness;
use crate::modules::{ModuleAction, OutputView, Update};

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// A tray item: an app's connection that owns `name`, answers its
/// property reads and remembers the calls it was sent.
struct Item {
    conn: Conn,
    name: String,
    props: Vec<u8>,
    calls: Vec<String>,
}

impl Item {
    fn new(daemon: &Daemon, name: &str, title: &str) -> Self {
        let mut conn = conn::connect(&daemon.path()).expect("the item connects");
        let mut body = Writer::new();
        body.str(name);
        body.u32(0);
        conn.call(
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
        let mut item = Self {
            conn,
            name: name.to_owned(),
            props: item_body(title, "Active", 4, 4, &solid(4, 4, 255, 30, 200, 30)),
            calls: Vec::new(),
        };
        // The name is owned once its reply is read.
        let start = Instant::now();
        while !item.serve_until_reply() {
            assert!(start.elapsed() < Duration::from_secs(10), "no name");
        }
        item
    }

    /// Registers with the bar's watcher, naming this item's service.
    fn register(&mut self) {
        let mut body = Writer::new();
        body.str(&self.name);
        self.conn
            .call(
                WATCHER,
                "/StatusNotifierWatcher",
                WATCHER,
                "RegisterStatusNotifierItem",
                "s",
                &body.take_body().unwrap(),
                0,
                2,
            )
            .unwrap();
    }

    /// Serves what arrived: answers `GetAll` (addressed to its sender,
    /// which the daemon requires), records any other call. Whether the
    /// `RequestName` reply was among it.
    fn serve_until_reply(&mut self) -> bool {
        let (events, _) = self.conn.pump();
        let mut replied = false;
        for event in events {
            match event {
                Event::Reply { token: 1, .. } => replied = true,
                Event::MethodCall {
                    sender,
                    member,
                    serial,
                    ..
                } => {
                    if member == "GetAll" {
                        let props = self.props.clone();
                        self.conn.reply_return(&sender, serial, "a{sv}", &props);
                    }
                    self.calls.push(member);
                }
                _ => {}
            }
        }
        replied
    }
}

/// Turns of the bar and of every item, until `done` holds or the test
/// fails: the real daemon is asynchronous, so this is polling, with a
/// ten second limit.
fn drive(harness: &mut Harness, items: &mut [&mut Item], mut done: impl FnMut(&Harness) -> bool) {
    let start = Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the bar and the daemon never got there"
        );
        for item in items.iter_mut() {
            item.serve_until_reply();
        }
        let _ = harness.wait(Duration::from_millis(20));
    }
}

fn shown(harness: &Harness) -> Vec<String> {
    harness
        .value_on(None)
        .and_then(|value| {
            value["items"].as_array().map(|items| {
                items
                    .iter()
                    .filter(|item| item["shown"].as_bool() == Some(true))
                    .filter_map(|item| item["title"].as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// An app registers, its icon appears, a click reaches it, and when it
/// dies without unregistering its icon goes with its owner.
#[test]
fn an_app_registers_is_clicked_and_crashes() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = Harness::new(start_with(BusAddr::Path(daemon.path())));
    // The bar owns the watcher name once its `RequestName` is answered.
    let mut app = Item::new(&daemon, "org.kde.StatusNotifierItem-4242-1", "Weather");
    drive(&mut harness, &mut [&mut app], |harness| {
        harness
            .value_on(None)
            .is_none_or(|value| value["watcher"] == "owner")
    });
    app.register();
    drive(&mut harness, &mut [&mut app], |harness| {
        shown(harness) == ["Weather"]
    });
    assert!(app.calls.iter().any(|call| call == "GetAll"));

    // A click reaches the app as an `Activate` call.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("activate", Some(0)), 1),
        Ok(Update::Unchanged)
    );
    let start = Instant::now();
    while !app.calls.iter().any(|call| call == "Activate") {
        assert!(start.elapsed() < Duration::from_secs(10), "no Activate");
        let _ = harness.wait(Duration::from_millis(20));
        app.serve_until_reply();
    }

    // The app dies without unregistering: the daemon says its name
    // lost its owner, and the icon is gone.
    drop(app);
    drive(&mut harness, &mut [], |harness| {
        harness.value_on(None).is_none()
    });
}

/// An item that registered before the bar started is found by listing
/// the bus, with no registration to the bar at all.
#[test]
fn an_item_listed_on_the_bus_before_the_bar_is_found() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut app = Item::new(&daemon, "org.kde.StatusNotifierItem-77-1", "Early");
    let mut harness = Harness::new(start_with(BusAddr::Path(daemon.path())));
    drive(&mut harness, &mut [&mut app], |harness| {
        shown(harness) == ["Early"]
    });
    let id = harness.value_on(None).unwrap()["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(id, "org.kde.StatusNotifierItem-77-1/StatusNotifierItem");
}
