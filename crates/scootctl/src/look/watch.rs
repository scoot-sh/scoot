//! `scoot look watch DIR`: the authoring loop.
//!
//! Validates and applies on save, debounced on `close_write`/`moved_to`,
//! under the lock, rendering into one reused cache slot
//! (`<cache>/<name>-watch/`). An invalid save is refused loudly and the
//! last good look stays: the swap validates everything before the record
//! moves, so a refusal touches nothing.
//!
//! The loop watches `DIR` non-recursively: `look.toml`, the wallpaper and
//! the top-level files are what an author saves. Files arriving in a
//! subdirectory (`fonts/`, `apps/`) apply on the next top-level save, or
//! start a second `watch` on that dir.
//!
//! Linux watches with inotify; other systems poll the directory snapshot
//! twice a second (the `scoot` Mac client is a client only, so its watch
//! loop never drives a local session anyway).

use std::path::{Path, PathBuf};
use std::time::Duration;

use scoot_look::Origin;
use scoot_look::catalog::{Catalog, Entry};
use scoot_look::state::{RecordRead, acquire_lock, ensure_link, read_record};

use super::apply::{
    ApplyFailure, ApplyReport, Checks, RenderSource, SwapRequest, SwapTarget, Validation,
    absolutize, check_inputs, collect_chain, complete_swap, render_for_swap,
};
use super::cycle::{PreviewRead, clear_preview, read_preview};
use super::reload::{BarReload, CompositorReload};
use super::sync::TargetSkips;

/// How long the loop waits for the saves to settle after the first
/// `close_write`/`moved_to` before applying: one quiet period covers an
/// editor's write-plus-rename pair without applying twice.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// The reused cache slot for a watched look: `<cache>/<name>-watch/`. The
/// name never matches the prune's `<name>-<16 hex>` pattern, so the prune
/// never collects it and the cache never grows per save.
pub fn watch_slot(name: &str) -> String {
    format!("{name}-watch")
}

/// What one watched save did, for the loop's output line.
#[derive(Debug)]
pub enum SaveOutcome {
    /// The save validated and swapped: the generation now live.
    Applied {
        name: String,
        generation: u64,
        reloaded: Vec<String>,
    },
    /// The save was invalid: the message names why, and the last good
    /// look is still live.
    Refused { message: String },
}

/// Options for one watched save, from flags and the environment.
pub struct WatchOptions {
    /// How long to wait on the lock.
    pub lock_wait: Duration,
    /// Skip the toolkit step (`--no-toolkit`).
    pub no_toolkit: bool,
    /// The origin the watched directory validates as (`--origin`, like
    /// `validate`).
    pub origin: Origin,
    /// dconf key paths home-manager owns: left alone however they read.
    pub nix_claims: Vec<String>,
    /// Apps whose Stylix target owns its path (the
    /// `look-stylix-targets` file beside the declared default): the
    /// reply names those instead of pretending the look reached them.
    pub stylix_targets: Vec<String>,
}

/// One watched save: takes the lock, reads the directory fresh, validates
/// it completely, renders into the reused slot, then records, repoints
/// and reloads in order through the same [`complete_swap`] a named apply
/// uses. Any refusal (unknown base, bad value, failed check) returns
/// before the record moves: nothing is touched and the last good look
/// stays live.
#[allow(clippy::too_many_arguments)]
pub fn apply_watched_dir(
    dir: &Path,
    options: &WatchOptions,
    catalog: &Catalog,
    cache: &Path,
    state_dir: &Path,
    config: Option<&Path>,
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
    checks: &dyn Checks,
    dconf: &dyn super::toolkit::Dconf,
    leaves: &dyn super::leaves::Leaves,
    live: &[PathBuf],
) -> Result<ApplyReport, ApplyFailure> {
    let refuse = |message: String| ApplyFailure {
        message,
        rolled_back_to: None,
        rollback_errors: Vec::new(),
    };
    // Work on absolute dirs throughout, like a named apply: a relative
    // cache would record a relative render path (which the next read
    // refuses as corrupt).
    let state_dir = absolutize(state_dir).map_err(refuse)?;
    let state_dir = state_dir.as_path();
    let cache = absolutize(cache).map_err(refuse)?;
    let cache = cache.as_path();
    std::fs::create_dir_all(state_dir).map_err(|error| {
        refuse(format!(
            "cannot use state dir {}: {error}",
            state_dir.display()
        ))
    })?;
    let _lock = acquire_lock(state_dir, options.lock_wait).map_err(refuse)?;
    let mut report = ApplyReport::default();
    match ensure_link(state_dir).map_err(refuse)? {
        scoot_look::state::LinkRepair::Healed(message) => report.notes.push(message),
        scoot_look::state::LinkRepair::Skipped(message) => {
            if !matches!(read_record(state_dir), RecordRead::Missing) {
                report.notes.push(message);
            }
        }
        scoot_look::state::LinkRepair::Consistent => {}
    }
    let previous = match read_record(state_dir) {
        RecordRead::Present(record) => Some(record),
        RecordRead::Missing => None,
        RecordRead::Corrupt(reason) => {
            return Err(refuse(format!(
                "cannot watch: the recorded choice is corrupt ({reason}): fix or remove {}",
                state_dir.join("look.json").display()
            )));
        }
    };
    // Read the directory fresh under the lock: the manifest, its name
    // check, and the origin/allowlist rules a named apply gets from the
    // catalog scan (a watched directory bypasses that scan).
    let canonical = std::fs::canonicalize(dir)
        .map_err(|error| refuse(format!("cannot watch {}: {error}", dir.display())))?;
    let dir_name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let bytes = super::apply::read_regular(&canonical.join("look.toml")).map_err(refuse)?;
    let raw = scoot_look::parse_look_toml(&bytes).map_err(|errors| {
        refuse(format!(
            "invalid save: {}: {} (keeping {})",
            canonical.join("look.toml").display(),
            errors
                .iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; "),
            previous
                .as_ref()
                .map(|record| record.name.as_str())
                .unwrap_or("(no look)")
        ))
    })?;
    if let Some(error) = scoot_look::check_name_matches_dir(&raw, &dir_name) {
        return Err(refuse(format!(
            "invalid save: {error} (keeping {})",
            previous
                .as_ref()
                .map(|record| record.name.as_str())
                .unwrap_or("(no look)")
        )));
    }
    let mut early = Validation::default();
    for error in scoot_look::validate_for_origin(&raw, options.origin)
        .into_iter()
        .chain(scoot_look::validate_apps_allowlist(&raw.apps))
    {
        early.errors.push(error.to_string());
    }
    if !early.errors.is_empty() {
        return Err(refuse(format!(
            "invalid save: {} (keeping {})",
            early.errors.join("; "),
            previous
                .as_ref()
                .map(|record| record.name.as_str())
                .unwrap_or("(no look)")
        )));
    }
    // The watched directory wins over any same-named catalog entry (a
    // tweaked copy never silently applies the catalog's look); its
    // `extends` base resolves through the catalog, so an installed base is
    // refused like anywhere else.
    let name = raw.name.clone();
    let mut overlay = Catalog {
        entries: vec![Entry {
            name: name.clone(),
            dir: canonical.clone(),
            manifest_dir: canonical,
            origin: options.origin,
            raw,
            bytes: bytes.clone(),
        }],
        shadowed: catalog.shadowed.clone(),
        notes: catalog.notes.clone(),
    };
    for entry in &catalog.entries {
        if entry.name == name {
            overlay
                .shadowed
                .push((entry.name.clone(), entry.dir.clone()));
        } else {
            overlay.entries.push(entry.clone());
        }
    }
    let chain = collect_chain(&overlay, &name).map_err(refuse)?;
    let merged = scoot_look::resolve(&overlay.sources(), &name)
        .map_err(|error| refuse(format!("invalid save: cannot resolve `{name}`: {error}")))?;
    let contrast = scoot_look::check_contrast(&merged, options.origin);
    let mut contrast_errors = Vec::new();
    for refusal in contrast.refusals() {
        contrast_errors.push(format!(
            "{} is {} (needs {})",
            refusal.name,
            scoot_look::format_ratio(refusal.ratio),
            refusal.threshold
        ));
    }
    if !contrast_errors.is_empty() {
        return Err(refuse(format!(
            "invalid save: cannot apply `{name}`: {}",
            contrast_errors.join("; ")
        )));
    }
    for warning in contrast.warnings() {
        report.notes.push(format!(
            "contrast: {} is {} (needs {})",
            warning.name,
            scoot_look::format_ratio(warning.ratio),
            warning.threshold
        ));
    }
    let mut validation = Validation::default();
    let inputs = match check_inputs(&chain, &merged, checks, &mut validation) {
        Ok(inputs) => inputs,
        Err(_) => {
            let detail = validation.errors.join("; ");
            let detail = if detail.trim().is_empty() {
                "invalid look (no detail)".to_owned()
            } else {
                detail
            };
            return Err(refuse(format!(
                "invalid save: cannot apply `{name}`: {detail}"
            )));
        }
    };
    report.notes.extend(validation.notes);
    report.notes.extend(inputs.details);
    let slot = watch_slot(&name);
    let (target, _) = render_for_swap(
        &name,
        &chain,
        &merged,
        inputs.inputs,
        RenderSource::FixedSlot { slot },
        cache,
        checks,
        &mut report.notes,
        false,
    )
    .map_err(refuse)?;
    let swap = SwapRequest {
        name: name.clone(),
        target: SwapTarget::Point(target),
        chosen_by: "watch".to_owned(),
        declared_default: previous
            .as_ref()
            .and_then(|record| record.declared_default.clone()),
        no_toolkit: options.no_toolkit,
        no_helix: false,
        no_btop: false,
        nix_claims: options.nix_claims.clone(),
        stylix_targets: options.stylix_targets.clone(),
        target_skips: TargetSkips::default(),
    };
    complete_swap(
        &mut report,
        state_dir,
        cache,
        previous.as_ref(),
        &swap,
        Some(&merged),
        config,
        compositor,
        bars,
        dconf,
        leaves,
        live,
    )?;
    Ok(report)
}

/// A snapshot of the watched directory: every regular file's relative
/// path, length and mtime, capped. Two snapshots that compare equal mean
/// no save happened (an editor's temp-file dance leaves no trace here),
/// so the loop applies nothing.
fn snapshot_dir(dir: &Path) -> Vec<(String, u64, u64, u32)> {
    const CAP: usize = 512;
    let mut out = Vec::new();
    snapshot_into(dir, dir, &mut out, CAP);
    out.sort();
    out
}

fn snapshot_into(base: &Path, dir: &Path, out: &mut Vec<(String, u64, u64, u32)>, cap: usize) {
    if out.len() >= cap {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut names: Vec<_> = entries.filter_map(|entry| entry.ok()).collect();
    names.sort_by_key(|entry| entry.file_name());
    for entry in names {
        if out.len() >= cap {
            return;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(base)
            .map(|rel| rel.display().to_string())
            .unwrap_or_default();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.is_symlink() {
            // A symlink swapped under the watcher re-reads as its target
            // at apply time (which refuses symlinks for `look.toml`
            // itself); its own dance is not a save.
            continue;
        }
        if meta.is_dir() {
            // One level: `fonts/`, `apps/` and their files count; deeper
            // trees are not looks.
            if path == base || path.parent() == Some(base) {
                snapshot_into(base, &path, out, cap);
            }
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|age| (age.as_secs(), age.subsec_nanos()))
            .unwrap_or_default();
        out.push((rel, meta.len(), mtime.0, mtime.1));
    }
}

/// Runs the loop until the directory goes away or the watcher fails: an
/// initial apply, then one debounced apply per save. Each save reports
/// through `emit`; the loop returns only on a fatal error (the watched
/// directory is gone, the state dir is unusable).
#[allow(clippy::too_many_arguments)]
pub fn watch_loop(
    dir: &Path,
    options: &WatchOptions,
    catalog: &Catalog,
    cache: &Path,
    state_dir: &Path,
    config: Option<&Path>,
    make_compositor: &dyn Fn() -> Box<dyn CompositorReload>,
    make_bars: &dyn Fn() -> Vec<Box<dyn BarReload>>,
    checks: &dyn Checks,
    dconf: &dyn super::toolkit::Dconf,
    leaves: &dyn super::leaves::Leaves,
    emit: &mut dyn FnMut(SaveOutcome, &ApplyReport),
) -> Result<(), String> {
    let canonical = std::fs::canonicalize(dir)
        .map_err(|error| format!("cannot watch {}: {error}", dir.display()))?;
    if !canonical.is_dir() {
        return Err(format!("cannot watch {}: not a directory", dir.display()));
    }
    // A plain apply supersedes any pending timed preview; a watch owns
    // the session's look from here on, so forget the file once, loudly.
    if matches!(read_preview(state_dir), PreviewRead::Present(_)) {
        clear_preview(state_dir);
    }
    let mut watcher = Watcher::open(&canonical)?;
    let mut last: Option<Vec<(String, u64, u64, u32)>> = None;
    loop {
        // The initial pass applies whatever the directory holds now, so
        // the screen follows from the start.
        let snapshot = snapshot_dir(&canonical);
        if last.as_ref() != Some(&snapshot) {
            last = Some(snapshot);
            let mut compositor = make_compositor();
            let bars = make_bars();
            let bar_refs: Vec<&dyn BarReload> = bars
                .iter()
                .map(|bar| bar.as_ref() as &dyn BarReload)
                .collect();
            match apply_watched_dir(
                &canonical,
                options,
                catalog,
                cache,
                state_dir,
                config,
                compositor.as_mut(),
                &bar_refs,
                checks,
                dconf,
                leaves,
                &super::apply::live_render_targets(),
            ) {
                Ok(report) => emit(
                    SaveOutcome::Applied {
                        name: report.name.clone(),
                        generation: report.generation,
                        reloaded: report.reloaded.clone(),
                    },
                    &report,
                ),
                Err(failure) => {
                    let empty = ApplyReport::default();
                    emit(
                        SaveOutcome::Refused {
                            message: failure.to_string(),
                        },
                        &empty,
                    );
                }
            }
        }
        if !watcher.wait_for_save()? {
            return Err(format!(
                "cannot watch {}: the directory is gone",
                canonical.display()
            ));
        }
    }
}

/// Waits for the next save in the watched directory: `true` for a
/// debounced `close_write`/`moved_to`, `false` when the directory itself
/// is gone.
struct Watcher {
    #[cfg(target_os = "linux")]
    inotify: Option<InotifyWatcher>,
    #[cfg(not(target_os = "linux"))]
    last: Vec<(String, u64, u64, u32)>,
    dir: PathBuf,
}

impl Watcher {
    fn open(dir: &Path) -> Result<Self, String> {
        Ok(Self {
            #[cfg(target_os = "linux")]
            inotify: Some(InotifyWatcher::open(dir)?),
            #[cfg(not(target_os = "linux"))]
            last: snapshot_dir(dir),
            dir: dir.to_owned(),
        })
    }

    fn wait_for_save(&mut self) -> Result<bool, String> {
        #[cfg(target_os = "linux")]
        {
            let Some(watcher) = self.inotify.as_mut() else {
                return Err("the watcher is closed".to_owned());
            };
            watcher.wait_for_save(&self.dir)
        }
        #[cfg(not(target_os = "linux"))]
        {
            // Twice a second: the Mac client drives no local session, so
            // this only paces the loop, never a live desktop.
            for _ in 0..(10 * 60 * 60 * 24) {
                std::thread::sleep(Duration::from_millis(500));
                if !self.dir.is_dir() {
                    return Ok(false);
                }
                let now = snapshot_dir(&self.dir);
                if now != self.last {
                    self.last = now;
                    return Ok(true);
                }
            }
            Err("the watcher gave up after a day".to_owned())
        }
    }
}

#[cfg(target_os = "linux")]
struct InotifyWatcher {
    fd: std::os::fd::OwnedFd,
    buffer: Vec<std::mem::MaybeUninit<u8>>,
}

#[cfg(target_os = "linux")]
impl InotifyWatcher {
    fn open(dir: &Path) -> Result<Self, String> {
        use rustix::fs::inotify::{CreateFlags, WatchFlags};
        let fd = rustix::fs::inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC)
            .map_err(|error| format!("cannot watch {}: {error}", dir.display()))?;
        rustix::fs::inotify::add_watch(
            &fd,
            dir,
            WatchFlags::CLOSE_WRITE
                | WatchFlags::MOVED_TO
                | WatchFlags::MOVE_SELF
                | WatchFlags::DELETE_SELF,
        )
        .map_err(|error| format!("cannot watch {}: {error}", dir.display()))?;
        Ok(Self {
            fd,
            buffer: vec![std::mem::MaybeUninit::uninit(); 4096],
        })
    }

    /// Blocks until a debounced save lands: drains the qualifying events,
    /// then waits out one quiet [`DEBOUNCE`] period, extending it while
    /// saves keep arriving. `Ok(false)` when the watched directory
    /// itself moved or was deleted. The first wait blocks in `poll(2)`
    /// with no timeout, so an idle watch sleeps with zero wakeups; a
    /// signal with the default disposition (SIGINT/SIGTERM) still kills
    /// the process out of the blocking wait, since no handler is
    /// installed.
    fn wait_for_save(&mut self, dir: &Path) -> Result<bool, String> {
        loop {
            if !dir.is_dir() {
                return Ok(false);
            }
            self.wait_readable(None)?;
            match self.drain() {
                Ok(Drain::Save) => break,
                Ok(Drain::Gone) => return Ok(false),
                Ok(Drain::Empty) => {}
                Err(error) => return Err(error),
            }
        }
        // The debounce: one quiet period with no qualifying event ends
        // the save; every event restarts it. Each 50 ms slice blocks in
        // `poll(2)`, so the debounce wakes only on events or slice ends.
        let mut quiet = 0u32;
        while quiet * 50 < DEBOUNCE.as_millis() as u32 {
            self.wait_readable(Some(Duration::from_millis(50)))?;
            match self.drain() {
                Ok(Drain::Save) => quiet = 0,
                Ok(Drain::Gone) => return Ok(false),
                Ok(Drain::Empty) => quiet += 1,
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }

    /// Blocks in `poll(2)` until the inotify fd reads (or `timeout`
    /// elapses for `Some`): `None` waits forever. `EINTR` (a caught
    /// signal) retries; anything else is a watcher error.
    fn wait_readable(&self, timeout: Option<Duration>) -> Result<(), String> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        let spec;
        let when = match timeout.as_ref() {
            Some(within) => {
                spec = Timespec::try_from(*within)
                    .map_err(|error| format!("cannot watch for saves: {error}"))?;
                Some(&spec)
            }
            None => None,
        };
        loop {
            let mut fds = [PollFd::new(&self.fd, PollFlags::IN)];
            match poll(&mut fds, when) {
                Ok(_) => return Ok(()),
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(format!("cannot watch for saves: {error}")),
            }
        }
    }

    fn drain(&mut self) -> Result<Drain, String> {
        use rustix::fs::inotify::Reader;
        let mut outcome = Drain::Empty;
        loop {
            let mut reader = Reader::new(&self.fd, &mut self.buffer);
            match reader.next() {
                Err(rustix::io::Errno::WOULDBLOCK) => return Ok(outcome),
                Err(error) => {
                    return Err(format!("cannot watch for saves: {error}"));
                }
                Ok(event) => {
                    use rustix::fs::inotify::ReadFlags;
                    let flags = event.events();
                    if flags.intersects(ReadFlags::IGNORED)
                        || flags.intersects(ReadFlags::MOVE_SELF)
                        || flags.intersects(ReadFlags::DELETE_SELF)
                    {
                        return Ok(Drain::Gone);
                    }
                    if flags.intersects(ReadFlags::QUEUE_OVERFLOW) {
                        // The queue overflowed: something saved faster
                        // than the loop drains. Re-validate once rather
                        // than trust the lost events.
                        outcome = Drain::Save;
                    }
                    if flags.intersects(ReadFlags::CLOSE_WRITE)
                        || flags.intersects(ReadFlags::MOVED_TO)
                    {
                        outcome = Drain::Save;
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
enum Drain {
    Empty,
    Save,
    Gone,
}

#[cfg(test)]
mod tests;
