//! `scoot look auto` tests: grammar, the due side, golden unit files,
//! hostile paths, and the install/remove/status lifecycle against a
//! recording fake manager. Nothing here touches a real systemd.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use super::systemctl::{Systemctl, TimerState, parse_show};
use super::unit::{
    Clock, MARKER, Plan, SERVICE, Schedule, Side, TIMER, quote_exe, quote_path, read_service,
};
use super::{
    Action, Slot, config_home_from, fire_target, inspect, install, parse, remove, stable_exe,
    status, unit_exe,
};
use crate::cli::Error;

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

fn clock(text: &str) -> Clock {
    Clock::parse(text).expect("valid clock")
}

fn schedule(light_from: &str, dark_from: &str) -> Schedule {
    Schedule::parse("dawn", "moonrise", &format!("{light_from}/{dark_from}")).expect("valid")
}

fn plan() -> Plan {
    Plan {
        schedule: schedule("07:00", "19:30"),
        exe: PathBuf::from("/usr/bin/scoot"),
        state: None,
    }
}

// ---- grammar ----

#[test]
fn clock_accepts_exactly_hh_mm() {
    for ok in ["00:00", "07:05", "12:00", "23:59"] {
        assert!(Clock::parse(ok).is_some(), "{ok}");
    }
    assert_eq!(clock("00:00").minutes(), 0);
    assert_eq!(clock("23:59").minutes(), 23 * 60 + 59);
    assert_eq!(clock("07:05").to_string(), "07:05");
}

#[test]
fn clock_refuses_everything_else() {
    for bad in [
        "",
        "24:00",
        "7:5",
        "7:05",
        "07:5",
        "07:60",
        "0700",
        "07-00",
        "07:00 ",
        " 07:00",
        "+7:05",
        "07:000",
        "007:00",
        "07:0a",
        "٠٧:٠٠",
        "07:00\n",
        "-1:00",
        "24:60",
        "99:99",
    ] {
        assert!(Clock::parse(bad).is_none(), "`{bad}` must refuse");
    }
}

#[test]
fn schedule_refuses_bad_words() {
    let bad = |light: &str, dark: &str, sched: &str| {
        Schedule::parse(light, dark, sched).expect_err("must refuse")
    };
    // Equal times, in both spellings of "equal".
    assert!(
        bad("a", "b", "07:00/07:00")
            .to_string()
            .contains("must differ")
    );
    assert!(
        bad("a", "b", "00:00/00:00")
            .to_string()
            .contains("must differ")
    );
    // Equal looks.
    assert!(
        bad("a", "a", "07:00/19:00")
            .to_string()
            .contains("must differ")
    );
    // Shape.
    for sched in [
        "",
        "07:00",
        "07:00/",
        "/19:00",
        "07:00/19:00/20:00",
        "07:00-19:00",
        "7:5/19:00",
        "24:00/19:00",
        "07:00/24:00",
    ] {
        bad("a", "b", sched);
    }
    // Hostile names never reach a unit file.
    for name in [
        "",
        "A",
        "../x",
        "a b",
        "a\nExecStart=/bin/sh",
        "x;rm",
        "a%n",
        "$HOME",
        "-x?",
        &"a".repeat(33),
        "é",
    ] {
        assert_eq!(
            bad(name, "b", "07:00/19:00"),
            Error::Invalid {
                what: "look name",
                value: name.to_owned()
            },
            "light `{name}`"
        );
        bad("a", name, "07:00/19:00");
    }
    // A boundary that is valid still parses.
    assert!(Schedule::parse(&"a".repeat(32), "b", "00:00/23:59").is_ok());
}

#[test]
fn due_follows_the_wall_clock() {
    let day = schedule("07:00", "19:00");
    for (now, want) in [
        ("00:00", Side::Dark),
        ("06:59", Side::Dark),
        ("07:00", Side::Light),
        ("12:00", Side::Light),
        ("18:59", Side::Light),
        ("19:00", Side::Dark),
        ("23:59", Side::Dark),
    ] {
        assert_eq!(day.due(clock(now)), want, "07:00/19:00 at {now}");
    }
    // Light wraps midnight: light 22:00, dark 06:00.
    let night_owl = schedule("22:00", "06:00");
    for (now, want) in [
        ("21:59", Side::Dark),
        ("22:00", Side::Light),
        ("23:59", Side::Light),
        ("00:00", Side::Light),
        ("05:59", Side::Light),
        ("06:00", Side::Dark),
        ("12:00", Side::Dark),
    ] {
        assert_eq!(night_owl.due(clock(now)), want, "22:00/06:00 at {now}");
    }
    // Midnight boundaries.
    let midnight = schedule("00:00", "12:00");
    assert_eq!(midnight.due(clock("00:00")), Side::Light);
    assert_eq!(midnight.due(clock("11:59")), Side::Light);
    assert_eq!(midnight.due(clock("12:00")), Side::Dark);
    assert_eq!(midnight.due(clock("23:59")), Side::Dark);
}

#[test]
fn every_minute_has_exactly_one_side_and_each_boundary_starts_its_own() {
    // Whatever the pair, the start minute belongs to the side that starts.
    for (l, d) in [
        (0, 1),
        (1, 0),
        (0, 1439),
        (1439, 0),
        (420, 1140),
        (1140, 420),
    ] {
        let s = Schedule {
            light: "a".into(),
            dark: "b".into(),
            light_from: Clock::parse(&format!("{:02}:{:02}", l / 60, l % 60)).unwrap(),
            dark_from: Clock::parse(&format!("{:02}:{:02}", d / 60, d % 60)).unwrap(),
        };
        assert_eq!(s.due(s.light_from), Side::Light);
        assert_eq!(s.due(s.dark_from), Side::Dark);
    }
}

#[test]
fn fire_skips_a_look_that_is_already_active() {
    let s = schedule("07:00", "19:00");
    assert_eq!(
        fire_target(&s, clock("08:00"), Some("moonrise"), false),
        Some("dawn")
    );
    assert_eq!(fire_target(&s, clock("08:00"), Some("dawn"), false), None);
    assert_eq!(fire_target(&s, clock("08:00"), None, false), Some("dawn"));
    assert_eq!(
        fire_target(&s, clock("20:00"), Some("dawn"), false),
        Some("moonrise")
    );
}

#[test]
fn a_pending_preview_is_not_the_active_look() {
    // `apply moonrise --for 20` at 18:59, the 19:00 boundary wants moonrise:
    // the preview's reverter would put dawn back, so the boundary applies
    // (and keeps) it even though it is the previewed look.
    let s = schedule("07:00", "19:00");
    assert_eq!(
        fire_target(&s, clock("19:00"), Some("moonrise"), true),
        Some("moonrise")
    );
    assert_eq!(
        fire_target(&s, clock("19:00"), Some("moonrise"), false),
        None
    );
}

// ---- command line ----

#[test]
fn parse_shapes() {
    assert_eq!(parse(&[], false), Ok(Action::Status));
    assert_eq!(parse(&words(&["--off"]), false), Ok(Action::Off));
    assert_eq!(
        parse(
            &words(&[
                "--light",
                "dawn",
                "--dark",
                "moonrise",
                "--schedule",
                "07:00/19:30"
            ]),
            false
        ),
        Ok(Action::On(schedule("07:00", "19:30")))
    );
    // `=` spelling and any order.
    assert_eq!(
        parse(
            &words(&["--schedule=07:00/19:30", "--dark=moonrise", "--light=dawn"]),
            false
        ),
        Ok(Action::On(schedule("07:00", "19:30")))
    );
    assert_eq!(
        parse(
            &words(&[
                "--light",
                "dawn",
                "--dark",
                "moonrise",
                "--schedule",
                "07:00/19:30"
            ]),
            true
        ),
        Ok(Action::Fire(schedule("07:00", "19:30")))
    );
}

#[test]
fn parse_refusals() {
    let on = |args: &[&str]| parse(&words(args), false);
    assert!(matches!(on(&["--light", "a"]), Err(Error::Missing(_))));
    assert!(matches!(
        on(&["--light", "a", "--dark", "b"]),
        Err(Error::Missing(_))
    ));
    assert!(matches!(
        on(&["--schedule", "07:00/19:00"]),
        Err(Error::Missing(_))
    ));
    assert!(matches!(on(&["--light"]), Err(Error::Missing(_))));
    assert!(on(&["--off", "--light", "a"]).is_err());
    assert!(on(&["--off=1"]).is_err());
    assert!(
        on(&[
            "--light",
            "a",
            "--light",
            "b",
            "--dark",
            "c",
            "--schedule",
            "07:00/19:00"
        ])
        .is_err()
    );
    assert!(on(&["bogus"]).is_err());
    // Not available yet, named as such.
    for flag in ["--sun", "--follow-portal"] {
        let err = on(&[flag]).expect_err("not built");
        assert!(err.to_string().contains("--schedule only so far"), "{err}");
    }
    // The hidden verb takes no --off and needs all three.
    assert!(parse(&words(&["--off"]), true).is_err());
    assert!(matches!(parse(&[], true), Err(Error::Missing(_))));
}

// ---- unit files ----

#[test]
fn units_match_the_golden_files() {
    assert_eq!(
        plan().service().unwrap(),
        include_str!("golden/service.golden")
    );
    assert_eq!(plan().timer(), include_str!("golden/timer.golden"));
    let with_state = Plan {
        exe: PathBuf::from("/opt/my scoot/bin/scoot"),
        state: Some(PathBuf::from("/tmp/st%ate")),
        ..plan()
    };
    assert_eq!(
        with_state.service().unwrap(),
        include_str!("golden/service-state.golden")
    );
}

#[test]
fn quoting_neutralizes_what_systemd_expands() {
    assert_eq!(quote_path(Path::new("/a b/c")).unwrap(), "\"/a b/c\"");
    assert_eq!(quote_path(Path::new("/a\"b")).unwrap(), "\"/a\\\"b\"");
    assert_eq!(quote_path(Path::new("/a\\b")).unwrap(), "\"/a\\\\b\"");
    assert_eq!(quote_path(Path::new("/a%nb")).unwrap(), "\"/a%%nb\"");
    assert_eq!(quote_path(Path::new("/a$HOME")).unwrap(), "\"/a$$HOME\"");
    // A semicolon is a separator only unquoted.
    assert_eq!(quote_path(Path::new("/a;b")).unwrap(), "\"/a;b\"");
}

#[test]
fn quoting_refuses_what_it_cannot_say() {
    assert!(quote_path(Path::new("relative/scoot")).is_err());
    assert!(quote_path(Path::new("/a\nExecStartPre=/bin/sh")).is_err());
    assert!(quote_path(Path::new("/a\0b")).is_err());
    assert!(quote_path(Path::new("/a\tb")).is_err());
    use std::os::unix::ffi::OsStrExt as _;
    assert!(quote_path(Path::new(std::ffi::OsStr::from_bytes(b"/a\xffb"))).is_err());
    // A plan with such a path never renders.
    let bad = Plan {
        exe: PathBuf::from("/x\ny"),
        ..plan()
    };
    assert!(bad.service().is_err());
}

#[test]
fn a_hostile_state_path_stays_on_one_execstart_line_and_round_trips() {
    // The state directory is an ARGUMENT: quotes, backslashes and `$` are
    // escaped and read back.
    for dir in [
        "/a b/st", "/a\"b/st", "/a%h/st", "/a$X/st", "/a\\b/st", "/a;b/st", "/a'b/st", "/é/st",
    ] {
        let p = Plan {
            state: Some(PathBuf::from(dir)),
            ..plan()
        };
        let text = p.service().unwrap();
        assert_eq!(
            text.lines().filter(|l| l.starts_with("ExecStart=")).count(),
            1
        );
        assert_eq!(text.lines().count(), 7, "{dir}: no injected line");
        assert_eq!(read_service(&text).unwrap(), p, "{dir}");
    }
}

#[test]
fn the_executable_word_takes_space_percent_and_semicolon() {
    for exe in ["/a b/scoot", "/a%h/scoot", "/a;b/scoot", "/é/scoot"] {
        let p = Plan {
            exe: PathBuf::from(exe),
            ..plan()
        };
        let text = p.service().unwrap();
        assert_eq!(text.lines().count(), 7, "{exe}");
        assert_eq!(read_service(&text).unwrap(), p, "{exe}");
    }
    assert!(
        plan()
            .service()
            .unwrap()
            .contains("ExecStart=\"/usr/bin/scoot\" look")
    );
    assert_eq!(
        quote_exe(Path::new("/a%b/scoot")).unwrap(),
        "\"/a%%b/scoot\""
    );
}

#[test]
fn the_executable_word_refuses_what_systemd_cannot_run() {
    // Measured on systemd 261.2: `"`, `\` and `'` fail verify, `$$` fails
    // to start (203/EXEC). Each refuses before any file is written.
    for (c, exe) in [
        ('"', "/a\"b/scoot"),
        ('\'', "/home/o'brien/.local/bin/scoot"),
        ('\\', "/a\\b/scoot"),
        ('$', "/a$b/scoot"),
    ] {
        let error = quote_exe(Path::new(exe)).unwrap_err();
        assert!(error.contains(&format!("containing `{c}`")), "{error}");
        assert!(error.contains("plain path"), "{error}");
        let bad = Plan {
            exe: PathBuf::from(exe),
            ..plan()
        };
        let (_root, dir) = unit_dir();
        let manager = Fake::default();
        assert!(install(&dir, &bad, &manager).is_err(), "{exe}");
        assert!(!dir.exists(), "{exe}: nothing written");
        assert!(
            !manager
                .calls()
                .iter()
                .any(|call| call.starts_with("enable")),
            "{exe}: the manager is never asked to enable"
        );
    }
}

#[test]
fn a_relative_xdg_config_home_is_ignored() {
    use std::ffi::OsStr;
    let home = Some(OsStr::new("/home/u"));
    assert_eq!(
        config_home_from(Some(OsStr::new("rel/cfg")), home),
        Some(PathBuf::from("/home/u/.config"))
    );
    assert_eq!(
        config_home_from(Some(OsStr::new("/x/cfg")), home),
        Some(PathBuf::from("/x/cfg"))
    );
    assert_eq!(
        config_home_from(Some(OsStr::new("")), home),
        Some(PathBuf::from("/home/u/.config"))
    );
    assert_eq!(config_home_from(Some(OsStr::new("rel")), None), None);
}

#[test]
fn the_service_reader_trusts_only_its_own_line() {
    let good = plan().service().unwrap();
    assert_eq!(read_service(&good).unwrap(), plan());
    assert!(
        read_service("[Service]\nExecStart=/bin/sh\n").is_err(),
        "no marker"
    );
    for edit in [
        ("--schedule 07:00/19:30", "--schedule 07:00/07:00"),
        ("--light dawn", "--light DAWN"),
        ("--dark moonrise", "--dark moonrise --extra"),
        ("look _auto-fire", "look apply"),
        ("\"/usr/bin/scoot\"", "/usr/bin/scoot"),
        ("\"/usr/bin/scoot\"", "\"/usr/bin/scoot\\n\""),
    ] {
        let edited = good.replace(edit.0, edit.1);
        assert!(
            read_service(&edited).is_err(),
            "edit `{}` must be refused",
            edit.1
        );
    }
    assert!(
        read_service(&format!("# {MARKER}\n")).is_err(),
        "no ExecStart"
    );
}

#[test]
fn the_timer_and_service_agree_on_the_boundaries() {
    let p = Plan {
        schedule: schedule("00:00", "23:59"),
        ..plan()
    };
    assert!(p.timer().contains("OnCalendar=*-*-* 00:00:00\n"));
    assert!(p.timer().contains("OnCalendar=*-*-* 23:59:00\n"));
    assert!(p.timer().contains("Persistent=true\n"));
    assert!(p.service().unwrap().contains("--schedule 00:00/23:59"));
}

// ---- lifecycle against a fake manager ----

#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
    down: bool,
    fail_enable: bool,
    fail_disable: bool,
}

impl Fake {
    fn log(&self, call: impl Into<String>) {
        self.calls.borrow_mut().push(call.into());
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Systemctl for Fake {
    fn available(&self) -> Result<(), String> {
        self.log("available");
        if self.down {
            Err("no systemd user manager answers".to_owned())
        } else {
            Ok(())
        }
    }
    fn daemon_reload(&self) -> Result<(), String> {
        self.log("reload");
        Ok(())
    }
    fn enable_now(&self, unit: &str) -> Result<(), String> {
        self.log(format!("enable {unit}"));
        if self.fail_enable {
            Err("boom".to_owned())
        } else {
            Ok(())
        }
    }
    fn restart(&self, unit: &str) -> Result<(), String> {
        self.log(format!("restart {unit}"));
        Ok(())
    }
    fn disable_now(&self, unit: &str) -> Result<(), String> {
        self.log(format!("disable {unit}"));
        if self.fail_disable {
            Err("boom".to_owned())
        } else {
            Ok(())
        }
    }
    fn timer_state(&self, _unit: &str) -> Result<TimerState, String> {
        Ok(TimerState {
            active: "active".into(),
            enabled: "enabled".into(),
            next: "tomorrow".into(),
        })
    }
}

fn unit_dir() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("systemd/user");
    (root, dir)
}

fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

#[test]
fn install_writes_both_files_then_reloads_enables_and_restarts() {
    let (_root, dir) = unit_dir();
    let manager = Fake::default();
    let done = install(&dir, &plan(), &manager).unwrap();
    assert_eq!(done.wrote, vec![SERVICE, TIMER]);
    assert_eq!(names(&dir), vec![SERVICE.to_owned(), TIMER.to_owned()]);
    assert_eq!(
        std::fs::read_to_string(dir.join(TIMER)).unwrap(),
        plan().timer()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(SERVICE)).unwrap(),
        plan().service().unwrap()
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
}

#[test]
fn a_rerun_with_the_same_arguments_changes_nothing() {
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let before: Vec<_> = [SERVICE, TIMER]
        .iter()
        .map(|n| std::fs::metadata(dir.join(n)).unwrap().modified().unwrap())
        .collect();
    let again = Fake::default();
    let done = install(&dir, &plan(), &again).unwrap();
    assert!(done.wrote.is_empty());
    // No reload and no restart: the manager is only asked to make sure the
    // timer is enabled.
    assert_eq!(again.calls(), ["available", "enable scoot-look-auto.timer"]);
    let after: Vec<_> = [SERVICE, TIMER]
        .iter()
        .map(|n| std::fs::metadata(dir.join(n)).unwrap().modified().unwrap())
        .collect();
    assert_eq!(before, after, "files untouched");
    assert_eq!(names(&dir).len(), 2, "no temp files left behind");
}

#[test]
fn a_changed_schedule_rewrites_and_restarts() {
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let moved = Plan {
        schedule: schedule("06:00", "18:00"),
        ..plan()
    };
    let manager = Fake::default();
    let done = install(&dir, &moved, &manager).unwrap();
    assert_eq!(done.wrote, vec![SERVICE, TIMER]);
    assert!(
        std::fs::read_to_string(dir.join(TIMER))
            .unwrap()
            .contains("06:00:00")
    );
    assert!(
        manager
            .calls()
            .contains(&"restart scoot-look-auto.timer".to_owned())
    );
}

#[test]
fn no_user_manager_writes_nothing() {
    let (_root, dir) = unit_dir();
    let manager = Fake {
        down: true,
        ..Fake::default()
    };
    let error = install(&dir, &plan(), &manager).unwrap_err();
    assert!(error.contains("no systemd user manager"), "{error}");
    assert!(!dir.exists(), "not even the directory");
    assert_eq!(manager.calls(), ["available"]);
}

#[test]
fn a_foreign_file_refuses_before_either_is_written() {
    let (_root, dir) = unit_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(TIMER), "[Timer]\nOnCalendar=daily\n").unwrap();
    let manager = Fake::default();
    let error = install(&dir, &plan(), &manager).unwrap_err();
    assert!(
        error.contains("was not written by `scoot look auto`"),
        "{error}"
    );
    assert_eq!(
        names(&dir),
        vec![TIMER.to_owned()],
        "the service was not written either"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(TIMER)).unwrap(),
        "[Timer]\nOnCalendar=daily\n"
    );
    assert_eq!(manager.calls(), ["available"]);
}

#[test]
fn a_symlink_is_foreign_even_when_it_points_at_our_text() {
    let (root, dir) = unit_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let store = root.path().join("store-file");
    std::fs::write(&store, plan().timer()).unwrap();
    std::os::unix::fs::symlink(&store, dir.join(TIMER)).unwrap();
    assert_eq!(inspect(&dir.join(TIMER)).unwrap(), Slot::Foreign);
    assert!(install(&dir, &plan(), &Fake::default()).is_err());
    assert!(
        std::fs::symlink_metadata(dir.join(TIMER))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    // And off leaves it.
    let out = remove(&dir, &Fake::default()).unwrap();
    assert!(out.removed.is_empty());
    assert_eq!(out.left, vec![dir.join(TIMER)]);
    assert!(std::fs::symlink_metadata(dir.join(TIMER)).is_ok());
}

#[test]
fn a_failed_enable_leaves_files_a_rerun_heals() {
    let (_root, dir) = unit_dir();
    let broken = Fake {
        fail_enable: true,
        ..Fake::default()
    };
    assert!(install(&dir, &plan(), &broken).is_err());
    let healed = Fake::default();
    // The files are there and current, the enable is retried.
    assert!(install(&dir, &plan(), &healed).unwrap().wrote.is_empty());
    assert!(
        healed
            .calls()
            .contains(&"enable scoot-look-auto.timer".to_owned())
    );
}

#[test]
fn off_disables_before_it_deletes_and_leaves_nothing_of_ours() {
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    std::fs::write(dir.join("scootbar.service"), "[Service]\n").unwrap();
    let manager = Fake::default();
    let out = remove(&dir, &manager).unwrap();
    assert_eq!(out.removed, vec![TIMER, SERVICE]);
    assert!(out.left.is_empty());
    assert_eq!(
        manager.calls(),
        ["available", "disable scoot-look-auto.timer", "reload"]
    );
    assert_eq!(
        names(&dir),
        vec!["scootbar.service".to_owned()],
        "only ours removed"
    );
}

#[test]
fn off_with_nothing_installed_is_a_quiet_success() {
    let (_root, dir) = unit_dir();
    let manager = Fake::default();
    assert_eq!(remove(&dir, &manager).unwrap(), super::Removed::default());
    assert!(manager.calls().is_empty(), "the manager is not even asked");
    assert!(!dir.exists());
    // And twice after a real off.
    install(&dir, &plan(), &Fake::default()).unwrap();
    remove(&dir, &Fake::default()).unwrap();
    assert_eq!(
        remove(&dir, &Fake::default()).unwrap(),
        super::Removed::default()
    );
}

#[test]
fn off_without_a_manager_still_removes_the_files() {
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let down = Fake {
        down: true,
        ..Fake::default()
    };
    let out = remove(&dir, &down).unwrap();
    assert!(out.manager_skipped);
    assert!(names(&dir).is_empty());
}

#[test]
fn off_that_cannot_disable_deletes_nothing() {
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let manager = Fake {
        fail_disable: true,
        ..Fake::default()
    };
    assert!(remove(&dir, &manager).is_err());
    assert_eq!(
        names(&dir).len(),
        2,
        "a dangling wants link is worse than a retry"
    );
}

#[test]
fn status_reads_the_schedule_back() {
    let (_root, dir) = unit_dir();
    let off = status(&dir, &Fake::default()).unwrap();
    assert!(off.plan.is_none() && off.timer.is_none() && !off.partial);
    install(&dir, &plan(), &Fake::default()).unwrap();
    let on = status(&dir, &Fake::default()).unwrap();
    assert_eq!(on.plan, Some(Ok(plan())));
    assert!(!on.partial);
    assert_eq!(on.timer.unwrap().unwrap().active, "active");
}

#[test]
fn status_reports_damage_instead_of_trusting_it() {
    let (_root, dir) = unit_dir();
    install(&dir, &plan(), &Fake::default()).unwrap();
    let hand_edited = std::fs::read_to_string(dir.join(SERVICE))
        .unwrap()
        .replace("--dark moonrise", "--dark ../x");
    std::fs::write(dir.join(SERVICE), hand_edited).unwrap();
    assert!(
        status(&dir, &Fake::default())
            .unwrap()
            .plan
            .unwrap()
            .is_err()
    );
    std::fs::remove_file(dir.join(SERVICE)).unwrap();
    let half = status(&dir, &Fake::default()).unwrap();
    assert!(half.partial);
    assert!(half.plan.unwrap().is_err());
}

// ---- the binary path ----

#[test]
fn a_stable_path_entry_wins_over_a_store_path() {
    let root = tempfile::tempdir().unwrap();
    let store = root.path().join("store/abc-scoot/bin");
    let profile = root.path().join("profile/bin");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(store.join("scoot"), "x").unwrap();
    std::os::unix::fs::symlink(store.join("scoot"), profile.join("scoot")).unwrap();
    let path = std::env::join_paths([Path::new("relative/bin"), profile.as_path()]).unwrap();
    let real = std::fs::canonicalize(store.join("scoot")).unwrap();
    assert_eq!(stable_exe(&real, Some(&path)), profile.join("scoot"));
}

#[test]
fn another_scoot_on_path_does_not_replace_the_running_one() {
    let root = tempfile::tempdir().unwrap();
    let here = root.path().join("here");
    let other = root.path().join("other");
    std::fs::create_dir_all(&here).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(here.join("scoot"), "a").unwrap();
    std::fs::write(other.join("scoot"), "b").unwrap();
    let path = std::env::join_paths([other.as_path()]).unwrap();
    let current = std::fs::canonicalize(here.join("scoot")).unwrap();
    assert_eq!(stable_exe(&current, Some(&path)), current);
    assert_eq!(stable_exe(&current, None), current);
}

#[test]
fn an_executable_that_exists_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("scoot");
    std::fs::write(&exe, "x").unwrap();
    assert_eq!(unit_exe(&exe, None), Ok(exe));
}

#[test]
fn a_missing_executable_is_refused_with_the_rerun_advice() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("scoot");
    let message = unit_exe(&exe, None).unwrap_err();
    assert!(message.contains(&exe.display().to_string()), "{message}");
    assert!(message.contains("no longer exists"), "{message}");
    assert!(message.contains("run `scoot look auto` again"), "{message}");
}

#[test]
fn a_deleted_suffixed_executable_is_refused() {
    // What `current_exe()` reads back on Linux after an upgrade replaced
    // the running binary; the suffixed name is not a file.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("scoot"), "x").unwrap();
    let exe = dir.path().join("scoot (deleted)");
    let message = unit_exe(&exe, None).unwrap_err();
    assert!(message.contains("scoot (deleted)"), "{message}");
}

#[test]
fn systemctl_show_output_parses() {
    let state = parse_show(
        "ActiveState=active\nUnitFileState=enabled\nNextElapseUSecRealtime=Fri 2026-11-06 19:30:00 UTC\n",
    );
    assert_eq!(state.active, "active");
    assert_eq!(state.enabled, "enabled");
    assert_eq!(state.next, "Fri 2026-11-06 19:30:00 UTC");
    assert_eq!(parse_show("garbage"), TimerState::default());
}
