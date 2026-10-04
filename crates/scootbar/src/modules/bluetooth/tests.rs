//! The bluetooth module against the scripted bus: what it shows, what
//! its click sends, what it does with a hostile or broken BlueZ, and that
//! it asks the bus for nothing it does not need. The real daemon's half
//! is `daemon_tests.rs`.

use std::time::{Duration, Instant};

use super::fake::{BLUEZ, Fake};
use super::{Settings, start_with};
use crate::action::ModuleAction;
use crate::dbus::bluez::build::{self, entry};
use crate::dbus::bluez::{ADAPTER, BATTERY, DEVICE};
use crate::dbus::link::Addr;
use crate::modules::harness::Harness;
use crate::modules::{Class, InvokeError, OutputView, Update};

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };
const HCI0: &str = "/org/bluez/hci0";
const DEV: &str = "/org/bluez/hci0/dev_11_22_33_44_55_66";

fn up_with(settings: &Settings) -> (Harness, Fake) {
    let (stream, fake) = Fake::pair();
    let harness = Harness::new(start_with(settings, Addr::Stream(stream)));
    (harness, fake)
}

fn up() -> (Harness, Fake) {
    up_with(&Settings::default())
}

/// Turns of the module and the bus until both are quiet for a few polls;
/// whether the module reported a change meanwhile.
fn settle(harness: &mut Harness, fake: &mut Fake) -> bool {
    let mut changed = false;
    let mut quiet = 0;
    for _ in 0..400 {
        let served = fake.pump();
        let woke = harness.wait(Duration::from_millis(5));
        if woke == Some(Update::Changed) {
            changed = true;
        }
        if served == 0 && woke.is_none() && !fake.pending() {
            quiet += 1;
            if quiet >= 4 {
                break;
            }
        } else {
            quiet = 0;
        }
    }
    changed
}

/// As [`settle`], then keeps turning for `ms` more: for what a timer
/// delivers.
fn settle_for(harness: &mut Harness, fake: &mut Fake, ms: u64) -> bool {
    let mut changed = settle(harness, fake);
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        fake.pump();
        if harness.wait(Duration::from_millis(5)) == Some(Update::Changed) {
            changed = true;
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

/// BlueZ owns the name but serves no objects: an adapter-less world.
fn empty_world(fake: &mut Fake) {
    fake.managed = build::managed(&|_| {});
}

#[test]
fn with_no_bluez_nothing_shows_and_nothing_is_enumerated() {
    let (mut harness, mut fake) = up();
    fake.owner = None;
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(shown(&harness), "");
    assert_eq!(fake.managed_calls, 0);
    // The four match rules went out, and nothing else was asked.
    assert_eq!(fake.matches.len(), 4);
    assert!(
        fake.matches
            .iter()
            .any(|rule| rule.contains("NameOwnerChanged"))
    );
    assert!(
        fake.matches
            .iter()
            .any(|rule| rule.contains("InterfacesAdded"))
    );
    assert!(
        fake.matches
            .iter()
            .any(|rule| rule.contains("InterfacesRemoved"))
    );
    assert!(
        fake.matches
            .iter()
            .any(|rule| rule.contains("PropertiesChanged"))
    );
    assert!(
        fake.matches
            .iter()
            .filter(|rule| !rule.contains("NameOwnerChanged"))
            .all(|rule| rule.contains("path_namespace='/org/bluez'"))
    );
}

#[test]
fn an_adapter_off_shows_off() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, false);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "off");
    assert_eq!(harness.view().class(), Class::Muted);
}

#[test]
fn an_adapter_on_with_nothing_connected_shows_on() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, false, Some("Headset"), Some("Headset"), None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "on");
    assert_eq!(harness.view().class(), Class::Normal);
}

#[test]
fn a_connected_device_shows_its_name_and_charge() {
    let (mut harness, mut fake) = up();
    fake.managed = build::small_world();
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset 72%");
    let value = harness
        .value_on(Some("DP-1"))
        .expect("a value while connected");
    assert_eq!(value["state"], "connected");
    assert_eq!(value["device"], "Headset");
    assert_eq!(value["battery"], 72);
    assert_eq!(value["connected"], 1);
}

#[test]
fn a_device_without_a_name_shows_its_alias_then_its_path() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, true, None, Some("Alias"), None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Alias");
    // Neither: the path's last element.
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, true, None, None, None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "dev_11_22_33_44_55_66");
}

#[test]
fn a_device_without_battery_shows_no_charge() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, true, Some("Headset"), None, None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset");
    let value = harness
        .value_on(Some("DP-1"))
        .expect("a value while connected");
    assert!(value.get("battery").is_none());
}

#[test]
fn long_names_are_cut_and_controls_stripped() {
    let (mut harness, mut fake) = up();
    let long = "x".repeat(300);
    let dirty = format!("ok\x07{long}");
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, true, Some(&dirty), None, None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    let text = shown(&harness);
    assert!(text.starts_with("okxxx"));
    assert!(!text.contains('\x07'));
    assert!(text.len() <= super::session::MAX_FIELD + " 72%".len());
}

#[test]
fn a_name_that_is_not_utf8_refuses_the_answer_and_keeps_the_last() {
    let (mut harness, mut fake) = up();
    fake.managed = build::small_world();
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset 72%");
    // Corrupt one byte of the name: the whole set is refused, and what
    // was shown stays.
    let mut broken = build::small_world();
    let at = broken
        .windows(b"Headset".len())
        .position(|w| w == b"Headset")
        .expect("the name is in the body");
    broken[at] = 0xff;
    fake.managed = broken;
    fake.properties_changed(
        BLUEZ,
        "/org/bluez/hci0/dev_00",
        &build::changed(DEVICE, &|_| {}, &[]),
    );
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(shown(&harness), "Headset 72%");
}

#[test]
fn adapter_power_dominates_a_connected_device() {
    // BlueZ disconnects on power-off, but the signals can order either
    // way: while the adapter is off the module shows off, not the stale
    // connection.
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, false);
        build::device_object(body, DEV, true, Some("Headset"), None, Some(72));
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "off");
    assert_eq!(harness.view().class(), Class::Muted);
    let value = harness
        .value_on(Some("DP-1"))
        .expect("a value with an adapter");
    assert_eq!(value["state"], "off");
}

#[test]
fn an_external_power_toggle_shows() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, false);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "off");
    fake.properties_changed(
        BLUEZ,
        HCI0,
        &build::changed(
            ADAPTER,
            &|p| entry(p, "Powered", "b", &|w| w.boolean(true)),
            &[],
        ),
    );
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "on");
}

#[test]
fn a_connect_and_disconnect_show() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, false, Some("Headset"), None, None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "on");
    fake.properties_changed(
        BLUEZ,
        DEV,
        &build::changed(
            DEVICE,
            &|p| entry(p, "Connected", "b", &|w| w.boolean(true)),
            &[],
        ),
    );
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset");
    fake.properties_changed(
        BLUEZ,
        DEV,
        &build::changed(
            DEVICE,
            &|p| entry(p, "Connected", "b", &|w| w.boolean(false)),
            &[],
        ),
    );
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "on");
}

#[test]
fn a_disconnect_storm_draws_ten_times_a_second_at_most() {
    use rustix::event::PollFlags;
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, false, Some("Headset"), None, None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    // Four hundred flips, each flushed to the socket and handed to the
    // module without blocking: the whole storm takes milliseconds, far
    // inside one 100 ms draw gap, so only the first change draws at
    // once and the rest wait for the one held timer.
    let mut draws = 0;
    for at in 0..400 {
        let connected = at % 2 == 0;
        fake.properties_changed(
            BLUEZ,
            DEV,
            &build::changed(
                DEVICE,
                &|p| entry(p, "Connected", "b", &|w| w.boolean(connected)),
                &[],
            ),
        );
        fake.pump();
        if harness.deliver(0, PollFlags::IN) == Update::Changed {
            draws += 1;
        }
    }
    assert!(draws <= 2, "{draws} draws for 400 flips");
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "on");
}

#[test]
fn a_hotplugged_adapter_and_device_show() {
    let (mut harness, mut fake) = up();
    empty_world(&mut fake);
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(shown(&harness), "");
    fake.interfaces_added(
        BLUEZ,
        &build::added(HCI0, &|b| {
            build::iface(b, ADAPTER, &|p| build::adapter_props(p, Some(true)));
        }),
    );
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "on");
    fake.interfaces_added(
        BLUEZ,
        &build::added(DEV, &|b| {
            build::iface(b, DEVICE, &|p| {
                build::device_props(p, Some(true), Some("Dongle"), None)
            });
        }),
    );
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Dongle");
    // The adapter unplugged (rfkill, a pulled dongle): back to nothing,
    // and the device with it.
    fake.interfaces_removed(BLUEZ, &build::removed(HCI0, &[ADAPTER]));
    fake.interfaces_removed(BLUEZ, &build::removed(DEV, &[DEVICE]));
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "");
}

#[test]
fn a_battery_appearing_and_vanishing_shows() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, true, Some("Headset"), None, None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset");
    fake.properties_changed(
        BLUEZ,
        DEV,
        &build::changed(
            BATTERY,
            &|p| entry(p, "Percentage", "y", &|w| w.u8(41)),
            &[],
        ),
    );
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset 41%");
    fake.interfaces_removed(BLUEZ, &build::removed(DEV, &[BATTERY]));
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset");
}

#[test]
fn an_invalidated_property_is_read_again() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, false, Some("Headset"), None, None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "on");
    // The signal says the connection changed without saying to what;
    // the re-read answers connected.
    fake.objects.push(super::fake::ObjectAnswer {
        path: DEV,
        adapter: None,
        device: Some(build::get_all(&|p| {
            build::device_props(p, Some(true), Some("Headset"), None)
        })),
        battery: None,
    });
    fake.properties_changed(BLUEZ, DEV, &build::changed(DEVICE, &|_| {}, &["Connected"]));
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset");
}

#[test]
fn a_signal_for_an_unknown_path_rereads_the_set_once() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    let calls = fake.managed_calls;
    // A change that raced the enumeration: read again, but only once no
    // matter how many such signals arrive.
    for _ in 0..3 {
        fake.properties_changed(
            BLUEZ,
            "/org/bluez/hci0/dev_99",
            &build::changed(
                DEVICE,
                &|p| entry(p, "Connected", "b", &|w| w.boolean(true)),
                &[],
            ),
        );
    }
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(fake.managed_calls, calls + 1);
    assert_eq!(shown(&harness), "on");
}

#[test]
fn forged_signals_change_nothing() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, false);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "off");
    // From a peer, not the owner: not believed.
    fake.properties_changed(
        ":1.forger",
        HCI0,
        &build::changed(
            ADAPTER,
            &|p| entry(p, "Powered", "b", &|w| w.boolean(true)),
            &[],
        ),
    );
    fake.interfaces_added(
        ":1.forger",
        &build::added(DEV, &|b| {
            build::iface(b, DEVICE, &|p| {
                build::device_props(p, Some(true), Some("Forged"), None)
            });
        }),
    );
    // A forged owner change, and one for another name: not believed.
    fake.name_owner_changed_from(":1.forger", "org.bluez", BLUEZ, ":1.other");
    fake.name_owner_changed_from(
        "org.freedesktop.DBus",
        "org.bluez.GattService1",
        "",
        ":1.other",
    );
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(shown(&harness), "off");
}

#[test]
fn a_bluez_restart_reenumerates_and_a_leave_empties() {
    let (mut harness, mut fake) = up();
    fake.managed = build::small_world();
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset 72%");
    // BlueZ leaves: nothing shows.
    fake.owner = None;
    fake.name_owner_changed(BLUEZ, "");
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "");
    assert!(harness.value_on(Some("DP-1")).is_none());
    // BlueZ comes back with nothing connected: the set is read again.
    let calls = fake.managed_calls;
    fake.owner = Some(BLUEZ.to_owned());
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
    });
    fake.name_owner_changed("", BLUEZ);
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "on");
    assert_eq!(fake.managed_calls, calls + 1);
}

#[test]
fn a_dead_bus_empties_and_waits() {
    let (mut harness, mut fake) = up();
    fake.managed = build::small_world();
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset 72%");
    fake.hang_up();
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "");
}

#[test]
fn a_click_toggles_the_first_adapters_power() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::adapter_object(body, "/org/bluez/hci1", true);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "on");
    assert_eq!(
        harness.invoke(&DP1, &action("toggle"), 1),
        Ok(Update::Unchanged)
    );
    // The `Set` wants no reply and changes nothing locally: BlueZ
    // answers with a signal, which the fake never sends.
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(
        fake.take_sets(),
        vec![super::fake::SetPowered {
            path: HCI0.to_owned(),
            powered: false,
        }]
    );
    // With no adapter the toggle is refused, naming why.
    let (mut harness, mut fake) = up();
    empty_world(&mut fake);
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(
        harness.invoke(&DP1, &action("toggle"), 1),
        Err(InvokeError::Refused("no adapter"))
    );
}

#[test]
fn the_menu_needs_a_command_and_devices() {
    let (mut harness, mut fake) = up();
    empty_world(&mut fake);
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(
        harness.invoke(&DP1, &action("menu"), 1),
        Err(InvokeError::Refused("no menu command configured"))
    );
    let (mut harness, mut fake) = up_with(&Settings {
        menu_command: ["true".to_owned()].into(),
        ..Settings::default()
    });
    empty_world(&mut fake);
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(
        harness.invoke(&DP1, &action("menu"), 1),
        Err(InvokeError::Refused("no adapter"))
    );
    let (mut harness, mut fake) = up_with(&Settings {
        menu_command: ["true".to_owned()].into(),
        ..Settings::default()
    });
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(
        harness.invoke(&DP1, &action("menu"), 1),
        Err(InvokeError::Refused("no devices seen yet"))
    );
}

#[test]
fn the_menu_lists_the_devices() {
    let (mut harness, mut fake) = up_with(&Settings {
        menu_command: ["true".to_owned()].into(),
        ..Settings::default()
    });
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, true, Some("Headset"), None, None);
        build::device_object(
            body,
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF",
            false,
            Some("Keyboard"),
            None,
            None,
        );
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    // `true` exits at once; the spawn is what is checked (the network
    // module's tests read the list off the pipe: here the command owns
    // the choice, and the reap is what must not linger).
    assert_eq!(
        harness.invoke(&DP1, &action("menu"), 1),
        Ok(Update::Unchanged)
    );
    // The picker runs and is reaped; the view never moves for it.
    settle_for(&mut harness, &mut fake, 200);
    assert_eq!(shown(&harness), "Headset");
}

#[test]
fn two_adapters_show_the_first_connected_device() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, false);
        build::adapter_object(body, "/org/bluez/hci1", true);
        build::device_object(
            body,
            "/org/bluez/hci1/dev_AA",
            true,
            Some("Second"),
            None,
            None,
        );
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Second");
    let value = harness
        .value_on(Some("DP-1"))
        .expect("a value with adapters");
    assert_eq!(value["adapters"], 2);
    assert_eq!(value["state"], "connected");
}

#[test]
fn an_oversize_enumeration_keeps_the_last_state() {
    let (mut harness, mut fake) = up();
    fake.managed = build::small_world();
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Headset 72%");
    // A world past the 1 MiB the client reads: the reply is skipped
    // whole, and what was shown stays.
    let big = build::huge(&|body| {
        build::adapter_object(body, HCI0, true);
        for n in 0..20000 {
            build::device_object(
                body,
                &format!("/org/bluez/hci0/dev_{n:06X}"),
                false,
                Some(&format!("Device {n} with a longish name to fill bytes")),
                None,
                Some(50),
            );
        }
    });
    assert!(
        big.len() > 1024 * 1024,
        "the world is oversize: {}",
        big.len()
    );
    fake.managed = big;
    // Force the re-read through an unknown path's signal.
    fake.properties_changed(
        BLUEZ,
        "/org/bluez/hci0/dev_FFFFFF",
        &build::changed(DEVICE, &|_| {}, &[]),
    );
    let calls = fake.managed_calls;
    // The re-read waits out the coalesce timer: one read, however many
    // signals arrived.
    assert!(!settle_for(&mut harness, &mut fake, 150));
    assert!(fake.managed_calls > calls);
    assert_eq!(shown(&harness), "Headset 72%");
    // And the next signal reads again (stale is not stuck asking at
    // once for the same answer, nor stuck silent).
    fake.properties_changed(
        BLUEZ,
        "/org/bluez/hci0/dev_FFFFFE",
        &build::changed(DEVICE, &|_| {}, &[]),
    );
    let calls = fake.managed_calls;
    assert!(!settle_for(&mut harness, &mut fake, 150));
    assert!(fake.managed_calls > calls);
    assert_eq!(shown(&harness), "Headset 72%");
}

/// A skipped reply no call can be named for releases every in-flight
/// read: without this a skipped `GetManagedObjects` leaves the
/// enumeration owed forever with nothing stale, and the set is never read
/// again (and a skipped object read never retries).
#[test]
fn an_unknown_drop_releases_every_in_flight_read() {
    use super::session::{Adapter, RefreshIface, start};
    use crate::dbus::bluez::NAME;
    use crate::dbus::conn::{self, Event};
    use crate::dbus::proto::Writer;
    use crate::dbus::testdaemon;
    let (client, mut daemon_end) = std::os::unix::net::UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        testdaemon::serve_setup(&mut daemon_end);
        daemon_end
    });
    let mut live = start(conn::setup(client).unwrap());
    let _daemon = server.join().unwrap();
    // BlueZ appears: the enumeration goes out and stays owed (the
    // scripted peer never answers; only the flights' state matters here).
    let mut body = Writer::new();
    body.str(NAME);
    body.str("");
    body.str(":1.bluez");
    live.apply(Event::Signal {
        sender: conn::BUS_NAME.to_owned(),
        path: conn::BUS_PATH.to_owned(),
        interface: conn::BUS_INTERFACE.to_owned(),
        member: "NameOwnerChanged".to_owned(),
        signature: "sss".to_owned(),
        body: body.take_body().unwrap(),
    });
    assert!(live.managed_in_flight);
    // One adapter, mid-read.
    live.adapters.push(Adapter {
        id: 7,
        path: HCI0.to_owned(),
        powered: true,
        asked: None,
        last_asked: None,
        stale: false,
    });
    live.refresh(7, RefreshIface::Adapter);
    assert!(live.adapters[0].asked.is_some());
    // The bus skips some reply past what is read: no call can be named.
    live.apply(Event::Dropped {
        token: conn::DROPPED_UNKNOWN,
    });
    assert!(!live.managed_in_flight);
    assert!(live.stale_managed);
    assert!(live.adapters[0].asked.is_none());
    assert!(live.adapters[0].stale);
}

#[test]
fn icons_follow_the_state_with_a_static_fallback() {
    use crate::icon::Icon;
    let settings = Settings {
        icon: Some(Icon::Glyph('S')),
        icon_off: Some(Icon::Glyph('0')),
        icon_on: Some(Icon::Glyph('1')),
        // No per-state connected glyph: the static one shows there.
        ..Settings::default()
    };
    let (mut harness, mut fake) = up_with(&settings);
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, false);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    let view = harness.view();
    assert_eq!(view.text(), "off");
    assert_eq!(view.icon(), Some('0'));
    // Power on with nothing connected: its own glyph.
    let (mut harness, mut fake) = up_with(&settings);
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, true);
        build::device_object(body, DEV, false, Some("Headset"), Some("Headset"), None);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    let view = harness.view();
    assert_eq!(view.text(), "on");
    assert_eq!(view.icon(), Some('1'));
    // Connected with no per-state glyph: the static one again.
    let (mut harness, mut fake) = up_with(&settings);
    fake.managed = build::small_world();
    assert!(settle_for(&mut harness, &mut fake, 150));
    let view = harness.view();
    assert_eq!(view.text(), "Headset 72%");
    assert_eq!(view.icon(), Some('S'));
}

#[test]
fn icon_only_draws_the_icon_with_the_text_in_the_tooltip() {
    use crate::icon::Icon;
    let settings = Settings {
        icon_off: Some(Icon::Glyph('0')),
        icon_on: Some(Icon::Glyph('1')),
        icon_connected: Some(Icon::Glyph('C')),
        show_text: false,
        ..Settings::default()
    };
    let (mut harness, mut fake) = up_with(&settings);
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, false);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    let view = harness.view();
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('0'));
    assert_eq!(view.tooltip(), "Bluetooth off");
    let (mut harness, mut fake) = up_with(&settings);
    fake.managed = build::small_world();
    assert!(settle_for(&mut harness, &mut fake, 150));
    let view = harness.view();
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('C'));
    // The tooltip lists the devices, so the hidden name is moved into
    // it.
    assert!(view.tooltip().contains("Headset"), "{}", view.tooltip());
}

#[test]
fn without_icons_the_states_show_text_alone() {
    let (mut harness, mut fake) = up();
    fake.managed = build::managed(&|body| {
        build::adapter_object(body, HCI0, false);
    });
    assert!(settle_for(&mut harness, &mut fake, 150));
    let view = harness.view();
    assert_eq!(view.text(), "off");
    assert!(view.icon().is_none() && view.art().is_none());
}
