//! The network module through the harness: canned links, addresses,
//! routes and nl80211 notices in, views and queries out. Every `Harness`
//! call below is a real one (the registry test counts them); `/bin/sh`
//! is the one outside assumption, for the menu's stdin test (this crate
//! builds Linux-only, where it exists).

use std::time::Duration;

use rustix::event::PollFlags;

use super::Settings;
use super::WifiIcon;
use super::fake::{self, Fake};
use crate::action::ModuleAction;
use crate::icon::Icon;
use crate::modules::Update;
use crate::modules::harness::Harness;

const UP: u32 = 0x1;
const ETH0: u32 = 2;
const WLAN0: u32 = 3;

/// An ethernet interface, up with an address but no route yet.
fn plug(fake: &Fake) {
    fake.rt(&fake::link(16, ETH0, UP, 6, "eth0", None));
    fake.rt(&fake::addr(20, ETH0, 2));
}

/// The default route via `oif`.
fn route_via(fake: &Fake, oif: u32) {
    fake.rt(&fake::route(24, 2, oif));
}

/// A wireless interface, associated to `ssid` at `signal` dBm.
fn wifi(fake: &Fake, ssid: &[u8], signal: i8) {
    fake.rt(&fake::link(16, WLAN0, UP, 6, "wlan0", Some("nl80211")));
    fake.rt(&fake::addr(20, WLAN0, 2));
    fake.rt(&fake::route(24, 2, WLAN0));
    fake.genl(&fake::interface(WLAN0, "wlan0", Some(ssid)));
    fake.genl(&fake::scan(&[(ssid, signal as i32 * 100, true)]));
    fake.genl(&fake::station(signal));
}

fn drive(harness: &mut Harness) -> Update {
    let mut update = Update::Unchanged;
    // The socketpair delivers at once; the trailing wait only proves
    // nothing more is coming.
    for _ in 0..10 {
        match harness.wait(Duration::from_millis(50)) {
            Some(Update::Changed) => update = Update::Changed,
            Some(Update::Unchanged) => {}
            None => break,
        }
    }
    update
}

#[test]
fn the_stand_in_starts_the_module_as_the_bar_does() {
    let spec = crate::modules::find(super::ID).expect("registered");
    let settings = crate::modules::Settings::default();
    // Real sockets where they open; the contract drives both this and
    // the stand-in through every event.
    match Harness::start(spec, &settings) {
        Ok(harness) => {
            let _ = harness.view();
        }
        Err(why) => assert!(!why.trim().is_empty()),
    }
}

#[test]
fn ethernet_up_with_a_route_shows_its_name() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    assert_eq!(harness.view().text(), "");
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "eth0");
    assert_eq!(view.class(), crate::modules::Class::Normal);
    let _ = fake.sent();
}

#[test]
fn no_route_is_offline() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "offline");
    assert_eq!(harness.view().class(), crate::modules::Class::Warn);
    // A selected-but-down interface is offline too, loudly.
    let settings = Settings {
        interface: Some("eth0".to_owned()),
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    fake.rt(&fake::link(16, ETH0, 0, 2, "eth0", None));
    fake.rt(&fake::addr(20, ETH0, 2));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "offline");
    assert_eq!(harness.view().class(), crate::modules::Class::Warn);
    assert_eq!(
        harness.value_on(None),
        Some(serde_json::json!({"state": "disconnected"}))
    );
    let _ = fake.sent();
}

#[test]
fn wifi_shows_the_ssid_alone() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "eth0");
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    // The default route moved to wlan0 with the canned route: the text
    // is just the SSID, the strength the tooltip's dBm and `query`'s
    // `bars`.
    assert_eq!(harness.view().text(), "Wimbly");
    let value = harness.value_on(None).expect("a value");
    assert_eq!(value["state"], "wifi");
    assert_eq!(value["ssid"], "Wimbly");
    assert_eq!(value["signal"], -54);
    assert_eq!(value["bars"], 4);
    assert_eq!(value["interface"], "wlan0");
    let _ = fake.sent();
}

#[test]
fn a_hidden_ssid_stays_private() {
    let settings = Settings {
        show_ssid: false,
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    wifi(&fake, b"Wimbly", -72);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "WiFi");
    let value = harness.value_on(None).expect("a value");
    assert!(value.get("ssid").is_none(), "{value}");
    assert_eq!(value["bars"], 2);
    // The picker stays closed too, even with a command configured: the
    // scan list would expose what the bar hides. Nothing spawns.
    let settings = Settings {
        show_ssid: false,
        menu_command: vec!["true".to_owned()],
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    wifi(&fake, b"Wimbly", -72);
    assert_eq!(drive(&mut harness), Update::Changed);
    let before = harness.source_count();
    let menu = ModuleAction::new("menu", None);
    let output = crate::modules::OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &menu, 1),
        Err(crate::modules::InvokeError::Refused(
            "ssid hidden: the picker stays closed while show-ssid is false"
        ))
    );
    assert_eq!(harness.source_count(), before, "no menu spawned");
    let _ = fake.sent();
}

#[test]
fn untrusted_ssids_reach_neither_view_nor_menu() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"evil\nssid\x07", -60);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert!(
        !view.text().chars().any(char::is_control),
        "{:?}",
        view.text()
    );
    assert!(view.text().starts_with("evil"), "{:?}", view.text());
    let _ = fake.sent();
}

#[test]
fn a_disconnect_clears_within_the_turn() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("Wimbly"));
    // Address gone and link down in one burst: one change, offline, and
    // the signal timer with it.
    fake.rt(&fake::addr(21, WLAN0, 2));
    fake.rt(&fake::link(16, WLAN0, 0, 2, "wlan0", Some("nl80211")));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "offline");
    assert_eq!(harness.source_count(), 2);
    // Settled: nothing more to say.
    assert_eq!(harness.wait(Duration::from_millis(50)), None);
    let _ = fake.sent();
}

#[test]
fn a_deauth_clears_the_ssid() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    fake.genl(&fake::deauth(WLAN0));
    assert_eq!(drive(&mut harness), Update::Changed);
    // The link is still up with an address, but nothing is associated.
    assert_eq!(harness.view().text(), "offline");
    let _ = fake.sent();
}

#[test]
fn a_finished_dump_releases_the_next() {
    use super::netlink;
    // Every dump request sent, oldest first; singles carry no DUMP flag.
    let dumps = |genl: &[u8]| {
        netlink::messages(genl)
            .filter(|msg| msg.flags & netlink::NLM_F_DUMP == netlink::NLM_F_DUMP)
            .map(|msg| msg.seq)
            .collect::<Vec<_>>()
    };
    let scans = |genl: &[u8]| {
        netlink::messages(genl)
            .filter_map(|msg| {
                netlink::genl_of(msg.body).and_then(|(command, _)| {
                    (command == netlink::NL80211_CMD_GET_SCAN
                        || command == netlink::NL80211_CMD_NEW_SCAN_RESULTS)
                        .then_some(msg.seq)
                })
            })
            .collect::<Vec<_>>()
    };
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    // The scan dump is outstanding (its station waits behind it).
    let (_, genl) = fake.sent();
    let scan = scans(&genl);
    assert_eq!(scan.len(), 1);
    // A roam arrives while it is in flight: its station queues behind
    // it (its interface re-read is a single, not a dump).
    fake.genl(&fake::roam(WLAN0));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    assert!(dumps(&genl).is_empty(), "nothing while busy");
    // Its terminator releases the queue in order: the first station,
    // then the roam's scan.
    fake.genl(&fake::done_seq(scan[0]));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    let queued = dumps(&genl);
    assert_eq!(queued.len(), 1);
    fake.genl(&fake::done_seq(queued[0]));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    assert_eq!(scans(&genl).len(), 1, "the queued scan goes out");
}

#[test]
fn dump_bookkeeping_releases_and_requeues() {
    use super::netlink::Family;
    use super::{Dump, GenlDump, Nets};
    let mut nets = Nets::default();
    // Nothing outstanding: completions land nowhere, silently.
    nets.done_rt(7);
    nets.redump_rt(7);
    nets.done_genl(7);
    nets.redump_genl(7);
    nets.drop_genl(7);
    // A stale terminator does not release the in-flight dump.
    nets.rt_busy = Some((7, Dump::Routes(2)));
    nets.done_rt(8);
    assert!(nets.rt_busy.is_some());
    nets.done_rt(7);
    assert!(nets.rt_busy.is_none());
    // A failed dump goes back to the queue's end; a refused one drops.
    nets.rt_busy = Some((9, Dump::Addrs(2)));
    nets.redump_rt(9);
    assert!(nets.rt_busy.is_none());
    assert_eq!(nets.rt_dumps.pop_front(), Some(Dump::Addrs(2)));
    nets.genl_busy = Some((11, GenlDump::Scan(3)));
    nets.drop_genl(11);
    assert!(nets.genl_busy.is_none());
    assert!(nets.genl_dumps.is_empty());
    // A dump that keeps failing is dropped, not spun on: two failures
    // re-queue, the third drops, and a success resets the count.
    let mut poison = Nets {
        rt_busy: Some((12, Dump::Routes(10))),
        ..Nets::default()
    };
    poison.redump_rt(12);
    poison.rt_busy = Some((13, Dump::Routes(10)));
    poison.redump_rt(13);
    assert_eq!(poison.rt_dumps.pop_front(), Some(Dump::Routes(10)));
    assert_eq!(poison.rt_dumps.pop_front(), Some(Dump::Routes(10)));
    poison.rt_busy = Some((14, Dump::Routes(10)));
    poison.redump_rt(14);
    assert!(poison.rt_dumps.is_empty(), "the poison dump is dropped");
    poison.genl_busy = Some((20, GenlDump::Station(3)));
    poison.redump_genl(20);
    poison.genl_busy = Some((21, GenlDump::Station(3)));
    poison.redump_genl(21);
    poison.genl_busy = Some((22, GenlDump::Station(3)));
    poison.done_genl(22);
    poison.genl_busy = Some((23, GenlDump::Station(3)));
    poison.redump_genl(23);
    assert_eq!(poison.genl_dumps.len(), 3, "a success resets the count");
    // The starting set, in send order.
    let mut nets = Nets {
        family: Family {
            id: 30,
            scan: 20,
            mlme: 22,
        },
        ..Nets::default()
    };
    super::initial_dumps(&mut nets);
    assert_eq!(
        nets.rt_dumps.into_iter().collect::<Vec<_>>(),
        [
            Dump::Links,
            Dump::Addrs(2),
            Dump::Addrs(10),
            Dump::Routes(2),
            Dump::Routes(10)
        ]
    );
    assert_eq!(
        nets.genl_dumps.into_iter().collect::<Vec<_>>(),
        [GenlDump::Interfaces]
    );
}

#[test]
fn the_multicast_mask_joins_whatever_fits() {
    // A group id is the kernel's 1-based number, so id `n` is bit
    // `n - 1`: both fit, both join.
    assert_eq!(super::join_mask(20, 22), ((1 << 19) | (1 << 21), true));
    // Id 32 is the last bit that fits.
    assert_eq!(super::join_mask(32, 22), ((1 << 31) | (1 << 21), true));
    // One past 32: the other still joins, but not both.
    assert_eq!(super::join_mask(33, 22), (1 << 21, false));
    assert_eq!(super::join_mask(20, 40), (1 << 19, false));
    // Absent (id 0) joins nothing.
    assert_eq!(super::join_mask(0, 0), (0, false));
    assert_eq!(super::join_mask(0, 22), (1 << 21, false));
    // Neither fits: nothing joins.
    assert_eq!(super::join_mask(33, 40), (0, false));
}

#[test]
fn a_finished_scan_is_one_quiet_wake() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("Wimbly"));
    // Drain the start-up scan and station, so any re-dump below would go
    // out at once and be visible in what the module sends.
    let (_, genl) = fake.sent();
    fake.genl(&fake::done_seq(request_seq(
        &genl,
        super::netlink::NL80211_CMD_GET_SCAN,
    )));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    fake.genl(&fake::done_seq(request_seq(
        &genl,
        super::netlink::NL80211_CMD_GET_STATION,
    )));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    // The kernel finished a scan: the notice carries no networks and
    // nothing shown needs one, so nothing is re-dumped — one wake that
    // changes nothing. Fresh lists come from roam and from opening the
    // picker, which re-dumps for itself.
    fake.genl(&fake::scan_done());
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    assert!(
        super::netlink::messages(&genl).next().is_none(),
        "no re-dump on a bare scan notice"
    );
    let _ = fake.sent();
}

/// The sequence of the newest dump request for `command` in what the
/// module sent: the tests' way to complete an in-flight dump.
fn request_seq(genl: &[u8], command: u8) -> u32 {
    super::netlink::messages(genl)
        .filter_map(|msg| {
            super::netlink::genl_of(msg.body)
                .and_then(|(found, _)| (found == command).then_some(msg.seq))
        })
        .last()
        .expect("the request")
}

/// The ifindexes of the scan dumps in what the module sent, oldest first:
/// which radios' caches the list could come from.
fn scan_targets(genl: &[u8]) -> Vec<u32> {
    super::netlink::messages(genl)
        .filter_map(|msg| {
            super::netlink::genl_of(msg.body).and_then(|(command, fields)| {
                (command == super::netlink::NL80211_CMD_GET_SCAN
                    || command == super::netlink::NL80211_CMD_NEW_SCAN_RESULTS)
                    .then(|| super::netlink::find_u32(fields, super::netlink::ATTR_IFINDEX))
            })
        })
        .collect()
}

/// A `menu-command` writing what it was fed to a file, with its dir and
/// file: the menu-free way to read the scan list.
fn record_menu(tag: &str) -> (Vec<String>, std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "scootbar-network-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("list");
    (
        vec![
            "sh".to_owned(),
            "-c".to_owned(),
            format!("cat > {}", file.display()),
        ],
        dir,
        file,
    )
}

/// Two radios, the route on the shown one: the second radio's cache dumps
/// after the shown one's, and must not replace it.
#[test]
fn only_the_shown_radio_s_scan_is_dumped() {
    const DONGLE: u32 = 4;
    let (command, dir, file) = record_menu("shown-scan");
    let settings = Settings {
        menu_command: command,
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    for (index, name, ssid) in [
        (WLAN0, "wlan0", &b"Wimbly"[..]),
        (DONGLE, "wlan1", &b"FarAway"[..]),
    ] {
        fake.rt(&fake::link(16, index, UP, 6, name, None));
        fake.rt(&fake::addr(20, index, 2));
        fake.genl(&fake::interface(index, name, Some(ssid)));
    }
    route_via(&fake, WLAN0);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("Wimbly"));
    // The shown radio's scan went out first.
    let (_, genl) = fake.sent();
    assert_eq!(scan_targets(&genl), [WLAN0]);
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    // Its page fills the list; completing the dump must not send the
    // second radio's: there is nothing of its to send.
    fake.genl(&fake::scan(&[
        (b"Wimbly", -5400, true),
        (b"Cafe", -6000, false),
    ]));
    fake.genl(&fake::done_seq(scan));
    assert_eq!(drive(&mut harness), Update::Changed);
    // Complete the station beside the shown scan: the second radio's
    // scan would go out behind it.
    let (_, genl) = fake.sent();
    fake.genl(&fake::station(-54));
    fake.genl(&fake::done_seq(request_seq(
        &genl,
        super::netlink::NL80211_CMD_GET_STATION,
    )));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    let targets = scan_targets(&genl);
    assert!(
        targets.is_empty(),
        "no second radio's scan goes out: {targets:?}"
    );
    // The list is the shown radio's.
    let menu = ModuleAction::new("menu", None);
    let output = crate::modules::OutputView { name: None };
    assert_eq!(harness.invoke(&output, &menu, 1), Ok(Update::Unchanged));
    let _ = drive(&mut harness);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Wimbly\nCafe\n");
    std::fs::remove_dir_all(&dir).ok();
    let _ = fake.sent();
}

/// An access-point interface beside the station: up with an address, but
/// its empty cache is never dumped and never replaces the scan.
#[test]
fn an_ap_interface_s_empty_cache_never_replaces_the_scan() {
    const AP: u32 = 4;
    let (command, dir, file) = record_menu("ap-scan");
    let settings = Settings {
        menu_command: command,
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("Wimbly"));
    // The station's scan is in flight with its page already in the list;
    // completing the dump queues the station beside it.
    let (_, genl) = fake.sent();
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    fake.genl(&fake::done_seq(scan));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    // The hotspot appears beside the station.
    fake.rt(&fake::link(16, AP, UP, 6, "wlan1", None));
    fake.rt(&fake::addr(20, AP, 2));
    fake.genl(&fake::interface_with_type(
        AP,
        "wlan1",
        None,
        super::netlink::NL80211_IFTYPE_AP,
    ));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    // Complete the station beside the shown scan: anything the hotspot
    // queued behind it would go out now, and nothing may.
    let (_, genl) = fake.sent();
    fake.genl(&fake::station(-54));
    fake.genl(&fake::done_seq(request_seq(
        &genl,
        super::netlink::NL80211_CMD_GET_STATION,
    )));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    let targets = scan_targets(&genl);
    assert!(
        targets.is_empty(),
        "no access point's scan goes out: {targets:?}"
    );
    // The list is still the station's.
    let menu = ModuleAction::new("menu", None);
    let output = crate::modules::OutputView { name: None };
    assert_eq!(harness.invoke(&output, &menu, 1), Ok(Update::Unchanged));
    let _ = drive(&mut harness);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Wimbly\n");
    std::fs::remove_dir_all(&dir).ok();
    let _ = fake.sent();
}

/// The default route moving to the dongle re-dumps the scan there: the
/// list follows the shown radio instead of staying stale.
#[test]
fn the_default_route_moving_relists_the_new_radio() {
    const DONGLE: u32 = 4;
    let (command, dir, file) = record_menu("route-move");
    let settings = Settings {
        menu_command: command,
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    for (index, name, ssid) in [
        (WLAN0, "wlan0", &b"Wimbly"[..]),
        (DONGLE, "wlan1", &b"FarAway"[..]),
    ] {
        fake.rt(&fake::link(16, index, UP, 6, name, None));
        fake.rt(&fake::addr(20, index, 2));
        fake.genl(&fake::interface(index, name, Some(ssid)));
    }
    route_via(&fake, WLAN0);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("Wimbly"));
    // Settle the shown scan: only its dump went out.
    let (_, genl) = fake.sent();
    assert_eq!(scan_targets(&genl), [WLAN0]);
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    fake.genl(&fake::scan(&[(b"Wimbly", -5400, true)]));
    fake.genl(&fake::done_seq(scan));
    assert_eq!(drive(&mut harness), Update::Changed);
    // Settle the station beside it: the second radio's scan would go out
    // behind it, and must not while the route stands.
    let (_, genl) = fake.sent();
    fake.genl(&fake::station(-54));
    fake.genl(&fake::done_seq(request_seq(
        &genl,
        super::netlink::NL80211_CMD_GET_STATION,
    )));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    let targets = scan_targets(&genl);
    assert!(
        targets.is_empty(),
        "nothing more is owed while the route stands: {targets:?}"
    );
    // The route moves: the dongle's scan goes out with the move.
    route_via(&fake, DONGLE);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("FarAway"));
    let (_, genl) = fake.sent();
    assert_eq!(scan_targets(&genl), [DONGLE]);
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    // Its page replaces the list (the send reset it).
    fake.genl(&fake::scan(&[
        (b"FarAway", -6000, true),
        (b"Elsewhere", -7000, false),
    ]));
    fake.genl(&fake::done_seq(scan));
    assert_eq!(drive(&mut harness), Update::Changed);
    let menu = ModuleAction::new("menu", None);
    let output = crate::modules::OutputView { name: None };
    assert_eq!(harness.invoke(&output, &menu, 1), Ok(Update::Unchanged));
    let _ = drive(&mut harness);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "FarAway\nElsewhere\n"
    );
    std::fs::remove_dir_all(&dir).ok();
    let _ = fake.sent();
}

/// The shown radio vanishing re-dumps the survivor's scan: the picker
/// keeps working on the radio that is left.
#[test]
fn the_shown_radio_vanishing_relists_the_survivor() {
    const DONGLE: u32 = 4;
    let (command, dir, file) = record_menu("vanish");
    let settings = Settings {
        menu_command: command,
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("Wimbly"));
    // Settle the shown scan and station: nothing is owed after.
    let (_, genl) = fake.sent();
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    fake.genl(&fake::done_seq(scan));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    let station = request_seq(&genl, super::netlink::NL80211_CMD_GET_STATION);
    fake.genl(&fake::done_seq(station));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    assert!(scan_targets(&genl).is_empty());
    // A second radio appears beside it: not shown, nothing is owed.
    fake.rt(&fake::link(16, DONGLE, UP, 6, "wlan1", None));
    fake.rt(&fake::addr(20, DONGLE, 2));
    fake.genl(&fake::interface(DONGLE, "wlan1", Some(b"FarAway")));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    // Drain whatever the arrival sent, so what follows is only the
    // vanishing's answer.
    let _ = fake.sent();
    // The shown radio vanishes: its interface and link go away.
    fake.genl(&fake::del_interface(WLAN0));
    fake.rt(&fake::link(17, WLAN0, 0, 2, "wlan0", Some("nl80211")));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "offline");
    // The survivor's scan went out with the vanishing.
    let (_, genl) = fake.sent();
    assert_eq!(scan_targets(&genl), [DONGLE]);
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    fake.genl(&fake::scan(&[(b"FarAway", -6000, true)]));
    fake.genl(&fake::done_seq(scan));
    // Unchanged: with no route nothing is selected, so the survivor's
    // signal moves no view — the menu below proves the list refilled.
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let menu = ModuleAction::new("menu", None);
    let output = crate::modules::OutputView { name: None };
    assert_eq!(harness.invoke(&output, &menu, 1), Ok(Update::Unchanged));
    let _ = drive(&mut harness);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "FarAway\n");
    std::fs::remove_dir_all(&dir).ok();
    let _ = fake.sent();
}

/// An idle station is scanned while nothing is associated: no route, no
/// SSID anywhere, yet the lone radio's cache is still the list, so the
/// picker works off-network. A no-regression pin: this passed before the
/// fix (every interface was dumped) and must keep passing.
#[test]
fn an_idle_station_is_scanned_while_nothing_is_associated() {
    let (command, dir, file) = record_menu("idle-scan");
    let settings = Settings {
        menu_command: command,
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    // Up with an address but no route and no association.
    fake.rt(&fake::link(16, WLAN0, UP, 6, "wlan0", None));
    fake.rt(&fake::addr(20, WLAN0, 2));
    fake.genl(&fake::interface(WLAN0, "wlan0", None));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "offline");
    let (_, genl) = fake.sent();
    assert_eq!(scan_targets(&genl), [WLAN0]);
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    fake.genl(&fake::scan(&[(b"Cafe", -6000, false)]));
    fake.genl(&fake::done_seq(scan));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let menu = ModuleAction::new("menu", None);
    let output = crate::modules::OutputView { name: None };
    assert_eq!(harness.invoke(&output, &menu, 1), Ok(Update::Unchanged));
    let _ = drive(&mut harness);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Cafe\n");
    std::fs::remove_dir_all(&dir).ok();
    let _ = fake.sent();
}

/// A deauthentication re-targets the kept scan at once: the shown
/// interface is ethernet, two stations stand behind it with the
/// associated one listed second, and its deauth sends the surviving
/// idle station's scan without waiting for another event.
#[test]
fn a_deauth_retargets_the_scan_to_the_surviving_station() {
    const IDLE: u32 = WLAN0;
    const ASSOC: u32 = 4;
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    route_via(&fake, ETH0);
    // The idle station first, the associated one second: the kept scan
    // is the associated one's.
    fake.rt(&fake::link(16, IDLE, UP, 6, "wlan0", None));
    fake.rt(&fake::addr(20, IDLE, 2));
    fake.genl(&fake::interface(IDLE, "wlan0", None));
    fake.rt(&fake::link(16, ASSOC, UP, 6, "wlan1", None));
    fake.rt(&fake::addr(20, ASSOC, 2));
    fake.genl(&fake::interface(ASSOC, "wlan1", Some(b"FarAway")));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "eth0");
    // Settle the associated scan and its station: nothing owed after.
    let (_, genl) = fake.sent();
    assert_eq!(scan_targets(&genl), [ASSOC]);
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    fake.genl(&fake::scan(&[(b"FarAway", -6000, true)]));
    fake.genl(&fake::done_seq(scan));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    fake.genl(&fake::station(-60));
    fake.genl(&fake::done_seq(request_seq(
        &genl,
        super::netlink::NL80211_CMD_GET_STATION,
    )));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    assert!(scan_targets(&genl).is_empty());
    // The associated station deauthenticates: the idle one's scan goes
    // out with the notice, not with the next event.
    fake.genl(&fake::deauth(ASSOC));
    let _ = drive(&mut harness);
    let (_, genl) = fake.sent();
    assert_eq!(scan_targets(&genl), [IDLE]);
    let _ = fake.sent();
}

/// A station reply in flight for the old target does not land on the new
/// target's signal: the reply belongs to the in-flight dump's interface,
/// not whatever `station_of` has moved to since it was queued. Fails
/// with `station_of` read at reply time (the new target briefly shows
/// the old one's dBm until its own reply overwrites it).
#[test]
fn a_late_station_reply_keeps_the_old_target_s_signal() {
    const OLD: u32 = WLAN0;
    const NEXT: u32 = 4;
    let (mut harness, fake) = Fake::start(&Settings::default());
    for (index, name, ssid) in [
        (OLD, "wlan0", &b"Wimbly"[..]),
        (NEXT, "wlan1", &b"FarAway"[..]),
    ] {
        fake.rt(&fake::link(16, index, UP, 6, name, None));
        fake.rt(&fake::addr(20, index, 2));
        fake.genl(&fake::interface(index, name, Some(ssid)));
    }
    route_via(&fake, OLD);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("Wimbly"));
    // The shown scan is settled; its station is now in flight.
    let (_, genl) = fake.sent();
    assert_eq!(scan_targets(&genl), [OLD]);
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    fake.genl(&fake::scan(&[(b"Wimbly", -5000, true)]));
    fake.genl(&fake::done_seq(scan));
    assert_eq!(drive(&mut harness), Update::Changed);
    let (_, genl) = fake.sent();
    let station = request_seq(&genl, super::netlink::NL80211_CMD_GET_STATION);
    // The route moves while the old target's station is outstanding: the
    // new target's scan queues behind it, and nothing goes out yet.
    route_via(&fake, NEXT);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("FarAway"));
    let (_, genl) = fake.sent();
    assert!(
        scan_targets(&genl).is_empty(),
        "nothing goes out while the old station is in flight"
    );
    // The old target's reply lands: on the old target, not the shown
    // one. A different dBm than the scan lent, so a misattribution
    // shows in the value.
    fake.genl(&fake::station(-51));
    fake.genl(&fake::done_seq(station));
    let _ = drive(&mut harness);
    let value = harness.value_on(None).expect("a value");
    assert_eq!(value["ssid"], "FarAway");
    assert!(
        value.get("signal").is_none(),
        "the old target's dBm must not land on the new target: {value}"
    );
    // The new target's own cycle then reports its own signal.
    let (_, genl) = fake.sent();
    assert_eq!(scan_targets(&genl), [NEXT]);
    let scan = request_seq(&genl, super::netlink::NL80211_CMD_GET_SCAN);
    fake.genl(&fake::scan(&[(b"FarAway", -8000, true)]));
    fake.genl(&fake::done_seq(scan));
    let _ = drive(&mut harness);
    let (_, genl) = fake.sent();
    fake.genl(&fake::station(-80));
    fake.genl(&fake::done_seq(request_seq(
        &genl,
        super::netlink::NL80211_CMD_GET_STATION,
    )));
    let _ = drive(&mut harness);
    let value = harness.value_on(None).expect("a value");
    assert_eq!(value["signal"], -80);
    let _ = fake.sent();
}

#[test]
fn two_radios_scans_do_not_mix() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    for (index, name, ssid) in [
        (WLAN0, "wlan0", &b"Wimbly"[..]),
        (4, "wlan1", &b"FarAway"[..]),
    ] {
        fake.rt(&fake::link(16, index, UP, 6, name, None));
        fake.rt(&fake::addr(20, index, 2));
        fake.genl(&fake::interface(index, name, Some(ssid)));
    }
    route_via(&fake, WLAN0);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("Wimbly"));
    // wlan0's scan is in flight (sent first); wlan1's waits behind it.
    // wlan0's page arrives while wlan1's scan is queued: it still belongs
    // to wlan0, and wlan1 keeps the SSID its interface named.
    fake.genl(&fake::scan(&[(b"Wimbly", -5400, true)]));
    route_via(&fake, 4);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(
        harness.view().text().starts_with("FarAway"),
        "{:?}",
        harness.view().text()
    );
    let _ = fake.sent();
}

#[test]
fn a_roam_renames_within_the_turn() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    fake.genl(&fake::roam(WLAN0));
    fake.genl(&fake::scan(&[(b"FarAway", -7800, true)]));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "FarAway");
    let _ = fake.sent();
}

#[test]
fn a_roam_asks_the_interface_for_the_new_ssid() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    // The roam queues an interface re-read beside the scan and station:
    // the scan cache predates the roam (nothing associated), and the
    // interface names the new network.
    fake.genl(&fake::roam(WLAN0));
    fake.genl(&fake::scan(&[(b"FarAway", -7800, false)]));
    fake.genl(&fake::interface(WLAN0, "wlan0", Some(b"FarAway")));
    fake.genl(&fake::station(-78));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "FarAway");
    let _ = fake.sent();
}

#[test]
fn a_rename_keeps_the_default_route_s_interface() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "eth0");
    // Same index, new name: the selection follows the index, and the
    // shown name moves with it.
    fake.rt(&fake::link(16, ETH0, UP, 6, "enp0s1", None));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "enp0s1");
    let _ = fake.sent();
}

#[test]
fn a_vpn_is_a_state_and_a_marker() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    route_via(&fake, ETH0);
    fake.rt(&fake::link(16, 9, UP, 6, "wg0", Some("wireguard")));
    fake.rt(&fake::addr(20, 9, 2));
    assert_eq!(drive(&mut harness), Update::Changed);
    // Ethernet is shown, the VPN beside it marked.
    assert_eq!(harness.view().text(), "eth0 · VPN");
    // Routed through the tunnel, the tunnel is shown.
    route_via(&fake, 9);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "VPN");
    let value = harness.value_on(None).expect("a value");
    assert_eq!(value["state"], "vpn");
    assert_eq!(value["vpn"], true);
    let _ = fake.sent();
}

#[test]
fn a_missing_configured_interface_is_offline() {
    let settings = Settings {
        interface: Some("eth9".to_owned()),
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "offline");
    assert_eq!(harness.view().class(), crate::modules::Class::Warn);
    let _ = fake.sent();
}

#[test]
fn a_usable_v6_default_survives_a_down_v4() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    // eth0 carries the v4 default but is down; eth1 carries v6, up.
    fake.rt(&fake::link(16, ETH0, 0, 2, "eth0", None));
    fake.rt(&fake::addr(20, ETH0, 2));
    fake.rt(&fake::link(16, 4, UP, 6, "eth1", None));
    fake.rt(&fake::addr(20, 4, 10));
    route_via(&fake, ETH0);
    fake.rt(&fake::route(24, 10, 4));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "eth1");
    let _ = fake.sent();
}

#[test]
fn a_withdrawn_default_route_is_offline() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "eth0");
    fake.rt(&fake::route(25, 2, ETH0));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "offline");
    let _ = fake.sent();
}

#[test]
fn a_v6_address_and_route_show_ethernet() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    fake.rt(&fake::link(16, ETH0, UP, 6, "eth0", None));
    fake.rt(&fake::addr(20, ETH0, 10));
    fake.rt(&fake::route(24, 10, ETH0));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "eth0");
    let value = harness.value_on(None).expect("a value");
    assert_eq!(value["state"], "ethernet");
    let _ = fake.sent();
}

#[test]
fn a_deleted_wireless_interface_falls_back_to_its_name() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    // The interface is gone but its link entry lingers (the DELLINK
    // follows in the same burst on the wire): transiently ethernet.
    fake.genl(&fake::del_interface(WLAN0));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "wlan0");
    let _ = fake.sent();
}

#[test]
fn a_flapping_link_coalesces_into_one_change() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    for _ in 0..8 {
        fake.rt(&fake::link(16, ETH0, 0, 2, "eth0", None));
        fake.rt(&fake::link(16, ETH0, UP, 6, "eth0", None));
    }
    // Eight downs and ups drain in as many turns as it takes, but the
    // state never moved: no change is reported.
    assert_eq!(drive(&mut harness), Update::Unchanged);
    assert_eq!(harness.view().text(), "eth0");
    let _ = fake.sent();
}

#[test]
fn the_signal_timer_runs_only_for_shown_wifi() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    // Offline: the two sockets, nothing else.
    assert_eq!(harness.source_count(), 2);
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    // Ethernet: still no timer.
    assert_eq!(harness.source_count(), 2);
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    // Connected: the timer joins the poll.
    assert_eq!(harness.source_count(), 3);
    // Complete the scan dump in flight, or the timer's station queues
    // behind it, as on the wire.
    let (_, genl) = fake.sent();
    let scan = super::netlink::messages(&genl)
        .filter_map(|msg| {
            super::netlink::genl_of(msg.body).and_then(|(command, _)| {
                (command == super::netlink::NL80211_CMD_GET_SCAN
                    || command == super::netlink::NL80211_CMD_NEW_SCAN_RESULTS)
                    .then_some(msg.seq)
            })
        })
        .last()
        .expect("the scan dump");
    fake.genl(&fake::done_seq(scan));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    // A tick queues the station refresh and changes nothing by itself.
    assert_eq!(harness.deliver(2, PollFlags::IN), Update::Unchanged);
    let (_, genl) = fake.sent();
    let station = super::netlink::messages(&genl).any(|msg| {
        super::netlink::genl_of(msg.body).map(|(command, _)| command)
            == Some(super::netlink::NL80211_CMD_GET_STATION)
    });
    assert!(station, "a station query");
}

#[test]
fn a_refused_query_is_asked_once() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    // The newcomer eth0 draws one interface query...
    let (_, genl) = fake.sent();
    assert!(!genl.is_empty(), "the interface query");
    // ...which the kernel refuses. A refusal stops nothing globally, but
    // the same interface is never asked twice.
    fake.genl(&fake::error(-1));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    fake.rt(&fake::link(16, 4, UP, 6, "eth1", None));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    assert!(!genl.is_empty(), "the newcomer is still asked once");
    fake.rt(&fake::link(16, 4, UP, 6, "eth1", None));
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    assert!(genl.is_empty(), "never twice for the same interface");
}

#[test]
fn the_menu_needs_a_command_and_networks() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    let menu = ModuleAction::new("menu", None);
    let output = crate::modules::OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &menu, 1),
        Err(crate::modules::InvokeError::Refused(
            "no menu command configured"
        ))
    );
    let unknown = ModuleAction::new("bogus", None);
    assert_eq!(
        harness.invoke(&output, &unknown, 1),
        Err(crate::modules::InvokeError::Unknown)
    );
    let numbered = ModuleAction::new("menu", Some(2));
    assert_eq!(
        harness.invoke(&output, &numbered, 1),
        Err(crate::modules::InvokeError::NoArg)
    );
    let _ = fake.sent();
}

#[test]
fn the_menu_is_fed_the_scan_and_reaped() {
    let dir = std::env::temp_dir().join(format!(
        "scootbar-network-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("choice");
    let settings = Settings {
        menu_command: vec![
            "sh".to_owned(),
            "-c".to_owned(),
            format!("cat > {}", file.display()),
        ],
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    let menu = ModuleAction::new("menu", None);
    let output = crate::modules::OutputView { name: None };
    assert_eq!(harness.invoke(&output, &menu, 1), Ok(Update::Unchanged));
    // The menu runs: one more source while it does.
    assert_eq!(harness.source_count(), 4);
    // A second click replaces it (the first already exited here, so this
    // only reaps it and opens again): still one menu source, and the
    // list fed twice over.
    assert_eq!(harness.invoke(&output, &menu, 1), Ok(Update::Unchanged));
    assert_eq!(harness.source_count(), 4);
    let _ = drive(&mut harness);
    assert_eq!(harness.source_count(), 3);
    let fed = std::fs::read_to_string(&file).unwrap();
    assert_eq!(fed, "Wimbly\n");
    std::fs::remove_dir_all(&dir).ok();
    let _ = fake.sent();
}

#[test]
fn a_click_is_the_menu_and_nothing_else_is() {
    use crate::action::{Action, Trigger};
    use crate::density::Scale;
    use crate::modules::{ClickCtx, Input, OutputView};
    use crate::text::Text;
    use ab_glyph::{FontArc, FontVec};
    let (harness, _fake) = Fake::start(&Settings::default());
    let view = harness.view();
    let font = FontArc::new(FontVec::try_from_vec(crate::testfont::build()).unwrap());
    let text = Text::new(font);
    let ctx = ClickCtx {
        output: OutputView { name: None },
        x: 0,
        view: &view,
        text: &text,
        em: 50.0,
        padding: 10,
        span_width: 400,
        height: 60,
        scale: Scale::Integer(1),
    };
    let input = |trigger| harness.input(&Input { trigger, at: &ctx });
    assert_eq!(
        input(Trigger::Click),
        Some(Action::Module(ModuleAction::new("menu", None)))
    );
    assert_eq!(input(Trigger::ScrollUp), None);
    assert_eq!(input(Trigger::RightClick), None);
}

// ---------------------------------------------------------------------------
// Icons and the icon-only mode.
// ---------------------------------------------------------------------------

#[test]
fn icons_follow_the_state_with_a_static_fallback() {
    // A static icon plus one glyph each for ethernet and WiFi: a state
    // with its own glyph shows it, any other state the static one.
    let settings = Settings {
        icon: Some(Icon::Glyph('N')),
        icon_ethernet: Some(Icon::Glyph('E')),
        icon_wifi: Some(WifiIcon::One('W')),
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    // No route: offline has no per-state glyph, so the static icon.
    plug(&fake);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "offline");
    assert_eq!(view.icon(), Some('N'));
    assert_eq!(view.class(), crate::modules::Class::Warn);
    // Ethernet: its own glyph wins over the static one.
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "eth0");
    assert_eq!(view.icon(), Some('E'));
    // WiFi: its own glyph too; the text is the SSID alone.
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "Wimbly");
    assert_eq!(view.icon(), Some('W'));
    // Routed through the tunnel: no per-state VPN glyph, so the static
    // one again.
    fake.rt(&fake::link(16, 9, UP, 6, "wg0", Some("wireguard")));
    fake.rt(&fake::addr(20, 9, 2));
    route_via(&fake, 9);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "VPN");
    assert_eq!(view.icon(), Some('N'));
    let _ = fake.sent();
}

#[test]
fn a_four_glyph_wifi_icon_picks_the_level() {
    // Weakest to strongest, at every `bars_for` boundary: -55 is still
    // 4, -56 drops to 3, -68 to 2, -78 to 1.
    let settings = Settings {
        icon_wifi: Some(WifiIcon::Levels(['1', '2', '3', '4'])),
        ..Settings::default()
    };
    for (signal, want) in [
        (-40, '4'),
        (-55, '4'),
        (-56, '3'),
        (-60, '3'),
        (-67, '3'),
        (-68, '2'),
        (-72, '2'),
        (-77, '2'),
        (-78, '1'),
        (-100, '1'),
    ] {
        let (mut harness, fake) = Fake::start(&settings);
        wifi(&fake, b"Wimbly", signal);
        assert_eq!(drive(&mut harness), Update::Changed, "at {signal} dBm");
        let view = harness.view();
        assert_eq!(view.text(), "Wimbly", "at {signal} dBm");
        assert_eq!(view.icon(), Some(want), "at {signal} dBm");
        let _ = fake.sent();
    }
}

#[test]
fn a_single_wifi_glyph_shows_at_every_level() {
    // Backward compatible: one glyph for every signal, as before.
    let settings = Settings {
        icon_wifi: Some(WifiIcon::One('W')),
        ..Settings::default()
    };
    for signal in [-54, -60, -72, -80] {
        let (mut harness, fake) = Fake::start(&settings);
        wifi(&fake, b"Wimbly", signal);
        assert_eq!(drive(&mut harness), Update::Changed, "at {signal} dBm");
        let view = harness.view();
        assert_eq!(view.text(), "Wimbly", "at {signal} dBm");
        assert_eq!(view.icon(), Some('W'), "at {signal} dBm");
        let _ = fake.sent();
    }
}

#[test]
fn without_icons_the_states_show_text_alone() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "eth0");
    assert!(view.icon().is_none() && view.art().is_none());
    let _ = fake.sent();
}

#[test]
fn icon_only_draws_the_icon_with_the_text_in_the_tooltip() {
    let settings = Settings {
        icon: Some(Icon::Glyph('N')),
        icon_wifi: Some(WifiIcon::One('W')),
        show_text: false,
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    // Offline: the static icon, the text kept in the tooltip.
    plug(&fake);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('N'));
    assert_eq!(view.tooltip(), "offline · No network");
    assert_eq!(view.class(), crate::modules::Class::Warn);
    // Ethernet beside a VPN: the static icon, both names in the tooltip.
    fake.rt(&fake::link(16, 9, UP, 6, "wg0", Some("wireguard")));
    fake.rt(&fake::addr(20, 9, 2));
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('N'));
    assert_eq!(view.tooltip(), "eth0 · VPN");
    // WiFi: its own glyph, the SSID and signal in the tooltip (wg0 is
    // still up beside it, so the marker rides along, as in the text).
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('W'));
    assert_eq!(view.tooltip(), "Wimbly · -54 dBm on wlan0 · VPN");
    // Routed through the tunnel: the static icon, the VPN in the tooltip.
    route_via(&fake, 9);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('N'));
    assert_eq!(view.tooltip(), "VPN on wg0");
    let _ = fake.sent();
}

#[test]
fn icon_only_keeps_a_hidden_ssid_private() {
    let settings = Settings {
        icon: Some(Icon::Glyph('N')),
        show_ssid: false,
        show_text: false,
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    wifi(&fake, b"Wimbly", -72);
    assert_eq!(drive(&mut harness), Update::Changed);
    let view = harness.view();
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('N'));
    assert_eq!(view.tooltip(), "WiFi · -72 dBm on wlan0");
    let _ = fake.sent();
}

#[test]
fn query_reports_the_state_shape_with_icons_set() {
    // Icons and `show-text` change the view, never the value: the shape
    // per state, which an agent reads instead of the pixels.
    let settings = Settings {
        icon: Some(Icon::Glyph('N')),
        show_text: false,
        ..Settings::default()
    };
    let (mut harness, fake) = Fake::start(&settings);
    plug(&fake);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(
        harness.value_on(None),
        Some(serde_json::json!({"state": "disconnected"}))
    );
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(
        harness.value_on(None),
        Some(serde_json::json!({"state": "ethernet", "interface": "eth0", "vpn": false}))
    );
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(
        harness.value_on(None),
        Some(serde_json::json!({
            "state": "wifi",
            "ssid": "Wimbly",
            "signal": -54,
            "bars": 4,
            "interface": "wlan0",
            "vpn": false,
        }))
    );
    fake.rt(&fake::link(16, 9, UP, 6, "wg0", Some("wireguard")));
    fake.rt(&fake::addr(20, 9, 2));
    route_via(&fake, 9);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(
        harness.value_on(None),
        Some(serde_json::json!({"state": "vpn", "interface": "wg0", "vpn": true}))
    );
    let _ = fake.sent();
}

/// The wire against the real kernel: whatever interfaces this machine
/// has, one of them is named. Read-only; needs no radio and no permission.
#[test]
fn against_a_real_rtnetlink() {
    if std::env::var_os("SCOOTBAR_TEST_LIVE_NET").is_none() {
        return;
    }
    let network = super::open(&Settings::default()).expect("the sockets open");
    assert!(
        network
            .nets
            .ifaces
            .iter()
            .any(|iface| !iface.link.name_str().is_empty()),
        "named interfaces are tracked"
    );
    assert!(network.nets.has_usable(), "something besides lo exists");
    // The hardware type against the real kernel: every tracked link has
    // a nonzero type (`ifi_type` where the header puts it, native-endian).
    // A byte-swapped read of the family/pad bytes would be zero on all of
    // them (`lo` is not tracked at all: loopback never shows).
    assert!(
        network.nets.ifaces.iter().all(|iface| iface.link.arp != 0),
        "a byte-swapped type reads zero"
    );
}

/// The module against the real kernel, end to end: open, drain, and read
/// what the bar would show. Branches on what the machine has: WiFi proves
/// the SSID path (Asahi wlan0), ethernet the name path, and a machine
/// with neither says so and passes. Read-only; needs no permission.
#[test]
fn the_module_shows_the_real_connection() {
    if std::env::var_os("SCOOTBAR_TEST_LIVE_NET").is_none() {
        return;
    }
    let spec = crate::modules::find(super::ID).expect("registered");
    let settings = crate::modules::Settings::default();
    let mut harness = match Harness::start(spec, &settings) {
        Ok(harness) => harness,
        Err(why) => {
            eprintln!("live: network unavailable ({why}), skipping the end to end");
            return;
        }
    };
    let begin = std::time::Instant::now();
    for _ in 0..30 {
        if harness.wait(Duration::from_millis(100)).is_none() {
            break;
        }
    }
    let view = harness.view();
    let value = harness.value_on(None);
    eprintln!("live: view {:?} value {value:?}", view.text());
    match value
        .as_ref()
        .and_then(|v| v.get("state").and_then(|s| s.as_str()))
    {
        Some("wifi") => {
            let ssid = value
                .as_ref()
                .and_then(|v| v.get("ssid").and_then(|s| s.as_str()));
            assert!(ssid.is_some_and(|s| !s.is_empty()), "{value:?}");
            assert!(view.text().starts_with(ssid.unwrap()), "{:?}", view.text());
            let bars = value
                .as_ref()
                .and_then(|v| v.get("bars").and_then(|b| b.as_u64()));
            assert!(bars.is_some_and(|b| (1..=4).contains(&b)), "{value:?}");
        }
        Some("ethernet") => {
            let interface = value
                .as_ref()
                .and_then(|v| v.get("interface").and_then(|s| s.as_str()))
                .unwrap_or("");
            assert_eq!(view.text(), interface);
        }
        state => eprintln!("live: state {state:?}, nothing to prove"),
    }
    eprintln!("live: settled in {:?}", begin.elapsed());
}

/// Twenty quiet seconds are almost quiet: the only wakes are the
/// signal timer's (and a background scan's), and none of them moves the
/// state or SSID — the dBm may wobble, which is the radio being shown,
/// not polling. A stray real event (a carrier flap on a live machine)
/// fails it; rerun, don't loosen it.
#[test]
fn twenty_quiet_seconds_are_quiet() {
    if std::env::var_os("SCOOTBAR_TEST_LIVE_NET").is_none() {
        return;
    }
    let spec = crate::modules::find(super::ID).expect("registered");
    let settings = crate::modules::Settings::default();
    let mut harness = match Harness::start(spec, &settings) {
        Ok(harness) => harness,
        Err(why) => {
            eprintln!("live: network unavailable ({why}), skipping the idle watch");
            return;
        }
    };
    for _ in 0..30 {
        if harness.wait(Duration::from_millis(100)).is_none() {
            break;
        }
    }
    let mut wakes = 0;
    // The state and SSID the watch must not move: our own signal refresh
    // (and a background scan completing) may move the dBm, and the bars
    // with it — that is a real event being shown, not polling. A state
    // or SSID change is what has no cause here.
    let calm = harness.value_on(None).map(|v| {
        (
            v.get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_owned(),
            v.get("ssid")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_owned(),
        )
    });
    for _ in 0..20 {
        match harness.wait(Duration::from_secs(1)) {
            Some(Update::Unchanged) => wakes += 1,
            Some(Update::Changed) => {
                let now = harness.value_on(None).map(|v| {
                    (
                        v.get("state")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_owned(),
                        v.get("ssid")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_owned(),
                    )
                });
                assert_eq!(now, calm, "a state change with no network activity");
                wakes += 1;
            }
            None => {}
        }
    }
    eprintln!("live: {wakes} wakes in 20 quiet seconds");
    // The signal timer ticks every 10 s while connected; a background
    // scan completing adds at most a couple more. Anything beyond that is
    // polling, and anything past the dBm is a state moving with no cause.
    assert!(wakes <= 4, "only the timer (and a scan) wakes while idle");
}

/// The joined multicast, against the real kernel: the socket the module
/// listens on is subscribed to exactly the `scan`/`mlme` groups, read
/// back from the kernel itself (`/proc/net/netlink` names each socket's
/// port and mask, no privileges needed). A requested rescan was already
/// seen arriving as a scan-group notice on the corrected mask during
/// development (a `TRIGGER_SCAN` 19 ms after `nmcli device wifi rescan`,
/// where the old off-by-one mask heard nothing for minutes) — but a
/// rescan can roam the radio out from under the other live tests running
/// beside this one, so this test asserts the subscription, which is
/// deterministic, and only listens opportunistically: five seconds for
/// ambient traffic, reported, never failed on.
#[test]
fn multicast_membership_matches_the_mask() {
    if std::env::var_os("SCOOTBAR_TEST_LIVE_NET").is_none() {
        return;
    }
    let (genl, family, joined) = super::genl_socket().expect("the generic socket opens");
    if family.id == 0 {
        eprintln!("live: no nl80211 on this machine, skipping the multicast check");
        return;
    }
    eprintln!(
        "live: nl80211 id {} scan {} mlme {} joined {joined}",
        family.id, family.scan, family.mlme
    );
    assert!(joined, "both groups fit the mask on this machine");
    let genl = genl.expect("nl80211 resolved, so the socket is open");
    let (mask, _) = super::join_mask(family.scan, family.mlme);
    let table = std::fs::read_to_string("/proc/net/netlink").unwrap_or_default();
    if table.is_empty() {
        eprintln!("live: no /proc/net/netlink here, skipping the membership check");
        return;
    }
    // `sk Eth Pid Groups ...`: our process's generic sockets (protocol
    // 16) and their masks, in hex.
    let ours = std::process::id();
    let mut masks = Vec::new();
    for line in table.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let (Some(protocol), Some(pid), Some(groups)) =
            (fields.nth(1), fields.next(), fields.next())
        else {
            continue;
        };
        if protocol == "16" && pid.parse::<u32>().ok() == Some(ours) {
            masks.push(groups.to_owned());
        }
    }
    eprintln!("live: our generic masks: {masks:?}, want {mask:08x}");
    assert!(
        masks.iter().any(|groups| {
            u32::from_str_radix(groups.trim_start_matches('0'), 16).ok() == Some(mask)
        }),
        "the kernel holds our scan/mlme subscription"
    );
    // Opportunistic: five seconds for ambient traffic on the joined
    // socket. Reported, never failed on — background scans are rare on a
    // stable link.
    rustix::fs::fcntl_setfl(&genl, rustix::fs::OFlags::NONBLOCK).expect("nonblocking");
    let mut poll = [rustix::event::PollFd::new(&genl, PollFlags::IN)];
    let wait = rustix::time::Timespec {
        tv_sec: 5,
        tv_nsec: 0,
    };
    let mut buf = [0u8; super::netlink::READ_LEN];
    if let Ok(ready) = rustix::event::poll(&mut poll, Some(&wait)) {
        if ready > 0 {
            if let Ok((_, n)) = rustix::net::recv(&genl, &mut buf, rustix::net::RecvFlags::empty())
            {
                for msg in super::netlink::messages(&buf[..n]) {
                    if msg.kind == family.id {
                        let command = super::netlink::genl_of(msg.body).map(|(command, _)| command);
                        eprintln!("live: ambient multicast command {command:?}");
                    }
                }
            }
        }
    }
    eprintln!("live: five quiet seconds on the joined socket");
}

/// The nl80211 resolve against the real kernel, and the signal-strategy
/// measurement: `SET_CQM` with RSSI thresholds at the first wireless
/// interface. The module re-reads the signal on a timer (see
/// `SIGNAL_SECS`); this test records what the driver answers, refusal or
/// ack — the timer stands either way (uniform across drivers, no
/// per-driver branching). Needs a radio; without one it says so and
/// passes.
#[test]
fn the_cqm_probe_records_the_driver_s_answer() {
    if std::env::var_os("SCOOTBAR_TEST_LIVE_NET").is_none() {
        return;
    }
    let (_, family, _) = super::genl_socket().expect("the generic socket opens");
    if family.id == 0 {
        eprintln!("live: no nl80211 on this machine, skipping the CQM probe");
        return;
    }
    let (genl, _, _) = super::genl_socket().expect("a second generic socket opens");
    let genl = genl.expect("nl80211 resolved, so the socket is open");
    let mut out = Vec::new();
    super::netlink::interface_request(&mut out, family.id, 77, 0);
    rustix::net::send(&genl, &out, rustix::net::SendFlags::empty()).expect("the dump sends");
    let mut buf = [0u8; super::netlink::READ_LEN];
    let mut index = 0;
    // To the terminator: leaving the DONE unread would hand the scan
    // dump a stale one.
    for _ in 0..64 {
        let (_, n) =
            rustix::net::recv(&genl, &mut buf, rustix::net::RecvFlags::empty()).expect("a reply");
        if n == 0 {
            break;
        }
        let mut done = false;
        for msg in super::netlink::messages(&buf[..n]) {
            if msg.kind == super::netlink::NLMSG_DONE {
                done = true;
                continue;
            }
            if msg.kind != family.id {
                continue;
            }
            if let Some((command, _)) = super::netlink::genl_of(msg.body) {
                if command == super::netlink::NL80211_CMD_NEW_INTERFACE {
                    // The full body: `parse_interface` strips the generic
                    // header itself.
                    let wifi = super::netlink::parse_interface(msg.body);
                    if index == 0 {
                        index = wifi.index;
                    }
                }
            }
        }
        if done {
            break;
        }
    }
    if index == 0 {
        eprintln!("live: no wireless interface on this machine, skipping the CQM probe");
        return;
    }
    // Real bytes for the fuzz corpus, when asked: the scan dump of the
    // first wireless interface.
    if let Some(dir) = std::env::var_os("SCOOTBAR_TEST_DUMP_DIR") {
        let mut out = Vec::new();
        super::netlink::scan_request(&mut out, family.id, 79, index);
        rustix::net::send(&genl, &out, rustix::net::SendFlags::empty()).expect("scan sends");
        let mut dump = Vec::new();
        for _ in 0..64 {
            let (_, n) =
                rustix::net::recv(&genl, &mut buf, rustix::net::RecvFlags::empty()).expect("scan");
            if n == 0 {
                break;
            }
            dump.extend_from_slice(&buf[..n]);
            if super::netlink::messages(&buf[..n]).any(|m| m.kind == super::netlink::NLMSG_DONE) {
                break;
            }
        }
        let path = std::path::Path::new(&dir).join("real-scan");
        std::fs::write(&path, &dump).expect("the dump writes");
        eprintln!(
            "live: wrote {} scan bytes to {}",
            dump.len(),
            path.display()
        );
    }
    let mut out = Vec::new();
    super::netlink::cqm_request(&mut out, family.id, 78, index, &[-70, -80]);
    rustix::net::send(&genl, &out, rustix::net::SendFlags::empty()).expect("the probe sends");
    let mut errno = None;
    for _ in 0..16 {
        let (_, n) =
            rustix::net::recv(&genl, &mut buf, rustix::net::RecvFlags::empty()).expect("an answer");
        if n == 0 {
            break;
        }
        for msg in super::netlink::messages(&buf[..n]) {
            if msg.kind == super::netlink::NLMSG_ERROR {
                errno = Some(super::netlink::error_of(msg.body));
            }
        }
        if errno.is_some() {
            break;
        }
    }
    eprintln!("live: SET_CQM on ifindex {index} answered {errno:?}");
    assert!(
        errno.is_some(),
        "the probe got a definitive answer, refusal or ack, which is why the signal timer stands either way"
    );
}

// ---------------------------------------------------------------------------
// The native list (`popup`) and `connect N`.
// ---------------------------------------------------------------------------

#[cfg(feature = "popup")]
mod popup_list {
    use super::*;
    use crate::popup::{Content, Kind as Widget};

    /// A wireless interface associated to `ssid`, seeing `others` beside
    /// it (SSID, signal dBm): one scan page, the associated first.
    fn wifi_many(fake: &Fake, ssid: &[u8], signal: i8, others: &[(&[u8], i8)]) {
        fake.rt(&fake::link(16, WLAN0, UP, 6, "wlan0", Some("nl80211")));
        fake.rt(&fake::addr(20, WLAN0, 2));
        fake.rt(&fake::route(24, 2, WLAN0));
        fake.genl(&fake::interface(WLAN0, "wlan0", Some(ssid)));
        let mut nets: Vec<(&[u8], i32, bool)> = vec![(ssid, i32::from(signal) * 100, true)];
        nets.extend(
            others
                .iter()
                .map(|(ssid, signal)| (*ssid, i32::from(*signal) * 100, false)),
        );
        fake.genl(&fake::scan(&nets));
        fake.genl(&fake::station(signal));
    }

    fn content_of(harness: &mut Harness) -> (bool, Content) {
        let mut content = Content::default();
        let shown = harness.popup(&mut content);
        (shown, content)
    }

    fn connect(n: i32) -> ModuleAction {
        ModuleAction::new("connect", Some(n))
    }

    #[test]
    fn the_popup_lists_the_scan_with_the_associated_selected() {
        let (mut harness, fake) = Fake::start(&Settings::default());
        wifi_many(&fake, b"Wimbly", -50, &[(b"Cafe", -60), (b"Far", -72)]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let (shown, content) = content_of(&mut harness);
        assert!(shown);
        let widgets = content.widgets();
        assert_eq!(widgets.len(), 3);
        // Rows are plain SSIDs: the strength is the bar icon's level,
        // not text bars.
        assert_eq!(content.label(&widgets[0]), "Wimbly");
        assert_eq!(content.label(&widgets[1]), "Cafe");
        assert_eq!(content.label(&widgets[2]), "Far");
        for (n, widget) in widgets.iter().enumerate() {
            assert_eq!(
                widget.kind,
                Widget::Button {
                    action: "connect",
                    arg: Some(n as i32),
                    selected: n == 0,
                    closes: true,
                },
                "row {n}"
            );
        }
        let _ = fake.sent();
    }

    #[test]
    fn unnamed_networks_are_not_rows() {
        let (mut harness, fake) = Fake::start(&Settings::default());
        wifi_many(&fake, b"Wimbly", -50, &[(b"", -60)]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let (shown, content) = content_of(&mut harness);
        assert!(shown);
        assert_eq!(content.widgets().len(), 1);
        assert_eq!(content.label(&content.widgets()[0]), "Wimbly");
        let _ = fake.sent();
    }

    #[test]
    fn a_plugged_dongle_keeps_the_open_list() {
        // The list is open on the shown radio's scan; a dongle plugged in
        // beside it sees nothing, and its empty cache must not reach the
        // rows.
        const DONGLE: u32 = 4;
        let (mut harness, fake) = Fake::start(&Settings::default());
        wifi_many(&fake, b"Wimbly", -50, &[(b"Cafe", -60), (b"Far", -72)]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let (shown, content) = content_of(&mut harness);
        assert!(shown);
        assert_eq!(content.widgets().len(), 3);
        // Complete the shown scan so anything the dongle queues behind it
        // would go out.
        let (_, genl) = fake.sent();
        fake.genl(&fake::done_seq(request_seq(
            &genl,
            crate::modules::network::netlink::NL80211_CMD_GET_SCAN,
        )));
        assert_eq!(drive(&mut harness), Update::Unchanged);
        fake.rt(&fake::link(16, DONGLE, UP, 6, "wlan1", None));
        fake.rt(&fake::addr(20, DONGLE, 2));
        fake.genl(&fake::interface(DONGLE, "wlan1", Some(b"Other")));
        assert_eq!(drive(&mut harness), Update::Unchanged);
        // Complete the station beside the shown scan: the dongle's scan
        // must never go out behind it.
        let (_, genl) = fake.sent();
        fake.genl(&fake::station(-50));
        fake.genl(&fake::done_seq(request_seq(
            &genl,
            crate::modules::network::netlink::NL80211_CMD_GET_STATION,
        )));
        assert_eq!(drive(&mut harness), Update::Unchanged);
        let (_, genl) = fake.sent();
        let targets = scan_targets(&genl);
        assert!(targets.is_empty(), "no dongle's scan goes out: {targets:?}");
        // The open list still shows the shown radio's networks.
        let (shown, content) = content_of(&mut harness);
        assert!(shown);
        assert_eq!(content.widgets().len(), 3);
        assert_eq!(content.label(&content.widgets()[0]), "Wimbly");
        let _ = fake.sent();
    }

    #[test]
    fn the_popup_rows_carry_the_strength_glyph_with_four_levels() {
        // -50 dBm is level 4, -60 level 3, -72 level 2: each row starts
        // with its network's glyph. A single glyph (or none) leaves
        // rows as plain SSIDs, as above.
        let settings = Settings {
            icon_wifi: Some(WifiIcon::Levels(['1', '2', '3', '4'])),
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        wifi_many(&fake, b"Wimbly", -50, &[(b"Cafe", -60), (b"Far", -72)]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let (shown, content) = content_of(&mut harness);
        assert!(shown);
        let widgets = content.widgets();
        assert_eq!(widgets.len(), 3);
        assert_eq!(content.label(&widgets[0]), "4 Wimbly");
        assert_eq!(content.label(&widgets[1]), "3 Cafe");
        assert_eq!(content.label(&widgets[2]), "2 Far");
        for (n, widget) in widgets.iter().enumerate() {
            assert_eq!(
                widget.kind,
                Widget::Button {
                    action: "connect",
                    arg: Some(n as i32),
                    selected: n == 0,
                    closes: true,
                },
                "row {n}"
            );
        }
        // The choice is still the SSID alone, not the row's label with
        // its glyph.
        let dir = tempdir("glyph-connect");
        let (command, file) = recording(&dir);
        let settings = Settings {
            icon_wifi: Some(WifiIcon::Levels(['1', '2', '3', '4'])),
            connect_command: command,
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        wifi_many(&fake, b"Wimbly", -50, &[]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let mut content = Content::default();
        assert!(harness.popup(&mut content));
        assert_eq!(content.label(&content.widgets()[0]), "4 Wimbly");
        let output = crate::modules::OutputView { name: None };
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Ok(Update::Unchanged)
        );
        let _ = drive(&mut harness);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Wimbly\n");
        std::fs::remove_dir_all(&dir).ok();
        let _ = fake.sent();
    }

    #[test]
    fn no_list_while_the_ssid_is_hidden_or_nothing_is_seen() {
        let settings = Settings {
            show_ssid: false,
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        wifi_many(&fake, b"Wimbly", -50, &[]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let (shown, content) = content_of(&mut harness);
        assert!(!shown);
        assert!(content.is_empty());
        let (mut harness, _fake) = Fake::start(&Settings::default());
        let (shown, content) = content_of(&mut harness);
        assert!(!shown);
        assert!(content.is_empty());
    }

    #[test]
    fn connect_takes_a_number_and_needs_a_command() {
        let (mut harness, fake) = Fake::start(&Settings::default());
        wifi_many(&fake, b"Wimbly", -50, &[]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let mut content = Content::default();
        assert!(harness.popup(&mut content));
        let output = crate::modules::OutputView { name: None };
        // No number is refused before anything runs.
        assert_eq!(
            harness.invoke(&output, &ModuleAction::new("connect", None), 1),
            Err(crate::modules::InvokeError::NeedsArg)
        );
        // Past the end, and below zero, name no network.
        for bad in [1, 99, -1, i32::MIN] {
            assert_eq!(
                harness.invoke(&output, &connect(bad), 1),
                Err(crate::modules::InvokeError::Refused("no such network")),
                "connect {bad}"
            );
        }
        // Nothing configured: refused naming why, and nothing runs.
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Err(crate::modules::InvokeError::Refused(
                "no connect command configured"
            ))
        );
        assert_eq!(harness.source_count(), 3);
        let unknown = ModuleAction::new("bogus", None);
        assert_eq!(
            harness.invoke(&output, &unknown, 1),
            Err(crate::modules::InvokeError::Unknown)
        );
        let _ = fake.sent();
    }

    /// The `connect-command` records what it was given: the SSID as one
    /// argument (`$0` after `sh -c`'s script), written to `file`.
    fn recording(dir: &std::path::Path) -> (Vec<String>, std::path::PathBuf) {
        let file = dir.join("ssid");
        (
            vec![
                "sh".to_owned(),
                "-c".to_owned(),
                format!("echo \"$0\" > {}", file.display()),
            ],
            file,
        )
    }

    fn tempdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "scootbar-network-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn connect_spawns_the_command_with_the_ssid_as_one_argument() {
        let dir = tempdir("connect");
        let (command, file) = recording(&dir);
        let settings = Settings {
            connect_command: command,
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        wifi_many(&fake, b"Wimbly", -50, &[]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let mut content = Content::default();
        assert!(harness.popup(&mut content));
        let output = crate::modules::OutputView { name: None };
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Ok(Update::Unchanged)
        );
        let _ = drive(&mut harness);
        // The SSID alone (not the row's label with its bars), as one
        // argument.
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Wimbly\n");
        // Reaped: no source past the sockets and the timer.
        assert_eq!(harness.source_count(), 3);
        std::fs::remove_dir_all(&dir).ok();
        let _ = fake.sent();
    }

    #[test]
    fn an_attacker_ssid_reaches_the_command_as_data_not_as_code() {
        // The shell target is a fixed short path: the whole evil SSID
        // must fit the radio's 32 bytes on every machine, whatever its
        // temp directory is. (Only this test uses it; cleaned first and
        // last.)
        let shell_target = std::path::PathBuf::from("/tmp/pnl-evil-P");
        std::fs::remove_file(&shell_target).ok();
        let dir = tempdir("evil");
        let (command, file) = recording(&dir);
        let settings = Settings {
            connect_command: command,
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        // Shell metacharacters, controls and non-UTF-8: the sanitizer
        // keeps the first (they are data) and folds the rest. The touch
        // targets the fixed path, so a shell would leave it there.
        // (`\xff` is built as a byte: it is not a string escape.)
        let mut evil = format!("a;b$(touch {})", shell_target.display()).into_bytes();
        evil.push(0x01);
        evil.push(0xff);
        assert!(evil.len() <= 32, "the SSID fits the radio: {}", evil.len());
        wifi_many(&fake, &evil, -60, &[]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let mut content = Content::default();
        assert!(harness.popup(&mut content));
        assert_eq!(content.widgets().len(), 1);
        let output = crate::modules::OutputView { name: None };
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Ok(Update::Unchanged)
        );
        let _ = drive(&mut harness);
        let line = crate::modules::network::netlink::sanitize(&evil);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("{line}\n"));
        // Through a shell, `$(touch ...)` would have run: nothing was
        // created beside the recording.
        assert!(
            !shell_target.exists(),
            "the SSID ran as a command: {:?}",
            std::fs::read_dir(&dir).unwrap().collect::<Vec<_>>()
        );
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_file(&shell_target).ok();
        let _ = fake.sent();
    }

    #[test]
    fn connect_names_what_the_popup_showed_not_what_moved_under_it() {
        let dir = tempdir("stale");
        let (command, file) = recording(&dir);
        let settings = Settings {
            connect_command: command,
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        wifi_many(&fake, b"Alpha", -50, &[(b"Beta", -60)]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let mut content = Content::default();
        assert!(harness.popup(&mut content));
        assert_eq!(content.widgets().len(), 2);
        // The radio roams: the scan is Beta alone now, before the popup
        // is refilled.
        fake.genl(&fake::roam(WLAN0));
        fake.genl(&fake::scan(&[(b"Beta", -6000, true)]));
        assert_eq!(drive(&mut harness), Update::Changed);
        let output = crate::modules::OutputView { name: None };
        // Row 0 showed Alpha, which is gone: refused, not connected to
        // whatever row 0 holds now.
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Err(crate::modules::InvokeError::Refused(
                "that network is no longer seen; open the list again"
            ))
        );
        assert!(!file.exists(), "a stale row connected");
        // Row 1 showed Beta, still there: connects to Beta.
        assert_eq!(
            harness.invoke(&output, &connect(1), 1),
            Ok(Update::Unchanged)
        );
        let _ = drive(&mut harness);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Beta\n");
        // Refilled, the list is Beta alone, and row 0 is Beta again
        // (-60 dBm is three bars).
        let mut content = Content::default();
        assert!(harness.popup(&mut content));
        assert_eq!(content.widgets().len(), 1);
        assert_eq!(content.label(&content.widgets()[0]), "Beta");
        std::fs::remove_dir_all(&dir).ok();
        let _ = fake.sent();
    }

    /// Whether `pid` names a live process: the tests' way to prove a
    /// replaced or dropped child is really gone (reaped, not a zombie).
    fn alive(pid: u32) -> bool {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }

    /// The pids `command` recorded so far, waiting up to 5 s for `want:
    /// a spawn writes its pidfile at once, but the test never assumes
    /// when the scheduler runs it.
    fn wait_pids(pids: &std::path::Path, want: usize) -> Vec<u32> {
        let start = std::time::Instant::now();
        loop {
            let found = std::fs::read_to_string(pids)
                .unwrap_or_default()
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect::<Vec<_>>();
            if found.len() >= want || start.elapsed() > Duration::from_secs(5) {
                return found;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A command that records its pid in `pids` and never exits: the
    /// stuck child the ticket names (`sleep infinity`). With `trap`,
    /// the shell ignores `SIGTERM` first, so only `SIGKILL` ends it.
    fn lingering(dir: &std::path::Path, trap: &str) -> Vec<String> {
        let pids = dir.join("pids");
        vec![
            "sh".to_owned(),
            "-c".to_owned(),
            format!("echo $$ >> {}; {trap}exec sleep infinity", pids.display()),
        ]
    }

    #[test]
    fn a_second_connect_replaces_the_running_one() {
        // `sleep infinity` never exits on its own: the second connect
        // ends it and starts its own, instead of being refused.
        let dir = tempdir("connect-replace");
        let settings = Settings {
            connect_command: lingering(&dir, ""),
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        wifi_many(&fake, b"Wimbly", -50, &[]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let mut content = Content::default();
        assert!(harness.popup(&mut content));
        let output = crate::modules::OutputView { name: None };
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Ok(Update::Unchanged)
        );
        assert_eq!(harness.source_count(), 4);
        let first = wait_pids(&dir.join("pids"), 1);
        assert_eq!(first.len(), 1);
        assert!(alive(first[0]), "the first connect runs");
        // The replacement: the old child is ended, the new one runs in
        // the one connect slot.
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Ok(Update::Unchanged)
        );
        let pids = wait_pids(&dir.join("pids"), 2);
        assert_eq!(pids.len(), 2);
        assert_ne!(pids[0], pids[1]);
        assert!(!alive(pids[0]), "the replaced connect is reaped");
        assert!(alive(pids[1]), "the new connect runs");
        assert_eq!(harness.source_count(), 4);
        // Dropping the module (a reload, a removal) ends the survivor.
        drop(harness);
        assert!(!alive(pids[1]), "the dropped module reaps its connect");
        std::fs::remove_dir_all(&dir).ok();
        let _ = fake.sent();
    }

    #[test]
    fn a_second_menu_replaces_the_running_one() {
        // A picker left open never blocks the next one: the second menu
        // ends it and opens fresh, as a second connect does.
        let dir = tempdir("menu-replace");
        let settings = Settings {
            menu_command: lingering(&dir, ""),
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        wifi_many(&fake, b"Wimbly", -50, &[]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let menu = ModuleAction::new("menu", None);
        let output = crate::modules::OutputView { name: None };
        assert_eq!(harness.invoke(&output, &menu, 1), Ok(Update::Unchanged));
        assert_eq!(harness.source_count(), 4);
        let first = wait_pids(&dir.join("pids"), 1);
        assert_eq!(first.len(), 1);
        assert!(alive(first[0]), "the first menu runs");
        assert_eq!(harness.invoke(&output, &menu, 1), Ok(Update::Unchanged));
        let pids = wait_pids(&dir.join("pids"), 2);
        assert_eq!(pids.len(), 2);
        assert_ne!(pids[0], pids[1]);
        assert!(!alive(pids[0]), "the replaced menu is reaped");
        assert!(alive(pids[1]), "the new menu runs");
        assert_eq!(harness.source_count(), 4);
        drop(harness);
        assert!(!alive(pids[1]), "the dropped module reaps its menu");
        std::fs::remove_dir_all(&dir).ok();
        let _ = fake.sent();
    }

    #[test]
    fn dropping_the_module_ends_both_running_children() {
        // A reload while a connect and a menu both run: neither child
        // outlives the module, unreaped.
        let dir = tempdir("drop-both");
        let menu_dir = dir.join("menu");
        let connect_dir = dir.join("connect");
        std::fs::create_dir_all(&menu_dir).unwrap();
        std::fs::create_dir_all(&connect_dir).unwrap();
        let settings = Settings {
            menu_command: lingering(&menu_dir, ""),
            connect_command: lingering(&connect_dir, ""),
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        wifi_many(&fake, b"Wimbly", -50, &[]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let output = crate::modules::OutputView { name: None };
        assert_eq!(
            harness.invoke(&output, &ModuleAction::new("menu", None), 1),
            Ok(Update::Unchanged)
        );
        let mut content = Content::default();
        assert!(harness.popup(&mut content));
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Ok(Update::Unchanged)
        );
        let menu = wait_pids(&menu_dir.join("pids"), 1);
        let connect = wait_pids(&connect_dir.join("pids"), 1);
        assert!(alive(menu[0]) && alive(connect[0]), "both children run");
        drop(harness);
        assert!(!alive(menu[0]), "the dropped module reaps its menu");
        assert!(!alive(connect[0]), "the dropped module reaps its connect");
        std::fs::remove_dir_all(&dir).ok();
        let _ = fake.sent();
    }

    #[test]
    fn a_child_that_ignores_sigterm_is_killed() {
        // `trap '' TERM` survives the `exec` (ignored dispositions do),
        // so `SIGTERM` never ends it: the replacement escalates to
        // `SIGKILL`, and the drop does too.
        let dir = tempdir("sigterm-proof");
        let settings = Settings {
            connect_command: lingering(&dir, "trap '' TERM; "),
            ..Settings::default()
        };
        let (mut harness, fake) = Fake::start(&settings);
        wifi_many(&fake, b"Wimbly", -50, &[]);
        assert_eq!(drive(&mut harness), Update::Changed);
        let mut content = Content::default();
        assert!(harness.popup(&mut content));
        let output = crate::modules::OutputView { name: None };
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Ok(Update::Unchanged)
        );
        let first = wait_pids(&dir.join("pids"), 1);
        assert!(alive(first[0]), "the proof child runs");
        assert_eq!(
            harness.invoke(&output, &connect(0), 1),
            Ok(Update::Unchanged)
        );
        let pids = wait_pids(&dir.join("pids"), 2);
        assert_eq!(pids.len(), 2);
        assert!(
            !alive(pids[0]),
            "a SIGTERM-proof child is killed on replacement"
        );
        assert!(alive(pids[1]), "the new connect runs");
        drop(harness);
        assert!(!alive(pids[1]), "a SIGTERM-proof child is killed on drop");
        std::fs::remove_dir_all(&dir).ok();
        let _ = fake.sent();
    }
}
