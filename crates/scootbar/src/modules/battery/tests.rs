//! The battery module through the harness: sysfs fixtures stand in for
//! the machine's batteries, and crafted datagrams on a socketpair stand in
//! for the kernel's uevents (userspace cannot send on
//! `NETLINK_KOBJECT_UEVENT` at all — a send there fails `EPERM` on the
//! Asahi box — so the production socket is exercised by construction only,
//! and everything else by fixtures here).

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::event::PollFlags;

use super::{
    Batteries, Settings, has_power_supply, merge, parse_capacity, read_level, recv_lost_events,
    start_with,
};
use crate::action::Action;
use crate::modules::harness::Harness;
use crate::modules::{Class, Update};

/// One fixture battery's files: `None` leaves the file out.
#[derive(Clone, Copy)]
pub(super) struct Fixture {
    pub(super) present: Option<&'static str>,
    pub(super) capacity: Option<&'static str>,
    pub(super) status: Option<&'static str>,
}

fn unique(slug: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Unique per call, per thread and per process: `cargo test` shares a
    // process across threads, and nextest does not. Leaked on purpose (a
    // few tiny files per run): the stand-in cannot hand a guard back
    // through its signature. (The snapshots keep the same convention
    // under `$TMPDIR/scootbar-snapshots/`.)
    std::env::temp_dir().join(format!(
        "scootbar-battery-{slug}-{}-{:?}-{id}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// Writes (or rewrites) one fixture battery: `type` is always `Battery`
/// unless `write_typed` says otherwise.
pub(super) fn write_fixture(root: &Path, name: &str, fixture: Fixture) {
    write_typed(root, name, "Battery", fixture);
}

fn write_typed(root: &Path, name: &str, kind: &str, fixture: Fixture) {
    let battery = root.join(name);
    std::fs::create_dir_all(&battery).unwrap();
    std::fs::write(battery.join("type"), kind).unwrap();
    write_opt(&battery, "present", fixture.present);
    write_opt(&battery, "capacity", fixture.capacity);
    write_opt(&battery, "status", fixture.status);
}

fn write_opt(battery: &Path, file: &str, contents: Option<&str>) {
    let path = battery.join(file);
    match contents {
        Some(text) => std::fs::write(path, text).unwrap(),
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// A datagram socketpair: the test writes crafted uevents to the peer, the
/// module reads them as if from its netlink tap. Nonblocking, so a fake
/// event with nothing to read ends in `Unchanged`, not a hung test.
pub(super) fn socketpair() -> (UnixDatagram, OwnedFd) {
    let (peer, ours) = UnixDatagram::pair().unwrap();
    peer.set_nonblocking(true).unwrap();
    ours.set_nonblocking(true).unwrap();
    (peer, OwnedFd::from(ours))
}

/// A fixture root holding `batteries`, with the module started on it.
fn started_with(
    settings: &Settings,
    batteries: &[(&'static str, Fixture)],
) -> (PathBuf, UnixDatagram, Harness) {
    let root = unique("test");
    std::fs::create_dir_all(&root).unwrap();
    for &(name, fixture) in batteries {
        write_fixture(&root, name, fixture);
    }
    let (peer, ours) = socketpair();
    let module = start_with(settings, &root, Some(ours)).unwrap();
    (root, peer, Harness::new(module))
}

fn discharging(capacity: &'static str) -> (&'static str, Fixture) {
    (
        "BAT0",
        Fixture {
            present: Some("1"),
            capacity: Some(capacity),
            status: Some("Discharging"),
        },
    )
}

/// One crafted uevent datagram: NUL-separated fields, as the kernel sends.
fn uevent(subsystem: &str) -> Vec<u8> {
    format!("ACTION=change\0DEVPATH=/devices/x\0SUBSYSTEM={subsystem}\0SEQNUM=1\0").into_bytes()
}

fn power_uevent(peer: &UnixDatagram) {
    peer.send(&uevent("power_supply")).unwrap();
}

fn changed(harness: &mut Harness) {
    assert_eq!(harness.wait(Duration::from_secs(1)), Some(Update::Changed));
}

#[test]
fn a_battery_shows_its_percent_state_and_class() {
    let (_root, peer, mut harness) = started_with(&Settings::default(), &[discharging("72")]);
    assert_eq!(harness.view().text(), "72%");
    assert_eq!(harness.view().tooltip(), "Discharging 72%");
    assert_eq!(harness.view().class(), Class::Normal);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["percent"].as_u64(), Some(72));
    assert_eq!(value["state"], "discharging");
    assert_eq!(value["batteries"].as_u64(), Some(1));
    // A fresh event with nothing new is not a redraw.
    power_uevent(&peer);
    assert_eq!(
        harness.wait(Duration::from_secs(1)),
        Some(Update::Unchanged)
    );
}

#[test]
fn warn_and_urgent_follow_the_thresholds_by_level_alone() {
    let settings = Settings {
        warn_below: 20,
        urgent_below: 10,
        ..Settings::default()
    };
    let (root, peer, mut harness) = started_with(&settings, &[discharging("21")]);
    let at = |root: &Path, peer: &UnixDatagram, harness: &mut Harness, capacity: &'static str| {
        write_fixture(
            root,
            "BAT0",
            Fixture {
                present: Some("1"),
                capacity: Some(capacity),
                status: Some("Charging"),
            },
        );
        power_uevent(peer);
        changed(harness);
        harness.view().class()
    };
    assert_eq!(harness.view().class(), Class::Normal);
    // Charging at 5% is still a battery at 5%: the class is the level's.
    assert_eq!(at(&root, &peer, &mut harness, "20"), Class::Warn);
    assert_eq!(at(&root, &peer, &mut harness, "11"), Class::Warn);
    assert_eq!(at(&root, &peer, &mut harness, "10"), Class::Urgent);
    assert_eq!(at(&root, &peer, &mut harness, "0"), Class::Urgent);
}

#[test]
fn no_battery_is_an_unavailable_with_a_reason() {
    let root = unique("empty");
    std::fs::create_dir_all(&root).unwrap();
    let (_peer, ours) = socketpair();
    let error = match start_with(&Settings::default(), &root, Some(ours)) {
        Ok(_) => panic!("an empty power_supply directory started a battery"),
        Err(error) => error,
    };
    assert!(!error.trim().is_empty(), "unavailable, silently");
    assert!(error.contains("no battery"), "{error}");
}

#[test]
fn a_missing_directory_is_unavailable_too() {
    let root = unique("absent");
    let (_peer, ours) = socketpair();
    assert!(start_with(&Settings::default(), &root, Some(ours)).is_err());
}

#[test]
fn unusable_entries_are_skipped_not_shown() {
    // Every one of these is skipped: the mains adapter, an empty slot, a
    // missing capacity, garbage capacities, and a battery with no `type`
    // file. The one good battery is all the level sees.
    let root = unique("skip");
    std::fs::create_dir_all(&root).unwrap();
    write_typed(
        &root,
        "AC",
        "Mains",
        Fixture {
            present: None,
            capacity: None,
            status: None,
        },
    );
    std::fs::write(root.join("AC").join("online"), "1").unwrap();
    write_fixture(
        &root,
        "BAT0",
        Fixture {
            present: Some("0"),
            capacity: Some("50"),
            status: Some("Discharging"),
        },
    );
    for name in ["BAT1", "BAT2", "BAT3", "BAT4", "BAT5"] {
        write_fixture(
            &root,
            name,
            Fixture {
                present: Some("1"),
                capacity: None,
                status: Some("Discharging"),
            },
        );
    }
    std::fs::write(root.join("BAT2").join("capacity"), "").unwrap();
    std::fs::write(root.join("BAT3").join("capacity"), "abc").unwrap();
    std::fs::write(root.join("BAT4").join("capacity"), "-5").unwrap();
    std::fs::write(root.join("BAT5").join("capacity"), "72%").unwrap();
    // A battery with no `type` file at all: not identifiably a battery.
    let typed = root.join("BAT7");
    std::fs::create_dir_all(&typed).unwrap();
    std::fs::write(typed.join("capacity"), "63").unwrap();
    std::fs::write(typed.join("status"), "Discharging").unwrap();
    // And the one good one the level is.
    write_fixture(
        &root,
        "BAT9",
        Fixture {
            present: Some("1"),
            capacity: Some("63"),
            status: Some("Charging"),
        },
    );
    let (peer, ours) = socketpair();
    let harness = Harness::new(start_with(&Settings::default(), &root, Some(ours)).unwrap());
    assert_eq!(harness.view().text(), "63%");
    assert_eq!(
        harness.value_on(None).unwrap()["batteries"].as_u64(),
        Some(1)
    );
    let _ = peer;
}

#[test]
fn one_bad_entry_skips_itself_not_the_directory() {
    use std::os::unix::ffi::OsStrExt;
    let (root, _peer, harness) = started_with(&Settings::default(), &[discharging("72")]);
    // A filename that is not UTF-8: the listing skips it, and the good
    // battery still reads.
    let bad = std::ffi::OsStr::from_bytes(b"\xff\xfe");
    std::fs::write(root.join(bad), "junk").unwrap();
    assert_eq!(harness.view().text(), "72%");
    assert_eq!(read_level(&root, Batteries::Combine).unwrap().percent, 72);
}

#[test]
fn capacity_past_100_is_clamped() {
    let (_root, _peer, harness) = started_with(
        &Settings::default(),
        &[(
            "BAT0",
            Fixture {
                present: Some("1"),
                capacity: Some("104"),
                status: Some("Full"),
            },
        )],
    );
    assert_eq!(harness.view().text(), "100%");
}

#[test]
fn invented_status_strings_are_unknown_not_a_refusal() {
    for (status, word, query) in [
        ("Blorp", "Unknown", "unknown"),
        ("Trickle", "Unknown", "unknown"),
        ("Not charging", "Not charging", "not-charging"),
        ("Full", "Full", "full"),
        ("Charging", "Charging", "charging"),
    ] {
        let (_root, _peer, harness) = started_with(
            &Settings::default(),
            &[(
                "BAT0",
                Fixture {
                    present: Some("1"),
                    capacity: Some("50"),
                    status: Some(status),
                },
            )],
        );
        let view = harness.view();
        assert_eq!(view.text(), "50%", "status {status}");
        assert_eq!(view.tooltip(), format!("{word} 50%"), "status {status}");
        assert_eq!(harness.value_on(None).unwrap()["state"], query);
    }
}

#[test]
fn combine_means_and_first_means_first() {
    let pair = |name: &'static str, capacity: &'static str, status: &'static str| {
        (
            name,
            Fixture {
                present: Some("1"),
                capacity: Some(capacity),
                status: Some(status),
            },
        )
    };
    let combine = Settings {
        batteries: Batteries::Combine,
        ..Settings::default()
    };
    let (_root, _peer, harness) = started_with(
        &combine,
        &[
            pair("BAT0", "80", "Charging"),
            pair("BAT1", "40", "Discharging"),
        ],
    );
    // The mean, with discharging winning the merge (power going out is
    // what matters), over both batteries.
    assert_eq!(harness.view().text(), "60%");
    assert_eq!(harness.value_on(None).unwrap()["state"], "discharging");
    assert_eq!(
        harness.value_on(None).unwrap()["batteries"].as_u64(),
        Some(2)
    );

    let first = Settings {
        batteries: Batteries::First,
        ..Settings::default()
    };
    let (_root, _peer, harness) = started_with(
        &first,
        &[
            pair("BAT0", "80", "Charging"),
            pair("BAT1", "40", "Discharging"),
        ],
    );
    assert_eq!(harness.view().text(), "80%");
    assert_eq!(harness.value_on(None).unwrap()["state"], "charging");
    assert_eq!(
        harness.value_on(None).unwrap()["batteries"].as_u64(),
        Some(1)
    );
}

#[test]
fn an_unrelated_uevent_changes_nothing() {
    let (root, peer, mut harness) = started_with(&Settings::default(), &[discharging("72")]);
    // The capacity moves, but no power_supply event arrives: the view is
    // stale until one does (the discharge timer covers this live; here
    // only the tap is driven).
    std::fs::write(root.join("BAT0").join("capacity"), "71").unwrap();
    peer.send(&uevent("usb")).unwrap();
    assert_eq!(
        harness.wait(Duration::from_secs(1)),
        Some(Update::Unchanged)
    );
    assert_eq!(harness.view().text(), "72%");
    power_uevent(&peer);
    changed(&mut harness);
    assert_eq!(harness.view().text(), "71%");
}

#[test]
fn a_uevent_storm_is_one_re_read_per_turn() {
    let (root, peer, mut harness) = started_with(&Settings::default(), &[discharging("72")]);
    std::fs::write(root.join("BAT0").join("capacity"), "70").unwrap();
    for _ in 0..200 {
        peer.send(&uevent("power_supply")).unwrap();
    }
    // However many datagrams arrived, the view settles within a few turns
    // (64 datagrams a turn), and the level is what sysfs says.
    let mut changed_turns = 0;
    for _ in 0..8 {
        match harness.wait(Duration::from_secs(1)) {
            Some(Update::Changed) => changed_turns += 1,
            Some(Update::Unchanged) => {}
            None => break,
        }
    }
    assert_eq!(harness.view().text(), "70%");
    assert!(
        (1..=4).contains(&changed_turns),
        "changed in {changed_turns} turns"
    );
}

#[test]
fn the_timer_runs_only_while_discharging() {
    // Discharging: the tap plus the timer.
    let (_root, _peer, harness) = started_with(&Settings::default(), &[discharging("72")]);
    assert_eq!(harness.source_count(), 2);
    // Full, then charging: the tap alone.
    let (root, peer, mut harness) = started_with(
        &Settings::default(),
        &[(
            "BAT0",
            Fixture {
                present: Some("1"),
                capacity: Some("100"),
                status: Some("Full"),
            },
        )],
    );
    assert_eq!(harness.source_count(), 1);
    std::fs::write(root.join("BAT0").join("status"), "Charging").unwrap();
    power_uevent(&peer);
    changed(&mut harness);
    assert_eq!(harness.source_count(), 1);
    // Back to discharging: the timer returns.
    std::fs::write(root.join("BAT0").join("status"), "Discharging").unwrap();
    power_uevent(&peer);
    changed(&mut harness);
    assert_eq!(harness.source_count(), 2);
}

#[test]
fn the_timer_re_reads() {
    let (root, _peer, mut harness) = started_with(&Settings::default(), &[discharging("72")]);
    std::fs::write(root.join("BAT0").join("capacity"), "69").unwrap();
    // Source 1 is the discharge timer (source 0 is the tap).
    assert_eq!(harness.deliver(1, PollFlags::IN), Update::Changed);
    assert_eq!(harness.view().text(), "69%");
}

#[test]
fn a_removed_battery_hides_and_a_returned_one_shows() {
    let (root, peer, mut harness) = started_with(&Settings::default(), &[discharging("72")]);
    std::fs::remove_dir_all(root.join("BAT0")).unwrap();
    power_uevent(&peer);
    changed(&mut harness);
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
    // The tap stays (the appearance watch); the timer is dropped.
    assert_eq!(harness.source_count(), 1);
    write_fixture(
        &root,
        "BAT0",
        Fixture {
            present: Some("1"),
            capacity: Some("72"),
            status: Some("Discharging"),
        },
    );
    power_uevent(&peer);
    changed(&mut harness);
    assert_eq!(harness.view().text(), "72%");
    assert_eq!(harness.source_count(), 2);
}

#[test]
fn on_low_fires_once_per_downward_crossing() {
    let argv = vec!["notify-send".to_owned(), "low".to_owned()];
    let settings = Settings {
        urgent_below: 10,
        on_low: Some(argv.clone()),
        ..Settings::default()
    };
    let (root, peer, mut harness) = started_with(&settings, &[discharging("50")]);
    let set = |root: &Path, peer: &UnixDatagram, harness: &mut Harness, capacity: &'static str| {
        write_fixture(
            root,
            "BAT0",
            Fixture {
                present: Some("1"),
                capacity: Some(capacity),
                status: Some("Discharging"),
            },
        );
        power_uevent(peer);
        changed(harness);
    };
    // Starting above: armed, nothing staged.
    assert_eq!(harness.take_action(), None);
    // Across: one hook.
    set(&root, &peer, &mut harness, "9");
    assert_eq!(harness.take_action(), Some(Action::Exec(argv.clone())));
    // Taken once: staying below fires nothing more.
    assert_eq!(harness.take_action(), None);
    set(&root, &peer, &mut harness, "8");
    assert_eq!(harness.take_action(), None);
    // Back above re-arms; crossing again fires again.
    set(&root, &peer, &mut harness, "50");
    assert_eq!(harness.take_action(), None);
    set(&root, &peer, &mut harness, "10");
    assert_eq!(harness.take_action(), Some(Action::Exec(argv.clone())));
}

#[test]
fn on_low_does_not_fire_for_a_start_below_the_threshold() {
    let argv = vec!["notify-send".to_owned(), "low".to_owned()];
    let settings = Settings {
        urgent_below: 10,
        on_low: Some(argv),
        ..Settings::default()
    };
    // Starting below is not a crossing: no hook until it rises and drops.
    let (_root, _peer, mut harness) = started_with(&settings, &[discharging("5")]);
    assert_eq!(harness.take_action(), None);
}

#[test]
fn without_on_low_nothing_is_staged() {
    let (root, peer, mut harness) = started_with(&Settings::default(), &[discharging("50")]);
    std::fs::write(root.join("BAT0").join("capacity"), "1").unwrap();
    power_uevent(&peer);
    changed(&mut harness);
    assert_eq!(harness.take_action(), None);
}

#[test]
fn against_the_real_power_supply() {
    // Where the machine allows: a real battery reads sane, and a missing
    // or empty directory is a named Unavailable. Both arms run without a
    // fixture.
    let root = Path::new("/sys/class/power_supply");
    let (_peer, ours) = socketpair();
    match start_with(&Settings::default(), root, Some(ours)) {
        Ok(module) => {
            let harness = Harness::new(module);
            let view = harness.view();
            assert!(!view.is_empty());
            let percent: u32 = view.text().strip_suffix('%').unwrap().parse().unwrap();
            assert!(percent <= 100);
            let value = harness.value_on(None).unwrap();
            assert_eq!(value["percent"].as_u64(), Some(u64::from(percent)));
            assert!(harness.source_count() <= 2);
        }
        Err(why) => {
            assert!(!why.trim().is_empty(), "unavailable, silently");
        }
    }
}

#[test]
fn a_real_uevent_socket_builds() {
    // The production tap constructs on Linux; userspace cannot send on it
    // (EPERM, measured), so this asserts construction only, with a
    // zero-timeout poll to prove it never blocks the loop.
    let fd = super::uevent_socket().unwrap();
    let mut fds = [rustix::event::PollFd::from_borrowed_fd(
        fd.as_fd(),
        PollFlags::IN,
    )];
    let timeout = rustix::event::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let _ = rustix::event::poll(&mut fds, Some(&timeout)).unwrap();
}

/// `SUBSYSTEM=power_supply` matched whole, or not at all.
#[test]
fn the_uevent_scan_matches_whole_fields() {
    assert!(has_power_supply(
        b"ACTION=change\0SUBSYSTEM=power_supply\0SEQNUM=1\0"
    ));
    assert!(has_power_supply(b"SUBSYSTEM=power_supply"));
    assert!(!has_power_supply(b""));
    assert!(!has_power_supply(b"ACTION=change\0SUBSYSTEM=usb\0"));
    assert!(!has_power_supply(
        b"ACTION=change\0SUBSYSTEM=power_supply_now\0"
    ));
    assert!(!has_power_supply(
        b"ACTION=change\0SUBSYSTEMX=power_supply\0"
    ));
    assert!(!has_power_supply(b"SUBSYSTEM=power_supplyX"));
}

#[test]
fn capacities_parse_strictly() {
    assert_eq!(parse_capacity("0"), Some(0));
    assert_eq!(parse_capacity("100"), Some(100));
    assert_eq!(parse_capacity("104"), Some(104));
    for bad in ["", " ", "abc", "-5", "+5", "72%", "7 2", "0x10", "５"] {
        assert_eq!(parse_capacity(bad), None, "{bad:?}");
    }
}

#[test]
fn states_merge_toward_what_matters() {
    use super::State;
    assert_eq!(merge(State::Full, State::Full), State::Full);
    assert_eq!(merge(State::Unknown, State::Full), State::Full);
    assert_eq!(merge(State::Full, State::Unknown), State::Full);
    assert_eq!(merge(State::Charging, State::Full), State::Charging);
    assert_eq!(
        merge(State::Charging, State::Discharging),
        State::Discharging
    );
    assert_eq!(merge(State::Unknown, State::Unknown), State::Unknown);
}

#[test]
fn levels_read_like_the_ticket() {
    // The Asahi shape: a battery plus its mains adapter, the adapter
    // skipped, `capacity` used (not `charge_now` / `charge_full`).
    let root = unique("ticket");
    std::fs::create_dir_all(&root).unwrap();
    write_typed(
        &root,
        "macsmc-ac",
        "Mains",
        Fixture {
            present: None,
            capacity: None,
            status: None,
        },
    );
    std::fs::write(root.join("macsmc-ac").join("online"), "1").unwrap();
    write_fixture(
        &root,
        "macsmc-battery",
        Fixture {
            present: Some("1"),
            capacity: Some("100"),
            status: Some("Full"),
        },
    );
    // Charge files the module must ignore sit beside capacity.
    std::fs::write(root.join("macsmc-battery").join("charge_now"), "3649000").unwrap();
    std::fs::write(root.join("macsmc-battery").join("charge_full"), "3840000").unwrap();
    let level = read_level(&root, Batteries::Combine).unwrap();
    assert_eq!(level.percent, 100);
    assert_eq!(level.state, super::State::Full);
    assert_eq!(level.count, 1);
}

/// A netlink overflow (`ENOBUFS`) lost events, so it counts as a power
/// change; an empty queue (`AGAIN`) does not.
#[test]
fn a_lost_event_error_counts_as_a_power_change() {
    use rustix::io::Errno;
    assert!(recv_lost_events(Errno::NOBUFS));
    assert!(recv_lost_events(Errno::INTR));
    assert!(!recv_lost_events(Errno::AGAIN));
}
