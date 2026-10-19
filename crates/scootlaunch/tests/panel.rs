//! The panel on a headless scoot: open, type through `scoot msg type`
//! and pick; toggle; dismiss; and the per-shipped-look goldens.
//!
//! Each test starts its own headless scoot with a fixture applications
//! tree (so no test reads the machine's desktop files) and drives the
//! real `scootlaunch` binary. Skipped without a `scoot` binary beside
//! the test binary; `SCOOTLAUNCH_REQUIRE_SCOOT` makes that a failure, so
//! CI cannot go green by skipping.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use scootui::test_support::{Session, Shot, rgb};

/// The require variable turning these tests' skips into failures.
const REQUIRE: &str = "SCOOTLAUNCH_REQUIRE_SCOOT";

fn launch_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scootlaunch"))
}

fn desktop(name: &str, exec: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n")
}

/// A session with a fixture applications tree: `apps` holds
/// `name.desktop` files, and nothing else is indexed.
fn session_with_apps(tag: &str, apps: &[(&str, &str)]) -> Option<(Session, PathBuf)> {
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

struct Panel {
    session: Session,
    child: Child,
    log: PathBuf,
    data_home: PathBuf,
}

impl Panel {
    fn start(tag: &str, apps: &[(&str, &str)], args: &[&str]) -> Option<Self> {
        let (session, data_home) = session_with_apps(tag, apps)?;
        Self::start_on(session, data_home, args)
    }

    /// Starts the panel on an existing session: the look layer (or any
    /// other sandbox file) lands before the open, so the panel reads it.
    fn start_on(session: Session, data_home: PathBuf, args: &[&str]) -> Option<Self> {
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
        let child = command.spawn().unwrap();
        Some(Self {
            session,
            child,
            log,
            data_home,
        })
    }

    fn ipc(&self, request: &str) -> serde_json::Value {
        let reply = self.session.scoot_ipc(request);
        assert_eq!(reply["type"], "ok", "{request}: {reply}");
        reply
    }

    fn shot(&self) -> Shot {
        self.session.scoot_screenshot(1)
    }

    /// Saves the output's screenshot as a PNG at
    /// `$SCOOTLAUNCH_SHOTS/NAME.png` when that directory is set: the
    /// evidence for the pixels the assertions read.
    fn save(&self, name: &str) {
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

    fn type_text(&self, text: &str) {
        self.ipc(&format!(
            "{{\"type\":\"type\",\"text\":{}}}",
            serde_json::to_string(text).unwrap()
        ));
    }

    fn key(&self, keys: &str) {
        self.ipc(&format!(r#"{{"type":"key","keys":"{keys}"}}"#));
    }

    fn click(&self, x: u32, y: u32) {
        self.ipc(&format!(
            r#"{{"type":"click","x":{x},"y":{y},"button":"left"}}"#
        ));
    }

    /// Waits for the panel's border color to appear on screen: the
    /// compositor mapped the surface and drew the chrome. The ring is 2
    /// px wide, so every other pixel still hits it.
    fn wait_shown(&self, border: [u8; 3]) {
        wait_for("the panel shown", Duration::from_secs(10), || {
            let shot = self.shot();
            (0..1600)
                .step_by(2)
                .any(|x| (0..1000).step_by(2).any(|y| shot.at(x, y) == border))
        });
    }

    fn wait_exit(&mut self) -> std::process::ExitStatus {
        wait_for("the panel to exit", Duration::from_secs(10), || {
            self.child.try_wait().unwrap().is_some()
        });
        self.child.try_wait().unwrap().unwrap()
    }

    /// Finds the panel through its opaque border ring: the ring's
    /// bounding box, whatever the compositor's placement.
    fn find_panel(&self, border: [u8; 3]) -> (u32, u32, u32, u32) {
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

    fn stdout(&mut self) -> String {
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

    fn stderr(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn clean(&self) {
        let log = self.stderr();
        assert!(!log.contains("protocol"), "{log}");
    }
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn apps() -> Vec<(&'static str, String)> {
    vec![
        ("firefox.desktop", desktop("Firefox", "/usr/bin/firefox")),
        ("files.desktop", desktop("Files", "/usr/bin/files")),
        ("terminal.desktop", desktop("Terminal", "/usr/bin/terminal")),
    ]
}

fn refs<'a>(bodies: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    bodies
        .iter()
        .map(|(file, body)| (*file, body.as_str()))
        .collect()
}

#[test]
fn open_type_select_prints_the_id() {
    let bodies = apps();
    let Some(mut panel) = Panel::start("type-select", &refs(&bodies), &[]) else {
        return;
    };
    // The defaults (no look layer in the sandbox): surface #1e1e2e,
    // focus #f9e2af.
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.type_text("term");
    std::thread::sleep(Duration::from_millis(500));
    panel.save("type-select");
    // The redraw reached the screen: only "Terminal" matches, so the
    // second row's middle is panel fill, not a row.
    let (x0, y0, x1, _) = panel.find_panel(rgb("#f9e2af"));
    assert_eq!(
        panel.shot().at((x0 + x1) / 2, y0 + 35 + 21 + 10),
        rgb("#1e1e2e"),
        "the filtered list drew"
    );
    panel.key("Return");
    let status = panel.wait_exit();
    let out = panel.stdout();
    assert!(status.success(), "exit 0 on a pick: {}", panel.stderr());
    assert_eq!(
        out,
        "terminal.desktop\n",
        "the pick prints: {}",
        panel.stderr()
    );
    panel.clean();
    assert!(
        !panel
            .session
            .scratch
            .0
            .join(format!(
                "scootlaunch-{}.sock",
                panel.session.wayland_display
            ))
            .exists(),
        "the socket is gone with the panel"
    );
}

#[test]
fn ctrl_n_moves_and_return_picks() {
    // The modifiers-mask Control path, end to end: Ctrl+N moves like
    // Down through a real compositor's modifiers events.
    let bodies = apps();
    let Some(mut panel) = Panel::start("ctrl-n", &refs(&bodies), &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("ctrl+n");
    std::thread::sleep(Duration::from_millis(300));
    panel.save("ctrl-n");
    panel.key("Return");
    let status = panel.wait_exit();
    let out = panel.stdout();
    assert!(status.success(), "exit 0 on a pick: {}", panel.stderr());
    // Empty query orders by name (Files, Firefox, Terminal); Ctrl+N
    // moves from Files to Firefox.
    assert_eq!(out, "firefox.desktop\n", "Ctrl+N moved: {}", panel.stderr());
    panel.clean();
}

#[test]
fn the_toggle_closes_a_running_instance() {
    let bodies = apps();
    let Some(mut first) = Panel::start("toggle-first", &refs(&bodies), &[]) else {
        return;
    };
    first.wait_shown(rgb("#f9e2af"));
    // The second invocation tells the first to close and exits itself.
    let log = first.session.scratch.0.join("panel2.log");
    let file = fs::File::create(&log).unwrap();
    let empty_dirs = first.session.scratch.0.join("empty-dirs");
    let mut second = Command::new(launch_bin());
    second
        .env("XDG_RUNTIME_DIR", &first.session.scratch.0)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env("WAYLAND_DISPLAY", &first.session.wayland_display)
        .env("XDG_DATA_HOME", &first.data_home)
        .env("XDG_DATA_DIRS", &empty_dirs)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(file);
    first.session.scratch.sandbox(&mut second);
    second
        .env("XDG_DATA_HOME", &first.data_home)
        .env("XDG_DATA_DIRS", &empty_dirs);
    let mut second = second.spawn().unwrap();
    wait_for("the toggle to exit", Duration::from_secs(10), || {
        second.try_wait().unwrap().is_some()
    });
    assert!(
        second.try_wait().unwrap().unwrap().success(),
        "the toggle exits 0"
    );
    let status = first.wait_exit();
    assert_eq!(status.code(), Some(1), "the first panel was dismissed");
    first.clean();
}

#[test]
fn escape_dismisses_without_a_pick() {
    let bodies = apps();
    let Some(mut panel) = Panel::start("escape", &refs(&bodies), &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "dismiss exits 1");
    assert_eq!(panel.stdout(), "", "no pick prints");
    panel.clean();
}

#[test]
fn pointer_motion_outside_does_not_dismiss() {
    // With exclusive keyboard the compositor holds focus on bare-desktop
    // clicks, so no dismiss signal arrives: moving the pointer off the
    // panel (and clicking the bare desktop) must not close it. Escape
    // still dismisses after.
    let bodies = apps();
    let Some(mut panel) = Panel::start("outside", &refs(&bodies), &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    // The panel is centered and far smaller than the output: the corner
    // is outside it.
    panel.click(10, 10);
    std::thread::sleep(Duration::from_millis(500));
    panel.wait_shown(rgb("#f9e2af"));
    panel.save("outside-stays-open");
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "Escape dismisses");
    panel.clean();
}

#[test]
fn a_click_on_a_row_picks_it() {
    let bodies: Vec<(&str, String)> = vec![
        ("alpha.desktop", desktop("Alpha", "/usr/bin/alpha")),
        ("beta.desktop", desktop("Beta", "/usr/bin/beta")),
        ("gamma.desktop", desktop("Gamma", "/usr/bin/gamma")),
    ];
    let Some(mut panel) = Panel::start("row-click", &refs(&bodies), &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    // The second row's middle, from the border box: entry at inner 10,
    // rows 21 px from list_y0 = 35.
    let (x0, y0, x1, _) = panel.find_panel(rgb("#f9e2af"));
    panel.click((x0 + x1) / 2, y0 + 35 + 21 + 10);
    let status = panel.wait_exit();
    let out = panel.stdout();
    assert!(
        status.success(),
        "exit 0 on a mouse pick: {}",
        panel.stderr()
    );
    assert_eq!(out, "beta.desktop\n", "the row picks: {}", panel.stderr());
    panel.clean();
}

/// The per-shipped-look goldens: the panel draws what fuzzel draws for
/// the look (the design's table), read back off a real screenshot. The
/// look layer is the rendered `launch.toml` for the shipped look, as
/// `scoot look apply` would write it.
#[test]
fn goldens_per_shipped_look() {
    const SHIPPED: &[&str] = &[
        "ginger-night",
        "moonrise",
        "music-desk",
        "radial-burst",
        "vinyl-sunset",
    ];
    for name in SHIPPED {
        golden_one(name);
    }
}

fn golden_one(name: &str) {
    let bodies: Vec<(&str, String)> = vec![
        ("alpha.desktop", desktop("Alpha", "/usr/bin/alpha")),
        ("beta.desktop", desktop("Beta", "/usr/bin/beta")),
        ("gamma.desktop", desktop("Gamma", "/usr/bin/gamma")),
    ];
    let refs = refs(&bodies);
    // The rendered fragment for the shipped look, as the state link
    // would carry it after `scoot look apply`. Written before the open:
    // the panel reads the layer at open, never after.
    let fragment = render_launch_fragment(name);
    let (session, data_home) = match session_with_apps(&format!("golden-{name}"), &refs) {
        Some(session) => session,
        None => return,
    };
    let look_dir = session.scratch.state_home().join("scoot").join("look");
    fs::create_dir_all(&look_dir).unwrap();
    fs::write(look_dir.join("launch.toml"), &fragment).unwrap();
    let Some(mut panel) = Panel::start_on(session, data_home, &[]) else {
        return;
    };
    let table: toml::Table = fragment.parse().expect("launch.toml parses");
    let colors = table["colors"].as_table().expect("colors").clone();
    let color = |key: &str| rgb(colors[key].as_str().unwrap());
    let surface = color("surface");
    let focus = color("focus");
    let selection = color("selection");
    let em = table["font-size"].as_integer().expect("font-size") as f32;
    let opacity = table["opacity"].as_float().unwrap_or(1.0);
    panel.wait_shown(focus);
    panel.save(&format!("golden-{name}"));
    let shot = panel.shot();
    // Finds the panel through its opaque border ring, whatever the
    // compositor's placement: the border's bounding box is the panel.
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for x in (0..1600).step_by(2) {
        for y in (0..1000).step_by(2) {
            if shot.at(x, y) == focus {
                xs.push(x);
                ys.push(y);
            }
        }
    }
    assert!(!xs.is_empty(), "{name}: no border ring on screen");
    let (x0, x1) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
    let (y0, y1) = (*ys.iter().min().unwrap(), *ys.iter().max().unwrap());
    let width = 40.0 * em;
    let row_h = em.ceil() + 8.0;
    let height = 2.0 * (2.0 + 8.0) + row_h * 11.0 + 4.0;
    assert!(
        (x1 - x0) as f32 >= width - 6.0 && (x1 - x0) as f32 <= width + 2.0,
        "{name}: panel width {} for {width}",
        x1 - x0
    );
    assert!(
        (y1 - y0) as f32 >= height - 6.0 && (y1 - y0) as f32 <= height + 2.0,
        "{name}: panel height {} for {height}",
        y1 - y0
    );
    // The pad under the last row is surface fill (blended at the look's
    // opacity over the black output); the first row is the selection.
    // The probe sits at the bottom-center: clear of the rounded corners
    // at any shipped radius.
    let expected_fill = blend_over_black(surface, opacity);
    let fill = shot.at((x0 + x1) / 2, y1 - 6);
    assert!(
        near(fill, expected_fill, 4),
        "{name}: fill {fill:?} for {expected_fill:?} at opacity {opacity}"
    );
    let row_center_y = y0 + 2 + 8 + row_h as u32 + 4 + row_h as u32 / 2;
    let selected = shot.at((x0 + x1) / 2, row_center_y);
    assert_eq!(
        selected, selection,
        "{name}: first row {selected:?} for selection {selection:?}"
    );
    panel.key("Escape");
    panel.wait_exit();
    panel.clean();
}

/// Renders the shipped look's `launch.toml`, as `scoot look apply`
/// writes it into the state link.
fn render_launch_fragment(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("looks")
        .join(name)
        .join("look.toml");
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let raw = scoot_look::parse_look_toml(&bytes).unwrap_or_else(|errors| {
        panic!(
            "{name}/look.toml refuses to parse: {}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    let map =
        std::collections::BTreeMap::from([(name.to_string(), (raw, scoot_look::Origin::Shipped))]);
    let look = scoot_look::resolve(&map, name).unwrap_or_else(|e| panic!("{name}: {e}"));
    scoot_look::render_launch(&look).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// `color` at `alpha` over the empty headless output (black): the same
/// premultiplication `Color::argb8888` writes, so an exact match is
/// expected (the test keeps a small tolerance for the compositor's own
/// rounding).
fn blend_over_black(color: [u8; 3], alpha: f64) -> [u8; 3] {
    let byte = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    color.map(|channel| ((u32::from(channel) * byte + 127) / 255).min(255) as u8)
}

fn near(a: [u8; 3], b: [u8; 3], tolerance: i32) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| (*x as i32 - *y as i32).abs() <= tolerance)
}
