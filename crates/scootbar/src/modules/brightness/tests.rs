//! The brightness module through the harness: sysfs fixtures stand in
//! for the machine's backlights, and crafted datagrams on a socketpair
//! stand in for the kernel's uevents (userspace cannot send on
//! `NETLINK_KOBJECT_UEVENT` at all — a send there fails `EPERM` on the
//! Asahi box — so the production socket is exercised by construction only,
//! and everything else by fixtures here). Fixture directories stay
//! writable, so `invoke` writes land in them and are read back.

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::event::PollFlags;

use super::{
    Settings, has_backlight, parse_raw, percent_to_raw, raw_to_percent, read_level, start_with,
};
use crate::action::Action;
use crate::modules::harness::Harness;
use crate::modules::{Class, InvokeError, OutputView, Update};

/// One fixture backlight's `brightness` file: `None` leaves the file out.
/// The `max_brightness` value goes to [`write_fixture`] separately, since
/// a missing max is its own case.
#[derive(Clone, Copy)]
pub(super) struct Fixture {
    pub(super) raw: Option<&'static str>,
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
        "scootbar-brightness-{slug}-{}-{:?}-{id}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// Writes (or rewrites) one fixture backlight with `max_brightness` `max`.
pub(super) fn write_fixture(root: &Path, name: &str, fixture: Fixture, max: u32) {
    let device = root.join(name);
    std::fs::create_dir_all(&device).unwrap();
    match fixture.raw {
        Some(text) => std::fs::write(device.join("brightness"), text).unwrap(),
        None => {
            let _ = std::fs::remove_file(device.join("brightness"));
        }
    }
    std::fs::write(device.join("max_brightness"), max.to_string()).unwrap();
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

/// A fixture root holding `devices`, with the module started on it.
fn started_with(
    settings: &Settings,
    devices: &[(&'static str, Fixture, u32)],
) -> (PathBuf, UnixDatagram, Harness) {
    let root = unique("test");
    std::fs::create_dir_all(&root).unwrap();
    for &(name, fixture, max) in devices {
        write_fixture(&root, name, fixture, max);
    }
    let (peer, ours) = socketpair();
    let module = start_with(settings, &root, Some(ours)).unwrap();
    (root, peer, Harness::new(module))
}

fn panel(raw: &'static str) -> (&'static str, Fixture, u32) {
    ("apple-panel-bl", Fixture { raw: Some(raw) }, 509)
}

/// One crafted uevent datagram: NUL-separated fields, as the kernel sends.
fn uevent(subsystem: &str) -> Vec<u8> {
    format!("ACTION=change\0DEVPATH=/devices/x\0SUBSYSTEM={subsystem}\0SEQNUM=1\0").into_bytes()
}

fn backlight_uevent(peer: &UnixDatagram) {
    peer.send(&uevent("backlight")).unwrap();
}

fn changed(harness: &mut Harness) {
    assert_eq!(harness.wait(Duration::from_secs(1)), Some(Update::Changed));
}

fn brightness_of(root: &Path, name: &str) -> String {
    std::fs::read_to_string(root.join(name).join("brightness")).unwrap()
}

#[test]
fn a_backlight_shows_its_percent_device_and_class() {
    let (_root, peer, mut harness) = started_with(&Settings::default(), &[panel("251")]);
    // 251 of 509 rounds to 49.
    assert_eq!(harness.view().text(), "49%");
    assert_eq!(harness.view().tooltip(), "apple-panel-bl: 49%");
    assert_eq!(harness.view().class(), Class::Normal);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["percent"].as_u64(), Some(49));
    assert_eq!(value["device"], "apple-panel-bl");
    // The module owns one fd, the uevent tap: no timer, ever.
    assert_eq!(harness.source_count(), 1);
    // A fresh event with nothing new is not a redraw.
    backlight_uevent(&peer);
    assert_eq!(
        harness.wait(Duration::from_secs(1)),
        Some(Update::Unchanged)
    );
}

#[test]
fn no_backlight_is_an_unavailable_with_a_reason() {
    let root = unique("empty");
    std::fs::create_dir_all(&root).unwrap();
    let (_peer, ours) = socketpair();
    let error = match start_with(&Settings::default(), &root, Some(ours)) {
        Ok(_) => panic!("an empty backlight directory started a brightness module"),
        Err(error) => error,
    };
    assert!(!error.trim().is_empty(), "unavailable, silently");
    assert!(error.contains("no backlight"), "{error}");
}

#[test]
fn a_missing_directory_is_unavailable_too() {
    let root = unique("absent");
    let (_peer, ours) = socketpair();
    assert!(start_with(&Settings::default(), &root, Some(ours)).is_err());
}

#[test]
fn a_pinned_device_that_is_missing_names_it() {
    let root = unique("pinned");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "other-bl", Fixture { raw: Some("3") }, 10);
    let (_peer, ours) = socketpair();
    let settings = Settings {
        device: Some("acpi_video0".to_owned()),
        ..Settings::default()
    };
    let error = match start_with(&settings, &root, Some(ours)) {
        Ok(_) => panic!("a missing pinned device started a brightness module"),
        Err(error) => error,
    };
    assert!(error.contains("acpi_video0"), "{error}");
}

#[test]
fn unusable_entries_are_skipped_not_shown() {
    // Every one of these is skipped: a missing brightness, a missing max,
    // a zero range, garbage values, and an empty range file. The one good
    // device is all the level sees.
    let root = unique("skip");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "a-missing-raw", Fixture { raw: None }, 100);
    let no_max = root.join("b-missing-max");
    std::fs::create_dir_all(&no_max).unwrap();
    std::fs::write(no_max.join("brightness"), "50").unwrap();
    write_fixture(&root, "c-zero-max", Fixture { raw: Some("0") }, 0);
    write_fixture(&root, "d-garbage-raw", Fixture { raw: Some("abc") }, 100);
    write_fixture(&root, "e-garbage-max", Fixture { raw: Some("50") }, 100);
    std::fs::write(root.join("e-garbage-max").join("max_brightness"), "10x").unwrap();
    write_fixture(&root, "f-empty-raw", Fixture { raw: Some("") }, 100);
    // A name that could escape the class directory: never joined.
    assert!(read_level(&root, Some("../escape")).is_none());
    assert!(read_level(&root, Some("a/missing-raw")).is_none());
    // And the one good one the level is.
    write_fixture(&root, "g-good", Fixture { raw: Some("63") }, 100);
    let (peer, ours) = socketpair();
    let harness = Harness::new(start_with(&Settings::default(), &root, Some(ours)).unwrap());
    assert_eq!(harness.view().text(), "63%");
    assert_eq!(harness.value_on(None).unwrap()["device"], "g-good");
    let _ = peer;
}

#[test]
fn one_bad_entry_skips_itself_not_the_directory() {
    use std::os::unix::ffi::OsStrExt;
    let (root, _peer, harness) = started_with(&Settings::default(), &[panel("251")]);
    // A filename that is not UTF-8: the listing skips it, and the good
    // device still reads.
    let bad = std::ffi::OsStr::from_bytes(b"\xff\xfe");
    std::fs::write(root.join(bad), "junk").unwrap();
    assert_eq!(harness.view().text(), "49%");
    assert_eq!(read_level(&root, None).unwrap().percent, 49);
}

#[test]
fn the_first_usable_device_shows_by_default_and_device_pins() {
    let root = unique("two");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "intel_backlight", Fixture { raw: Some("90") }, 100);
    write_fixture(&root, "acpi_video0", Fixture { raw: Some("3") }, 10);
    // `acpi_video0` sorts first, so it is the default.
    let (peer, ours) = socketpair();
    let harness = Harness::new(start_with(&Settings::default(), &root, Some(ours)).unwrap());
    assert_eq!(harness.value_on(None).unwrap()["device"], "acpi_video0");
    assert_eq!(harness.view().text(), "30%");
    let _ = peer;
    // Pinned, the other one shows instead.
    let settings = Settings {
        device: Some("intel_backlight".to_owned()),
        ..Settings::default()
    };
    let (_peer, ours) = socketpair();
    let harness = Harness::new(start_with(&settings, &root, Some(ours)).unwrap());
    assert_eq!(harness.view().text(), "90%");
    assert_eq!(harness.value_on(None).unwrap()["device"], "intel_backlight");
}

#[test]
fn a_raw_past_its_range_is_clamped() {
    let (_root, _peer, harness) = started_with(
        &Settings::default(),
        &[("panel-bl", Fixture { raw: Some("999") }, 100)],
    );
    assert_eq!(harness.view().text(), "100%");
}

#[test]
fn an_unrelated_uevent_changes_nothing() {
    let (root, peer, mut harness) = started_with(&Settings::default(), &[panel("251")]);
    // The brightness moves, but no backlight event arrives: the view is
    // stale until one does (the bar's own writes re-read at once; here
    // only the tap is driven).
    std::fs::write(root.join("apple-panel-bl").join("brightness"), "200").unwrap();
    peer.send(&uevent("usb")).unwrap();
    assert_eq!(
        harness.wait(Duration::from_secs(1)),
        Some(Update::Unchanged)
    );
    assert_eq!(harness.view().text(), "49%");
    backlight_uevent(&peer);
    changed(&mut harness);
    assert_eq!(harness.view().text(), "39%");
}

#[test]
fn a_uevent_storm_is_one_re_read_per_turn() {
    let (root, peer, mut harness) = started_with(&Settings::default(), &[panel("251")]);
    std::fs::write(root.join("apple-panel-bl").join("brightness"), "200").unwrap();
    for _ in 0..200 {
        peer.send(&uevent("backlight")).unwrap();
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
    assert_eq!(harness.view().text(), "39%");
    assert!(
        (1..=4).contains(&changed_turns),
        "changed in {changed_turns} turns"
    );
}

#[test]
fn scrolls_raise_and_lower_from_what_is_shown() {
    use crate::action::ModuleAction;
    let (root, _peer, mut harness) = started_with(&Settings::default(), &[panel("251")]);
    let output = OutputView { name: None };
    // 49% up one step (5 points) is 54%: raw 275 of 509.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Ok(Update::Changed)
    );
    assert_eq!(harness.view().text(), "54%");
    assert_eq!(brightness_of(&root, "apple-panel-bl"), "275");
    // Two steps down from there is 44%: raw 224.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("lower", None), 2),
        Ok(Update::Changed)
    );
    assert_eq!(harness.view().text(), "44%");
    assert_eq!(brightness_of(&root, "apple-panel-bl"), "224");
}

#[test]
fn set_is_absolute_clamped_and_floored() {
    use crate::action::ModuleAction;
    let (root, _peer, mut harness) = started_with(&Settings::default(), &[panel("251")]);
    let output = OutputView { name: None };
    let set = |harness: &mut Harness, arg: i32| {
        harness.invoke(
            &output,
            &ModuleAction {
                name: "set".into(),
                arg: Some(arg),
            },
            1,
        )
    };
    // Past the range clamps to full scale, not past it.
    assert_eq!(set(&mut harness, 200), Ok(Update::Changed));
    assert_eq!(harness.view().text(), "100%");
    assert_eq!(brightness_of(&root, "apple-panel-bl"), "509");
    // Zero never blanks the panel: the floor is raw 1.
    assert_eq!(set(&mut harness, 0), Ok(Update::Changed));
    assert_eq!(brightness_of(&root, "apple-panel-bl"), "1");
    assert_eq!(harness.view().text(), "0%");
    // Setting what is shown is not a write.
    assert_eq!(set(&mut harness, 0), Ok(Update::Unchanged));
    assert_eq!(brightness_of(&root, "apple-panel-bl"), "1");
}

#[test]
fn a_scroll_at_the_limit_writes_nothing() {
    use crate::action::ModuleAction;
    let (root, _peer, mut harness) = started_with(
        &Settings::default(),
        &[("panel-bl", Fixture { raw: Some("100") }, 100)],
    );
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 5),
        Ok(Update::Unchanged)
    );
    assert_eq!(brightness_of(&root, "panel-bl"), "100");
}

#[test]
fn an_unwritable_brightness_is_a_named_refusal() {
    use crate::action::ModuleAction;
    let (root, _peer, mut harness) = started_with(&Settings::default(), &[panel("251")]);
    // The file replaced by a directory: the open fails whatever the
    // user (a permission test would pass for root, this one fails for
    // every uid, like the config's file-as-parent case).
    let path = root.join("apple-panel-bl").join("brightness");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let output = OutputView { name: None };
    let error = harness
        .invoke(&output, &ModuleAction::new("raise", None), 1)
        .unwrap_err();
    assert!(
        matches!(error, InvokeError::Refused(_)),
        "a silent or unknown refusal: {error}"
    );
    assert!(error.to_string().contains("writable"), "{error}");
    // The failed write moves nothing.
    assert_eq!(harness.view().text(), "49%");
}

#[test]
fn invoke_names_and_arities() {
    use crate::action::ModuleAction;
    let (_root, _peer, mut harness) = started_with(&Settings::default(), &[panel("251")]);
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("frobnicate", None), 1),
        Err(InvokeError::Unknown)
    );
    assert_eq!(
        harness.invoke(
            &output,
            &ModuleAction {
                name: "raise".into(),
                arg: Some(5),
            },
            1
        ),
        Err(InvokeError::NoArg)
    );
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("set", None), 1),
        Err(InvokeError::NeedsArg)
    );
}

#[test]
fn a_removed_backlight_hides_and_a_returned_one_shows() {
    let (root, peer, mut harness) = started_with(&Settings::default(), &[panel("251")]);
    std::fs::remove_dir_all(root.join("apple-panel-bl")).unwrap();
    backlight_uevent(&peer);
    changed(&mut harness);
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
    // The tap stays (the appearance watch); there is no timer to drop.
    assert_eq!(harness.source_count(), 1);
    write_fixture(&root, "apple-panel-bl", Fixture { raw: Some("251") }, 509);
    backlight_uevent(&peer);
    changed(&mut harness);
    assert_eq!(harness.view().text(), "49%");
}

#[test]
fn click_and_scroll_have_defaults() {
    use crate::action::ModuleAction;
    use crate::density::Scale;
    use crate::modules::{ClickCtx, Input};
    use crate::text::Text;
    use ab_glyph::{FontArc, FontVec};
    let (_root, _peer, harness) = started_with(&Settings::default(), &[panel("251")]);
    let view = harness.view();
    let font = FontArc::new(FontVec::try_from_vec(crate::testfont::build()).unwrap());
    let text = Text::new(font);
    let output = OutputView { name: None };
    let ctx = ClickCtx {
        output,
        x: 10,
        view: &view,
        text: &text,
        em: 50.0,
        padding: 10,
        span_width: 400,
        height: 60,
        scale: Scale::Integer(1),
    };
    use crate::action::Trigger;
    let input = |trigger| harness.input(&Input { trigger, at: &ctx });
    // A click with no binding does nothing: there is nothing to toggle.
    assert_eq!(input(Trigger::Click), None);
    assert_eq!(input(Trigger::RightClick), None);
    assert_eq!(input(Trigger::MiddleClick), None);
    assert_eq!(
        input(Trigger::ScrollUp),
        Some(Action::Module(ModuleAction::new("raise", None)))
    );
    assert_eq!(
        input(Trigger::ScrollDown),
        Some(Action::Module(ModuleAction::new("lower", None)))
    );
}

#[test]
fn against_the_real_backlight() {
    // Where the machine allows: a real backlight reads sane, and a
    // missing or empty directory is a named Unavailable. Both arms run
    // without a fixture.
    use std::path::Path as RealPath;
    let root = RealPath::new("/sys/class/backlight");
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
            assert!(harness.source_count() <= 1);
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

/// `SUBSYSTEM=backlight` matched whole, or not at all: a well-formed
/// match, and the near-misses pinned apart (a passing scan that never
/// compares a name is the volume module's `scan_names` lesson).
#[test]
fn the_uevent_scan_matches_whole_fields() {
    assert!(has_backlight(
        b"ACTION=change\0DEVPATH=/devices/backlight/apple-panel-bl\0SUBSYSTEM=backlight\0SEQNUM=1\0"
    ));
    assert!(has_backlight(b"SUBSYSTEM=backlight"));
    assert!(!has_backlight(b""));
    assert!(!has_backlight(b"ACTION=change\0SUBSYSTEM=usb\0"));
    assert!(!has_backlight(b"ACTION=change\0SUBSYSTEM=backlight_now\0"));
    assert!(!has_backlight(b"ACTION=change\0SUBSYSTEMX=backlight\0"));
    assert!(!has_backlight(b"SUBSYSTEM=backlightX"));
}

#[test]
fn raws_parse_strictly() {
    assert_eq!(parse_raw("0"), Some(0));
    assert_eq!(parse_raw("509"), Some(509));
    assert_eq!(parse_raw("4294967295"), Some(u32::MAX));
    for bad in [
        "",
        " ",
        "abc",
        "-5",
        "+5",
        "72%",
        "7 2",
        "0x10",
        "５",
        "4294967296",
    ] {
        assert_eq!(parse_raw(bad), None, "{bad:?}");
    }
}

#[test]
fn percents_round_half_up_and_raws_floor_at_one() {
    // The Asahi range: 251 of 509 is 49.31, shown 49.
    assert_eq!(raw_to_percent(251, 509), 49);
    assert_eq!(raw_to_percent(0, 509), 0);
    assert_eq!(raw_to_percent(509, 509), 100);
    assert_eq!(raw_to_percent(1, 2), 50);
    // A tiny range: 1 of 2 is exactly half, rounding up.
    assert_eq!(raw_to_percent(1, 1), 100);
    // Back the other way: 54% of 509 is 274.86, written 275.
    assert_eq!(percent_to_raw(54, 509), 275);
    assert_eq!(percent_to_raw(100, 509), 509);
    // Past the range clamps; zero never blanks the panel.
    assert_eq!(percent_to_raw(200, 509), 509);
    assert_eq!(percent_to_raw(0, 509), 1);
    assert_eq!(percent_to_raw(0, 1), 1);
}

#[test]
fn levels_read_like_the_ticket() {
    // The Asahi shape: one backlight at 107 of 509, linear, with the
    // `actual_brightness` the module must ignore beside it.
    let root = unique("ticket");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "apple-panel-bl", Fixture { raw: Some("107") }, 509);
    std::fs::write(root.join("apple-panel-bl").join("actual_brightness"), "107").unwrap();
    std::fs::write(root.join("apple-panel-bl").join("scale"), "linear").unwrap();
    let level = read_level(&root, None).unwrap();
    assert_eq!(level.percent, 21);
    assert_eq!(level.device, "apple-panel-bl");
    assert_eq!(level.raw, 107);
    assert_eq!(level.max, 509);
}
