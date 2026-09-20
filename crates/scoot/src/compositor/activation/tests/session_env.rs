//! What session-identity environment a [`State::spawn`] child observes.
//!
//! [`resolve`](crate::compositor::session_env::resolve)'s ownership rules
//! (unconditional vs fill-the-vacuum) are pinned as pure unit tests beside
//! the function, where no process-global environment can interfere. What
//! needs a live child is the spawn site applying them: a `sh` probe writes
//! whatever the three variables it saw to a file -- the same shape
//! [`spawn`](super::spawn) uses for the activation token -- which proves the
//! variables are really in the child's environment, not just set on a
//! `Command` nobody executed.
//!
//! The `XDG_CURRENT_DESKTOP` assertion is value-shaped (`scoot`, exactly):
//! `resolve` is unconditional, so the test passes on any host no matter what
//! the test process itself inherited, and removing the spawn-site lines fails
//! it everywhere (an unset variable on CI, the host desktop's name on a
//! developer machine). The other two are presence-shaped: they are
//! fill-the-vacuum, so their values legitimately depend on the host, but
//! `resolve` guarantees neither is ever empty.
//!
//! Like every other live-[`State`] test module here, these need a writable
//! `$XDG_RUNTIME_DIR` ([`State::new`] binds a real listening socket) and a
//! real `sh`. [`Harness::bare`] is enough: spawning needs no mapped client.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::compositor::session_env::{
    CURRENT_DESKTOP, DESKTOP_NAME, SESSION_DESKTOP, SESSION_TYPE,
};

/// A live compositor with no clients, the shape the fd-inheritance test in
/// [`spawn`](super::spawn) builds: spawning needs a [`State`], nothing more.
type Fixture = Harness<(), ()>;

/// Spawns a real child through the real [`State::spawn`] that writes what it
/// saw for each session variable, one `name=value` line per line, to a fresh
/// file, and hands back that file's path.
///
/// An explicitly unset variable writes `name=<unset>` rather than an empty
/// value, so "the child ran and the variable was absent" stays
/// distinguishable from "the child never ran" (no file at all, which
/// [`read_probe`] reports as a timeout instead).
fn spawn_probe(fixture: &mut Fixture) -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "scoot-spawn-session-env-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let script = format!(
        "for v in {CURRENT_DESKTOP} {SESSION_TYPE} {SESSION_DESKTOP}; do \
         eval \"val=\\\"\\${{$v:-<unset>}}\\\"\"; printf '%s=%s\\n' \"$v\" \"$val\"; \
         done > \"$1\""
    );
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        script,
        "sh".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    fixture.state.spawn(&command);
    path
}

/// Waits for the probe child's file to appear with all three lines, reads
/// them, and removes the file.
fn read_probe(path: &Path) -> Vec<(String, String)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(body) = std::fs::read_to_string(path)
            && body.lines().count() >= 3
        {
            let _ = std::fs::remove_file(path);
            return body
                .lines()
                .filter_map(|line| {
                    line.split_once('=')
                        .map(|(name, value)| (name.to_string(), value.to_string()))
                })
                .collect();
        }
        assert!(
            Instant::now() < deadline,
            "the spawned child never wrote its session-environment file: {path:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn value_of<'a>(seen: &'a [(String, String)], name: &str) -> &'a str {
    seen.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or_else(|| panic!("the probe child reported no {name} line: {seen:?}"))
}

#[test]
fn a_spawned_child_sees_the_session_environment() {
    let mut fixture: Fixture = Harness::bare(Appearance::default());
    let path = spawn_probe(&mut fixture);
    let seen = read_probe(&path);
    assert_eq!(
        value_of(&seen, CURRENT_DESKTOP),
        DESKTOP_NAME,
        "State::spawn gave its child no XDG_CURRENT_DESKTOP=scoot"
    );
    for name in [SESSION_TYPE, SESSION_DESKTOP] {
        let value = value_of(&seen, name);
        assert!(
            !value.is_empty() && value != "<unset>",
            "State::spawn left {name} empty in its child"
        );
    }
}
