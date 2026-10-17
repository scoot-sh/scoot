//! `scoot look preview`: show or capture a look's preview image.
//!
//! Bare `preview [NAME]` reports the look's `preview.png` (beside its
//! `look.toml`) and whether it exists. `preview --capture` renders the
//! look, seeds a scratch user state with it, starts a private headless
//! session on that state, screenshots it, and writes the PNG — so the
//! image shows the look itself, and the author's real session, home and
//! cache are never touched. Works with no Nix: the session is the same
//! plain binary, and the catalog is plain files.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use scoot_look::Origin;
use scoot_look::catalog::Catalog;
use scoot_look::state::{Record, RecordRead, read_record, write_record};

use super::apply::{BuildTarget, Checks, build_to_out};

/// How long `--capture` waits for the headless session to answer.
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(60);

/// What `--capture` wrote.
#[derive(Debug)]
pub struct CaptureReport {
    /// The look captured.
    pub name: String,
    /// The PNG written.
    pub png: PathBuf,
    /// The screenshot's size.
    pub width: u32,
    pub height: u32,
    /// The PNG's length in bytes.
    pub bytes: usize,
}

/// Options for one `--capture`, from flags.
pub struct CaptureOptions {
    /// Where the PNG goes (`None` means beside the look's `look.toml`).
    pub out: Option<PathBuf>,
    /// The headless session's output size.
    pub width: u32,
    pub height: u32,
    /// How long to wait for the session.
    pub timeout: Duration,
}

/// Resolves what to preview: a catalog name, a source directory (anything
/// with a slash, `.` or `..`), or the active look when no target is
/// given. Returns the look's name and its source directory (where a
/// captured `preview.png` lands by default).
pub fn resolve_preview_target(
    target: Option<&str>,
    origin: Origin,
    catalog: &Catalog,
    state_dir: &Path,
) -> Result<(String, PathBuf, Origin), String> {
    match target {
        Some(given) => {
            let is_path = given.contains('/') || given == "." || given == "..";
            if is_path {
                let dir = match std::fs::canonicalize(given) {
                    Ok(dir) => dir,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Err(super::not_a_dir(given, "preview"));
                    }
                    Err(error) => return Err(format!("cannot resolve `{given}`: {error}")),
                };
                if !dir.is_dir() {
                    return Err(super::not_a_dir(given, "preview"));
                }
                let bytes = super::apply::read_regular(&dir.join("look.toml"))
                    .map_err(|error| format!("cannot preview `{given}`: {error}"))?;
                let raw = scoot_look::parse_look_toml(&bytes).map_err(|errors| {
                    errors
                        .iter()
                        .map(|error| error.to_string())
                        .collect::<Vec<_>>()
                        .join("; ")
                })?;
                Ok((raw.name, dir, origin))
            } else {
                let entry = catalog.find(given).ok_or_else(|| {
                    let mut names: Vec<&str> = catalog
                        .entries
                        .iter()
                        .map(|entry| entry.name.as_str())
                        .collect();
                    names.sort();
                    if names.is_empty() {
                        format!("unknown look `{given}`: {}", super::empty_catalog_hint())
                    } else {
                        format!("unknown look `{given}` (available: {})", names.join(", "))
                    }
                })?;
                Ok((entry.name.clone(), entry.dir.clone(), entry.origin))
            }
        }
        None => {
            let record = match read_record(state_dir) {
                RecordRead::Present(record) => record,
                RecordRead::Missing => {
                    return Err(
                        "no look applied yet: name one (`scoot look preview NAME --capture`)"
                            .to_owned(),
                    );
                }
                RecordRead::Corrupt(reason) => {
                    return Err(format!("the recorded choice is corrupt: {reason}"));
                }
            };
            let entry = catalog.find(&record.name).ok_or_else(|| {
                format!(
                    "the active look `{}` is not in the catalog: name a look or a directory",
                    record.name
                )
            })?;
            Ok((entry.name.clone(), entry.dir.clone(), entry.origin))
        }
    }
}

/// Captures `dir` (a source look directory) into `out`: renders it into a
/// scratch cache, seeds a scratch user state pointing at the render,
/// starts a private headless session on that state, screenshots its
/// output, and writes the PNG. Every scratch dir lives under one temp
/// root (removed on return); the author's real state, cache, config and
/// home are never read or written — only `out` is.
pub fn capture(
    name: &str,
    dir: &Path,
    origin: Origin,
    out: &Path,
    options: &CaptureOptions,
    catalog: &Catalog,
    checks: &dyn Checks,
) -> Result<CaptureReport, String> {
    // The scratch session is the compositor, which only runs on Linux:
    // refuse before building, with one line instead of a failed startup
    // plus its log tail.
    if cfg!(not(target_os = "linux")) {
        return Err(
            "preview --capture needs Linux: the headless session is the compositor, which only runs there"
                .to_owned(),
        );
    }
    let root = scratch_root()?;
    let rendered = root.join("rendered");
    let build = build_to_out(
        &BuildTarget::Dir {
            path: dir.to_owned(),
            origin,
        },
        &rendered,
        catalog,
        checks,
        false,
    );
    if let Err(error) = build {
        std::fs::remove_dir_all(&root).ok();
        return Err(error);
    }
    // The scratch user state the headless session seeds its private dir
    // from: the record plus the link, like any swap leaves them.
    let user_state = root.join("state-home").join("scoot");
    std::fs::create_dir_all(&user_state)
        .map_err(|error| format!("cannot create {}: {error}", user_state.display()))?;
    let rendered_abs = absolutize_rendered(&rendered)?;
    let record = Record {
        name: name.to_owned(),
        rendered: rendered_abs,
        generation: 1,
        chosen_by: Some("preview".to_owned()),
        declared_default: None,
        toolkit: None,
        dconf_wrote: Default::default(),
        dconf_orig: Default::default(),
    };
    write_record(&user_state, &record).map_err(|error| {
        format!(
            "cannot seed {}: {error}",
            user_state.join("look.json").display()
        )
    })?;
    super::apply::point_link(&user_state, &record.rendered)?;
    let runtime = root.join("run");
    std::fs::create_dir_all(&runtime)
        .map_err(|error| format!("cannot create {}: {error}", runtime.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot seal {}: {error}", runtime.display()))?;
    }
    let socket = runtime.join("scoot.sock");
    let log_path = root.join("headless.log");
    let mut session =
        HeadlessSession::start(&socket, &root, &user_state, &runtime, &log_path, options)?;
    let shot = session.screenshot(&socket, &log_path, options)?;
    let bytes = shot.png.len();
    if bytes < 8 || shot.png[..8] != [137, 80, 78, 71, 13, 10, 26, 10] {
        return Err(format!(
            "the session answered with {} bytes, not a PNG (log:\n{})",
            bytes,
            tail(&log_path)
        ));
    }
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    std::fs::write(out, &shot.png)
        .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    Ok(CaptureReport {
        name: name.to_owned(),
        png: out.to_owned(),
        width: shot.width,
        height: shot.height,
        bytes,
    })
}

/// A temp root for one capture, removed on return.
fn scratch_root() -> Result<PathBuf, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // pid plus a per-process counter plus nanos: two captures in one
    // process within the same nanos never share a root, and a planted
    // symlink at a guessable name is never opened (`create_dir` refuses
    // it instead of writing through one).
    let dir = std::env::temp_dir().join(format!(
        "scoot-look-preview-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    ));
    std::fs::create_dir(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    Ok(dir)
}

/// The render's absolute path for the seeded record (the state reader
/// refuses a relative one as corrupt).
fn absolutize_rendered(rendered: &Path) -> Result<String, String> {
    let absolute = std::fs::canonicalize(rendered)
        .map_err(|error| format!("cannot resolve {}: {error}", rendered.display()))?;
    absolute
        .into_os_string()
        .into_string()
        .map_err(|_| "the render path is not UTF-8".to_owned())
}

/// The last lines of the headless session's log, for failure reports.
fn tail(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let mut lines: Vec<&str> = text.lines().rev().take(20).collect();
    lines.reverse();
    lines.join("\n")
}

/// A private headless session for one capture, killed on drop (whatever
/// the capture did: the session never outlives it).
struct HeadlessSession {
    child: Child,
    root: PathBuf,
}

impl HeadlessSession {
    #[allow(clippy::too_many_arguments)]
    fn start(
        socket: &Path,
        root: &Path,
        user_state: &Path,
        runtime: &Path,
        log_path: &Path,
        options: &CaptureOptions,
    ) -> Result<Self, String> {
        let state_home = user_state
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.join("state-home"));
        let config = root.join("config");
        let cache = root.join("cache");
        let home = root.join("home");
        for dir in [&config, &cache, &home] {
            std::fs::create_dir_all(dir)
                .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        }
        let exe = std::env::current_exe()
            .map_err(|error| format!("cannot find the scoot binary: {error}"))?;
        let log_file = std::fs::File::create(log_path)
            .map_err(|error| format!("cannot create {}: {error}", log_path.display()))?;
        let child = Command::new(exe)
            .arg("--headless")
            .arg("--outputs")
            .arg("1")
            .arg("--width")
            .arg(options.width.to_string())
            .arg("--height")
            .arg(options.height.to_string())
            .arg("--socket")
            .arg(socket)
            .arg("--")
            .arg("sleep")
            .arg("120")
            .env("XDG_STATE_HOME", &state_home)
            .env("XDG_RUNTIME_DIR", runtime)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_CACHE_HOME", &cache)
            .env("HOME", &home)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("SCOOT_SOCKET")
            .env_remove("SCOOT_LOOK_STATE")
            .env_remove("SCOOT_LOOKS_DIR")
            .env("DBUS_SESSION_BUS_ADDRESS", "disabled:")
            .stdout(Stdio::null())
            .stderr(log_file)
            .spawn()
            .map_err(|error| format!("cannot start a headless session: {error}"))?;
        Ok(Self {
            child,
            root: root.to_owned(),
        })
    }

    /// Screenshots the session's output, waiting up to the capture
    /// timeout: the first screenshot doubles as the readiness probe (it
    /// fails until the session answers).
    fn screenshot(
        &mut self,
        socket: &Path,
        log_path: &Path,
        options: &CaptureOptions,
    ) -> Result<scoot_ipc::Screenshot, String> {
        let deadline = Instant::now() + options.timeout;
        loop {
            match self.try_screenshot(socket) {
                Ok(shot) => return Ok(shot),
                Err(error) => {
                    if self
                        .child
                        .try_wait()
                        .map_err(|error| format!("cannot wait on the session: {error}"))?
                        .is_some()
                    {
                        return Err(format!(
                            "the headless session exited during startup ({error}); log:\n{}",
                            tail(log_path)
                        ));
                    }
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "the headless session never answered ({error}); log:\n{}",
                            tail(log_path)
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    fn try_screenshot(&mut self, socket: &Path) -> Result<scoot_ipc::Screenshot, String> {
        let mut client = scoot_ipc::Client::connect(socket)
            .map_err(|error| format!("no session yet: {error}"))?;
        let response = client
            .request(&scoot_ipc::Request::Screenshot {
                output: None,
                cursor: None,
            })
            .map_err(|error| format!("no session yet: {error}"))?;
        match response {
            scoot_ipc::Response::Screenshot(shot) => Ok(shot),
            scoot_ipc::Response::Error { message } => Err(message),
            other => Err(format!("unexpected reply: {other:?}")),
        }
    }
}

impl Drop for HeadlessSession {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        std::fs::remove_dir_all(&self.root).ok();
    }
}

#[cfg(test)]
mod tests;
