//! The panel tests' harness: a headless scoot beside fixture
//! applications, and stub launch targets recording what the launch
//! gave them. Shared by the panel tests and the spawn tests.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub use scootui::test_support::rgb;
use scootui::test_support::{Session, Shot};

/// The require variable turning these tests' skips into failures.
pub const REQUIRE: &str = "SCOOTLAUNCH_REQUIRE_SCOOT";

pub fn launch_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scootlaunch"))
}

/// Shared across the panel and spawn test targets; each target uses a subset.
#[allow(dead_code)]
pub fn desktop(name: &str, exec: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n")
}

/// A session with a fixture applications tree: `apps` holds
/// `name.desktop` files, and nothing else is indexed.
pub fn session_with_apps(tag: &str, apps: &[(&str, &str)]) -> Option<(Session, PathBuf)> {
    let session = Session::scoot_requiring(&launch_bin(), tag, 1, "", REQUIRE)?;
    let data_home = session.scratch.0.join("data-home");
    let applications = data_home.join("applications");
    fs::create_dir_all(&applications).unwrap();
    for (file, body) in apps {
        fs::write(applications.join(file), body).unwrap();
    }
    let empty_dirs = session.scratch.0.join("empty-dirs");
    fs::create_dir_all(&empty_dirs).unwrap();
    Some((session, data_home))
}

fn shell_quote(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy().replace('\'', "'\\''");
    format!("'{raw}'")
}

/// A session whose fixture entries launch stub scripts: each `name`
/// gets `name.desktop` (Name `name`,quoted `Exec`) running a script
/// that appends `"$0 ${XDG_ACTIVATION_TOKEN-unset}
/// ${DESKTOP_STARTUP_ID-unset}"` to its marker. Returns the session,
/// the data home, and the marker per name, in order.
pub fn session_with_stubs(tag: &str, names: &[&str]) -> Option<(Session, PathBuf, Vec<PathBuf>)> {
    let session = Session::scoot_requiring(&launch_bin(), tag, 1, "", REQUIRE)?;
    let data_home = session.scratch.0.join("data-home");
    let applications = data_home.join("applications");
    fs::create_dir_all(&applications).unwrap();
    let bins = session.scratch.0.join("bins");
    fs::create_dir_all(&bins).unwrap();
    let mut markers = Vec::new();
    let mut apps = Vec::new();
    for name in names {
        let script = bins.join(format!("{name}.sh"));
        let marker = session.scratch.0.join(format!("{name}.launched"));
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$0 ${{XDG_ACTIVATION_TOKEN-unset}} ${{DESKTOP_STARTUP_ID-unset}} `pwd`\" >> {}\n",
                shell_quote(&marker)
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        // The working directory comes through too: the stub prints it.
        apps.push((
            format!("{name}.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{}\"\n",
                script.display()
            ),
        ));
        markers.push(marker);
    }
    for (file, body) in &apps {
        fs::write(applications.join(file), body).unwrap();
    }
    let empty_dirs = session.scratch.0.join("empty-dirs");
    fs::create_dir_all(&empty_dirs).unwrap();
    Some((session, data_home, markers))
}

pub struct Panel {
    pub session: Session,
    pub child: Child,
    pub log: PathBuf,
    #[allow(dead_code)]
    pub data_home: PathBuf,
}

impl Panel {
    #[allow(dead_code)]
    pub fn start(tag: &str, apps: &[(&str, &str)], args: &[&str]) -> Option<Self> {
        let (session, data_home) = session_with_apps(tag, apps)?;
        Self::start_on(session, data_home, args, &[])
    }

    #[allow(dead_code)]
    pub fn start_with_env(
        tag: &str,
        apps: &[(&str, &str)],
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Option<Self> {
        let (session, data_home) = session_with_apps(tag, apps)?;
        Self::start_on(session, data_home, args, env)
    }

    /// Starts the panel on an existing session: the look layer (or any
    /// other sandbox file) lands before the open, so the panel reads it.
    pub fn start_on(
        session: Session,
        data_home: PathBuf,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Option<Self> {
        let log = session.scratch.0.join("panel.log");
        let file = fs::File::create(&log).unwrap();
        let empty_dirs = session.scratch.0.join("empty-dirs");
        let mut command = Command::new(launch_bin());
        command
            .env("XDG_RUNTIME_DIR", &session.scratch.0)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("WAYLAND_DEBUG")
            .env("WAYLAND_DISPLAY", &session.wayland_display)
            .arg("--font")
            .arg(session.font())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(file);
        session.scratch.sandbox(&mut command);
        // The sandbox pins config/state/cache; the fixture data homes
        // ride above, so the panel indexes the fixture, not the machine.
        command
            .env("XDG_DATA_HOME", &data_home)
            .env("XDG_DATA_DIRS", &empty_dirs);
        // The caller's overrides last (a parent token, a dead bus).
        for (key, value) in env {
            command.env(key, value);
        }
        let child = command.spawn().unwrap();
        Some(Self {
            session,
            child,
            log,
            data_home,
        })
    }

    pub fn ipc(&self, request: &str) -> serde_json::Value {
        let reply = self.session.scoot_ipc(request);
        assert_eq!(reply["type"], "ok", "{request}: {reply}");
        reply
    }

    pub fn shot(&self) -> Shot {
        self.session.scoot_screenshot(1)
    }

    /// Saves the output's screenshot as a PNG at
    /// `$SCOOTLAUNCH_SHOTS/NAME.png` when that directory is set: the
    /// evidence for the pixels the assertions read.
    #[allow(dead_code)]
    pub fn save(&self, name: &str) {
        use base64::Engine as _;
        let Some(dir) = std::env::var_os("SCOOTLAUNCH_SHOTS") else {
            return;
        };
        let reply = self
            .session
            .scoot_ipc(r#"{"type":"screenshot","output":1,"cursor":false}"#);
        let png = base64::engine::general_purpose::STANDARD
            .decode(reply["png"].as_str().unwrap())
            .unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(PathBuf::from(dir).join(format!("{name}.png")), png).unwrap();
    }

    #[allow(dead_code)]
    pub fn type_text(&self, text: &str) {
        self.ipc(&format!(
            "{{\"type\":\"type\",\"text\":{}}}",
            serde_json::to_string(text).unwrap()
        ));
    }

    pub fn key(&self, keys: &str) {
        self.ipc(&format!(r#"{{"type":"key","keys":"{keys}"}}"#));
    }

    pub fn click(&self, x: u32, y: u32) {
        self.ipc(&format!(
            r#"{{"type":"click","x":{x},"y":{y},"button":"left"}}"#
        ));
    }

    /// Waits for the panel's border color to appear on screen: the
    /// compositor mapped the surface and drew the chrome. The ring is 2
    /// px wide, so every other pixel still hits it.
    pub fn wait_shown(&self, border: [u8; 3]) {
        wait_for("the panel shown", Duration::from_secs(10), || {
            let shot = self.shot();
            (0..1600)
                .step_by(2)
                .any(|x| (0..1000).step_by(2).any(|y| shot.at(x, y) == border))
        });
    }

    pub fn wait_exit(&mut self) -> std::process::ExitStatus {
        wait_for("the panel to exit", Duration::from_secs(10), || {
            self.child.try_wait().unwrap().is_some()
        });
        self.child.try_wait().unwrap().unwrap()
    }

    /// Finds the panel through its opaque border ring: the ring's
    /// bounding box, whatever the compositor's placement.
    pub fn find_panel(&self, border: [u8; 3]) -> (u32, u32, u32, u32) {
        let shot = self.shot();
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for x in (0..1600).step_by(2) {
            for y in (0..1000).step_by(2) {
                if shot.at(x, y) == border {
                    xs.push(x);
                    ys.push(y);
                }
            }
        }
        assert!(!xs.is_empty(), "no border ring on screen");
        (
            *xs.iter().min().unwrap(),
            *ys.iter().min().unwrap(),
            *xs.iter().max().unwrap(),
            *ys.iter().max().unwrap(),
        )
    }

    pub fn stdout(&mut self) -> String {
        use std::io::Read as _;
        let mut out = String::new();
        self.child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut out)
            .unwrap();
        out
    }

    pub fn stderr(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    pub fn clean(&self) {
        let log = self.stderr();
        assert!(!log.contains("protocol"), "{log}");
    }
}

pub fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits for a stub's marker to appear (the launch ran it), and
/// answers its lines.
pub fn wait_marker(marker: &std::path::Path) -> String {
    wait_for(
        &format!("the launch of {}", marker.display()),
        Duration::from_secs(10),
        || marker.exists(),
    );
    // One turn for the line to land after the file does.
    std::thread::sleep(Duration::from_millis(200));
    fs::read_to_string(marker).unwrap_or_default()
}
