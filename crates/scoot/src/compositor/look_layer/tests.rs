//! The look state dir and fragment loading: resolution order, the missing
//! link staying silent, and a dangling link warning rather than failing.
//!
//! Merge rules, strict validation and the bad-user-value fallback live
//! beside the types they merge in `config.rs`'s tests; end-to-end reload
//! behavior (precedence, shadowed keys, the `Look` event) in
//! `reload/tests.rs`. This file pins only what this module owns.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use super::{FragmentAbsent, FragmentLoad, load, state_dir};

fn os(value: &str) -> Option<OsString> {
    Some(OsString::from(value))
}

#[test]
fn an_explicit_dir_wins_over_every_env_var() {
    assert_eq!(
        state_dir(
            Some(Path::new("/explicit")),
            os("/env"),
            os("/xdg"),
            os("/home"),
        ),
        Some(Path::new("/explicit").to_owned())
    );
}

#[test]
fn the_env_wins_over_the_home_fallback_in_order() {
    assert_eq!(
        state_dir(None, os("/env"), os("/xdg"), os("/home")),
        Some(Path::new("/env").to_owned())
    );
    assert_eq!(
        state_dir(None, None, os("/xdg"), os("/home")),
        Some(Path::new("/xdg/scoot").to_owned())
    );
    assert_eq!(
        state_dir(None, None, None, os("/home")),
        Some(Path::new("/home/.local/state/scoot").to_owned())
    );
    assert_eq!(state_dir(None, None, None, None), None);
}

#[test]
fn empty_env_vars_fall_through_like_unset_ones() {
    assert_eq!(
        state_dir(None, os(""), os(""), os("")),
        None,
        "empty vars must not resolve to the current directory"
    );
    assert_eq!(
        state_dir(None, os(""), os("/xdg"), os("/home")),
        Some(Path::new("/xdg/scoot").to_owned())
    );
}

#[test]
fn a_missing_link_is_a_silent_absence() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let outcome = load(dir.path());
    assert!(
        matches!(
            outcome.load,
            FragmentLoad::Absent(FragmentAbsent::Missing)
        ),
        "no link is the default state, not a problem"
    );
    assert_eq!(outcome.warn, None);
}

#[test]
fn a_dangling_link_warns_and_loads_nothing() {
    let dir = tempfile::tempdir().expect("a temp dir");
    symlink(dir.path().join("gone"), dir.path().join("look")).expect("a dangling link");
    let outcome = load(dir.path());
    assert!(
        matches!(
            outcome.load,
            FragmentLoad::Absent(FragmentAbsent::Broken)
        )
    );
    let warn = outcome.warn.expect("a dangling link warns");
    assert!(
        warn.contains("cannot be read"),
        "the warning names the problem: {warn}"
    );
}

#[test]
fn a_fragment_loads_with_its_identity() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let rendered = dir.path().join("rendered");
    fs::create_dir(&rendered).expect("a rendered dir");
    fs::write(
        rendered.join("scoot.toml"),
        "[appearance]\nbackground_color = \"#112233\"\n",
    )
    .expect("a fragment");
    fs::write(
        rendered.join("render.json"),
        r#"{"name":"moonrise","source_hash":"9f2c"}"#,
    )
    .expect("an identity");
    symlink(&rendered, dir.path().join("look")).expect("a link");
    let outcome = load(dir.path());
    let FragmentLoad::Present(fragment) = outcome.load else {
        panic!("a linked fragment loads");
    };
    assert_eq!(outcome.warn, None);
    assert_eq!(fragment.identity.name.as_deref(), Some("moonrise"));
    assert_eq!(fragment.identity.source_hash, "9f2c");
    assert!(fragment.text.contains("background_color"));
}
