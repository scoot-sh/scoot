//! `scoot look next|prev|random` and timed previews (`apply --for`, `--keep`).
//!
//! Cycling walks the catalog in search order (user, installed,
//! home-manager, system, shipped), wrapping at either end. A timed preview
//! applies a look and reverts it after a delay: the revert records the
//! generation it reverts from and does nothing when a later apply moved it,
//! so a preview never undoes a newer choice.
//!
//! The revert survives the CLI dying: the previewing apply spawns a
//! detached child (its own session, stdio to null) that sleeps out the
//! delay and reverts through the same [`apply_with`] transaction under the
//! same lock. No resident daemon, no watcher, no work between swaps.

use std::path::{Path, PathBuf};
use std::time::Duration;

use scoot_look::grammar::is_name;
use scoot_look::state::{self, PREVIEW_FILE};

use super::apply::{ApplyOptions, PREVIEW_SUPERSEDED, apply_with};
use super::reload::{BarReload, CompositorReload};
use super::sync::read_nix_claims;
use scoot_look::catalog::Catalog;

/// How many times the detached reverter waits out a concurrent swap's
/// lock past each attempt's own wait before giving up loudly: twelve
/// five-second waits ride out a swap whose apps hang their bounded
/// reloads, while anything else (a refusal, a corrupt record) returns at
/// once. Nothing is held between attempts.
pub const REVERT_RETRIES: u32 = 12;
/// The wait between a reverter's lock-contention retries.
pub const REVERT_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Which way `cycle_target` steps through the catalog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Next,
    Prev,
}

/// The catalog neighbor of `current` in `step`'s direction, wrapping at
/// either end. `current` names the active look; when it is missing or no
/// longer in the catalog, `Next` starts at the first entry and `Prev` at
/// the last. `None` only when the catalog is empty.
pub fn cycle_target(catalog: &Catalog, current: Option<&str>, step: Step) -> Option<String> {
    if catalog.entries.is_empty() {
        return None;
    }
    let at = current.and_then(|name| catalog.entries.iter().position(|entry| entry.name == name));
    let index = match (at, step) {
        (Some(at), Step::Next) => (at + 1) % catalog.entries.len(),
        (Some(at), Step::Prev) => (at + catalog.entries.len() - 1) % catalog.entries.len(),
        (None, Step::Next) => 0,
        (None, Step::Prev) => catalog.entries.len() - 1,
    };
    Some(catalog.entries[index].name.clone())
}

/// One catalog entry, picked at random (`/dev/urandom`, modulo the
/// catalog length), never the active look: with more than one entry the
/// draw runs over every other look, so a second press always moves. `None`
/// only when the catalog is empty. A one-entry catalog returns its only
/// look.
pub fn random_target(catalog: &Catalog, current: Option<&str>) -> Option<String> {
    if catalog.entries.is_empty() {
        return None;
    }
    if catalog.entries.len() == 1 {
        return Some(catalog.entries[0].name.clone());
    }
    match current.and_then(|name| catalog.entries.iter().position(|entry| entry.name == name)) {
        // Draw in `0..n-1` and skip over the active entry: uniform over
        // every other look.
        Some(at) => {
            let draw = random_index(catalog.entries.len() - 1);
            let index = if draw >= at { draw + 1 } else { draw };
            Some(catalog.entries[index].name.clone())
        }
        None => Some(
            catalog.entries[random_index(catalog.entries.len())]
                .name
                .clone(),
        ),
    }
}

/// A random index in `0..n` (`n > 0`): eight bytes from the OS, reduced
/// modulo `n`. Falls back to a time-and-pid mix when the OS gives nothing
/// (a closed `/dev/urandom` on a strange box): still a pick, never a panic.
fn random_index(n: usize) -> usize {
    debug_assert!(n > 0);
    if let Ok(bytes) = capped_random(8)
        && bytes.len() == 8
    {
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes);
        return (u64::from_ne_bytes(word) % n as u64) as usize;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|age| age.subsec_nanos() as u64 ^ age.as_secs())
        .unwrap_or(0x9e3779b9);
    let mixed = nanos ^ (std::process::id() as u64).wrapping_mul(0x9e3779b97f4a7c15);
    (mixed % n as u64) as usize
}

/// At most eight bytes from `/dev/urandom`, short reads tolerated. `Err`
/// when the device cannot be opened or read at all.
fn capped_random(n: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let handle = std::fs::File::open("/dev/urandom")?;
    let mut bytes = Vec::new();
    handle.take(n as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// A pending timed preview: what to revert to, and the generation the
/// previewing apply wrote. The reverter reverts only while the live
/// generation still equals `from_generation`; any later apply moved it and
/// the revert does nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// The look to restore, or `None` when the previewing apply was the
    /// first choice (reverting clears the layer).
    pub revert_to: Option<String>,
    /// The generation the previewing apply wrote.
    pub from_generation: u64,
}

/// What reading the preview file found.
#[derive(Debug, PartialEq)]
pub enum PreviewRead {
    Present(Preview),
    Missing,
    Corrupt(String),
}

/// The preview file's JSON shape: `{"revert-to": NAME|null,
/// "from-generation": N}`.
fn preview_json(preview: &Preview) -> String {
    let revert_to = match &preview.revert_to {
        Some(name) => format!("\"{name}\""),
        None => "null".to_owned(),
    };
    format!(
        "{{\"revert-to\":{revert_to},\"from-generation\":{}}}",
        preview.from_generation
    )
}

/// Records a pending preview, atomically (temp file, sync, rename) under
/// the caller's lock. `revert_to` is a catalog name validated by the
/// grammar before it is stored, never a path.
pub fn write_preview(dir: &Path, preview: &Preview) -> Result<(), String> {
    if let Some(name) = &preview.revert_to
        && !is_name(name)
    {
        return Err(format!(
            "refusing to record a preview of `{name}`: not a look name"
        ));
    }
    state::write_atomic(dir, PREVIEW_FILE, preview_json(preview).as_bytes())
}

/// Reads the preview file: capped, no-follow, regular-file only, like the
/// record. A missing file means no preview is pending (`--keep` says so,
/// the reverter exits quiet).
pub fn read_preview(dir: &Path) -> PreviewRead {
    let text = match state::read_capped(&dir.join(PREVIEW_FILE)) {
        Ok(text) => text,
        Err(error) if std::io::ErrorKind::NotFound == error.kind() => {
            return PreviewRead::Missing;
        }
        Err(error) => return PreviewRead::Corrupt(format!("{PREVIEW_FILE}: {error}")),
    };
    match parse_preview(&text) {
        Ok(preview) => PreviewRead::Present(preview),
        Err(reason) => PreviewRead::Corrupt(reason),
    }
}

fn parse_preview(text: &str) -> Result<Preview, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("{PREVIEW_FILE}: invalid JSON: {error}"))?;
    let from_generation = value
        .get("from-generation")
        .and_then(|generation| generation.as_u64())
        .ok_or_else(|| format!("{PREVIEW_FILE}: missing `from-generation`"))?;
    let revert_to = match value.get("revert-to") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(name)) => {
            if !is_name(name) {
                return Err(format!(
                    "{PREVIEW_FILE}: `revert-to` must match [a-z0-9-]{{1,32}}, not `{name}`"
                ));
            }
            Some(name.clone())
        }
        Some(_) => {
            return Err(format!(
                "{PREVIEW_FILE}: `revert-to` must be a look name or null"
            ));
        }
    };
    Ok(Preview {
        revert_to,
        from_generation,
    })
}

/// Forgets a pending preview. Missing is fine (a `--keep` after the revert
/// already ran, or after a plain apply cleared it).
pub fn clear_preview(dir: &Path) {
    std::fs::remove_file(dir.join(PREVIEW_FILE)).ok();
}

/// What the detached reverter did.
#[derive(Debug, PartialEq)]
pub enum RevertOutcome {
    /// Back on the previous look (or the layer cleared, when the
    /// previewing apply was the first choice).
    Reverted { name: Option<String> },
    /// A later apply moved the generation (or `--keep` cleared the file):
    /// the preview stays out of its way.
    Superseded,
    /// No preview file at all.
    NothingPending,
}

/// Reverts a timed preview through the same transaction an apply uses,
/// under the same lock — but only while the live generation still equals
/// `from_generation`. Anything else (a later apply moved it, `--keep`
/// cleared it, no file) reverts nothing.
///
/// Never holds the state lock across the revert itself: [`apply_with`]
/// takes it, so holding one here would deadlock the reverter against
/// itself. The preview file is removed only when it still carries this
/// preview's generation, so a newer preview's file is never cleared.
#[allow(clippy::too_many_arguments)]
pub fn revert_if_current(
    from_generation: u64,
    catalog: &Catalog,
    cache: &Path,
    state_dir: &Path,
    config: Option<&Path>,
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
    checks: &dyn super::apply::Checks,
    dconf: &dyn super::toolkit::Dconf,
    leaves: &dyn super::leaves::Leaves,
    lock_wait: Duration,
) -> Result<RevertOutcome, String> {
    // Like an apply: the state dir must exist before the lock is taken.
    // (The previewing apply created it; this only hardens a reverter
    // racing a deleted state.)
    std::fs::create_dir_all(state_dir)
        .map_err(|error| format!("cannot use state dir {}: {error}", state_dir.display()))?;
    {
        let _lock = state::acquire_lock(state_dir, lock_wait)
            .map_err(|error| format!("cannot lock the state dir: {error}"))?;
        match read_preview(state_dir) {
            PreviewRead::Missing => return Ok(RevertOutcome::NothingPending),
            PreviewRead::Corrupt(reason) => {
                clear_preview(state_dir);
                return Err(format!(
                    "the pending preview is corrupt ({reason}): dropped"
                ));
            }
            PreviewRead::Present(preview) if preview.from_generation != from_generation => {
                // A newer preview replaced this one's file: its own
                // reverter owns that file now.
                return Ok(RevertOutcome::Superseded);
            }
            PreviewRead::Present(_) => {}
        }
        match state::read_record(state_dir) {
            state::RecordRead::Present(record) if record.generation != from_generation => {
                clear_preview(state_dir);
                return Ok(RevertOutcome::Superseded);
            }
            state::RecordRead::Present(_) => {}
            // No recorded choice (a concurrent unapply, or the state
            // cleared under the preview): nothing holds the preview, so
            // the file is dead.
            state::RecordRead::Missing => {
                clear_preview(state_dir);
                return Ok(RevertOutcome::Superseded);
            }
            state::RecordRead::Corrupt(reason) => {
                return Err(format!(
                    "cannot revert the preview: the recorded choice is corrupt ({reason})"
                ));
            }
        }
        // The lock drops here, before the revert takes its own.
    }
    let preview = match read_preview(state_dir) {
        PreviewRead::Present(preview) => preview,
        // Cleared between the check and the revert (`--keep` won the
        // race): staying out of its way is the correct revert.
        PreviewRead::Missing => return Ok(RevertOutcome::Superseded),
        PreviewRead::Corrupt(reason) => {
            return Err(format!(
                "the pending preview is corrupt ({reason}): dropped"
            ));
        }
    };
    if preview.from_generation != from_generation {
        // Replaced between the check and the revert: a newer preview owns
        // the file now. Touch nothing; its own reverter owns it.
        return Ok(RevertOutcome::Superseded);
    }
    let outcome = match &preview.revert_to {
        Some(name) => {
            let options = ApplyOptions {
                lock_wait,
                no_toolkit: false,
                trust: false,
                no_helix: false,
                no_btop: false,
                from_compositor: false,
                preview: false,
                // Atomic with the revert itself: a newer apply between
                // the pre-check above and this swap refuses here instead
                // of undoing that newer choice.
                only_if_generation: Some(from_generation),
                nix_claims: read_nix_claims(),
            };
            match apply_with(
                name, &options, catalog, cache, state_dir, config, compositor, bars, checks, dconf,
                leaves,
            ) {
                Ok(_) => RevertOutcome::Reverted {
                    name: Some(name.clone()),
                },
                Err(failure) if failure.message.starts_with(PREVIEW_SUPERSEDED) => {
                    forget_if_same(state_dir, &preview, lock_wait);
                    return Ok(RevertOutcome::Superseded);
                }
                Err(failure) => {
                    return Err(format!("the preview could not revert: {failure}"));
                }
            }
        }
        None => match unapply(
            state_dir,
            compositor,
            bars,
            lock_wait,
            Some(from_generation),
            dconf,
        )? {
            true => RevertOutcome::Reverted { name: None },
            false => RevertOutcome::Superseded,
        },
    };
    forget_if_same(state_dir, &preview, lock_wait);
    Ok(outcome)
}

/// Whether a revert failure is a busy state-dir lock (a swap running
/// now): worth retrying, unlike a refusal or a corrupt record, which a
/// retry would only repeat. Matches [`scoot_look::state::LOCK_CONTENDED`].
pub fn is_lock_contention(message: &str) -> bool {
    message.contains(scoot_look::state::LOCK_CONTENDED)
}

/// Reverts a timed preview, retrying while a concurrent swap holds the
/// state-dir lock: each attempt re-runs [`revert_if_current`]'s guards, so
/// a newer choice that lands mid-wait stands the revert down instead of
/// being undone. Only lock contention retries (up to `retries` waits of
/// `retry_delay` past each attempt's own `lock_wait`); anything else —
/// a refusal, a corrupt record, a failed revert — returns at once, since a
/// retry could only repeat it (or, after a rolled-back revert, re-apply).
#[allow(clippy::too_many_arguments)]
pub fn revert_with_retry(
    from_generation: u64,
    catalog: &Catalog,
    cache: &Path,
    state_dir: &Path,
    config: Option<&Path>,
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
    checks: &dyn super::apply::Checks,
    dconf: &dyn super::toolkit::Dconf,
    leaves: &dyn super::leaves::Leaves,
    lock_wait: Duration,
    retries: u32,
    retry_delay: Duration,
) -> Result<RevertOutcome, String> {
    let mut waits = 0;
    loop {
        match revert_if_current(
            from_generation,
            catalog,
            cache,
            state_dir,
            config,
            compositor,
            bars,
            checks,
            dconf,
            leaves,
            lock_wait,
        ) {
            Err(message) if is_lock_contention(&message) && waits < retries => {
                waits += 1;
                std::thread::sleep(retry_delay);
            }
            outcome => return outcome,
        }
    }
}

/// Forgets the preview file, but only when it still carries this preview:
/// a newer preview armed during the revert owns its own file.
fn forget_if_same(state_dir: &Path, preview: &Preview, lock_wait: Duration) {
    if state::acquire_lock(state_dir, lock_wait).is_ok()
        && read_preview(state_dir) == PreviewRead::Present(preview.clone())
    {
        clear_preview(state_dir);
    }
}

/// Clears the look layer: the record and the link go away and every running
/// app reloads scoped to the look, landing without a layer. Startup with no
/// record runs the same way, so this is converge, not invention. The
/// preview's owned toolkit keys are restored first (through
/// [`toolkit_off`], best-effort like the reloads: with no record left,
/// the next apply would otherwise adopt the preview's values as the
/// user's own baseline); toolkit files stay (marked ones converge on the
/// next apply, unmarked ones were never touched). Refusals are best-effort
/// here (there is no newer choice to roll toward): they are reported, and
/// the choice stays cleared.
///
/// `expect_generation` guards the clear the way [`ApplyOptions`]'s guard
/// does the reverting swap: when the live generation moved past it, the
/// choice belongs to a newer apply and nothing is cleared (`Ok(false)`).
/// `Ok(true)` cleared it.
pub fn unapply(
    state_dir: &Path,
    compositor: &mut dyn CompositorReload,
    bars: &[&dyn BarReload],
    lock_wait: Duration,
    expect_generation: Option<u64>,
    dconf: &dyn super::toolkit::Dconf,
) -> Result<bool, String> {
    let _lock = state::acquire_lock(state_dir, lock_wait)
        .map_err(|error| format!("cannot lock the state dir: {error}"))?;
    if let Some(want) = expect_generation {
        let live = match state::read_record(state_dir) {
            state::RecordRead::Present(record) => Some(record.generation),
            state::RecordRead::Missing => None,
            state::RecordRead::Corrupt(reason) => {
                return Err(format!(
                    "cannot revert the preview: the recorded choice is corrupt ({reason})"
                ));
            }
        };
        if live != Some(want) {
            return Ok(false);
        }
    }
    // Best-effort, before the record (and its ownership maps) goes away:
    // without this the preview's keys survive with no record, and the
    // next apply adopts them as user values. The restore's notes have no
    // channel on the detached path (stdio is null), so they are dropped;
    // reload refusals below stay errors because they mean apps visibly
    // kept the old layer.
    if let state::RecordRead::Present(mut record) = state::read_record(state_dir) {
        let _ = super::toolkit::toolkit_off(state_dir, &mut record, dconf);
    }
    std::fs::remove_file(state_dir.join(state::RECORD_FILE)).ok();
    std::fs::remove_file(state_dir.join(state::LINK_NAME)).ok();
    let mut refusals = Vec::new();
    match compositor.reload_look() {
        Ok(_) | Err(super::reload::ReloadError::NotRunning(_)) => {}
        Err(super::reload::ReloadError::Refused(message))
        | Err(super::reload::ReloadError::Unanswered(message)) => {
            refusals.push(format!("compositor: {message}"));
        }
    }
    for bar in bars {
        match bar.reload_look() {
            Ok(_) | Err(super::reload::ReloadError::NotRunning(_)) => {}
            Err(super::reload::ReloadError::Refused(message))
            | Err(super::reload::ReloadError::Unanswered(message)) => {
                refusals.push(format!("bar {}: {message}", bar.display()));
            }
        }
    }
    if refusals.is_empty() {
        Ok(true)
    } else {
        Err(format!(
            "the choice cleared but some apps kept the old layer: {}",
            refusals.join("; ")
        ))
    }
}

/// Keeps a timed preview: forgets the pending revert, staying on the
/// current look. Refuses when no preview is pending, or when a later apply
/// already superseded it (the file is dead then, and is cleared).
pub fn keep_preview(state_dir: &Path, lock_wait: Duration) -> Result<String, String> {
    // Like an apply: the state dir must exist before the lock is taken,
    // and an unwritable one refuses loudly rather than misreporting.
    std::fs::create_dir_all(state_dir)
        .map_err(|error| format!("cannot use state dir {}: {error}", state_dir.display()))?;
    let _lock = state::acquire_lock(state_dir, lock_wait)
        .map_err(|error| format!("cannot lock the state dir: {error}"))?;
    let preview = match read_preview(state_dir) {
        PreviewRead::Missing => {
            return Err("no timed preview is pending".to_owned());
        }
        PreviewRead::Corrupt(reason) => {
            clear_preview(state_dir);
            return Err(format!(
                "the pending preview is corrupt ({reason}): dropped"
            ));
        }
        PreviewRead::Present(preview) => preview,
    };
    let record = match state::read_record(state_dir) {
        state::RecordRead::Present(record) => record,
        state::RecordRead::Missing => {
            clear_preview(state_dir);
            return Err("no timed preview is pending".to_owned());
        }
        state::RecordRead::Corrupt(reason) => {
            return Err(format!("the recorded choice is corrupt: {reason}"));
        }
    };
    if record.generation != preview.from_generation {
        clear_preview(state_dir);
        return Err(format!(
            "no timed preview is pending: {} moved past it",
            record.name
        ));
    }
    clear_preview(state_dir);
    Ok(record.name)
}

/// The detached reverter's argv: the same binary re-invoked with the hidden
/// `_preview-revert` verb, the state dir pinned (absolute) and the socket
/// when the previewing apply named one explicitly.
pub fn reverter_argv(
    exe: &Path,
    state_dir: &Path,
    socket: Option<&Path>,
    from_generation: u64,
    after_secs: u64,
) -> (PathBuf, Vec<String>) {
    let mut args = vec![
        "look".to_owned(),
        "--state".to_owned(),
        state_dir.display().to_string(),
    ];
    if let Some(socket) = socket {
        args.push("--socket".to_owned());
        args.push(socket.display().to_string());
    }
    args.push("_preview-revert".to_owned());
    args.push("--from".to_owned());
    args.push(from_generation.to_string());
    args.push("--after".to_owned());
    args.push(after_secs.to_string());
    (exe.to_owned(), args)
}

/// Spawns the detached reverter: its own session (a terminal closing on
/// the previewing CLI sends no HUP to it), stdio to null, no wait. The
/// parent exits immediately; the child sleeps out the delay, then reverts
/// only while the generation still matches.
pub fn spawn_reverter(exe: &Path, args: &[String]) -> Result<(), String> {
    let mut command = std::process::Command::new(exe);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // A new session: surviving the terminal closing is the point of
        // the detached child (the design's "`--for` revert ... survives
        // the terminal closing").
        //
        // SAFETY: `pre_exec` runs after `fork`, before `exec`. The closure
        // does nothing but the async-signal-safe `setsid` syscall (no
        // allocation, no locks), which is exactly what `pre_exec` allows.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()
                    .map(|_| ())
                    .map_err(std::io::Error::from)
            });
        }
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("cannot start the preview reverter: {error}"))
}

#[cfg(test)]
mod tests;
