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
        matches!(outcome.load, FragmentLoad::Absent(FragmentAbsent::Missing)),
        "no link is the default state, not a problem"
    );
    assert_eq!(outcome.warn, None);
}

#[test]
fn a_dangling_link_warns_and_loads_nothing() {
    let dir = tempfile::tempdir().expect("a temp dir");
    symlink(dir.path().join("gone"), dir.path().join("look")).expect("a dangling link");
    let outcome = load(dir.path());
    assert!(matches!(
        outcome.load,
        FragmentLoad::Absent(FragmentAbsent::Broken)
    ));
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

fn link_rendered_to(dir: &std::path::Path) -> std::path::PathBuf {
    let rendered = dir.join("rendered");
    fs::create_dir(&rendered).expect("a rendered dir");
    symlink(&rendered, dir.join("look")).expect("a link");
    rendered
}

fn load_on_thread(state_dir: std::path::PathBuf) -> super::LoadOutcome {
    // A FIFO with no writer blocks a plain `read_to_string` forever, which
    // would hang the test suite itself on the pre-fix code: read on a
    // thread with a timeout, so the failure is a failed test, never a wedge.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done_tx.send(load(&state_dir));
    });
    done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("reading the layer must not block: the load took over 5 s")
}

#[test]
fn a_fifo_fragment_refuses_without_blocking() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let rendered = link_rendered_to(dir.path());
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        rendered.join(super::FRAGMENT),
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .expect("a fifo");
    let outcome = load_on_thread(dir.path().to_owned());
    assert!(
        matches!(outcome.load, FragmentLoad::Absent(FragmentAbsent::Broken)),
        "a FIFO is not a layer"
    );
    let warn = outcome.warn.expect("a FIFO warns");
    assert!(
        warn.contains("regular"),
        "the warning names the refusal: {warn}"
    );
}

#[test]
fn a_directory_fragment_refuses() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let rendered = link_rendered_to(dir.path());
    fs::create_dir(rendered.join(super::FRAGMENT)).expect("a directory at the fragment path");
    let outcome = load_on_thread(dir.path().to_owned());
    assert!(
        matches!(outcome.load, FragmentLoad::Absent(FragmentAbsent::Broken)),
        "a directory is not a layer"
    );
}

#[test]
fn an_oversize_fragment_refuses_without_reading_it_whole() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let rendered = link_rendered_to(dir.path());
    let big = "[appearance]\nbackground_color = \"#112233\"\n".repeat(4096);
    assert!(
        big.len() as u64 > super::MAX_FILE,
        "the test file is past the cap"
    );
    fs::write(rendered.join(super::FRAGMENT), big).expect("a big fragment");
    let outcome = load(dir.path());
    assert!(
        matches!(outcome.load, FragmentLoad::Absent(FragmentAbsent::Broken)),
        "an oversize fragment is refused"
    );
    let warn = outcome.warn.expect("an oversize fragment warns");
    assert!(
        warn.contains("larger"),
        "the warning names the refusal: {warn}"
    );
}

#[test]
fn an_unreadable_identity_falls_back_without_blocking() {
    // `render.json` is advisory: a FIFO, a directory or an oversize file
    // there must neither hang the load nor refuse the layer -- the fragment
    // loads with its content-hash identity instead.
    let dir = tempfile::tempdir().expect("a temp dir");
    let rendered = link_rendered_to(dir.path());
    fs::write(
        rendered.join(super::FRAGMENT),
        "[appearance]\nbackground_color = \"#112233\"\n",
    )
    .expect("a fragment");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        rendered.join(super::RENDER_JSON),
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .expect("a fifo");
    let outcome = load_on_thread(dir.path().to_owned());
    let FragmentLoad::Present(fragment) = outcome.load else {
        panic!("a FIFO identity must not refuse the layer");
    };
    assert_eq!(outcome.warn, None);
    assert_eq!(fragment.identity.name, None);
    assert!(
        fragment.identity.source_hash.starts_with("fragment:"),
        "the fallback is the content hash: {}",
        fragment.identity.source_hash
    );
}

#[test]
fn equal_length_fragments_without_metadata_compare_different() {
    let first = super::fragment_fallback("[appearance]\ngap = 1\n");
    let second = super::fragment_fallback("[appearance]\ngap = 2\n");
    assert_ne!(
        first, second,
        "same-length fragments must not share an identity"
    );
}
