#![cfg(target_os = "linux")]
//! Golden screenshots, one per shipped look, from a real headless session.
//!
//! Each look gets its own test: a `scoot --headless` session with a private
//! `XDG_STATE_HOME` and `XDG_RUNTIME_DIR` (so a swap can never repoint a
//! real desktop), two fixed clients, `scoot msg look-apply NAME`, then
//! `scoot msg screenshot --no-cursor`, compared pixel by pixel against the
//! committed golden in `tests/goldens/`. The scene is the same for every
//! look, so what differs between goldens is the look itself: the background
//! color, the focused window's ring, and the bar's pill and digits.
//!
//! The scene, fixed for every look:
//!
//! - One 800x600 output at scale 1, `--renderer cpu`. The CPU tier is
//!   integer-deterministic across machines (the same pixman code, no
//!   driver), while GLES output can move by an LSB between drivers. The
//!   goldens pin look colors, not renderer tiers; the tiers have their own
//!   tests (`compositor/render/tests.rs`). `tier=cpu` is in the session
//!   log, like the minimal smoke run's.
//! - A solid-color test client: a real `xdg_toplevel` over the session's
//!   Wayland socket, filled flat magenta (`#FF00FF`, a color no shipped
//!   look uses, so no assertion can pass by accident). It acks its
//!   configure and commits a buffer at exactly the configured size, the
//!   way a real toolkit does, so no scaling filter touches its pixels.
//! - The bar: a real `scootbar daemon` with one module (`workspaces`, on
//!   the left) and nothing else, so the only text on screen is the active
//!   workspace's digit. No third-party app appears: their fonts and
//!   versions make pixels flaky, which is why this ticket excludes them.
//!
//! Hermetic `PATH`: every child runs with a `PATH` holding only the stub
//! `bin/` dir above, never the host's. In particular no `scootbg` is
//! reachable, so the compositor's `[wallpaper]` spawn fails with NotFound
//! and every image-wallpaper look renders its flat background color
//! instead (four of the five shipped looks name an image). The
//! `scoot look apply` child the compositor spawns still finds its
//! `scootbar` beside its own binary, so the bar-fragment check runs for
//! real. Every `scoot msg` child also runs with a scratch `HOME`,
//! `XDG_*` and `TMPDIR`, so no test command can read or write the real
//! ones.
//!
//! Fonts: the looks' families (`DroidSansM Nerd Font Propo`, `FiraCode
//! Nerd Font`, `DejaVu Sans`, `DejaVu Sans Mono`) are answered by a stub
//! `fc-match` on `PATH` with a copy of a real font file, the way
//! `look_apply_cli.rs` and the smoke test's looks section do, so the test
//! needs no Nerd Fonts. The bar draws its digit with that same file
//! through `ab_glyph` (pure Rust: the pixels depend only on the file's
//! bytes, never on a fontconfig or freetype version), and every child
//! also runs under a `FONTCONFIG_FILE` that lists only the scratch font
//! directory.
//!
//! Tolerance: at most 2 per channel per pixel. The PNG codec is lossless,
//! so the only rounding in the pipeline is blending: the focus ring and
//! the rounded window corners composite once over what is behind them,
//! and each composite can round by 1 LSB. 2 covers two such roundings on
//! one pixel. A genuinely wrong color is off by tens (the deliberately
//! broken look below misses by 200+), so the tolerance cannot hide a real
//! regression. The failure message prints the worst pixel, its expected
//! and actual color, and how many pixels missed, and saves the actual
//! screenshot beside the golden's name for inspection.
//!
//! Regenerating the goldens (after an intended visual change, or on a box
//! whose font file differs from CI's):
//!
//! ```sh
//! cargo build -p scootbar
//! SCOOT_GOLDEN_REGENERATE=1 cargo nextest run -p scoot --test look_golden_screenshots
//! ```
//!
//! which writes `tests/goldens/look-<name>.png` and passes. Inspect the
//! diff before committing: a regenerate that also moves pixels it should
//! not have moved is a bug, not a baseline update. CI never regenerates;
//! it compares, and uploads the actual screenshots as the
//! `look-golden-actual` artifact when a comparison fails.
//!
//! The bar binary is `$SCOOT_GOLDEN_SCOOTBAR` if set, else the `scootbar`
//! beside the test's `scoot` in the same target directory (a workspace
//! build puts it there). Without one the tests print why and pass as
//! skipped, unless `SCOOT_GOLDEN_REQUIRE_BAR` is set, which turns the
//! skip into a failure so CI cannot go green by skipping it. CI builds
//! `-p scootbar` in the default `linux` job before the suite and sets the
//! requirement in that job's two test steps (nextest and cargo test);
//! the feature jobs carry no bar and skip the scenes there.
//!
//! `every_shipped_look_has_a_golden` keeps the set complete: a look added
//! to `looks/` without a golden (or a golden without a look) fails it.
//! `a_broken_look_is_nothing_like_its_golden` is the negative control: a
//! look with a white focus ring must mismatch ginger-night's golden by a
//! mile, proving the comparison is not vacuous.

use std::ffi::OsString;
use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long anything here waits: a debug `scoot` starts in well under a
/// second, applies take seconds at most, so this only bounds a wedge.
const PATIENCE: Duration = Duration::from_secs(60);

/// The output the golden sessions render at. Small, so the committed PNGs
/// stay a few kilobytes each; fixed, so geometry never drifts.
const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// The per-channel tolerance, and why 2: see the module doc.
const TOLERANCE: u8 = 2;

/// The test client's fill, `#FF00FF`: no shipped look uses it.
const MAGENTA: [u8; 3] = [0xFF, 0x00, 0xFF];

/// Every shipped look, exactly the directories under `looks/`.
/// `every_shipped_look_has_a_golden` fails if this drifts from either the
/// catalog or the goldens directory.
const LOOKS: &[&str] = &[
    "ginger-night",
    "moonrise",
    "music-desk",
    "radial-burst",
    "vinyl-sunset",
];

fn goldens_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens")
}

fn golden_path(name: &str) -> PathBuf {
    goldens_dir().join(format!("look-{name}.png"))
}

/// The catalog the sessions apply from: the repo's real shipped looks.
fn shipped_looks_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../looks");
    dir.canonicalize()
        .unwrap_or_else(|error| panic!("the shipped looks dir {}: {error}", dir.display()))
}

fn scoot() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scoot"))
}

/// The bar binary, or `None` when there is none to run: `$SCOOT_GOLDEN_SCOOTBAR`,
/// else the `scootbar` beside this test's `scoot` (a workspace build puts it
/// there), else `scootbar` on `PATH`.
fn scootbar_bin() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SCOOT_GOLDEN_SCOOTBAR").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
        return None;
    }
    let sibling = scoot().with_file_name("scootbar");
    if sibling.is_file() {
        return Some(sibling);
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("scootbar"))
            .find(|candidate| candidate.is_file())
    })
}

/// Says why the golden tests skip, or panics when `SCOOT_GOLDEN_REQUIRE_BAR`
/// is set (so CI, which sets it, fails instead of passing without running
/// anything). None means "run".
fn bar_or_skip() -> Option<PathBuf> {
    match scootbar_bin() {
        Some(bin) => Some(bin),
        None => {
            let why = "there is no scootbar binary beside the test's scoot \
                (cargo build -p scootbar, or set SCOOT_GOLDEN_SCOOTBAR)";
            if std::env::var_os("SCOOT_GOLDEN_REQUIRE_BAR").is_some() {
                panic!("SCOOT_GOLDEN_REQUIRE_BAR is set but {why}");
            }
            eprintln!("skipped -- {why} (set SCOOT_GOLDEN_REQUIRE_BAR=1 to fail, not skip)");
            None
        }
    }
}

/// A real, loadable font file: first the host `fc-match`, then a direct
/// scan of the usual font roots (an empty `FONTCONFIG_FILE` blinds the
/// former but not the filesystem). The bar draws with a copy of it (see
/// [`Scratch::font_file`]), and the stub below answers it, so the file is
/// found without fontconfig either way.
fn real_font_file() -> PathBuf {
    for family in ["DejaVu Sans", "DejaVu Sans Mono", "sans-serif"] {
        if let Ok(out) = Command::new("fc-match")
            .arg("--format=%{file}\n")
            .arg(family)
            .output()
            && out.status.success()
        {
            let path = String::from_utf8(out.stdout).unwrap_or_default();
            let path = path.trim().to_owned();
            if !path.is_empty() && Path::new(&path).is_file() {
                return PathBuf::from(path);
            }
        }
    }
    let roots = [
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/run/current-system/sw/share/fonts"),
        PathBuf::from("/run/current-system/sw/share/X11/fonts"),
    ];
    let mut tried = Vec::new();
    for root in &roots {
        tried.push(root.display().to_string());
        if let Some(found) = first_font_under(root, 4) {
            return found;
        }
    }
    panic!(
        "no loadable font file: fc-match found nothing and no .ttf/.otf under {}",
        tried.join(", ")
    );
}

fn first_font_under(root: &Path, depth: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if path.is_file()
            && let Some(ext) = path.extension().and_then(|ext| ext.to_str())
            && (ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf"))
        {
            return Some(path);
        }
    }
    if depth == 0 {
        return None;
    }
    dirs.into_iter()
        .find_map(|dir| first_font_under(&dir, depth - 1))
}

/// A hermetic `fc-match`, as in `look_apply_cli.rs` and the smoke test's
/// looks section: answers the shipped looks' families with a real font
/// file (and the requested family name, so the production family check
/// passes) and every other family the way real `fc-match` answers a
/// missing one. CI runners have no Nerd Fonts; the dev VM has them; the
/// stub makes both behave the same.
fn stub_fc_match_dir(scratch: &Scratch) -> PathBuf {
    let font = scratch.font_file();
    let dir = scratch.dir.join("bin");
    std::fs::create_dir_all(&dir).expect("a stub bin dir");
    let stub = dir.join("fc-match");
    std::fs::write(
        &stub,
        format!(
            r#"#!/bin/sh
fmt="$1"
fam="$2"
case "$fam" in
  "DroidSansM Nerd Font Propo"|"DejaVu Sans"|"FiraCode Nerd Font"|"DejaVu Sans Mono")
    case "$fmt" in
      *{{file}}*) printf '%s\n' "{font}" ;;
      *) printf '%s\n' "$fam" ;;
    esac
    ;;
  *)
    case "$fmt" in
      *{{file}}*) printf '%s\n' "{font}" ;;
      *) printf 'DejaVu Sans\n' ;;
    esac
    ;;
esac
"#,
            font = font.display(),
        ),
    )
    .expect("a stub fc-match");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("an executable stub");
    }
    dir
}

/// The `PATH` every child runs with: the stub `bin/` dir alone, never
/// the host's. Stub-only keeps `scootbg` (and every other host binary)
/// out of reach, so image-wallpaper looks render their flat background
/// color; see the module doc. `scoot` and `scootbar` are always spawned
/// by absolute path (or found beside the running binary), so nothing
/// they need is lost.
fn hermetic_path(stub: &Path) -> OsString {
    stub.as_os_str().to_owned()
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        // Short: the Unix socket paths inside must fit 107 bytes. The
        // counter keeps parallel tests in one process apart even when
        // the clock does not (only the sub-second part is used).
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!(
            "lg-{tag}-{}-{nanos:08x}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        Self { dir }
    }

    fn sub(&self, name: &str) -> PathBuf {
        let dir = self.dir.join(name);
        std::fs::create_dir_all(&dir).expect("a scratch subdir");
        if name == "runtime" {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                    .expect("an owner-only runtime dir");
            }
        }
        dir
    }

    /// The font file every child uses: a copy of a real one, so the bytes
    /// are fixed for the whole test and the `FONTCONFIG_FILE` below lists
    /// nothing else. The bar draws its digit from this file through
    /// `ab_glyph`, which reads only the file.
    fn font_file(&self) -> PathBuf {
        let dir = self.dir.join("fonts");
        std::fs::create_dir_all(&dir).expect("a scratch fonts dir");
        let path = dir.join("golden.ttf");
        if !path.is_file() {
            let real = real_font_file();
            std::fs::copy(&real, &path).unwrap_or_else(|error| {
                panic!("copy {} to {}: {error}", real.display(), path.display())
            });
        }
        path
    }

    /// A fontconfig file listing only the scratch fonts directory, so no
    /// host font directory can change a result through any library that
    /// reads fontconfig directly (the stub covers `fc-match` itself).
    fn fontconfig_file(&self) -> PathBuf {
        self.font_file();
        let dir = self.dir.join("fonts");
        let cache = self.dir.join("fc-cache");
        std::fs::create_dir_all(&cache).expect("a scratch fontconfig cache dir");
        let path = self.dir.join("fonts.conf");
        std::fs::write(
            &path,
            format!(
                "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n\
                 <fontconfig><dir>{}</dir><cachedir>{}</cachedir></fontconfig>\n",
                dir.display(),
                cache.display(),
            ),
        )
        .expect("a fontconfig file");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn is_socket(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt as _;
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false)
}

struct Session {
    scratch: Scratch,
    child: Child,
    bar: Option<Child>,
    bar_pid: Option<u32>,
    socket: PathBuf,
    runtime: PathBuf,
    wayland_display: String,
    state_home: PathBuf,
    log: PathBuf,
    bar_log: PathBuf,
    path_env: OsString,
    fontconfig: PathBuf,
}

impl Session {
    /// Starts `scoot --headless` on a scratch tree: private runtime, state,
    /// config, cache, home and tmp dirs, the shipped looks as its catalog,
    /// the stub `fc-match` first on `PATH`, a DejaVu-only `FONTCONFIG_FILE`,
    /// and no session bus. Waits for both the IPC socket and the Wayland
    /// socket, then starts the bar (which needs the display name).
    fn start(scratch: Scratch, catalog: &Path) -> Self {
        let stub = stub_fc_match_dir(&scratch);
        let path_env = hermetic_path(&stub);
        let fontconfig = scratch.fontconfig_file();
        let runtime = scratch.sub("runtime");
        let cache = scratch.sub("cache");
        let config = scratch.sub("config");
        let home = scratch.sub("home");
        let tmp = scratch.sub("tmp");
        let state_home = scratch.sub("state-home");
        let socket = runtime.join("scoot.sock");
        let log = scratch.dir.join("scoot.log");
        let log_file = std::fs::File::create(&log).expect("a log file");
        let mut child = Command::new(scoot());
        child
            .arg("--headless")
            .arg("--outputs")
            .arg("1")
            .arg("--width")
            .arg(WIDTH.to_string())
            .arg("--height")
            .arg(HEIGHT.to_string())
            .arg("--renderer")
            .arg("cpu")
            .arg("--socket")
            .arg(&socket)
            .env("XDG_STATE_HOME", &state_home)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_CACHE_HOME", &cache)
            .env("HOME", &home)
            .env("TMPDIR", &tmp)
            .env("SCOOT_LOOKS_DIR", catalog)
            .env("FONTCONFIG_FILE", &fontconfig)
            .env("PATH", &path_env)
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("SCOOT_SOCKET")
            .env_remove("SCOOT_LOOK_STATE")
            .stdout(Stdio::null())
            .stderr(log_file);
        // No `--` client: the test maps its own window once the session is
        // up, so the scene is identical with and without it.
        let mut session = Self {
            scratch,
            child: child.spawn().expect("a headless scoot"),
            bar: None,
            bar_pid: None,
            socket,
            runtime,
            wayland_display: String::new(),
            state_home,
            log,
            bar_log: PathBuf::new(),
            path_env,
            fontconfig,
        };
        session.wayland_display = session.wait_until_up();
        session
    }

    /// Waits for the IPC socket and the Wayland socket, and for `version`
    /// over IPC. Returns the Wayland display name (`wayland-N`).
    fn wait_until_up(&mut self) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let ipc_up = is_socket(&self.socket);
            let wayland = std::fs::read_dir(&self.runtime).ok().and_then(|entries| {
                entries.filter_map(Result::ok).find_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    (name.starts_with("wayland-") && is_socket(&entry.path())).then_some(name)
                })
            });
            if ipc_up && wayland.is_some() {
                let up = Command::new(scoot())
                    .arg("msg")
                    .arg("version")
                    .env("SCOOT_SOCKET", &self.socket)
                    .env("PATH", &self.path_env)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success());
                if up {
                    return wayland.expect("a Wayland socket name");
                }
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
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let mut tail = text.lines().rev().take(20).collect::<Vec<_>>();
        tail.reverse();
        tail.join("\n")
    }

    fn bar_log_tail(&self) -> String {
        if self.bar_log.as_os_str().is_empty() {
            return "(no bar log)".to_owned();
        }
        let text = std::fs::read_to_string(&self.bar_log).unwrap_or_default();
        let mut tail = text.lines().rev().take(20).collect::<Vec<_>>();
        tail.reverse();
        tail.join("\n")
    }

    /// The session's private look state dir: the one `scoot-look-*/` entry
    /// under the runtime dir. A headless session always keeps one, so
    /// swaps never touch the user state below.
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

    /// Starts the bar: one `workspaces` module on the left, nothing else,
    /// the scratch font for its digit, the session's private look state so
    /// a swap reloads it. Its stderr goes to the bar log.
    fn start_bar(&mut self, scootbar: &Path) {
        let config = self.scratch.dir.join("bar.toml");
        std::fs::write(&config, "left = [\"workspaces\"]\n[bar]\nheight = 28\n")
            .expect("a bar config");
        let log_path = self.scratch.dir.join("scootbar.log");
        let log_file = std::fs::File::create(&log_path).expect("a bar log file");
        let mut command = Command::new(scootbar);
        command
            .arg("daemon")
            .arg("--font")
            .arg(self.scratch.font_file())
            .arg("--config")
            .arg(&config)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("XDG_CONFIG_HOME", self.scratch.dir.join("config"))
            .env("XDG_CACHE_HOME", self.scratch.dir.join("cache"))
            .env("XDG_STATE_HOME", &self.state_home)
            .env("HOME", self.scratch.dir.join("home"))
            .env("TMPDIR", self.scratch.dir.join("tmp"))
            .env("WAYLAND_DISPLAY", &self.wayland_display)
            .env("SCOOT_LOOK_STATE", self.private_dir())
            .env("FONTCONFIG_FILE", &self.fontconfig)
            .env("PATH", &self.path_env)
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("SCOOT_SOCKET")
            .stdout(Stdio::null())
            .stderr(log_file);
        self.bar_log = log_path;
        let bar = command.spawn().expect("a bar");
        self.bar_pid = Some(bar.id());
        self.bar = Some(bar);
    }

    /// Whether the bar process is still alive (a zombie counts as dead):
    /// reads `/proc` directly, the way `look_ipc.rs` spots apply children.
    /// `try_wait` would need `&mut`, and reaping here would race the
    /// `Drop` reaper; a stat read observes without touching.
    fn bar_alive(&self) -> bool {
        let Some(pid) = self.bar_pid else {
            return true;
        };
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let Some(after) = stat.rsplit_once(')') else {
            return false;
        };
        match after.1.split_whitespace().next() {
            Some("Z") | Some("X") | Some("x") | None => false,
            Some(_) => true,
        }
    }

    fn msg(&self, args: &[&str]) -> Output {
        Command::new(scoot())
            .arg("msg")
            .args(args)
            .env("PATH", &self.path_env)
            .env("SCOOT_SOCKET", &self.socket)
            // A `msg` child never needs the real home: point every
            // directory it could read at the scratch tree.
            .env("HOME", self.scratch.dir.join("home"))
            .env("XDG_CONFIG_HOME", self.scratch.dir.join("config"))
            .env("XDG_CACHE_HOME", self.scratch.dir.join("cache"))
            .env("XDG_STATE_HOME", &self.state_home)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("TMPDIR", self.scratch.dir.join("tmp"))
            .output()
            .expect("scoot msg runs")
    }

    fn check_msg(&self, args: &[&str]) -> serde_json::Value {
        let out = self.msg(args);
        assert!(
            out.status.success(),
            "`scoot msg {}` failed:\nstdout:\n{}\nstderr:\n{}\nlog:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
            self.log_tail(),
        );
        serde_json::from_slice(&out.stdout).expect("JSON stdout")
    }

    /// `scoot msg look-apply NAME`, asserting the swap landed: the reply
    /// names the look and the compositor reloaded.
    fn look_apply(&self, name: &str) {
        let reply = self.check_msg(&["look-apply", name]);
        assert_eq!(reply["type"], "look_applied", "{reply}");
        assert_eq!(reply["name"], name, "{reply}");
        assert!(
            reply["applied"]
                .as_array()
                .expect("an applied list")
                .iter()
                .any(|app| app == "compositor"),
            "the compositor reloaded: {reply}"
        );
        if self.bar.is_some() {
            // A dead bar would fail the screenshot below with a confusing
            // diff; name it here instead.
            assert!(
                self.bar_alive(),
                "the bar died around the swap; bar log:\n{}",
                self.bar_log_tail()
            );
        }
    }

    /// Polls `scoot msg windows` until a window with `app_id` is listed.
    fn wait_for_window(&self, app_id: &str) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let reply = self.check_msg(&["windows"]);
            let found = reply["windows"]
                .as_array()
                .map(|windows| windows.iter().any(|w| w["app_id"].as_str() == Some(app_id)))
                .unwrap_or(false);
            if found {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "no window with app_id {app_id} appeared; log:\n{}",
                self.log_tail()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// One quiet moment: waits until the compositor has been still for
    /// 200 ms.
    fn wait_idle(&self) {
        let out = self.msg(&["wait-idle", "--quiet-ms", "200"]);
        assert!(
            out.status.success(),
            "wait-idle failed: {}\nlog:\n{}",
            String::from_utf8_lossy(&out.stderr),
            self.log_tail()
        );
    }

    /// One settled screenshot: waits for a quiet compositor, then captures
    /// without the cursor.
    fn screenshot(&self, tag: &str) -> PathBuf {
        self.wait_idle();
        let shot = self.scratch.dir.join(format!("shot-{tag}.png"));
        let out = self.msg(&[
            "screenshot",
            "--no-cursor",
            "--out",
            &shot.display().to_string(),
        ]);
        assert!(
            out.status.success(),
            "screenshot failed: {}\nlog:\n{}",
            String::from_utf8_lossy(&out.stderr),
            self.log_tail()
        );
        shot
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut bar) = self.bar.take() {
            bar.kill().ok();
            bar.wait().ok();
        }
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// The solid-color test client: one `xdg_toplevel` filled flat magenta,
/// mapped through the session's Wayland socket the way a real toolkit
/// maps (commit, ack the configure, commit at the configured size). Held
/// alive until after the screenshot: dropping it unmaps the window.
struct SolidWindow {
    _conn: Connection,
    _queue: EventQueue<TestClient>,
    _surface: wl_surface::WlSurface,
    _xdg: xdg_surface::XdgSurface,
    _toplevel: xdg_toplevel::XdgToplevel,
    _pool: wl_shm_pool::WlShmPool,
    _buffer: wl_buffer::WlBuffer,
    _file: std::fs::File,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    /// The latest unacked `xdg_surface.configure`, with the toplevel's
    /// size from its own configure (0, 0 means the client chooses).
    pending: Option<(u32, i32, i32)>,
    size: Option<(i32, i32)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
    fn event(
        client: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    client.compositor = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_shm" => {
                    client.shm = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "xdg_wm_base" => {
                    client.wm_base = Some(registry.bind(name, version.min(1), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for TestClient {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            let size = client.size.unwrap_or((0, 0));
            client.pending = Some((serial, size.0, size.1));
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            client.size = Some((width, height));
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);

/// One magenta ARGB8888 buffer of `width` by `height` in a memfd pool.
/// Pixels are written once and never touched again: a static client.
fn magenta_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
) -> (wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, std::fs::File) {
    let len = (width as usize) * (height as usize) * 4;
    let fd =
        rustix::fs::memfd_create("scoot-golden", rustix::fs::MemfdFlags::CLOEXEC).expect("a memfd");
    rustix::fs::ftruncate(&fd, len as u64).expect("a sized pool file");
    let mut file = std::fs::File::from(fd);
    // ARGB8888 in memory order: B, G, R, A.
    let pixel = [0x00u8, 0x00, 0xFF, 0xFF];
    let bytes: Vec<u8> = pixel.into_iter().cycle().take(len).collect();
    file.write_all(&bytes).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        width,
        height,
        width * 4,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    (pool, buffer, file)
}

/// Maps the solid window on `socket` and holds it: commit, ack the
/// configure, commit at the configured size. Times out with what arrived
/// instead of hanging on a compositor that never configures.
fn map_solid_window(socket: &Path) -> SolidWindow {
    let stream = UnixStream::connect(socket).expect("a Wayland connection");
    let conn = Connection::from_socket(stream).expect("a Wayland client");
    let mut queue: EventQueue<TestClient> = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    let deadline = Instant::now() + PATIENCE;
    queue.roundtrip(&mut client).expect("registry globals");
    drop(registry);
    let compositor = client.compositor.clone().expect("wl_compositor");
    let shm = client.shm.clone().expect("wl_shm");
    let wm_base = client.wm_base.clone().expect("xdg_wm_base");
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_app_id("scoot-golden".to_owned());
    toplevel.set_title("golden".to_owned());
    // First commit at an arbitrary size: it provokes the configure whose
    // size the second commit matches.
    let (pool, buffer, file) = magenta_buffer(&shm, &qh, 400, 300);
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, 400, 300);
    surface.commit();
    let (serial, width, height) = loop {
        queue.roundtrip(&mut client).expect("Wayland events");
        if let Some(pending) = client.pending {
            break pending;
        }
        assert!(
            Instant::now() < deadline,
            "the compositor never configured the test window"
        );
    };
    xdg.ack_configure(serial);
    let (pool, buffer, file) = if width > 0 && height > 0 {
        let (pool, buffer, file) = magenta_buffer(&shm, &qh, width, height);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        queue.roundtrip(&mut client).expect("Wayland events");
        (pool, buffer, file)
    } else {
        surface.commit();
        queue.roundtrip(&mut client).expect("Wayland events");
        (pool, buffer, file)
    };
    SolidWindow {
        _conn: conn,
        _queue: queue,
        _surface: surface,
        _xdg: xdg,
        _toplevel: toplevel,
        _pool: pool,
        _buffer: buffer,
        _file: file,
    }
}

/// A decoded screenshot: 8-bit RGBA, row-major.
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Decodes `path` (what `scoot msg screenshot` writes: 8-bit RGB or RGBA)
/// to RGBA, so the compare below never depends on the encoder's color
/// type choice.
fn read_png(path: &Path) -> Image {
    let file = std::fs::File::open(path)
        .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().expect("a PNG reader");
    let mut buf = vec![0; reader.output_buffer_size().expect("a PNG buffer size")];
    let info = reader.next_frame(&mut buf).expect("a PNG frame");
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(buf.len() / 3 * 4);
            for rgb in buf.chunks_exact(3) {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0xFF]);
            }
            rgba
        }
        other => panic!("an RGB(A) screenshot at {}, not {other:?}", path.display()),
    };
    Image {
        width: info.width,
        height: info.height,
        rgba,
    }
}

/// How far apart two screenshots are: the count of pixels past the
/// tolerance, and the single worst channel miss with its position and
/// colors, for the failure message.
struct Diff {
    mismatched: u64,
    worst: u8,
    worst_at: (u32, u32),
    expected: [u8; 4],
    actual: [u8; 4],
}

fn diff(expected: &Image, actual: &Image) -> Diff {
    assert_eq!(
        (actual.width, actual.height),
        (expected.width, expected.height),
        "a golden at {}x{} never resizes to {}x{}",
        expected.width,
        expected.height,
        actual.width,
        actual.height,
    );
    let mut out = Diff {
        mismatched: 0,
        worst: 0,
        worst_at: (0, 0),
        expected: [0; 4],
        actual: [0; 4],
    };
    for (i, (want, got)) in expected.rgba.iter().zip(actual.rgba.iter()).enumerate() {
        let miss = want.abs_diff(*got);
        if miss <= TOLERANCE {
            continue;
        }
        if i % 4 == 0 {
            out.mismatched += 1;
        }
        if miss > out.worst {
            let pixel = i / 4;
            out.worst = miss;
            out.worst_at = (
                (pixel as u32) % expected.width,
                (pixel as u32) / expected.width,
            );
            let base = pixel * 4;
            out.expected = expected.rgba[base..base + 4].try_into().expect("a pixel");
            out.actual = actual.rgba[base..base + 4].try_into().expect("a pixel");
        }
    }
    out
}

/// Checks `actual` against the committed golden for `name`: with
/// `SCOOT_GOLDEN_REGENERATE=1` writes the golden and passes (CI never
/// sets it), else fails past the tolerance, saving the actual beside the
/// golden for inspection and for CI's `look-golden-actual` artifact.
fn check_golden(name: &str, actual: &Image) {
    let golden = golden_path(name);
    if std::env::var_os("SCOOT_GOLDEN_REGENERATE").is_some() {
        std::fs::create_dir_all(goldens_dir()).expect("a goldens dir");
        let mut file = std::fs::File::create(&golden)
            .unwrap_or_else(|error| panic!("write {}: {error}", golden.display()));
        let mut encoder = png::Encoder::new(&mut file, actual.width, actual.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("a PNG header")
            .write_image_data(&actual.rgba)
            .expect("a PNG body");
        return;
    }
    let expected = read_png(&golden);
    let compared = diff(&expected, actual);
    if compared.mismatched > 0 {
        let actual_path = goldens_dir().join(format!("look-{name}.actual.png"));
        let mut file = std::fs::File::create(&actual_path)
            .unwrap_or_else(|error| panic!("write {}: {error}", actual_path.display()));
        let mut encoder = png::Encoder::new(&mut file, actual.width, actual.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("a PNG header")
            .write_image_data(&actual.rgba)
            .expect("a PNG body");
        panic!(
            "`{name}` moved {n} pixels past {TOLERANCE}/channel against {}: \
             worst miss {worst} at ({x}, {y}), want rgba({er}, {eg}, {eb}, {ea}), \
             got rgba({ar}, {ag}, {ab}, {aa}); actual saved to {}. \
             If the move is intended, regenerate (see the module doc) and inspect the diff.",
            golden.display(),
            actual_path.display(),
            n = compared.mismatched,
            worst = compared.worst,
            x = compared.worst_at.0,
            y = compared.worst_at.1,
            er = compared.expected[0],
            eg = compared.expected[1],
            eb = compared.expected[2],
            ea = compared.expected[3],
            ar = compared.actual[0],
            ag = compared.actual[1],
            ab = compared.actual[2],
            aa = compared.actual[3],
        );
    }
}

/// One settled frame: screenshots until two in a row are byte-identical
/// (the encoder is deterministic, so identical pixels mean identical
/// bytes), bounding a scene that never stills. The bar mapping, the
/// window mapping and the look reload each move pixels once; the loop
/// absorbs all three without naming any look's colors.
fn settled_png(session: &Session, tag: &str) -> Image {
    let deadline = Instant::now() + PATIENCE;
    let mut previous: Option<Vec<u8>> = None;
    for round in 0.. {
        session.wait_idle();
        let shot = session.screenshot(&format!("{tag}-{round}"));
        let bytes =
            std::fs::read(&shot).unwrap_or_else(|error| panic!("read {}: {error}", shot.display()));
        if previous.as_ref() == Some(&bytes) {
            return read_png(&shot);
        }
        previous = Some(bytes);
        assert!(
            Instant::now() < deadline,
            "the scene never settled for a screenshot; log:\n{}",
            session.log_tail()
        );
        assert!(
            session.bar_alive(),
            "the bar died while settling; bar log:\n{}",
            session.bar_log_tail()
        );
    }
    unreachable!("the deadline above always fires first");
}

/// The full scene for one look out of `catalog`: a headless session, the
/// bar, the magenta window, `look-apply`, then a settled screenshot.
/// Returns `None` without running anything when there is no bar binary
/// (see [`bar_or_skip`]).
fn render_look(tag: &str, name: &str, catalog: &Path, bar_bin: &Path) -> Option<Image> {
    let scratch = Scratch::new(tag);
    let mut session = Session::start(scratch, catalog);
    session.start_bar(bar_bin);
    let socket = session.runtime.join(&session.wayland_display);
    let held = map_solid_window(&socket);
    session.wait_for_window("scoot-golden");
    session.look_apply(name);
    let image = settled_png(&session, name);
    drop(held);
    Some(image)
}

/// One shipped look, end to end.
fn golden_case(name: &str) {
    let Some(bar_bin) = bar_or_skip() else {
        return;
    };
    let catalog = shipped_looks_dir();
    let actual = render_look(name, name, &catalog, &bar_bin).expect("a screenshot");
    check_golden(name, &actual);
}

#[test]
fn golden_ginger_night() {
    golden_case("ginger-night");
}

#[test]
fn golden_moonrise() {
    golden_case("moonrise");
}

#[test]
fn golden_music_desk() {
    golden_case("music-desk");
}

#[test]
fn golden_radial_burst() {
    golden_case("radial-burst");
}

#[test]
fn golden_vinyl_sunset() {
    golden_case("vinyl-sunset");
}

/// The set stays complete: a look added to `looks/` without a golden (or
/// a golden without a look) fails here, naming the drift.
#[test]
fn every_shipped_look_has_a_golden() {
    let catalog = shipped_looks_dir();
    let mut looks: Vec<String> = std::fs::read_dir(&catalog)
        .unwrap_or_else(|error| panic!("read {}: {error}", catalog.display()))
        .filter_map(|entry| {
            let entry = entry.expect("a catalog entry");
            let look = entry.file_name().into_string().ok()?;
            entry.path().join("look.toml").is_file().then_some(look)
        })
        .collect();
    looks.sort();
    let listed: Vec<String> = LOOKS.iter().map(|name| name.to_string()).collect();
    assert_eq!(
        looks,
        listed,
        "LOOKS matches the catalog dirs under {}",
        catalog.display()
    );
    let mut goldens: Vec<String> = std::fs::read_dir(goldens_dir())
        .unwrap_or_else(|error| {
            panic!(
                "read {}: {error} (regenerate, see the module doc)",
                goldens_dir().display()
            )
        })
        .filter_map(|entry| {
            entry
                .expect("a goldens entry")
                .file_name()
                .into_string()
                .ok()?
                .strip_prefix("look-")?
                .strip_suffix(".png")
                .map(str::to_owned)
        })
        .collect();
    goldens.sort();
    assert_eq!(
        goldens,
        listed,
        "one golden per shipped look in {}",
        goldens_dir().display()
    );
}

/// The negative control: ginger-night with a white focus ring must miss
/// its golden by a mile, proving the comparison is not vacuous. The ring
/// is the only change, so the failure is all ring pixels: thousands of
/// them, off by up to 207 in blue (`#FF9A30` to `#FFFFFF`).
#[test]
fn a_broken_look_is_nothing_like_its_golden() {
    let Some(bar_bin) = bar_or_skip() else {
        return;
    };
    let scratch = Scratch::new("broken");
    let catalog = scratch.dir.join("catalog").join("broken-white");
    let source = shipped_looks_dir().join("ginger-night");
    copy_dir(&source, &catalog);
    let manifest = catalog.join("look.toml");
    let text = std::fs::read_to_string(&manifest).expect("a look manifest");
    let text = text
        .replace("name = \"ginger-night\"", "name = \"broken-white\"")
        .replace("focus = \"#FF9A30\"", "focus = \"#FFFFFF\"");
    assert!(
        text.contains("focus = \"#FFFFFF\""),
        "the broken look really changed the focus ring"
    );
    std::fs::write(&manifest, text).expect("a broken look manifest");
    let actual = render_look(
        "broken",
        "broken-white",
        scratch.dir.join("catalog").as_path(),
        &bar_bin,
    )
    .expect("a screenshot");
    let golden = golden_path("ginger-night");
    assert!(
        golden.is_file(),
        "regenerate first (see the module doc): {}",
        golden.display()
    );
    let expected = read_png(&golden);
    let compared = diff(&expected, &actual);
    assert!(
        compared.mismatched >= 100,
        "a white ring moves at least hundreds of pixels, not {}",
        compared.mismatched
    );
    assert!(
        compared.worst >= 100,
        "a white ring misses by ~200 a channel, not {}",
        compared.worst
    );
}

/// Copies `source` (a look dir) to `target`, file by file, so the broken
/// look keeps the image wallpaper and every other token of its parent.
fn copy_dir(source: &Path, target: &Path) {
    std::fs::create_dir_all(target)
        .unwrap_or_else(|error| panic!("make {}: {error}", target.display()));
    for entry in std::fs::read_dir(source)
        .unwrap_or_else(|error| panic!("read {}: {error}", source.display()))
    {
        let entry = entry.expect("a look file");
        let to = target.join(entry.file_name());
        let from = entry.path();
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap_or_else(|error| {
                panic!("copy {} to {}: {error}", from.display(), to.display())
            });
        }
    }
}
