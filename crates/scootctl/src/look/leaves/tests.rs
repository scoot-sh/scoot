//! Tests for the leaf steps: every backend scripts through fakes, so no
//! test touches the session bus or a real daemon.

use super::*;

fn look_named(name: &str, night: Option<i64>) -> scoot_look::ResolvedLook {
    use std::collections::BTreeMap;
    let text = super::super::catalog::tests::look_toml(name);
    let raw = scoot_look::parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([(name.to_string(), (raw, scoot_look::Origin::User))]);
    let mut look = scoot_look::resolve(&map, name).expect("resolves");
    look.night_temperature = night;
    look
}

#[test]
fn all_backends_ok() {
    let report = apply_leaves(
        Path::new("/state"),
        &look_named("moonrise", Some(3400)),
        &ScriptedLeaves::ok(),
    );
    assert_eq!(
        report.reloaded,
        vec![
            "mako",
            "fcitx5",
            "scoot-osd unit",
            "scoot-nightlight unit (3400K)"
        ]
    );
    assert!(report.notes.is_empty(), "notes: {:?}", report.notes);
}

#[test]
fn missing_backends_are_notes() {
    let leaves = ScriptedLeaves::failing();
    let report = apply_leaves(
        Path::new("/state"),
        &look_named("moonrise", Some(3400)),
        &leaves,
    );
    assert!(report.reloaded.is_empty());
    assert_eq!(report.notes.len(), 4, "notes: {:?}", report.notes);
    assert!(report.notes.iter().any(|n| n.contains("mako")));
    assert!(report.notes.iter().any(|n| n.contains("fcitx5")));
    assert!(report.notes.iter().any(|n| n.contains("wob")));
    assert!(report.notes.iter().any(|n| n.contains("3400K")));
    // Both units were still attempted (best-effort runs everything).
    assert_eq!(
        leaves.restarted().as_slice(),
        &["scoot-osd", "scoot-nightlight"]
    );
}

#[test]
fn no_night_temperature_skips_the_unit() {
    let leaves = ScriptedLeaves::ok();
    let report = apply_leaves(Path::new("/state"), &look_named("x", None), &leaves);
    assert_eq!(report.reloaded, vec!["mako", "fcitx5", "scoot-osd unit"]);
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("no night temperature")),
        "notes: {:?}",
        report.notes
    );
    assert_eq!(leaves.restarted().as_slice(), &["scoot-osd"]);
}

#[test]
fn private_state_runs_nothing() {
    let leaves = ScriptedLeaves::ok();
    let dir = Path::new("/run/user/1000/scoot-look-test");
    let report = apply_leaves(dir, &look_named("moonrise", Some(3400)), &leaves);
    assert!(report.reloaded.is_empty());
    assert_eq!(report.notes.len(), 1);
    assert!(report.notes[0].contains("private look state"));
    assert!(leaves.restarted().is_empty());
}

/// A unit that was never installed is skipped silently: no reload, no
/// note. (On a non-systemd box every swap would otherwise nag about
/// units that never existed.)
#[test]
fn absent_units_are_silent() {
    struct AbsentUnits;
    impl super::Leaves for AbsentUnits {
        fn mako_reload(&self) -> Result<(), String> {
            Ok(())
        }
        fn fcitx5_reload(&self) -> Result<(), String> {
            Ok(())
        }
        fn unit_try_restart(&self, _unit: &str) -> Result<(), String> {
            Err(super::UNIT_ABSENT.to_owned())
        }
    }
    let report = apply_leaves(
        Path::new("/state"),
        &look_named("moonrise", Some(3400)),
        &AbsentUnits,
    );
    assert_eq!(report.reloaded, vec!["mako", "fcitx5"]);
    assert!(report.notes.is_empty(), "notes: {:?}", report.notes);
}

#[test]
fn unit_absent_patterns() {
    for output in [
        "Unit scoot-osd.service not found.",
        "Failed to get unit: Unit not found.",
        "Unknown unit: scoot-osd.service",
        "System has not been booted with systemd as init system.",
        "No such file or directory",
        "Could not find unit scoot-osd",
    ] {
        assert!(super::is_unit_absent(output), "absent: {output}");
    }
    for output in [
        "Failed to restart unit: Unit entered failed state.",
        "no `systemctl` on PATH",
        "Job failed. See \"journalctl --user -xe\" for details.",
    ] {
        assert!(!super::is_unit_absent(output), "present: {output}");
    }
}
