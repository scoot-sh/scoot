//! The panel tests' harness: a headless scoot beside fixture
//! applications, and stub launch targets recording what the launch
//! gave them. Shared by the panel, spawn and agent tests: each target
//! uses a different subset, so unused items are allowed here rather
//! than in every consumer.
//!
#![allow(dead_code, unused_imports)]

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub use scootui::test_support::rgb;
use scootui::test_support::{Session, Shot};

/// The require variable turning these tests' skips into failures.
pub const REQUIRE: &str = "SCOOTLAUNCH_REQUIRE_SCOOT";

/// The binary under test: this build's, or `SCOOTLAUNCH_TEST_BIN` (the
/// before side of an A/B run against another build).
pub fn launch_bin() -> PathBuf {
    std::env::var_os("SCOOTLAUNCH_TEST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_scootlaunch")))
}

/// Shared across the panel and spawn test targets; each target uses a subset.
#[allow(dead_code)]
pub fn desktop(name: &str, exec: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n")
}

/// A desktop entry with an `Icon=` key: the panel resolves it through
/// the icon theme, falling back to a letter tile when nothing matches.
#[allow(dead_code)]
pub fn desktop_with_icon(name: &str, exec: &str, icon: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nIcon={icon}\n")
}

/// A fixture icon theme under `data_home/icons/FixtureTheme`: a terminal
/// `alpha.png` (a warm-orange square with a white `>_` mark), a browser
/// `beta.svg` (a blue disc with a white ring), and no `gamma`, so two
/// rows draw image icons (one per decoder) and one the letter tile.
/// Answers the theme name for the look layer's `[icons] theme`.
///
/// The icons are small designed marks rather than solid squares, so the
/// docs screenshots show what the panel looks like with real apps — but
/// every pixel is generated below (rect fills for the PNG, two SVG
/// elements), so the committed PNGs stay reproducible from this helper
/// alone.
///
/// The tests count the icons' exact palette colors over the rows: the
/// orange and white of the terminal mark, the blue and white of the
/// browser disc. Opaque interiors read back exact over any row fill, so
/// the counts hold on the selected row as well as plain ones.
///
/// Run the panel tests with `SCOOTLAUNCH_SHOTS` set to regenerate the
/// docs screenshots from this same fixture (see `icons_docs_one` in
/// `panel.rs`); the bytes below are the whole capture setup, so the
/// committed PNGs are reproducible from this helper alone.
#[allow(dead_code)]
pub fn fixture_icon_theme(data_home: &std::path::Path) -> String {
    let theme = data_home.join("icons").join("FixtureTheme");
    let dir = theme.join("48x48").join("apps");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("alpha.png"), terminal_png()).unwrap();
    fs::write(dir.join("beta.svg"), browser_svg()).unwrap();
    fs::write(
        theme.join("index.theme"),
        "[Icon Theme]\nName=FixtureTheme\nDirectories=48x48/apps\n\n[48x48/apps]\nSize=48\nType=Fixed\n",
    )
    .unwrap();
    "FixtureTheme".to_string()
}

/// The terminal mark's palette, as exact RGB triples the tests count.
#[allow(dead_code)]
pub const TERMINAL_ORANGE: [u8; 3] = [224, 106, 44];
/// The browser disc's palette, as exact RGB triples the tests count.
#[allow(dead_code)]
pub const BROWSER_BLUE: [u8; 3] = [59, 120, 231];
/// The glyph white shared by both marks.
#[allow(dead_code)]
pub const MARK_WHITE: [u8; 3] = [255, 255, 255];

/// A 48x48 RGBA terminal icon: a warm-orange square with a white `>_`
/// prompt mark drawn from rects. Fully opaque, so the resampled rows
/// read back mostly exact pixels in either look.
#[allow(dead_code)]
fn terminal_png() -> Vec<u8> {
    const SIDE: usize = 48;
    let mut px = vec![0u8; SIDE * SIDE * 4];
    let orange = [
        TERMINAL_ORANGE[0],
        TERMINAL_ORANGE[1],
        TERMINAL_ORANGE[2],
        255,
    ];
    for chunk in px.as_chunks_mut::<4>().0 {
        chunk.copy_from_slice(&orange);
    }
    // The `>` chevron: five 6x6 steps down and back up.
    let white = [MARK_WHITE[0], MARK_WHITE[1], MARK_WHITE[2], 255];
    for (x, y) in [(14, 12), (19, 17), (24, 22), (19, 27), (14, 32)] {
        fill_rect(&mut px, SIDE, x, y, 6, 6, white);
    }
    // The `_` underscore.
    fill_rect(&mut px, SIDE, 14, 39, 17, 5, white);
    let mut buf = Vec::new();
    let mut encoder = png::Encoder::new(&mut buf, SIDE as u32, SIDE as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&px).unwrap();
    drop(writer);
    buf
}

/// Fills one rect of the RGBA buffer, clipped to the square.
#[allow(dead_code)]
fn fill_rect(px: &mut [u8], side: usize, x: i32, y: i32, w: i32, h: i32, color: [u8; 4]) {
    for yy in y..y + h {
        for xx in x..x + w {
            if xx < 0 || yy < 0 || xx >= side as i32 || yy >= side as i32 {
                continue;
            }
            let at = (yy as usize * side + xx as usize) * 4;
            px[at..at + 4].copy_from_slice(&color);
        }
    }
}

/// A 48x48 browser icon: a blue disc on transparency with a white ring,
/// as two SVG elements (rasterized through `resvg` like any theme SVG).
#[allow(dead_code)]
fn browser_svg() -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"48\" height=\"48\">\
         <circle cx=\"24\" cy=\"24\" r=\"19\" fill=\"#{:02x}{:02x}{:02x}\"/>\
         <circle cx=\"24\" cy=\"24\" r=\"10\" fill=\"none\" stroke=\"#ffffff\" stroke-width=\"5\"/>\
         </svg>",
        BROWSER_BLUE[0], BROWSER_BLUE[1], BROWSER_BLUE[2]
    )
}

/// A session with a fixture applications tree: `apps` holds
/// `name.desktop` files, and nothing else is indexed.
pub fn session_with_apps(tag: &str, apps: &[(&str, &str)]) -> Option<(Session, PathBuf)> {
    session_with_apps_config(tag, apps, "")
}

/// [`session_with_apps`] under a compositor config (`[output]\nscale = 2\n`
/// for a HiDPI output).
pub fn session_with_apps_config(
    tag: &str,
    apps: &[(&str, &str)],
    config: &str,
) -> Option<(Session, PathBuf)> {
    let session = Session::scoot_requiring(&launch_bin(), tag, 1, config, REQUIRE)?;
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
    /// `SCOOTLAUNCH_TEST_FONT` overrides the typeface (a real font for
    /// the docs screenshots); unset, the hermetic test font is used, so
    /// CI never depends on machine fonts.
    pub fn start_on(
        session: Session,
        data_home: PathBuf,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Option<Self> {
        let log = session.scratch.0.join("panel.log");
        let file = fs::File::create(&log).unwrap();
        let empty_dirs = session.scratch.0.join("empty-dirs");
        let font = std::env::var_os("SCOOTLAUNCH_TEST_FONT")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .unwrap_or_else(|| session.font());
        let mut command = Command::new(launch_bin());
        command
            .env("XDG_RUNTIME_DIR", &session.scratch.0)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("WAYLAND_DEBUG")
            .env("WAYLAND_DISPLAY", &session.wayland_display)
            .arg("--font")
            .arg(font)
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

    /// Moves the pointer without pressing anything.
    pub fn hover(&self, x: u32, y: u32) {
        self.ipc(&format!(r#"{{"type":"pointer_move","x":{x},"y":{y}}}"#));
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

/// The panel's geometry at scale 1, restated from `src/layout.rs` (an
/// integration test cannot import a binary's modules; the unit tests
/// there pin the same numbers for the default 13 px look). In pixels
/// from the panel's top-left.
pub struct Geom {
    /// Border plus padding: the field's and every row's inset.
    pub inner: u32,
    pub row_h: u32,
    pub gap: u32,
    pub side: u32,
    pub lead: u32,
}

impl Geom {
    pub fn new(em: f32, radius: u32) -> Self {
        let round = |v: f32| v.round() as u32;
        let pad = round(em * 0.75).max((radius * 3).div_ceil(10));
        let row_h = (em * 1.75).ceil() as u32;
        let side = round(em * 1.2).clamp(1, row_h.saturating_sub(4).max(1));
        let radius = radius.min(row_h / 2);
        Self {
            inner: 2 + pad,
            row_h,
            gap: round(em * 0.4).max(1),
            side,
            lead: round(em * 0.6).max(radius.div_ceil(2)),
        }
    }

    /// The first list row's top.
    pub fn list_y(&self) -> u32 {
        self.inner + self.row_h + self.gap
    }

    /// The panel's height for `rows` list rows.
    pub fn height(&self, rows: u32) -> u32 {
        2 * self.inner + self.row_h * (rows + 1) + self.gap
    }
}

/// Renders the shipped look's `launch.toml`, as `scoot look apply`
/// writes it into the state link.
pub fn render_launch_fragment(name: &str) -> String {
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

/// A fully transparent `blank.png` in the fixture theme: an icon that
/// resolves and decodes but draws nothing (a PWA entry's placeholder).
#[allow(dead_code)]
pub fn add_blank_icon(data_home: &std::path::Path) {
    let dir = data_home
        .join("icons")
        .join("FixtureTheme")
        .join("48x48")
        .join("apps");
    let mut buf = Vec::new();
    let mut encoder = png::Encoder::new(&mut buf, 48, 48);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&vec![0u8; 48 * 48 * 4]).unwrap();
    drop(writer);
    fs::write(dir.join("blank.png"), buf).unwrap();
}

/// The numbers inside the parentheses of every `WAYLAND_DEBUG` line
/// calling `method` (`create_buffer`, `set_destination`,
/// `preferred_scale`), in order. Names such as `wl_buffer@18` are not
/// numbers and are skipped.
pub fn calls(log: &str, method: &str) -> Vec<Vec<u32>> {
    let needle = format!(".{method}");
    log.lines()
        .filter_map(|line| {
            let at = line.find(&needle)? + needle.len();
            let rest = &line[at..];
            let open = rest.find('(')?;
            let close = rest.rfind(')')?;
            let inner = rest.get(open + 1..close)?;
            Some(
                inner
                    .split(',')
                    .filter_map(|part| part.trim().parse::<u32>().ok())
                    .collect(),
            )
        })
        .collect()
}

/// The `(width, height)` of every buffer the client created, in order.
pub fn created_buffers(log: &str) -> Vec<(u32, u32)> {
    calls(log, "create_buffer")
        .iter()
        .filter_map(|n| Some((*n.get(1)?, *n.get(2)?)))
        .collect()
}

/// A buffer side for `logical` pixels at `v120`/120, rounded half away from
/// zero as `wp_fractional_scale_v1` says.
pub fn scaled(logical: u32, v120: u32) -> u32 {
    ((u64::from(logical) * u64::from(v120) + 60) / 120).max(1) as u32
}
