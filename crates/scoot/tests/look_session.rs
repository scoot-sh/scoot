#![cfg(target_os = "linux")]
//! Per-session look state, end to end: a `--headless` session seeds a
//! private state from the user's, exports it to its children, and a swap
//! inside the session leaves the user state byte-identical.
//!
//! The session runs the real `scoot` binary (`CARGO_BIN_EXE_scoot`, the way
//! `msg_broken_pipe.rs` does) with scratch `XDG_STATE_HOME`,
//! `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME` and `HOME`, so no developer's real
//! look is ever in reach. A swap is simulated by writing the private
//! `look.json` and repointing the private link directly (the `apply` tool
//! that will own that write lands in `look-apply-cli`); what this pins is
//! the isolation the tool will rely on: the seed, the export, and the user
//! state staying byte-identical across a reload of a changed private link.
//!
//! Evidence this does not cover (no host compositor here): a `--nested`
//! session's swap persisting across a restart (it resolves the user dir --
//! pinned by the `wants_private_session` unit test -- and nothing in a
//! restart deletes the user state) and `--private-look` on nested (same
//! seed path as headless, with the display's own socket name).

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long anything here waits: a debug `scoot` starts in well under a
/// second, so this only bounds a wedge.
const PATIENCE: Duration = Duration::from_secs(20);

fn scoot() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scoot"))
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "scoot-look-session-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        Self { dir }
    }

    fn sub(&self, name: &str) -> PathBuf {
        let dir = self.dir.join(name);
        std::fs::create_dir_all(&dir).expect("a scratch subdir");
        // The runtime dir holds Wayland sockets: owner-only, like a real
        // login session provides.
        #[cfg(unix)]
        if name == "runtime" {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .expect("owner-only runtime dir");
        }
        dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

struct Session {
    child: Child,
    socket: PathBuf,
    state_home: PathBuf,
    runtime: PathBuf,
    env_out: PathBuf,
    log: PathBuf,
}

impl Session {
    fn start(scratch: &Scratch) -> Self {
        let state_home = scratch.sub("state-home");
        let runtime = scratch.sub("runtime");
        let config_home = scratch.sub("config-home");
        let home = scratch.sub("home");
        let socket = runtime.join("scoot.sock");
        let env_out = scratch.dir.join("child-env");

        // The user's choice: a record plus a link at a rendered look, the
        // shape `apply` will maintain (owned by that ticket).
        let rendered = scratch.dir.join("rendered-moonrise");
        std::fs::create_dir(&rendered).expect("a rendered dir");
        std::fs::write(rendered.join("scoot.toml"), "[appearance]\n").expect("a fragment");
        let user_state = state_home.join("scoot");
        std::fs::create_dir(&user_state).expect("the user state dir");
        let record = r#"{"name":"moonrise","rendered":"PLACEHOLDER","generation":7,"chosen-by":"apply","declared-default":"ginger-night"}"#
            .replace("PLACEHOLDER", &rendered.to_string_lossy());
        std::fs::write(user_state.join("look.json"), &record).expect("the user record");
        symlink(&rendered, user_state.join("look")).expect("the user link");

        // The `--` command reports the look state the session hands its
        // children (or its absence), then sleeps so the session stays up.
        // Session logs go to a file: a startup failure then names itself
        // instead of panicking blind.
        let log = scratch.dir.join("scoot.log");
        let log_file = std::fs::File::create(&log).expect("a log file");
        let env_out_str = env_out.to_string_lossy().into_owned();
        let child = Command::new(scoot())
            .arg("--headless")
            .arg("--socket")
            .arg(&socket)
            .arg("--")
            .arg("sh")
            .arg("-c")
            .arg(format!(
                "echo \"${{SCOOT_LOOK_STATE:-UNSET}}\" > \"{env_out_str}\"; sleep 60"
            ))
            .env("XDG_STATE_HOME", &state_home)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("XDG_CONFIG_HOME", &config_home)
            .env("HOME", &home)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("SCOOT_SOCKET")
            .env_remove("SCOOT_LOOK_STATE")
            .stdout(Stdio::null())
            .stderr(log_file)
            .spawn()
            .expect("a headless scoot");
        let mut session = Self {
            child,
            socket,
            state_home,
            runtime,
            env_out,
            log,
        };
        session.wait_until_up();
        session
    }

    /// Readiness is "the IPC socket answers `version`", the same readiness
    /// `resources/scoot-session` waits for -- never a blind sleep.
    fn wait_until_up(&mut self) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let up = Command::new(scoot())
                .arg("msg")
                .arg("version")
                .env("SCOOT_SOCKET", &self.socket)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            if up {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the headless session never answered IPC; log:\n{}",
                self.log_tail()
            );
            assert!(
                self.child.try_wait().expect("waitable").is_none(),
                "the headless session exited during startup; log:\n{}",
                self.log_tail()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let mut tail = text.lines().rev().take(20).collect::<Vec<_>>();
        tail.reverse();
        tail.join("\n")
    }

    fn msg(&self, args: &[&str]) -> std::process::Output {
        Command::new(scoot())
            .arg("msg")
            .args(args)
            .env("SCOOT_SOCKET", &self.socket)
            .output()
            .expect("scoot msg runs")
    }

    /// The session's private state dir: the one `scoot-look-<display>/`
    /// entry under the runtime dir.
    fn private_dir(&self) -> PathBuf {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&self.runtime).expect("readable runtime dir") {
            let entry = entry.expect("an entry");
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("scoot-look-")
            {
                found.push(entry.path());
            }
        }
        assert_eq!(found.len(), 1, "one private dir, not {found:?}");
        found.pop().expect("one private dir")
    }

    fn user_record(&self) -> Vec<u8> {
        std::fs::read(self.state_home.join("scoot").join("look.json")).expect("the user record")
    }

    fn user_link(&self) -> PathBuf {
        std::fs::read_link(self.state_home.join("scoot").join("look")).expect("the user link")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

fn wait_for_env(env_out: &Path) -> String {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Ok(text) = std::fs::read_to_string(env_out) {
            let trimmed = text.trim().to_owned();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the session child never reported"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_headless_session_seeds_exports_and_isolates_its_private_state() {
    let scratch = Scratch::new("private");
    let session = Session::start(&scratch);
    let user_record_before = session.user_record();
    let user_link_before = session.user_link();

    // The seed: the private record copies the user's byte for byte, and the
    // private link names the same rendered dir.
    let private = session.private_dir();
    assert_eq!(
        std::fs::read(private.join("look.json")).expect("the seeded record"),
        user_record_before,
        "the seed copies the user record byte for byte"
    );
    assert_eq!(
        std::fs::read_link(private.join("look")).expect("the seeded link"),
        user_link_before,
        "the seed replicates the user link's target"
    );

    // The export: the session's direct child sees exactly this private dir.
    assert_eq!(
        wait_for_env(&session.env_out),
        private.to_string_lossy(),
        "only the private session exports SCOOT_LOOK_STATE, to its direct children"
    );

    // A swap inside the session: a new rendered look, a new record, the
    // link repointed -- the write `apply` will own (in `look-apply-cli`).
    let rendered = scratch.dir.join("rendered-ginger");
    std::fs::create_dir(&rendered).expect("a second rendered dir");
    std::fs::write(rendered.join("scoot.toml"), "[appearance]\n").expect("a fragment");
    let record = format!(
        r#"{{"name":"ginger-night","rendered":"{}","generation":8,"chosen-by":"apply"}}"#,
        rendered.to_string_lossy()
    );
    std::fs::write(private.join("look.json"), record).expect("the swapped record");
    std::fs::remove_file(private.join("look")).expect("the old link");
    symlink(&rendered, private.join("look")).expect("the repointed link");
    let reload = session.msg(&["reload"]);
    assert!(
        reload.status.success(),
        "a reload of the swapped private state applies: {}",
        String::from_utf8_lossy(&reload.stderr)
    );

    // The isolation: the user state is byte-identical after the swap.
    assert_eq!(
        session.user_record(),
        user_record_before,
        "a headless session's swap leaves the user record byte-identical"
    );
    assert_eq!(
        session.user_link(),
        user_link_before,
        "a headless session's swap leaves the user link where it was"
    );

    // And the session is still up on its new look.
    let version = session.msg(&["version"]);
    assert!(
        version.status.success(),
        "the session survives its own swap"
    );
}

#[test]
fn a_clean_exit_removes_the_private_dir() {
    let scratch = Scratch::new("exit-clean");
    let mut session = Session::start(&scratch);
    let private = session.private_dir();
    assert!(
        private.join("look.json").is_file(),
        "the session seeds its private dir first"
    );

    let quit = session.msg(&["action", "quit"]);
    assert!(
        quit.status.success(),
        "quit stops the loop cleanly: {}",
        String::from_utf8_lossy(&quit.stderr)
    );
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(status) = session.child.try_wait().expect("waitable") {
            assert!(status.success(), "a quit session exits zero, not {status}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the session never exited after quit; log:\n{}",
            session.log_tail()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        std::fs::symlink_metadata(&private).is_err(),
        "a clean exit removes the private dir"
    );
}
