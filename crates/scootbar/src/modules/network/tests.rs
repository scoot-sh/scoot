//! The network module through the harness: canned links, addresses,
//! routes and nl80211 notices in, views and queries out. Every `Harness`
//! call below is a real one (the registry test counts them); `/bin/sh`
//! is the one outside assumption, for the menu's stdin test (this crate
//! builds Linux-only, where it exists).

use std::time::Duration;

use rustix::event::PollFlags;

use super::Settings;
use super::fake::{self, Fake};
use crate::action::ModuleAction;
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
fn wifi_shows_ssid_and_bars() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    plug(&fake);
    route_via(&fake, ETH0);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "eth0");
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    // The default route moved to wlan0 with the canned route.
    assert_eq!(harness.view().text(), "Wimbly ▂▄▆█");
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
    assert_eq!(harness.view().text(), "WiFi ▂▄");
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
    // The scan dump is outstanding (the timer's station waits behind it).
    let (_, genl) = fake.sent();
    let scan = scans(&genl);
    assert_eq!(scan.len(), 1);
    // A new scan finishes while it is in flight: queued behind it.
    fake.genl(&fake::scan_done());
    assert_eq!(drive(&mut harness), Update::Unchanged);
    let (_, genl) = fake.sent();
    assert!(dumps(&genl).is_empty(), "nothing while busy");
    // Its terminator releases the queue in order: the timer's station,
    // then the scan.
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
fn a_finished_scan_refreshes_the_list() {
    let (mut harness, fake) = Fake::start(&Settings::default());
    wifi(&fake, b"Wimbly", -54);
    assert_eq!(drive(&mut harness), Update::Changed);
    assert!(harness.view().text().starts_with("Wimbly"));
    // Drain the start-up scan and station: the re-dump below must go out
    // at once (nothing in flight), resetting the list, so the new page is
    // all there is.
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
    // The kernel finished a scan: the module re-dumps, and the new list
    // (with a stronger Wimbly) is what shows.
    fake.genl(&fake::scan_done());
    assert_eq!(drive(&mut harness), Update::Unchanged);
    fake.genl(&fake::scan(&[(b"Wimbly", -5000, true)]));
    assert_eq!(drive(&mut harness), Update::Changed);
    assert_eq!(harness.view().text(), "Wimbly ▂▄▆█");
    let value = harness.value_on(None).expect("a value");
    assert_eq!(value["signal"], -50, "the new page replaced the old");
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
    assert_eq!(harness.view().text(), "FarAway ▂");
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
    assert_eq!(harness.view().text(), "FarAway ▂");
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
    // A second click while it runs opens nothing more.
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

/// The joined multicast, against the real kernel: with the mask joining
/// bit `id - 1`, the socket hears the scan group (a scan completing) and
/// the mlme group. Best-effort asks NetworkManager for a fresh scan
/// first; without one the wait is for its periodic background scan.
/// Read-only and non-disruptive: nothing disconnects, roams or
/// re-associates, and the socket sends nothing (so any nl80211 message on
/// it is multicast, not a reply).
#[test]
fn multicast_notices_arrive_on_the_joined_groups() {
    if std::env::var_os("SCOOTBAR_TEST_LIVE_NET").is_none() {
        return;
    }
    let (genl, family, joined) = super::genl_socket().expect("the generic socket opens");
    if family.id == 0 {
        eprintln!("live: no nl80211 on this machine, skipping the multicast wait");
        return;
    }
    eprintln!(
        "live: nl80211 id {} scan {} mlme {} joined {joined}",
        family.id, family.scan, family.mlme
    );
    assert!(joined, "both groups fit the mask on this machine");
    let genl = genl.expect("nl80211 resolved, so the socket is open");
    // A fresh scan's completion notice, if NetworkManager allows one.
    let rescan = std::process::Command::new("nmcli")
        .args(["device", "wifi", "rescan", "ifname", "wlan0"])
        .output()
        .map(|out| out.status.code());
    eprintln!("live: rescan request: {rescan:?}");
    rustix::fs::fcntl_setfl(&genl, rustix::fs::OFlags::NONBLOCK).expect("nonblocking");
    let mut poll = [rustix::event::PollFd::new(&genl, PollFlags::IN)];
    let second = rustix::time::Timespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    let mut buf = [0u8; super::netlink::READ_LEN];
    let begin = std::time::Instant::now();
    while begin.elapsed() < Duration::from_secs(150) {
        match rustix::event::poll(&mut poll, Some(&second)) {
            Ok(0) | Err(_) => continue,
            Ok(_) => {}
        }
        let Ok((_, n)) = rustix::net::recv(&genl, &mut buf, rustix::net::RecvFlags::empty()) else {
            continue;
        };
        for msg in super::netlink::messages(&buf[..n]) {
            if msg.kind != family.id {
                continue;
            }
            let command = super::netlink::genl_of(msg.body).map(|(command, _)| command);
            eprintln!(
                "live: multicast command {command:?} after {:?}",
                begin.elapsed()
            );
            return;
        }
    }
    panic!("no nl80211 multicast in 150 s on the joined socket");
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
