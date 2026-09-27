//! The glue on a live `State`: a stand-in for `scootbg` (a shell script
//! that records its argv and environment) is spawned, reaped through the
//! real `SIGCHLD` reaper, its exit logged, and runs serialized.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use scoot_ipc::Response;

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, capture_logs};
use crate::compositor::wallpaper::{
    PROFILE, PROFILE_HEADLESS, PROFILE_NESTED, Section, WallpaperSetting, profile_for,
};

type Fixture = Harness<(), ()>;

/// How long anything here may take: a debug build spawning `sh`.
const PATIENCE: Duration = Duration::from_secs(10);

/// A stand-in `scootbg`, in its own directory:
///
/// - `argv`: one line per run, each argument in brackets;
/// - `env`: the last run's `WAYLAND_DISPLAY`;
/// - `overlap`: written if a run started while another was running (the
///   `lock` directory is taken atomically by `mkdir`);
/// - `hold`: while it exists, a run waits before exiting;
/// - `status`: the exit status to end with (0 without it).
struct Stub {
    dir: tempfile::TempDir,
}

impl Stub {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let script = format!(
            r#"#!/bin/sh
d='{dir}'
mkdir "$d/lock" 2>/dev/null || echo overlap >>"$d/overlap"
line=''
for a in "$@"; do line="$line[$a]"; done
printf '%s\n' "$line" >>"$d/argv"
printf '%s\n' "$WAYLAND_DISPLAY" >"$d/env"
while [ -e "$d/hold" ]; do sleep 0.01; done
rmdir "$d/lock"
exit "$(cat "$d/status" 2>/dev/null || echo 0)"
"#,
            dir = dir.path().display()
        );
        let path = dir.path().join("scootbg");
        fs::write(&path, script).expect("the stub");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        Self { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("scootbg")
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn section(&self, json: &str) -> Section {
        Section {
            command: self.path().into_os_string(),
            json: json.to_owned(),
        }
    }

    fn setting(&self, json: &str) -> WallpaperSetting {
        WallpaperSetting::Section(self.section(json))
    }

    fn runs(&self) -> Vec<String> {
        fs::read_to_string(self.file("argv"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn hold(&self) {
        fs::write(self.file("hold"), "").unwrap();
    }

    fn release(&self) {
        fs::remove_file(self.file("hold")).unwrap();
    }

    fn exit_with(&self, status: u8) {
        fs::write(self.file("status"), status.to_string()).unwrap();
    }

    fn overlapped(&self) -> bool {
        self.file("overlap").exists()
    }
}

/// A live `State` with no backend. The `SIGCHLD` handler is deliberately
/// not installed here: it is process-global (the last install wins), so
/// under `cargo test` installing it would steal the wakeups of
/// `child_reaper`'s own tests running beside these. [`dispatch_until`]
/// calls `State::reap_children` itself instead, the same function the
/// handler's wake source calls, which `child_reaper/tests.rs` pins.
fn fixture() -> Fixture {
    Harness::bare(Appearance::default())
}

/// Dispatches (the loop runs the run timer) and reaps, until `done` holds,
/// failing after [`PATIENCE`].
fn dispatch_until(fixture: &mut Fixture, what: &str, mut done: impl FnMut(&Fixture) -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done(fixture) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
        fixture.state.reap_children();
    }
}

fn settle(fixture: &mut Fixture) {
    dispatch_until(fixture, "every run reaped", |fixture| {
        fixture.state.wallpaper.is_idle()
    });
}

fn run_line(profile: &str, json: &str) -> String {
    format!("[apply-config][--profile][{profile}][{json}]")
}

#[test]
fn a_section_runs_apply_config_with_the_profile_and_the_json() {
    let stub = Stub::new();
    let mut fixture = fixture();
    let json = r##"{"color":"#1e1e2e"}"##;
    fixture
        .state
        .start_wallpaper(&stub.setting(json), PROFILE_HEADLESS);
    settle(&mut fixture);
    assert_eq!(stub.runs(), [run_line("scoot-headless", json)]);
    let wayland = fs::read_to_string(stub.file("env")).unwrap();
    assert_eq!(
        wayland.trim_end(),
        fixture.state.socket_name.to_str().unwrap(),
        "the run gets the session's WAYLAND_DISPLAY"
    );
}

/// N3 (review of PR #297): each backend has its own profile, so a
/// headless session (an agent's, a test's) never writes the state the
/// user's real `--tty` login restores, nor shares a file with it.
#[test]
fn each_backend_has_its_own_profile() {
    assert_eq!(profile_for(true, false), PROFILE);
    assert_eq!(PROFILE, "scoot");
    assert_eq!(profile_for(false, true), PROFILE_NESTED);
    assert_eq!(PROFILE_NESTED, "scoot-nested");
    assert_eq!(profile_for(false, false), PROFILE_HEADLESS);
    assert_eq!(PROFILE_HEADLESS, "scoot-headless");
    assert_ne!(PROFILE_HEADLESS, PROFILE);
}

#[test]
fn a_nested_session_uses_its_own_profile() {
    let stub = Stub::new();
    let mut fixture = fixture();
    fixture
        .state
        .start_wallpaper(&stub.setting("{}"), PROFILE_NESTED);
    settle(&mut fixture);
    assert_eq!(stub.runs(), [run_line("scoot-nested", "{}")]);
}

#[test]
fn no_section_at_startup_runs_nothing() {
    let stub = Stub::new();
    let mut fixture = fixture();
    fixture
        .state
        .start_wallpaper(&WallpaperSetting::Absent, PROFILE_HEADLESS);
    let (_, logs) = capture_logs(|| {
        fixture.state.start_wallpaper(
            &WallpaperSetting::Invalid("broken".into()),
            PROFILE_HEADLESS,
        )
    });
    assert!(fixture.state.wallpaper.is_idle());
    assert!(stub.runs().is_empty());
    assert!(logs.contains("cannot be used"), "{logs}");
    assert!(logs.contains("broken"), "{logs}");
}

/// Every reload re-runs the section (an unchanged one included), and the
/// reload that removes it sends `{}` through the removed section's command.
#[test]
fn reloads_rerun_the_section_and_a_removal_sends_the_empty_one() {
    let stub = Stub::new();
    let mut fixture = fixture();
    let a = r##"{"color":"#000001"}"##;
    fixture
        .state
        .start_wallpaper(&stub.setting(a), PROFILE_HEADLESS);
    settle(&mut fixture);
    fixture.state.reload_wallpaper(&stub.setting(a));
    settle(&mut fixture);
    fixture.state.reload_wallpaper(&WallpaperSetting::Absent);
    settle(&mut fixture);
    // Absent again: nothing more to send.
    fixture.state.reload_wallpaper(&WallpaperSetting::Absent);
    settle(&mut fixture);
    assert_eq!(
        stub.runs(),
        [
            run_line("scoot-headless", a),
            run_line("scoot-headless", a),
            run_line("scoot-headless", "{}")
        ]
    );
}

/// Reloads while a run is in flight never start a second one beside it,
/// and only the newest waiting section runs after it.
#[test]
fn runs_never_overlap_and_the_newest_waiting_section_wins() {
    let stub = Stub::new();
    let mut fixture = fixture();
    stub.hold();
    fixture
        .state
        .start_wallpaper(&stub.setting("\"A\""), PROFILE_HEADLESS);
    dispatch_until(&mut fixture, "A to start", |_| stub.runs().len() == 1);
    fixture.state.reload_wallpaper(&stub.setting("\"B\""));
    fixture.state.reload_wallpaper(&stub.setting("\"C\""));
    // Give a wrongly started second run every chance to show itself.
    let until = Instant::now() + Duration::from_millis(200);
    dispatch_until(&mut fixture, "a quiet spell", |_| Instant::now() >= until);
    assert_eq!(stub.runs().len(), 1, "B and C wait for A");
    stub.release();
    dispatch_until(&mut fixture, "C to run", |_| stub.runs().len() == 2);
    settle(&mut fixture);
    assert_eq!(
        stub.runs(),
        [
            run_line("scoot-headless", "\"A\""),
            run_line("scoot-headless", "\"C\"")
        ],
        "B never ran"
    );
    assert!(!stub.overlapped(), "two runs overlapped");
}

#[test]
fn a_failed_run_is_logged_with_its_status_and_retried_on_a_reload() {
    let stub = Stub::new();
    let mut fixture = fixture();
    stub.exit_with(2);
    let (_, logs) = capture_logs(|| {
        fixture
            .state
            .start_wallpaper(&stub.setting("{}"), PROFILE_HEADLESS);
        settle(&mut fixture);
    });
    assert!(logs.contains("exit status 2"), "{logs}");
    assert!(
        logs.contains(stub.path().to_str().unwrap()),
        "names the command: {logs}"
    );
    assert_eq!(stub.runs().len(), 1, "no retry in a loop");
    // A reload that brings nothing new (the section is still absent from
    // this reload's point of view: invalid) still retries the held one.
    stub.exit_with(0);
    let (_, logs) = capture_logs(|| {
        fixture
            .state
            .reload_wallpaper(&WallpaperSetting::Invalid("x".into()));
        settle(&mut fixture);
    });
    assert_eq!(stub.runs().len(), 2, "retried on the reload");
    assert!(logs.contains("scootbg applied"), "{logs}");
}

/// A missing binary is a warning naming the command and what to do, never
/// a failure of the session; the next reload tries again.
#[test]
fn a_missing_binary_is_a_warning_and_retried_on_a_reload() {
    let mut fixture = fixture();
    let missing = Section {
        command: "/nonexistent/scoot-test/scootbg".into(),
        json: "{}".to_owned(),
    };
    let (_, logs) = capture_logs(|| {
        fixture.state.start_wallpaper(
            &WallpaperSetting::Section(missing.clone()),
            PROFILE_HEADLESS,
        )
    });
    assert!(fixture.state.wallpaper.is_idle(), "nothing started");
    assert!(logs.contains("was not found"), "{logs}");
    assert!(logs.contains("/nonexistent/scoot-test/scootbg"), "{logs}");
    assert!(logs.contains("Install scootbg"), "{logs}");
    let (_, logs) = capture_logs(|| {
        fixture
            .state
            .reload_wallpaper(&WallpaperSetting::Section(missing))
    });
    assert!(logs.contains("was not found"), "tried again: {logs}");
}

/// F4: a run that never ends is waited on only up to the bound, then the
/// newest queued section runs; the abandoned run is still reaped and its
/// end logged.
#[test]
fn a_hung_run_is_abandoned_after_the_bound() {
    let stub = Stub::new();
    let mut fixture = fixture();
    fixture
        .state
        .wallpaper
        .queue_mut()
        .set_patience(Duration::from_millis(300));
    stub.hold();
    let (_, logs) = capture_logs(|| {
        fixture
            .state
            .start_wallpaper(&stub.setting("\"A\""), PROFILE_HEADLESS);
        dispatch_until(&mut fixture, "A to start", |_| stub.runs().len() == 1);
        fixture.state.reload_wallpaper(&stub.setting("\"B\""));
        // The timer, not a reap, is what moves on: A is still running.
        dispatch_until(&mut fixture, "B to start after A's deadline", |_| {
            stub.runs().len() == 2
        });
        stub.release();
        settle(&mut fixture);
    });
    assert!(logs.contains("no longer waiting on it"), "{logs}");
    assert_eq!(stub.runs()[1], run_line("scoot-headless", "\"B\""));
    assert!(
        logs.contains("abandoned=true"),
        "the abandoned run's end is logged: {logs}"
    );
    // B ran beside the abandoned A: that is the point of the bound.
    assert!(stub.overlapped());
}

/// Through `State::reload`: the reply names the section when it changed,
/// is silent when it did not, and refuses a broken one by name while the
/// rest of the file applies.
#[test]
fn a_reload_reports_the_section() {
    let stub = Stub::new();
    let mut fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fixture.state.config_path = Some(path.clone());
    let write = |text: &str| fs::write(&path, text).unwrap();
    let command = stub.path();
    let command = command.to_str().unwrap();
    let reload = |fixture: &mut Fixture| match fixture.state.reload() {
        Response::Reloaded { applied, refused } => (applied, refused),
        other => panic!("unexpected reply {other:?}"),
    };

    write(&format!(
        "[wallpaper]\ncommand = \"{command}\"\ncolor = \"#000001\"\n"
    ));
    let (applied, refused) = reload(&mut fixture);
    assert_eq!(applied, ["wallpaper"]);
    assert!(refused.is_empty(), "{refused:?}");
    settle(&mut fixture);

    let (applied, refused) = reload(&mut fixture);
    assert!(
        applied.is_empty() && refused.is_empty(),
        "unchanged: {applied:?} {refused:?}"
    );
    settle(&mut fixture);

    write(&format!(
        "[layout]\ngap = 20\n[wallpaper]\ncommand = \"{command}\"\ncolour = \"#000002\"\n"
    ));
    let (applied, refused) = reload(&mut fixture);
    assert_eq!(applied, ["layout.gap"], "the rest of the file applies");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].starts_with("wallpaper ("), "{refused:?}");
    assert!(refused[0].contains("wallpaper.colour"), "{refused:?}");
    assert!(
        refused[0].contains("kept the running section"),
        "{refused:?}"
    );
    settle(&mut fixture);

    write("[layout]\ngap = 20\n");
    let (applied, _) = reload(&mut fixture);
    assert_eq!(applied, ["wallpaper"], "the removal is reported");
    settle(&mut fixture);
    assert_eq!(
        stub.runs(),
        [
            run_line("scoot-headless", r##"{"color":"#000001"}"##),
            run_line("scoot-headless", r##"{"color":"#000001"}"##),
            run_line("scoot-headless", r##"{"color":"#000001"}"##),
            run_line("scoot-headless", "{}"),
        ],
        "an unchanged section still runs; a refused one re-runs the running section (N6); \
         a removal sends {{}}"
    );
}

/// A changed `command` alone is reported under its own name.
#[test]
fn a_new_command_is_reported_apart() {
    let stub = Stub::new();
    let other = Stub::new();
    let mut fixture = fixture();
    fixture
        .state
        .start_wallpaper(&stub.setting("{}"), PROFILE_HEADLESS);
    settle(&mut fixture);
    let reloaded = fixture.state.reload_wallpaper(&other.setting("{}"));
    assert_eq!(
        reloaded,
        crate::compositor::wallpaper::Reloaded::Applied {
            values: false,
            command: true
        }
    );
    settle(&mut fixture);
    assert_eq!(other.runs().len(), 1, "the new command ran");
}
