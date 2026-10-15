//! Per-session look state: `<state>/look.json` plus the `look` link.
//!
//! The record (`look.json`) is what a swap writes first; the link (`look`,
//! pointing at one complete rendered look) is derived from it. The
//! compositor repairs the link from the record at startup ([`ensure_link`])
//! on the session's state dir, so a crash between the two renames heals on
//! the next start. Apps never read `look.json`: they read the link, which
//! is always a complete rendered directory or absent.
//!
//! The state-dir rule lives here once, for scoot and scootbar to share: an
//! explicit dir (the future `--state` flag), then `$SCOOT_LOOK_STATE` (a
//! private session's), then `$XDG_STATE_HOME/scoot`, then
//! `~/.local/state/scoot`. Empty counts as unset. A private session's dir is
//! `$XDG_RUNTIME_DIR/scoot-look-<display>/`, wiped and re-seeded at startup
//! from the user's record; only a private session exports
//! `SCOOT_LOOK_STATE`, to its direct children. The private dir is
//! owner-only (0700, its files 0600) and removed on a clean exit; a crash
//! or a kill leaves it behind until the next session with that display name
//! wipes and reseeds it.
//!
//! [`acquire_lock`], [`write_atomic`] and [`write_record`] are the swap path
//! the future `look-apply-cli` tool owns: every writer takes the
//! `<state>/look.lock` [`StateLock`] (a second writer waits a bounded time,
//! then refuses) and writes through [`write_atomic`] (a unique temp file,
//! opened without following symlinks, plus `rename`), so a reader racing a
//! swap sees the old or the new directory, never a half-written one.
//! Nothing calls them yet outside tests. A missing record is "no choice
//! recorded yet", not an error; a corrupt one (bad JSON, a name outside the
//! `[a-z0-9-]{1,32}` grammar, a `rendered` that is not an absolute path
//! without `..`) is a loud warning at the call site,
//! and the link is left alone: startup never fails over a look.
//!
//! Pure `std` plus the `serde_json` and `rustix` already in the tree: no
//! Wayland, no fontconfig, no image decoder, no process spawning, and
//! nothing here needs Nix. A plain binary install manages the same plain
//! files the same way.

use std::ffi::OsStr;
use std::fs::File;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::grammar::{check_display_text, is_name};

/// The private session's marker: set only there, only for direct children.
pub const STATE_ENV: &str = "SCOOT_LOOK_STATE";
/// The record filename inside a state dir.
pub const RECORD_FILE: &str = "look.json";
/// The link name inside a state dir, pointing at the active rendered look.
pub const LINK_NAME: &str = "look";
/// The lock filename inside a state dir, taken by every writer.
pub const LOCK_FILE: &str = "look.lock";
/// The private dir's prefix under `$XDG_RUNTIME_DIR`: `scoot-look-<display>`.
pub const PRIVATE_PREFIX: &str = "scoot-look-";

/// The largest state file this reads: a `look.json` is a few hundred bytes
/// of machine-written JSON, so anything past this is not a record. Opened
/// without blocking and required to be a regular file, so a FIFO planted at
/// the record path can neither hang a reader nor wedge a swap, and a huge
/// file is refused without reading it whole.
pub const MAX_FILE: u64 = 64 * 1024;

/// Resolves the look state dir: an explicit dir (the future `--state` flag),
/// then `$SCOOT_LOOK_STATE`, then `$XDG_STATE_HOME/scoot`, then
/// `~/.local/state/scoot`. Pure over its inputs, so tests never touch the
/// process environment (whose mutation is `unsafe` under a shared test
/// process). `None` only when no home resolves at all.
pub fn resolve_state_dir(
    explicit: Option<&Path>,
    session: Option<&OsStr>,
    xdg: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    if let Some(dir) = explicit {
        return Some(dir.to_owned());
    }
    let non_empty = |v: &&OsStr| !v.is_empty();
    if let Some(dir) = session.filter(non_empty) {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = xdg.filter(non_empty) {
        return Some(PathBuf::from(dir).join("scoot"));
    }
    home.filter(non_empty)
        .map(|dir| PathBuf::from(dir).join(".local/state/scoot"))
}

/// [`resolve_state_dir`] over the live environment, for production call
/// sites (the compositor, the future `scoot look` tool with its `--state`
/// flag as `explicit`).
pub fn state_dir_from_env(explicit: Option<&Path>) -> Option<PathBuf> {
    resolve_state_dir(
        explicit,
        std::env::var_os(STATE_ENV).as_deref(),
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// The private dir's name for a display: `scoot-look-<display>`. The display
/// is a socket name scoot itself bound (`wayland-1` …), but it still goes
/// through the value grammar: a newline, `/`, `=` or control character in
/// it would escape the runtime dir or confuse the seeding, so anything
/// outside `[A-Za-z0-9._-]{1,64}` is refused rather than sanitized (a
/// sanitized name could collide two displays onto one dir).
pub fn private_dir_name(display: &OsStr) -> Result<String, String> {
    let bytes = display.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(format!(
            "a display name is 1 to 64 bytes, not {}",
            bytes.len()
        ));
    }
    let ok = bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'.' || *b == b'_' || *b == b'-');
    if !ok {
        return Err(format!(
            "display name {display:?} is not a socket name; refusing to build a state dir from it"
        ));
    }
    // Checked ASCII-only above, so this cannot fail.
    Ok(format!(
        "{PRIVATE_PREFIX}{}",
        display.to_str().unwrap_or_default()
    ))
}

/// Whether this session runs on a private look state: `--headless` always
/// (its swaps must never touch the user's choice), or any backend given
/// `--private-look` (the nested preview). `--tty` and `--nested` otherwise
/// keep the user state, so their swaps survive a restart.
pub fn wants_private_session(nested: bool, tty: bool, private_look: bool) -> bool {
    private_look || (!nested && !tty)
}

/// The live choice, as `look.json` records it (design section 4.1).
/// `rendered` is the rendered look directory the link points at, as stored
/// by the writer (an absolute path from `apply` and `sync`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The active look's name (`[a-z0-9-]{1,32}`).
    pub name: String,
    /// The rendered look directory the link points at.
    pub rendered: String,
    /// Bumped by every swap; a `--for` preview records the generation it
    /// reverts from, so a preview never undoes a newer choice.
    pub generation: u64,
    /// What recorded the choice (`apply`, `sync`, `seed`).
    pub chosen_by: Option<String>,
    /// The declared default in force when the choice was made (`sync`'s
    /// rule 2 compares against it); `None` for no declared default.
    pub declared_default: Option<String>,
}

#[derive(serde::Deserialize)]
struct RawRecord {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    rendered: Option<String>,
    #[serde(default)]
    generation: Option<u64>,
    #[serde(default, rename = "chosen-by")]
    chosen_by: Option<String>,
    #[serde(default, rename = "declared-default")]
    declared_default: Option<String>,
}

/// How reading the record went.
#[derive(Debug)]
pub enum RecordRead {
    /// No `look.json`: no choice recorded yet, not a problem.
    Missing,
    /// Unreadable or invalid, naming why: the caller warns and leaves the
    /// link alone (startup never fails over a look).
    Corrupt(String),
    /// The recorded choice.
    Present(Record),
}

/// Reads `<dir>/look.json`: missing, corrupt (with the reason), or the
/// record. Never hangs on a FIFO and never reads past [`MAX_FILE`]; a
/// missing key, a wrong type, an unknown schema and a grammar violation are
/// all `Corrupt`, never a panic.
pub fn read_record(dir: &Path) -> RecordRead {
    let path = dir.join(RECORD_FILE);
    let text = match read_capped(&path) {
        Ok(text) => text,
        Err(error) if std::io::ErrorKind::NotFound == error.kind() => {
            // `read_capped` maps a missing file to a `NotFound` io error;
            // anything else (a FIFO, a directory, an oversize file, bad
            // UTF-8) is corruption worth naming.
            return RecordRead::Missing;
        }
        Err(error) => return RecordRead::Corrupt(format!("{RECORD_FILE}: {error}")),
    };
    match serde_json::from_str::<RawRecord>(&text) {
        Ok(raw) => match validate_record(raw) {
            Ok(record) => RecordRead::Present(record),
            Err(reason) => RecordRead::Corrupt(reason),
        },
        Err(error) => RecordRead::Corrupt(format!("{RECORD_FILE}: invalid JSON: {error}")),
    }
}

/// Validates a parsed record against the value grammar: the name is a look
/// name, `rendered` is an absolute path with no `..` component (a relative
/// or parent-climbing path would resolve against whatever process reads the
/// link, not the state dir), generation is present, and the text fields
/// carry no control characters.
fn validate_record(raw: RawRecord) -> Result<Record, String> {
    let Some(name) = raw.name else {
        return Err(format!("{RECORD_FILE}: missing `name`"));
    };
    if !is_name(&name) {
        return Err(format!(
            "{RECORD_FILE}: `name` must match [a-z0-9-]{{1,32}}, not `{name}`"
        ));
    }
    let Some(rendered) = raw.rendered else {
        return Err(format!("{RECORD_FILE}: missing `rendered`"));
    };
    if rendered.is_empty() || rendered.len() > 4096 {
        return Err(format!(
            "{RECORD_FILE}: `rendered` is a path of 1 to 4096 bytes"
        ));
    }
    if rendered.contains('\0') || rendered.chars().any(|c| c.is_control()) {
        return Err(format!(
            "{RECORD_FILE}: `rendered` must not contain control characters"
        ));
    }
    if !rendered.starts_with('/') {
        return Err(format!(
            "{RECORD_FILE}: `rendered` must be an absolute path, not `{rendered}`"
        ));
    }
    if Path::new(&rendered)
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(format!(
            "{RECORD_FILE}: `rendered` must not contain `..`: `{rendered}`"
        ));
    }
    let Some(generation) = raw.generation else {
        return Err(format!("{RECORD_FILE}: missing `generation`"));
    };
    if let Some(chosen_by) = &raw.chosen_by
        && check_display_text("chosen-by", chosen_by, 64).is_some()
    {
        return Err(format!(
            "{RECORD_FILE}: `chosen-by` must be plain text of at most 64 bytes"
        ));
    }
    if let Some(declared) = &raw.declared_default
        && !is_name(declared)
    {
        return Err(format!(
            "{RECORD_FILE}: `declared-default` must match [a-z0-9-]{{1,32}}, not `{declared}`"
        ));
    }
    Ok(Record {
        name,
        rendered,
        generation,
        chosen_by: raw.chosen_by,
        declared_default: raw.declared_default,
    })
}

/// Serializes a record the way [`write_record`] stores it.
pub fn record_json(record: &Record) -> String {
    let mut out = serde_json::Map::new();
    out.insert(
        "name".to_owned(),
        serde_json::Value::String(record.name.clone()),
    );
    out.insert(
        "rendered".to_owned(),
        serde_json::Value::String(record.rendered.clone()),
    );
    out.insert(
        "generation".to_owned(),
        serde_json::Value::Number(record.generation.into()),
    );
    if let Some(chosen_by) = &record.chosen_by {
        out.insert(
            "chosen-by".to_owned(),
            serde_json::Value::String(chosen_by.clone()),
        );
    }
    if let Some(declared) = &record.declared_default {
        out.insert(
            "declared-default".to_owned(),
            serde_json::Value::String(declared.clone()),
        );
    }
    serde_json::Value::Object(out).to_string()
}

/// Stores a record atomically: temp file plus `rename`, so a reader racing
/// the write sees the old or the new record, never a half-written one. The
/// caller holds the [`StateLock`]; the temp file is synced before the
/// rename, and the dir after it.
pub fn write_record(dir: &Path, record: &Record) -> Result<(), String> {
    write_atomic(dir, RECORD_FILE, record_json(record).as_bytes())
}

/// The temp-name counter behind [`write_atomic`] and [`ensure_link`]: the
/// process id plus this make every temp name unique, so two writers never
/// share one and a planted symlink at a guessable name is never opened.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_name(file: &str) -> String {
    let id = TMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!(".{file}.tmp.{}-{id}", std::process::id())
}

/// Writes `bytes` to `<dir>/file` atomically (temp file, sync, `rename`,
/// dir sync). Refuses a file name that could escape the dir. The temp file
/// is unique per call and opened `O_CREAT|O_EXCL|O_NOFOLLOW` with mode 0600:
/// a symlink planted at the temp path refuses rather than redirecting the
/// write elsewhere, and the stored file is owner-only whatever the umask is
/// (0600 has no group/other bits for the umask to mask, and is re-applied
/// after the open anyway).
pub fn write_atomic(dir: &Path, file: &str, bytes: &[u8]) -> Result<(), String> {
    if file.is_empty() || file.contains('/') || file.contains('\0') || file == "." || file == ".." {
        return Err(format!("refusing to write outside the state dir: `{file}`"));
    }
    for _ in 0..10 {
        let tmp = dir.join(tmp_name(file));
        let fd = match rustix::fs::open(
            &tmp,
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOCTTY,
            rustix::fs::Mode::from_bits_truncate(0o600),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::EXIST) => continue,
            Err(errno) => return Err(format!("cannot write {file}: {errno}")),
        };
        let write = (|| {
            rustix::fs::fchmod(&fd, rustix::fs::Mode::from_bits_truncate(0o600))
                .map_err(std::io::Error::from)?;
            let mut handle = File::from(fd);
            use std::io::Write as _;
            handle.write_all(bytes)?;
            handle.sync_all()?;
            drop(handle);
            std::fs::rename(&tmp, dir.join(file))?;
            sync_dir(dir)
        })();
        if let Err(error) = write {
            std::fs::remove_file(&tmp).ok();
            return Err(format!("cannot write {file}: {error}"));
        }
        return Ok(());
    }
    Err(format!(
        "cannot write {file}: no unique temp name after 10 tries"
    ))
}

/// Syncs a directory after a rename, so the rename itself is durable.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    let fd = rustix::fs::open(
        dir,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOCTTY,
        rustix::fs::Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    rustix::fs::fsync(fd).map_err(std::io::Error::from)?;
    Ok(())
}

/// The gated read the record (and the compositor's fragment loader) share:
/// opened `O_NONBLOCK` and required to be a regular file by `fstat`, with a
/// size cap. A symlink is followed and judged by its target; a loop in one
/// is the kernel's `ELOOP`. A missing file surfaces as a `NotFound` io
/// error, so [`read_record`] can tell "no choice yet" from corruption.
pub fn read_capped(path: &Path) -> Result<String, std::io::Error> {
    use std::io::Read as _;
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NOCTTY,
        rustix::fs::Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    let stat = rustix::fs::fstat(&fd).map_err(std::io::Error::from)?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file: refusing to read it",
        ));
    }
    let mut buf = Vec::new();
    File::from(fd).take(MAX_FILE + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_FILE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("larger than {MAX_FILE} bytes: refusing to read it"),
        ));
    }
    String::from_utf8(buf)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

/// What [`ensure_link`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum LinkRepair {
    /// The link names the recorded directory (or there is no record and no
    /// link): nothing to do.
    Consistent,
    /// The link disagreed with the record and now names it.
    Healed(String),
    /// The record is missing or corrupt, so there is nothing to repair
    /// from: the link is left exactly as it is, with the reason to log.
    Skipped(String),
}

/// Repairs `<dir>/look` from `<dir>/look.json` (the link is derived from
/// the record): a missing or disagreeing link is repointed at the recorded
/// directory through a temp symlink plus `rename`. A missing or corrupt
/// record leaves the link alone -- without a valid record there is no valid
/// target, and deleting a working link would drop a live layer. Never fails
/// startup: every I/O failure names the path and reason for the caller to
/// log.
pub fn ensure_link(dir: &Path) -> Result<LinkRepair, String> {
    let record = match read_record(dir) {
        RecordRead::Missing => {
            return Ok(LinkRepair::Skipped(format!(
                "no {RECORD_FILE}: leaving {} alone",
                dir.join(LINK_NAME).display()
            )));
        }
        RecordRead::Corrupt(reason) => {
            return Ok(LinkRepair::Skipped(format!(
                "{reason}: leaving {} alone",
                dir.join(LINK_NAME).display()
            )));
        }
        RecordRead::Present(record) => record,
    };
    let link = dir.join(LINK_NAME);
    match std::fs::read_link(&link) {
        Ok(target) if target.as_os_str().as_bytes() == record.rendered.as_bytes() => {
            Ok(LinkRepair::Consistent)
        }
        _ => {
            let tmp = dir.join(tmp_name(LINK_NAME));
            let pointed = std::os::unix::fs::symlink(&record.rendered, &tmp)
                .and_then(|()| std::fs::rename(&tmp, &link));
            if pointed.is_err() {
                std::fs::remove_file(&tmp).ok();
            }
            pointed.map_err(|error| {
                format!(
                    "cannot point {} at {}: {error}",
                    link.display(),
                    record.rendered
                )
            })?;
            sync_dir(dir).map_err(|error| format!("cannot sync {}: {error}", dir.display()))?;
            Ok(LinkRepair::Healed(format!(
                "repointed {} at {}",
                link.display(),
                record.rendered
            )))
        }
    }
}

/// Makes `dir` a fresh owner-only private state dir: whatever a previous
/// session with the same display name left behind is wiped, so the seed
/// that follows is exactly the new session's state. Refuses a symlink or a
/// non-directory at the path rather than wiping through one, and refuses a
/// dir owned by someone else; creates with an explicit 0700 (never the
/// umask's 0755) and re-applies it, so a reused dir a previous build left
/// world-readable is sealed.
pub fn prepare_private_dir(dir: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!(
                "refusing to seed {}: a symlink, not a directory",
                dir.display()
            ));
        }
        Ok(meta) if !meta.file_type().is_dir() => {
            return Err(format!(
                "refusing to seed {}: not a directory",
                dir.display()
            ));
        }
        Ok(meta) => {
            use std::os::unix::fs::MetadataExt as _;
            if meta.uid() != rustix::process::geteuid().as_raw() {
                return Err(format!(
                    "refusing to seed {}: owned by uid {}, not us",
                    dir.display(),
                    meta.uid()
                ));
            }
            std::fs::remove_dir_all(dir)
                .map_err(|error| format!("cannot clear {}: {error}", dir.display()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!("cannot inspect {}: {error}", dir.display()));
        }
    }
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    if !std::fs::symlink_metadata(dir)
        .map(|meta| meta.file_type().is_dir() && !meta.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!(
            "refusing to seed {}: not a directory after creating it",
            dir.display()
        ));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("cannot seal {}: {error}", dir.display()))?;
    Ok(())
}

/// Removes a private state dir on a clean exit. Only a leaf named
/// `scoot-look-<display>` is ever removed, and never through a symlink: a
/// programming error that hands this another path refuses instead of
/// deleting it.
pub fn remove_private_dir(dir: &Path) -> Result<(), String> {
    let leaf = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    if !leaf.starts_with(PRIVATE_PREFIX) {
        return Err(format!(
            "refusing to remove {}: not a private look dir",
            dir.display()
        ));
    }
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_dir() && !meta.file_type().is_symlink() => {}
        _ => {
            return Err(format!(
                "refusing to remove {}: not a directory",
                dir.display()
            ));
        }
    }
    std::fs::remove_dir_all(dir)
        .map_err(|error| format!("cannot remove {}: {error}", dir.display()))?;
    Ok(())
}

/// Resolves a seeded link target against the user dir: an absolute target
/// copies as is, a relative one (valid where the user's link lives, but
/// dangling one dir over) becomes the absolute path it names. Lexical only:
/// the target may dangle, so nothing is canonicalized through the
/// filesystem.
fn absolute_target(user_dir: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        return target.to_owned();
    }
    let mut resolved = user_dir.to_path_buf();
    for component in target.components() {
        match component {
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved = PathBuf::from("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => resolved.push(part),
        }
    }
    resolved
}

/// Seeds a private state dir from the user's: the dir is wiped and recreated
/// owner-only first ([`prepare_private_dir`]), so a stale record or link
/// from a previous session with the same display name cannot leak in; then
/// `look.json` copies byte for byte and the `look` link's target is
/// replicated (resolved to absolute when relative, so it still names the
/// same directory from the private dir). Never writes the user dir (a
/// private session must not move the user's choice, not even to repair
/// it); every problem on the user side becomes a note and a private state
/// without that piece, never a failed startup.
pub fn seed_private(user_dir: &Path, private_dir: &Path) -> Result<Vec<String>, String> {
    prepare_private_dir(private_dir)?;
    let mut notes = Vec::new();
    let record_path = user_dir.join(RECORD_FILE);
    match read_capped(&record_path) {
        Ok(bytes) => {
            write_atomic(private_dir, RECORD_FILE, bytes.as_bytes()).map_err(|error| {
                format!(
                    "cannot seed {}: {error}",
                    private_dir.join(RECORD_FILE).display()
                )
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            notes.push(format!(
                "no {} in the user state: starting with no look",
                record_path.display()
            ));
        }
        Err(error) => {
            notes.push(format!(
                "cannot read {} ({error}): starting with no recorded choice",
                record_path.display()
            ));
        }
    }
    let user_link = user_dir.join(LINK_NAME);
    match std::fs::read_link(&user_link) {
        Ok(target) => {
            let link = private_dir.join(LINK_NAME);
            let target = absolute_target(user_dir, &target);
            std::os::unix::fs::symlink(&target, &link)
                .map_err(|error| format!("cannot seed {}: {error}", link.display()))?;
            sync_dir(private_dir)
                .map_err(|error| format!("cannot sync {}: {error}", private_dir.display()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            notes.push(format!(
                "no {} in the user state: starting with no look layer",
                user_link.display()
            ));
        }
        Err(error) => {
            notes.push(format!(
                "cannot read {} ({error}): starting with no look layer",
                user_link.display()
            ));
        }
    }
    Ok(notes)
}

/// The `<state>/look.lock` hold: every writer (`apply`, `sync`, the `--for`
/// revert) takes one, so two swaps serialize instead of interleaving. The
/// kernel releases the `flock` with the file: dropping this is unlocking.
#[derive(Debug)]
pub struct StateLock {
    _file: File,
}

/// Takes the exclusive `<state>/look.lock`, waiting up to `wait` for a
/// holder, then refusing. The lock file is created owner-only; a symlink,
/// FIFO or any other non-regular file at its path refuses rather than
/// locking something else. The parent dir must exist (startup and the tool
/// create the state dir before locking it).
pub fn acquire_lock(dir: &Path, wait: Duration) -> Result<StateLock, String> {
    let path = dir.join(LOCK_FILE);
    let fd = rustix::fs::open(
        &path,
        rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOCTTY
            | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::from_bits_truncate(0o600),
    )
    .map_err(|errno| format!("cannot open {}: {errno}", path.display()))?;
    let stat = rustix::fs::fstat(&fd)
        .map_err(|errno| format!("cannot stat {}: {errno}", path.display()))?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::RegularFile {
        return Err(format!(
            "cannot lock {}: not a regular file, refusing to lock it",
            path.display()
        ));
    }
    let file = File::from(fd);
    let deadline = Instant::now() + wait;
    loop {
        match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => return Ok(StateLock { _file: file }),
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(errno) => {
                return Err(format!(
                    "cannot lock {}: another writer holds it ({errno})",
                    path.display()
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests;
