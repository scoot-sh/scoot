//! The tray against a real `dbus-daemon`, with an item that is not the
//! fake bus's script but a second connection of this client, answering
//! `GetAll` and recording what the bar calls on it: the path from an
//! app's registration to an icon, a click and a crash, over the real
//! daemon's routing.

use std::time::{Duration, Instant};

use super::fake::{item_body, item_body_with, solid};
use super::{Settings, start_with};
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::link::Addr;
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
    /// Answers `GetAll` with a valid message past what the bar reads (a
    /// 512 by 512 pixmap), built by hand since `Writer` would refuse it.
    oversized: bool,
    /// Serves a DBusMenu object at `/Menu`: answers `GetLayout` with
    /// this, records `Event` ids and `AboutToShow`s.
    layout: Option<Vec<u8>>,
    events: Vec<i32>,
    abouts: usize,
}

impl Item {
    fn new(daemon: &Daemon, name: &str, title: &str) -> Self {
        Self::named(daemon, name, title, 0)
    }

    /// As [`Item::new`], asking for `name` with `flags` (the
    /// `RequestName` ones: 1 allows replacement, 2 replaces).
    fn named(daemon: &Daemon, name: &str, title: &str, flags: u32) -> Self {
        let mut conn = conn::connect(&daemon.path()).expect("the item connects");
        let mut body = Writer::new();
        body.str(name);
        body.u32(flags);
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
            oversized: false,
            layout: None,
            events: Vec::new(),
            abouts: 0,
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
    /// which the daemon requires), the menu object's `GetLayout` where
    /// one is scripted, and records `AboutToShow`, `Event` and any
    /// other call. Whether the `RequestName` reply was among it.
    fn serve_until_reply(&mut self) -> bool {
        let (events, _) = self.conn.pump();
        let mut replied = false;
        for event in events {
            match event {
                Event::Reply { token: 1, .. } => replied = true,
                Event::MethodCall {
                    sender,
                    path,
                    member,
                    serial,
                    body,
                    ..
                } => {
                    if member == "GetAll" && self.oversized {
                        let props = item_body_with(
                            Writer::with_cap(4 << 20),
                            "Big",
                            "Active",
                            512,
                            512,
                            &solid(512, 512, 255, 200, 30, 30),
                        );
                        let mut reply = Writer::with_cap(4 << 20);
                        reply.begin_return_to(99, &sender, serial, "a{sv}");
                        reply.raw(&props);
                        let bytes = reply.finish().unwrap();
                        assert!(bytes.len() > crate::dbus::proto::MAX_MESSAGE);
                        self.conn.queue_raw(&bytes);
                    } else if member == "GetAll" {
                        let props = self.props.clone();
                        self.conn.reply_return(&sender, serial, "a{sv}", &props);
                    } else if path == "/Menu" && member == "GetLayout" {
                        if let Some(layout) = self.layout.clone() {
                            self.conn
                                .reply_return(&sender, serial, "u(ia{sv}av)", &layout);
                        }
                    } else if path == "/Menu" && member == "AboutToShow" {
                        self.abouts += 1;
                    } else if path == "/Menu" && member == "Event" {
                        let mut reader = crate::dbus::proto::Reader::le(&body);
                        if let Ok(id) = reader.i32() {
                            self.events.push(id);
                        }
                    }
                    self.calls.push(member);
                }
                _ => {}
            }
        }
        replied
    }

    /// Emits `LayoutUpdated(revision)` on the menu object: what the bar
    /// re-reads while the menu is open.
    #[cfg(feature = "popup")]
    fn send_layout_updated(&mut self, revision: u32) {
        let mut body = Writer::new();
        body.u32(revision);
        let bytes = body.take_body().unwrap();
        self.conn.signal(
            "/Menu",
            "com.canonical.dbusmenu",
            "LayoutUpdated",
            "u",
            &bytes,
        );
    }
}

/// Turns of the bar and of every item, until `done` holds or the test
/// fails: the real daemon is asynchronous, so this is polling, with a
/// ten second limit.
fn drive(
    harness: &mut Harness,
    items: &mut [&mut Item],
    mut done: impl FnMut(&mut Harness) -> bool,
) {
    let start = Instant::now();
    while !done(&mut *harness) {
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
    let mut harness = Harness::new(start_with(Addr::Path(daemon.path()), Settings::default()));
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
    let mut harness = Harness::new(start_with(Addr::Path(daemon.path()), Settings::default()));
    drive(&mut harness, &mut [&mut app], |harness| {
        shown(harness) == ["Early"]
    });
    let id = harness.value_on(None).unwrap()["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(id, "org.kde.StatusNotifierItem-77-1/StatusNotifierItem");
}

/// Who owns the watcher name, asked of the daemon by `app`.
fn watcher_owner(harness: &mut Harness, app: &mut Item) -> String {
    app.conn
        .call(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "GetNameOwner",
            "s",
            &{
                let mut body = Writer::new();
                body.str(WATCHER);
                body.take_body().unwrap()
            },
            0,
            50,
        )
        .unwrap();
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(10), "no owner");
        let _ = harness.wait(Duration::from_millis(10));
        let (events, _) = app.conn.pump();
        for event in events {
            if let Event::Reply {
                token: 50, body, ..
            } = event
            {
                return crate::dbus::proto::read_owner("s", &body).unwrap();
            }
        }
    }
}

/// The scene the hostile-peer tests share: the bar on the daemon, one
/// ordinary item `Steady` shown, and who owns the watcher name.
fn scene(daemon: &Daemon) -> (Harness, Item, String) {
    let mut harness = Harness::new(start_with(Addr::Path(daemon.path()), Settings::default()));
    let mut steady = Item::new(daemon, "org.kde.StatusNotifierItem-1-1", "Steady");
    drive(&mut harness, &mut [&mut steady], |harness| {
        harness
            .value_on(None)
            .is_none_or(|value| value["watcher"] == "owner")
    });
    steady.register();
    drive(&mut harness, &mut [&mut steady], |harness| {
        shown(harness) == ["Steady"]
    });
    let owner = watcher_owner(&mut harness, &mut steady);
    (harness, steady, owner)
}

/// After a hostile peer has had its go: the bar is on the same
/// connection (the daemon says the same owner of the watcher name), the
/// steady item is still shown, and a newcomer registers and is shown.
fn still_alive(harness: &mut Harness, steady: &mut Item, daemon: &Daemon, owner: &str) {
    let mut late = Item::new(daemon, "org.kde.StatusNotifierItem-2-2", "Late");
    late.register();
    drive(harness, &mut [steady, &mut late], |harness| {
        let mut titles = shown(harness);
        titles.sort();
        titles == ["Late", "Steady"]
    });
    assert_eq!(watcher_owner(harness, steady), owner, "the bar redialled");
}

/// A valid answer past what the bar reads (a 512 by 512 pixmap, there
/// being no way for SNI to ask for a size) loses that item's update only:
/// the other item stays, and so does the connection.
#[test]
fn an_over_cap_answer_loses_only_that_items_update() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let (mut harness, mut steady, owner) = scene(&daemon);
    let mut big = Item::new(&daemon, "org.kde.StatusNotifierItem-9-9", "Big");
    big.oversized = true;
    big.register();
    // Its answer is dropped, so it never gets an icon, and so takes no
    // room; wait for the bar to have been sent it.
    let start = Instant::now();
    while !big.calls.iter().any(|call| call == "GetAll") {
        assert!(start.elapsed() < Duration::from_secs(10), "never asked");
        let _ = harness.wait(Duration::from_millis(10));
        steady.serve_until_reply();
        big.serve_until_reply();
    }
    for _ in 0..30 {
        let _ = harness.wait(Duration::from_millis(20));
        steady.serve_until_reply();
        big.serve_until_reply();
    }
    assert_eq!(shown(&harness), ["Steady"]);
    still_alive(&mut harness, &mut steady, &daemon, &owner);
}

/// A call of 1.2 MiB addressed to the bar (valid to the spec) is skipped.
#[test]
fn an_over_cap_call_to_the_bar_is_skipped() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let (mut harness, mut steady, owner) = scene(&daemon);
    let mut peer = Item::new(&daemon, "org.example.Peer", "Peer");
    let mut body = Writer::with_cap(4 << 20);
    body.str(&"x".repeat(1_300_000));
    let body = body.take_body().unwrap();
    let mut call = Writer::with_cap(4 << 20);
    call.begin_call(
        77,
        WATCHER,
        "/StatusNotifierWatcher",
        WATCHER,
        "RegisterStatusNotifierItem",
        "s",
        0,
    );
    call.raw(&body);
    peer.conn.queue_raw(&call.finish().unwrap());
    let start = Instant::now();
    while peer.conn.want_write() {
        assert!(start.elapsed() < Duration::from_secs(10), "never sent");
        let _ = harness.wait(Duration::from_millis(5));
        steady.serve_until_reply();
        peer.serve_until_reply();
    }
    still_alive(&mut harness, &mut steady, &daemon, &owner);
}

/// A raw peer flooding the signal the bar listens for from anyone
/// (`NewIcon` on the item interface, no sender in the match rule): worked
/// a wake at a time, and the connection lives.
#[test]
fn a_signal_flood_does_not_cost_the_connection() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let (mut harness, mut steady, owner) = scene(&daemon);
    let mut flooder = Item::new(&daemon, "org.example.Flooder", "Flooder");
    // All of it is sent before the bar reads a byte of it: the daemon
    // holds the flood and hands it over as fast as the bar takes it, so
    // staging fills far past what it is allowed to hold.
    let mut sent = 0;
    let start = Instant::now();
    while sent < 60_000 {
        assert!(start.elapsed() < Duration::from_secs(60), "{sent} signals");
        for _ in 0..500 {
            flooder.conn.signal(
                "/StatusNotifierItem",
                "org.kde.StatusNotifierItem",
                "NewIcon",
                "",
                &[],
            );
            sent += 1;
        }
        while flooder.conn.want_write() {
            let _ = flooder.conn.pump();
        }
    }
    still_alive(&mut harness, &mut steady, &daemon, &owner);
}

/// A peer's unicast `NameOwnerChanged` (the daemon never sends one for a
/// name its owner still holds) is not the bus speaking: it removes
/// nothing.
#[test]
fn a_forged_owner_change_removes_nothing() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let (mut harness, mut steady, owner) = scene(&daemon);
    let mut peer = Item::new(&daemon, "org.example.Forger", "Forger");
    let bar = owner.clone();
    for (name, old, new) in [
        ("org.kde.StatusNotifierItem-1-1", steady.conn.unique(), ""),
        (steady.conn.unique(), steady.conn.unique(), ""),
    ] {
        let mut body = Writer::new();
        body.str(name);
        body.str(old);
        body.str(new);
        let body = body.take_body().unwrap();
        let mut signal = Writer::new();
        signal.begin_signal_to(
            300,
            &bar,
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
            "sss",
        );
        signal.raw(&body);
        peer.conn.queue_raw(&signal.finish().unwrap());
    }
    let start = Instant::now();
    while peer.conn.want_write() || start.elapsed() < Duration::from_millis(500) {
        assert!(start.elapsed() < Duration::from_secs(10), "never sent");
        let _ = harness.wait(Duration::from_millis(5));
        steady.serve_until_reply();
        peer.serve_until_reply();
    }
    assert_eq!(shown(&harness), ["Steady"]);
    still_alive(&mut harness, &mut steady, &daemon, &owner);
}

/// A well-known name that changes hands: the item is read from its new
/// owner, and the old owner going away afterwards does not take the new
/// owner's item with it.
#[test]
fn a_name_that_changes_hands_follows_its_new_owner() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = Harness::new(start_with(Addr::Path(daemon.path()), Settings::default()));
    let name = "org.kde.StatusNotifierItem-5-1";
    // The first owner allows replacement (1), the second replaces (2).
    let mut first = Item::named(&daemon, name, "First", 1);
    drive(&mut harness, &mut [&mut first], |harness| {
        harness
            .value_on(None)
            .is_none_or(|value| value["watcher"] == "owner")
    });
    first.register();
    drive(&mut harness, &mut [&mut first], |harness| {
        shown(harness) == ["First"]
    });
    let mut second = Item::named(&daemon, name, "Second", 2);
    drive(&mut harness, &mut [&mut first, &mut second], |harness| {
        shown(harness) == ["Second"]
    });
    // The first connection dies: the name is the second's.
    drop(first);
    for _ in 0..40 {
        let _ = harness.wait(Duration::from_millis(20));
        second.serve_until_reply();
    }
    assert_eq!(shown(&harness), ["Second"]);
}

/// A bus whose policy refuses the bar the watcher name leaves a tray that
/// shows nothing and does not crash; the bar still hosts, so an item that
/// registered elsewhere shows once a watcher exists (not asserted here:
/// the fake bus covers the hosting) and the daemon test is that it lives.
#[test]
fn a_bus_that_denies_the_watcher_name_does_not_crash_the_tray() {
    let Some(daemon) = Daemon::spawn_with(
        "<deny own=\"org.kde.StatusNotifierWatcher\"/>\
         <deny own=\"org.freedesktop.StatusNotifierWatcher\"/>",
    ) else {
        return;
    };
    let mut harness = Harness::new(start_with(Addr::Path(daemon.path()), Settings::default()));
    let mut app = Item::new(&daemon, "org.kde.StatusNotifierItem-3-3", "Orphan");
    for _ in 0..40 {
        let _ = harness.wait(Duration::from_millis(20));
        app.serve_until_reply();
    }
    assert!(shown(&harness).is_empty());
}

/// The menu over the real daemon: an item's layout opens from `menu`,
/// a row click reaches it as `Event clicked`, and an update re-reads.
/// The item is this client's own connection answering with its own
/// marshalled bytes; the live jeepney round (an independent marshaller)
/// is the screenshots in the report, not this test.
#[cfg(feature = "popup")]
#[test]
fn a_menu_opens_clicks_and_updates_over_the_daemon() {
    use super::fake;
    use crate::popup::{Content, Kind};

    fn rows(harness: &mut Harness) -> Option<Vec<String>> {
        let mut content = Content::default();
        if !harness.popup(&mut content) {
            return None;
        }
        Some(
            content
                .widgets()
                .iter()
                .map(|widget| {
                    let label = content.label(widget).to_owned();
                    match widget.kind {
                        Kind::Text => format!("text:{label}"),
                        Kind::Button {
                            action,
                            arg,
                            closes,
                            ..
                        } => {
                            format!(
                                "button:{action}:{}:{}:{label}",
                                arg.unwrap_or(-1),
                                closes as u8
                            )
                        }
                        Kind::Slider { .. } => "slider".to_owned(),
                    }
                })
                .collect(),
        )
    }

    fn layout(revision: u32, first: &str) -> Vec<u8> {
        fake::layout_reply(revision, &|w| {
            fake::layout_node(w, 0, &|_| {}, &|w| {
                fake::layout_kid(w, &|w| {
                    fake::layout_node(
                        w,
                        10,
                        &|w| {
                            fake::layout_prop(w, "label", "s", &|w| w.str(first));
                        },
                        &|_| {},
                    );
                });
            });
        })
    }

    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = Harness::new(start_with(Addr::Path(daemon.path()), Settings::default()));
    let mut app = Item::new(&daemon, "org.kde.StatusNotifierItem-6-6", "Dished");
    app.layout = Some(layout(1, "Open"));
    drive(&mut harness, &mut [&mut app], |harness| {
        harness
            .value_on(None)
            .is_none_or(|value| value["watcher"] == "owner")
    });
    app.register();
    drive(&mut harness, &mut [&mut app], |harness| {
        shown(harness) == ["Dished"]
    });
    // The menu opens on the layout the item serves.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    let open = vec!["button:menu-select:10:1:Open".to_owned()];
    drive(&mut harness, &mut [&mut app], |harness| {
        rows(harness).as_ref() == Some(&open)
    });
    assert!(app.abouts >= 1);
    // A row click reaches the item as `Event clicked` and closes the
    // menu.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", Some(10)), 1),
        Ok(Update::Changed)
    );
    drive(&mut harness, &mut [&mut app], |harness| {
        rows(harness).is_none()
    });
    let start = Instant::now();
    while app.events.is_empty() {
        assert!(start.elapsed() < Duration::from_secs(10), "no Event");
        let _ = harness.wait(Duration::from_millis(20));
        app.serve_until_reply();
    }
    assert_eq!(app.events, [10]);
    // Reopen on a new revision, then an update announced while open
    // re-fills from the re-read.
    app.layout = Some(layout(3, "Quit"));
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    let quit = vec!["button:menu-select:10:1:Quit".to_owned()];
    drive(&mut harness, &mut [&mut app], |harness| {
        rows(harness).as_ref() == Some(&quit)
    });
    app.layout = Some(layout(4, "Shut"));
    app.send_layout_updated(4);
    let shut = vec!["button:menu-select:10:1:Shut".to_owned()];
    drive(&mut harness, &mut [&mut app], |harness| {
        rows(harness).as_ref() == Some(&shut)
    });
    assert!(shown(&harness) == ["Dished"]);
}
