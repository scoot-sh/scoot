//! `--sun` tests: the command line, the unit files, the one-shot timer's
//! arming and retiring against a recording manager, and how the units and
//! the Sun agree about which side is due. Nothing here touches a real
//! systemd.

use std::path::PathBuf;

use scoot_look::sun::civil::unix_from_utc;
use scoot_look::sun::{self, Location};

use super::arm::{self, Armed, NEXT_TIMER};
use super::tests::Fake;
use super::unit::{PATH, Plan, Portal, SERVICE, Schedule, Side, Sun, TIMER, Trigger, read_service};
use super::{Action, fire_target, install, parse, remove, status};
use crate::cli::Error;

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

fn london() -> Sun {
    Sun::parse("dawn", "moonrise", "51.5074", "-0.1278").expect("valid")
}

fn plan() -> Plan {
    Plan {
        trigger: Trigger::Sun(london()),
        exe: PathBuf::from("/usr/bin/scoot"),
        state: None,
        watch: None,
    }
}

const LONDON: &[&str] = &[
    "--light",
    "dawn",
    "--dark",
    "moonrise",
    "--sun",
    "--latitude",
    "51.5074",
    "--longitude",
    "-0.1278",
];

fn dir() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("systemd/user");
    (root, dir)
}

// ---- command line ----

#[test]
fn parse_takes_the_place_in_any_order_and_spelling() {
    let want = Trigger::Sun(london());
    assert_eq!(parse(&words(LONDON), false), Ok(Action::On(want.clone())));
    assert_eq!(parse(&words(LONDON), true), Ok(Action::Fire(want.clone())));
    assert_eq!(
        parse(
            &words(&[
                "--longitude=-0.1278",
                "--sun",
                "--latitude=51.5074",
                "--dark=moonrise",
                "--light=dawn"
            ]),
            false
        ),
        Ok(Action::On(want))
    );
}

#[test]
fn parse_refuses_a_place_that_is_not_a_place() {
    let with = |lat: &str, lon: &str| {
        parse(
            &words(&[
                "--light",
                "a",
                "--dark",
                "b",
                "--sun",
                "--latitude",
                lat,
                "--longitude",
                lon,
            ]),
            false,
        )
    };
    assert!(with("0", "0").is_ok());
    assert!(with("90", "180").is_ok());
    assert!(with("-90", "-180").is_ok());
    for (lat, lon) in [
        ("90.0001", "0"),
        ("-91", "0"),
        ("0", "180.0001"),
        ("0", "-181"),
        ("nan", "0"),
        ("0", "inf"),
        ("1e1", "0"),
        ("", "0"),
        ("0", ""),
        ("51,5", "0"),
        ("+51", "0"),
        ("51°", "0"),
        ("0", "99999999999999999999999999999999"),
    ] {
        assert!(with(lat, lon).is_err(), "`{lat}` `{lon}` must refuse");
    }
    // A latitude where the longitude belongs is out of range, not swapped.
    assert!(with("151.2", "-33.9").is_err());
}

#[test]
fn parse_names_what_is_missing_or_misplaced() {
    let on = |args: &[&str]| parse(&words(args), false);
    // The place is required with --sun, both halves of it.
    let base = ["--light", "a", "--dark", "b", "--sun"];
    assert!(matches!(on(&base), Err(Error::Missing(m)) if m.contains("--latitude")));
    let mut lat_only = base.to_vec();
    lat_only.extend(["--latitude", "1"]);
    assert!(matches!(on(&lat_only), Err(Error::Missing(m)) if m.contains("--longitude")));
    let mut lon_only = base.to_vec();
    lon_only.extend(["--longitude", "1"]);
    assert!(matches!(on(&lon_only), Err(Error::Missing(m)) if m.contains("--latitude")));
    // A place belongs to --sun alone.
    let err = on(&[
        "--light",
        "a",
        "--dark",
        "b",
        "--schedule",
        "07:00/19:00",
        "--latitude",
        "1",
    ])
    .unwrap_err();
    assert!(err.to_string().contains("belong to --sun"), "{err}");
    assert!(
        on(&[
            "--light",
            "a",
            "--dark",
            "b",
            "--follow-portal",
            "--longitude",
            "1"
        ])
        .is_err()
    );
    assert!(on(&["--latitude", "1"]).is_err());
    // One trigger at a time, in every pairing.
    let mut with_schedule = LONDON.to_vec();
    with_schedule.extend(["--schedule", "07:00/19:00"]);
    let err = on(&with_schedule).unwrap_err();
    assert!(err.to_string().contains("one trigger"), "{err}");
    let mut with_portal = LONDON.to_vec();
    with_portal.push("--follow-portal");
    assert!(
        on(&with_portal)
            .unwrap_err()
            .to_string()
            .contains("one trigger")
    );
    // --sun is a bare flag, once, and --off takes nothing with it.
    assert!(on(&["--light", "a", "--dark", "b", "--sun=1"]).is_err());
    let mut twice = LONDON.to_vec();
    twice.push("--sun");
    assert!(on(&twice).is_err());
    let mut off = LONDON.to_vec();
    off.push("--off");
    assert!(on(&off).is_err());
    assert!(on(&["--off", "--sun"]).is_err());
    assert!(on(&["--off", "--latitude", "1"]).is_err());
    // The two looks must differ, as for the other triggers.
    assert!(
        on(&[
            "--light",
            "a",
            "--dark",
            "a",
            "--sun",
            "--latitude",
            "1",
            "--longitude",
            "1"
        ])
        .is_err()
    );
    assert!(parse(&words(&["--off"]), true).is_err());
}

// ---- which side is due ----

#[test]
fn the_side_follows_the_sun_at_a_place() {
    let s = london();
    // Midsummer noon and midnight, midwinter noon and midnight.
    assert_eq!(s.due(unix_from_utc(2024, 6, 21, 12, 0, 0)), Side::Light);
    assert_eq!(s.due(unix_from_utc(2024, 6, 21, 0, 0, 0)), Side::Dark);
    assert_eq!(s.due(unix_from_utc(2024, 12, 21, 12, 0, 0)), Side::Light);
    assert_eq!(s.due(unix_from_utc(2024, 12, 21, 1, 0, 0)), Side::Dark);
    // USNO: London 2024-12-21 rises 08:04 and sets 15:54.
    assert_eq!(s.due(unix_from_utc(2024, 12, 21, 7, 55, 0)), Side::Dark);
    assert_eq!(s.due(unix_from_utc(2024, 12, 21, 8, 13, 0)), Side::Light);
    assert_eq!(s.due(unix_from_utc(2024, 12, 21, 15, 45, 0)), Side::Light);
    assert_eq!(s.due(unix_from_utc(2024, 12, 21, 16, 3, 0)), Side::Dark);
    assert_eq!(s.look_for(Side::Light), "dawn");
    assert_eq!(s.look_for(Side::Dark), "moonrise");
}

#[test]
fn the_polar_sides_stay_put() {
    let tromso = Sun::parse("dawn", "moonrise", "69.6492", "18.9553").unwrap();
    for hour in 0..24 {
        assert_eq!(
            tromso.due(unix_from_utc(2024, 6, 21, hour, 0, 0)),
            Side::Light
        );
        assert_eq!(
            tromso.due(unix_from_utc(2024, 12, 21, hour, 0, 0)),
            Side::Dark
        );
    }
}

#[test]
fn a_swap_is_skipped_when_the_look_is_already_active() {
    assert_eq!(
        super::needs_swap("dawn", Some("moonrise"), false),
        Some("dawn")
    );
    assert_eq!(super::needs_swap("dawn", Some("dawn"), false), None);
    assert_eq!(super::needs_swap("dawn", None, false), Some("dawn"));
    // A pending preview of it still applies: the reverter would undo it.
    assert_eq!(super::needs_swap("dawn", Some("dawn"), true), Some("dawn"));
    // The schedule's decision is the same function.
    let s = Schedule::parse("dawn", "moonrise", "07:00/19:00").unwrap();
    assert_eq!(
        fire_target(
            &s,
            super::Clock::parse("08:00").unwrap(),
            Some("dawn"),
            false
        ),
        None
    );
}

// ---- unit files ----

#[test]
fn the_sun_units_match_the_golden_files() {
    assert_eq!(
        plan().service().unwrap(),
        include_str!("golden/service-sun.golden")
    );
    let (name, text) = plan().trigger_file().unwrap();
    assert_eq!(name, TIMER);
    assert_eq!(text, include_str!("golden/timer-sun.golden"));
    assert_eq!(
        arm::next_timer_text(1_700_000_000).unwrap(),
        include_str!("golden/timer-next.golden")
    );
}

#[test]
fn the_service_line_reads_back_to_the_same_place() {
    for (lat, lon) in [
        ("51.5074", "-0.1278"),
        ("0", "0"),
        ("-90", "-180"),
        ("90", "180"),
        ("-33.8688", "151.2093"),
        ("0.000001", "-0.000001"),
    ] {
        let p = Plan {
            trigger: Trigger::Sun(Sun::parse("dawn", "moonrise", lat, lon).unwrap()),
            ..plan()
        };
        let text = p.service().unwrap();
        assert_eq!(text.lines().count(), 7, "{lat} {lon}: one ExecStart");
        assert_eq!(read_service(&text).unwrap(), p, "{lat} {lon}");
    }
}

#[test]
fn a_hand_edited_place_is_unreadable_not_trusted() {
    let good = plan().service().unwrap();
    for edit in [
        ("--latitude 51.5074", "--latitude 051.5074"),
        ("--latitude 51.5074", "--latitude 51.50740"),
        ("--latitude 51.5074", "--latitude 91"),
        ("--latitude 51.5074", "--latitude nan"),
        ("--latitude 51.5074", "--latitude +51.5074"),
        ("--longitude -0.1278", "--longitude 181"),
        ("--longitude -0.1278", "--longitude -0.1278 --extra"),
        ("--longitude -0.1278", "--longitude"),
        ("--sun --latitude", "--latitude"),
        ("--sun", "--sun --schedule 07:00/19:00"),
    ] {
        let edited = good.replace(edit.0, edit.1);
        assert!(read_service(&edited).is_err(), "edit `{}`", edit.1);
    }
}

// ---- install, switch, off ----

#[test]
fn install_writes_the_daily_timer_and_its_service() {
    let (_root, dir) = dir();
    let manager = Fake::default();
    let done = install(&dir, &plan(), &manager).unwrap();
    assert_eq!(done.wrote, vec![SERVICE, TIMER]);
    assert_eq!(
        std::fs::read_to_string(dir.join(TIMER)).unwrap(),
        include_str!("golden/timer-sun.golden")
    );
    assert_eq!(
        manager.calls(),
        [
            "available",
            "reload",
            "enable scoot-look-auto.timer",
            "restart scoot-look-auto.timer"
        ]
    );
    // And again: nothing to rewrite.
    let again = Fake::default();
    assert!(install(&dir, &plan(), &again).unwrap().wrote.is_empty());
    assert_eq!(again.calls(), ["available", "enable scoot-look-auto.timer"]);
}

#[test]
fn a_moved_place_rewrites_the_service_only() {
    let (_root, dir) = dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let moved = Plan {
        trigger: Trigger::Sun(Sun::parse("dawn", "moonrise", "40.7128", "-74.006").unwrap()),
        ..plan()
    };
    let done = install(&dir, &moved, &Fake::default()).unwrap();
    // The timer text does not carry the place: the service does.
    assert_eq!(done.wrote, vec![SERVICE]);
    assert!(
        std::fs::read_to_string(dir.join(SERVICE))
            .unwrap()
            .contains("--latitude 40.7128 --longitude -74.006")
    );
}

#[test]
fn schedule_and_sun_share_the_timer_slot() {
    let (_root, dir) = dir();
    let schedule = Plan {
        trigger: Trigger::Schedule(Schedule::parse("dawn", "moonrise", "07:00/19:00").unwrap()),
        ..plan()
    };
    install(&dir, &schedule, &Fake::default()).unwrap();
    let to_sun = install(&dir, &plan(), &Fake::default()).unwrap();
    assert_eq!(to_sun.wrote, vec![SERVICE, TIMER]);
    assert_eq!(to_sun.replaced, None, "same unit, rewritten in place");
    let back = install(&dir, &schedule, &Fake::default()).unwrap();
    assert_eq!(back.wrote, vec![SERVICE, TIMER]);
    assert!(
        std::fs::read_to_string(dir.join(TIMER))
            .unwrap()
            .contains("OnCalendar=*-*-* 07:00:00")
    );
}

#[test]
fn the_portal_replaces_the_sun_timer() {
    let (_root, dir) = dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let portal = Plan {
        trigger: Trigger::Portal(Portal::parse("dawn", "moonrise").unwrap()),
        watch: Some(PathBuf::from("/home/me/.config/dconf/user")),
        ..plan()
    };
    let manager = Fake::default();
    let done = install(&dir, &portal, &manager).unwrap();
    assert_eq!(done.replaced, Some(TIMER));
    assert!(!dir.join(TIMER).exists());
    assert!(dir.join(PATH).exists());
    assert!(
        manager
            .calls()
            .contains(&"disable scoot-look-auto.timer".to_owned())
    );
}

#[test]
fn status_reads_the_sun_back() {
    let (_root, dir) = dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let found = status(&dir, &Fake::default()).unwrap();
    assert_eq!(found.plan, Some(Ok(plan())));
    assert!(!found.partial);
    assert_eq!(found.trigger.unwrap().0, TIMER);
}

#[test]
fn off_removes_the_sun_pair() {
    let (_root, dir) = dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let out = remove(&dir, &Fake::default()).unwrap();
    assert_eq!(out.removed, vec![TIMER, SERVICE]);
    assert!(!dir.join(TIMER).exists() && !dir.join(SERVICE).exists());
}

// ---- the one-shot timer ----

fn runtime() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let dir = arm::runtime_dir(Some(root.path().as_os_str())).unwrap();
    (root, dir)
}

fn place() -> Location {
    london().place
}

/// 2024-12-21 00:00 UTC in London: the sunrise is about 08:04.
fn midnight() -> i64 {
    unix_from_utc(2024, 12, 21, 0, 0, 0)
}

#[test]
fn the_runtime_dir_is_the_managers_and_absolute() {
    use std::ffi::OsStr;
    assert_eq!(
        arm::runtime_dir(Some(OsStr::new("/run/user/1000"))),
        Ok(PathBuf::from("/run/user/1000/systemd/user"))
    );
    for bad in [
        None,
        Some(OsStr::new("")),
        Some(OsStr::new("run/user/1000")),
    ] {
        let why = arm::runtime_dir(bad).unwrap_err();
        assert!(why.contains("XDG_RUNTIME_DIR"), "{why}");
    }
}

#[test]
fn arming_writes_the_next_boundary_in_utc_and_starts_it() {
    let (_root, dir) = runtime();
    let manager = Fake::default();
    let armed = arm::arm(&dir, place(), midnight(), &manager).unwrap();
    let Armed::At { change, rewrote } = armed else {
        panic!("{armed:?}");
    };
    assert!(rewrote && change.day, "the first boundary is the sunrise");
    // About 08:04 UTC in London in midwinter, to the second.
    let want = unix_from_utc(2024, 12, 21, 8, 4, 0);
    assert!((change.at - want).abs() <= 60);
    let text = std::fs::read_to_string(dir.join(NEXT_TIMER)).unwrap();
    assert_eq!(text, arm::next_timer_text(change.at).unwrap());
    assert!(text.contains("OnCalendar=2024-12-21 08:0"), "{text}");
    assert!(text.contains(" UTC\n"), "{text}");
    assert!(text.contains("Unit=scoot-look-auto.service\n"));
    assert_eq!(
        manager.calls(),
        ["reload", "restart scoot-look-auto-next.timer"]
    );
}

#[test]
fn a_run_that_changes_nothing_never_reloads() {
    let (_root, dir) = runtime();
    arm::arm(&dir, place(), midnight(), &Fake::default()).unwrap();
    // An hour later the same boundary is next and the timer stands for it.
    let again = Fake::default();
    let armed = arm::arm(&dir, place(), midnight() + 3600, &again).unwrap();
    assert!(
        matches!(armed, Armed::At { rewrote: false, .. }),
        "{armed:?}"
    );
    assert_eq!(again.calls(), ["state scoot-look-auto-next.timer"]);
    // The same, but the timer is not waiting (stopped, or elapsed and gone):
    // it is started again.
    let idle = Fake {
        timer_idle: true,
        ..Fake::default()
    };
    let armed = arm::arm(&dir, place(), midnight() + 3600, &idle).unwrap();
    assert!(
        matches!(armed, Armed::At { rewrote: true, .. }),
        "{armed:?}"
    );
    assert!(
        idle.calls()
            .contains(&"restart scoot-look-auto-next.timer".to_owned())
    );
}

#[test]
fn after_a_boundary_the_next_one_is_armed() {
    let (_root, dir) = runtime();
    let Armed::At { change: rise, .. } =
        arm::arm(&dir, place(), midnight(), &Fake::default()).unwrap()
    else {
        panic!()
    };
    // The timer fires at the sunrise; the run it starts arms the sunset.
    let manager = Fake::default();
    let Armed::At {
        change: set,
        rewrote,
    } = arm::arm(&dir, place(), rise.at, &manager).unwrap()
    else {
        panic!()
    };
    assert!(rewrote && !set.day && set.at > rise.at);
    let want = unix_from_utc(2024, 12, 21, 15, 54, 0);
    assert!((set.at - want).abs() <= 60);
    assert_eq!(
        std::fs::read_to_string(dir.join(NEXT_TIMER)).unwrap(),
        arm::next_timer_text(set.at).unwrap()
    );
}

#[test]
fn a_run_just_before_or_after_a_boundary_agrees_with_the_timer() {
    // Catch-up runs pick the side from the clock: whenever one lands
    // around the boundary the timer named, the side is the new one from
    // that second on and the old one before it.
    let s = london();
    let (_root, dir) = runtime();
    let Armed::At { change, .. } = arm::arm(&dir, s.place, midnight(), &Fake::default()).unwrap()
    else {
        panic!()
    };
    assert_eq!(s.due(change.at - 1), Side::Dark);
    for late in [0, 1, 2, 30, 3600] {
        assert_eq!(s.due(change.at + late), Side::Light, "+{late} s");
    }
}

#[test]
fn a_polar_stretch_arms_nothing_and_retires_a_stale_timer() {
    let tromso = Sun::parse("dawn", "moonrise", "69.6492", "18.9553")
        .unwrap()
        .place;
    let (_root, dir) = runtime();
    // Midwinter: no boundary, no file, and the manager is never asked.
    let quiet = Fake::default();
    let night = unix_from_utc(2024, 12, 21, 12, 0, 0);
    assert_eq!(arm::arm(&dir, tromso, night, &quiet).unwrap(), Armed::Idle);
    assert!(quiet.calls().is_empty());
    assert!(!dir.exists());
    // A timer left from a day with a boundary is stopped and removed.
    arm::arm(&dir, place(), midnight(), &Fake::default()).unwrap();
    assert!(dir.join(NEXT_TIMER).exists());
    let manager = Fake::default();
    assert_eq!(
        arm::arm(&dir, tromso, night, &manager).unwrap(),
        Armed::Idle
    );
    assert!(!dir.join(NEXT_TIMER).exists());
    assert_eq!(
        manager.calls(),
        ["available", "stop scoot-look-auto-next.timer", "reload"]
    );
}

#[test]
fn a_foreign_file_in_the_timer_slot_is_never_overwritten_or_removed() {
    let (_root, dir) = runtime();
    std::fs::create_dir_all(&dir).unwrap();
    let theirs = "[Timer]\nOnCalendar=daily\n";
    std::fs::write(dir.join(NEXT_TIMER), theirs).unwrap();
    let manager = Fake::default();
    let error = arm::arm(&dir, place(), midnight(), &manager).unwrap_err();
    assert!(
        error.contains("was not written by `scoot look auto`"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(NEXT_TIMER)).unwrap(),
        theirs
    );
    assert!(manager.calls().is_empty());
    assert_eq!(arm::retire(&dir, &manager), Ok(false));
    assert_eq!(
        std::fs::read_to_string(dir.join(NEXT_TIMER)).unwrap(),
        theirs
    );
    // Polar night with a foreign file: left alone, no error.
    let tromso = Sun::parse("a", "b", "69.6492", "18.9553").unwrap().place;
    let night = unix_from_utc(2024, 12, 21, 12, 0, 0);
    assert_eq!(
        arm::arm(&dir, tromso, night, &manager).unwrap(),
        Armed::Idle
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(NEXT_TIMER)).unwrap(),
        theirs
    );
}

#[test]
fn retire_without_a_manager_still_removes_the_file() {
    let (_root, dir) = runtime();
    arm::arm(&dir, place(), midnight(), &Fake::default()).unwrap();
    let down = Fake {
        down: true,
        ..Fake::default()
    };
    assert_eq!(arm::retire(&dir, &down), Ok(true));
    assert!(!dir.join(NEXT_TIMER).exists());
    assert_eq!(down.calls(), ["available"]);
    // Nothing there: quiet, and the manager is not asked.
    let none = Fake::default();
    assert_eq!(arm::retire(&dir, &none), Ok(false));
    assert!(none.calls().is_empty());
}

#[test]
fn an_unspellable_instant_is_refused_not_written() {
    // A place and a clock whose next boundary is not a year systemd names
    // cannot happen in this century; the text function says so.
    assert!(arm::next_timer_text(i64::MAX).is_none());
    assert!(sun::civil::on_calendar_utc(i64::MIN).is_none());
}
