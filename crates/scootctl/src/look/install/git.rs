//! `scoot look install GIT-URL`: someone else's repository, read without
//! ever checking it out.
//!
//! A clone is foreign data, so it crosses the same gate a handed-over
//! directory does ([`super::finish_install`]), and it is read in a way
//! that gives a repository nothing to act on:
//!
//! - **No working tree.** `git clone --bare --depth 1` fetches the
//!   default branch's tip into a private temp directory (0700, removed on
//!   every return path), and the look is read straight from git's object
//!   store: `git ls-tree` lists the files, `git cat-file blob` writes each
//!   raw blob. Checkout machinery therefore never runs: no smudge or
//!   `filter=` driver (LFS included), no `.gitattributes` conversion
//!   (`eol`, `ident`, `working-tree-encoding`), no `post-checkout` hook, no
//!   symlink or submodule materialized on disk, and `.git` is simply not
//!   part of the listing.
//! - **The tree listing is the first gate.** A symlink (mode 120000), a
//!   submodule (160000, type `commit`), any mode other than a plain or
//!   executable file, a non-UTF-8 path, a path no look uses, more than
//!   [`super::MAX_INSTALL_FILES`] files, or a blob over its kind cap all
//!   refuse from the listing alone, before a byte is read. The repository
//!   root is the look directory: a `.gitignore` or `.github/` there refuses
//!   by name, exactly as it would in a directory.
//! - **A hostile-git-proof invocation.** Argument vector and environment
//!   are plain data ([`Git::clone_args`], [`Git::env`]) pinned by tests:
//!   the parent environment is cleared down to an allowlist (`PATH`, `HOME`,
//!   locale, proxy and CA variables, `SSH_AUTH_SOCK`), so an inherited
//!   `GIT_CONFIG_*`, `GIT_SSH_COMMAND` or `GIT_EXEC_PATH` cannot undo the
//!   rest; the system and global git config are off (`GIT_CONFIG_NOSYSTEM`,
//!   `GIT_CONFIG_GLOBAL=/dev/null`, so no `insteadOf` rewrite and no
//!   credential helper); the protocol allowlist is `https:ssh`
//!   (`GIT_ALLOW_PROTOCOL` and `protocol.*.allow`); hooks and fsmonitor
//!   are off; `fsckObjects` is on; redirects are not followed; prompts are
//!   off (`GIT_TERMINAL_PROMPT=0`, ssh `BatchMode`). Scoot reads and
//!   stores no credential: ssh uses the user's own agent and keys, and a
//!   private `https://` repository does not install.
//! - **Bounded.** The clone gets [`CLONE_SECS`] and the temp directory is
//!   stopped at [`SCRATCH_CAP_BYTES`], measured every 10 ms while git runs
//!   and once more when it exits, so an oversized clone fails rather than
//!   fills the disk, by at most one interval's writes (git itself is killed; the transport helpers it
//!   started end when their pipes to it close). Each blob read is bounded
//!   too, and git's own words reach the user only escaped and truncated.
//!
//! One gap: git runs in the terminal's foreground process group (it is
//! deliberately not given its own, so the terminal's Ctrl-C reaches it),
//! so Ctrl-C stops it with scoot, but no destructor runs on a signal and the temp
//! directory (at most the cap, mode 0700, `scoot-look-clone-*` in
//! `$TMPDIR`) stays until the user deletes it. Exit paths scoot controls
//! (errors, timeouts, the size cap, success) always remove it.

mod url;
pub use url::{Source, classify};

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use scoot_look::catalog::Catalog;
use scoot_look::grammar::check_relative_path;

use super::{
    InstallReport, LookFile, MAX_INSTALL_BYTES, MAX_INSTALL_FILES, UrlFetch, finish_install,
    gate_file, kind_cap, not_a_look_file, permitted_rel,
};
use crate::look::apply::{self, Checks};
pub use url::GitUrl;

/// The protocols git may use in production: `GIT_ALLOW_PROTOCOL` and
/// `protocol.<name>.allow`, with everything else `never`.
pub const PROD_PROTOCOLS: &[&str] = &["https", "ssh"];

/// Wall-clock bound on the clone.
pub const CLONE_SECS: u64 = 120;
/// Bound on each `ls-tree` and `cat-file` run (local, so brief).
const LOCAL_SECS: u64 = 30;
/// The clone's temp directory may not grow past this while git runs.
/// A depth-1 look repository is a few megabytes; the install gate itself
/// caps a look at 96 MiB, and git keeps the objects packed.
pub const SCRATCH_CAP_BYTES: u64 = 256 * 1024 * 1024;
/// A directory walk gives up (and reports the cap exceeded) past this many
/// entries, so a repository of millions of tiny files cannot stall it.
const SCRATCH_MAX_ENTRIES: usize = 50_000;
/// The `ls-tree` listing is at most 64 permitted files; a listing past
/// this is a repository with far more, which refuses.
const LISTING_MAX_BYTES: u64 = 1024 * 1024;
/// How often a running git is checked for exit, deadline and size.
const POLL: Duration = Duration::from_millis(10);

/// The `git` binary and the protocols it may speak.
pub struct Git {
    bin: PathBuf,
    protocols: &'static [&'static str],
    scratch_cap: u64,
    clone_timeout: Duration,
}

impl Git {
    /// `git` on `PATH`, speaking [`PROD_PROTOCOLS`]. A missing git is a
    /// clear error naming the way around it, never a panic.
    pub fn from_path() -> Result<Self, String> {
        let bin = crate::look::toolkit::find_on_path("git").ok_or_else(|| {
            "cannot install from a git URL: no `git` on PATH (install git, or clone the \
             repository yourself and run `scoot look install DIRECTORY`)"
                .to_owned()
        })?;
        Ok(Self {
            bin,
            protocols: PROD_PROTOCOLS,
            scratch_cap: SCRATCH_CAP_BYTES,
            clone_timeout: Duration::from_secs(CLONE_SECS),
        })
    }

    /// A runner with its own binary, protocols and limits: the same
    /// hardening, a different transport, so tests clone a local repository
    /// without widening the production allowlist.
    #[cfg(test)]
    pub fn for_test(
        bin: PathBuf,
        protocols: &'static [&'static str],
        scratch_cap: u64,
        clone_timeout: Duration,
    ) -> Self {
        Self {
            bin,
            protocols,
            scratch_cap,
            clone_timeout,
        }
    }

    /// The git configuration passed with `-c` on every run.
    fn config_args(&self) -> Vec<OsString> {
        let mut config = vec![
            "protocol.allow=never".to_owned(),
            "core.hooksPath=/dev/null".to_owned(),
            "core.fsmonitor=false".to_owned(),
            "core.askPass=".to_owned(),
            "credential.helper=".to_owned(),
            "transfer.fsckObjects=true".to_owned(),
            "http.followRedirects=false".to_owned(),
            "http.lowSpeedLimit=1000".to_owned(),
            "http.lowSpeedTime=30".to_owned(),
            "submodule.recurse=false".to_owned(),
            "gc.auto=0".to_owned(),
        ];
        config.extend(
            self.protocols
                .iter()
                .map(|name| format!("protocol.{name}.allow=always")),
        );
        let mut args = vec![OsString::from("--no-replace-objects")];
        for item in config {
            args.push("-c".into());
            args.push(item.into());
        }
        args
    }

    /// The configuration plus `--git-dir=<repo>`, ahead of a subcommand
    /// that reads the cloned repository.
    fn repo_args(&self, repo: &Path) -> Vec<OsString> {
        let mut args = self.config_args();
        let mut git_dir = OsString::from("--git-dir=");
        git_dir.push(repo);
        args.push(git_dir);
        args
    }

    /// The full `git clone` argument vector (after the program name).
    pub fn clone_args(&self, url: &str, dest: &Path) -> Vec<OsString> {
        let mut args = self.config_args();
        args.extend(
            [
                "clone",
                "--quiet",
                "--bare",
                "--depth",
                "1",
                "--single-branch",
                "--no-tags",
                "--no-recurse-submodules",
                "--template=",
                "--",
            ]
            .map(OsString::from),
        );
        args.push(url.into());
        args.push(dest.into());
        args
    }

    /// The child's whole environment: the allowlisted parts of `parent`
    /// plus the fixed isolation variables. Nothing else is inherited.
    pub fn env(&self, parent: impl Fn(&str) -> Option<OsString>) -> Vec<(String, OsString)> {
        const PASSED: &[&str] = &[
            "PATH",
            "HOME",
            "USER",
            "LOGNAME",
            "TMPDIR",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "SSH_AUTH_SOCK",
            "http_proxy",
            "HTTP_PROXY",
            "https_proxy",
            "HTTPS_PROXY",
            "all_proxy",
            "ALL_PROXY",
            "no_proxy",
            "NO_PROXY",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NIX_SSL_CERT_FILE",
            "CURL_CA_BUNDLE",
        ];
        let mut env: Vec<(String, OsString)> = PASSED
            .iter()
            .filter_map(|name| parent(name).map(|value| ((*name).to_owned(), value)))
            .collect();
        let fixed = [
            ("GIT_CONFIG_NOSYSTEM", "1".to_owned()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".to_owned()),
            ("GIT_TERMINAL_PROMPT", "0".to_owned()),
            ("GIT_OPTIONAL_LOCKS", "0".to_owned()),
            ("GIT_ALLOW_PROTOCOL", self.protocols.join(":")),
            (
                "GIT_SSH_COMMAND",
                "ssh -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=15 \
                 -o ServerAliveCountMax=2"
                    .to_owned(),
            ),
        ];
        env.extend(
            fixed
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.into())),
        );
        env
    }

    /// Clones `url` and reads the look out of it: the files of the default
    /// branch's tip, every one through the gate. Removes its temp
    /// directory (under `scratch_base`) before returning, on every path.
    fn fetch_files(&self, url: &str, scratch_base: &Path) -> Result<Vec<LookFile>, String> {
        let scratch = Scratch::new(scratch_base)?;
        let repo = scratch.path().join("repo");
        self.run(
            &scratch,
            &self.clone_args(url, &repo),
            None,
            self.clone_timeout,
        )?;
        self.collect(&scratch, &repo)
    }

    /// Lists the tip's tree, gates the listing, then reads each blob.
    fn collect(&self, scratch: &Scratch, repo: &Path) -> Result<Vec<LookFile>, String> {
        let listing_path = scratch.path().join("listing");
        let mut args = self.repo_args(repo);
        args.extend(["ls-tree", "-r", "-z", "-l", "--full-tree", "HEAD"].map(OsString::from));
        self.run(
            scratch,
            &args,
            Some(&listing_path),
            Duration::from_secs(LOCAL_SECS),
        )
        .map_err(|error| format!("{error} (an empty repository has no look to install)"))?;
        let size = std::fs::symlink_metadata(&listing_path)
            .map_err(|error| format!("cannot read the tree listing: {error}"))?
            .len();
        if size > LISTING_MAX_BYTES {
            return Err(format!(
                "refused: more than {MAX_INSTALL_FILES} files: a look is look.toml plus a few \
                 images, fonts and apps/ leaves"
            ));
        }
        let listing = std::fs::read(&listing_path)
            .map_err(|error| format!("cannot read the tree listing: {error}"))?;
        let entries = parse_listing(&listing)?;
        let blob_path = scratch.path().join("blob");
        let mut files = Vec::with_capacity(entries.len());
        let mut total: u64 = 0;
        for entry in entries {
            gate_file(&entry.rel, entry.size, files.len(), total)?;
            let mut args = self.repo_args(repo);
            args.extend(["cat-file", "blob"].map(OsString::from));
            args.push(entry.oid.into());
            self.run(
                scratch,
                &args,
                Some(&blob_path),
                Duration::from_secs(LOCAL_SECS),
            )?;
            // Same capped, no-follow, regular-file read a directory install
            // does: the bytes are re-measured, not trusted from the listing.
            let bytes = apply::read_regular_capped(&blob_path, kind_cap(&entry.rel))?;
            let _ = std::fs::remove_file(&blob_path);
            total += bytes.len() as u64;
            if total > MAX_INSTALL_BYTES {
                return Err(format!(
                    "refused: the look is larger than {MAX_INSTALL_BYTES} bytes in total"
                ));
            }
            files.push(LookFile {
                rel: entry.rel,
                bytes,
            });
        }
        files.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(files)
    }

    fn over_cap(&self) -> String {
        format!(
            "refused: the repository is larger than {} bytes and was stopped",
            self.scratch_cap
        )
    }

    /// Runs git with `args` in the scratch directory: environment cleared
    /// to [`Git::env`], stdin closed, stdout to `stdout_to` (or nowhere),
    /// stderr to a file in the scratch directory (never a pipe, so a chatty
    /// git cannot block on a full one). Killed at `timeout`, or when the
    /// scratch directory passes its size cap.
    fn run(
        &self,
        scratch: &Scratch,
        args: &[OsString],
        stdout_to: Option<&Path>,
        timeout: Duration,
    ) -> Result<(), String> {
        let stderr_path = scratch.path().join("stderr");
        let open = |path: &Path| {
            File::create(path).map_err(|error| format!("cannot write {}: {error}", path.display()))
        };
        let stdout = match stdout_to {
            Some(path) => Stdio::from(open(path)?),
            None => Stdio::null(),
        };
        let mut child = Command::new(&self.bin)
            .env_clear()
            .envs(self.env(|name| std::env::var_os(name)))
            .args(args)
            .current_dir(scratch.path())
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(Stdio::from(open(&stderr_path)?))
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "cannot install from a git URL: no `git` on PATH".to_owned()
                } else {
                    format!("cannot run `{}`: {error}", self.bin.display())
                }
            })?;
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("cannot wait on git: {error}"));
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "git did not finish in {}s and was stopped",
                    timeout.as_secs()
                ));
            }
            if dir_exceeds(scratch.path(), self.scratch_cap) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(self.over_cap());
            }
            std::thread::sleep(POLL);
        };
        // A run that finished inside one poll interval was never measured
        // by the loop: the cap holds for it too.
        if dir_exceeds(scratch.path(), self.scratch_cap) {
            return Err(self.over_cap());
        }
        if status.success() {
            return Ok(());
        }
        let said = stderr_tail(&stderr_path);
        Err(format!(
            "git failed ({status}){}",
            if said.is_empty() {
                String::new()
            } else {
                format!(": {said}")
            }
        ))
    }
}

/// Installs the look at `url`'s default branch into `dest_parent`: clone,
/// read, then exactly the checks and trust semantics of a directory install
/// (`apps/` leaves dropped unless `trust`; nothing lands on failure).
#[allow(clippy::too_many_arguments)]
pub fn install_git_into(
    git: &Git,
    url: &GitUrl,
    scratch_base: &Path,
    dest_parent: &Path,
    catalog: &Catalog,
    checks: &dyn Checks,
    trust: bool,
    fetch: &dyn UrlFetch,
) -> Result<InstallReport, String> {
    let files = git
        .fetch_files(url.as_str(), scratch_base)
        .map_err(|error| format!("{}: {error}", url.display()))?;
    finish_install(files, dest_parent, catalog, checks, trust, fetch)
}

/// One listed file that passed the listing gate.
#[derive(Debug)]
struct Entry {
    rel: String,
    oid: String,
    size: u64,
}

/// Parses `git ls-tree -r -z -l` output, refusing on the first entry no
/// look may contain.
fn parse_listing(listing: &[u8]) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for record in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let record_text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| "git printed an unreadable tree listing".to_owned())?;
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| "refused: a path in the repository is not valid UTF-8".to_owned())?;
        let meta = record_text(&record[..tab]);
        let mut fields = meta.split_whitespace();
        let (Some(mode), Some(kind), Some(oid), Some(size)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err("git printed an unreadable tree listing".to_owned());
        };
        match (mode, kind) {
            ("100644" | "100755", "blob") => {}
            ("120000", _) => {
                return Err(format!(
                    "refused: {path} is a symlink: installed looks carry regular files only"
                ));
            }
            ("160000", _) | (_, "commit") => {
                return Err(format!(
                    "refused: {path} is a submodule: installed looks carry regular files only"
                ));
            }
            _ => {
                return Err(format!(
                    "refused: {path} is not a regular file: installed looks carry regular files only"
                ));
            }
        }
        if let Some(error) = check_relative_path("file", path) {
            return Err(format!("refused: {path}: {error}"));
        }
        if !permitted_rel(path) {
            return Err(not_a_look_file(path));
        }
        let size: u64 = size
            .parse()
            .map_err(|_| "git printed an unreadable tree listing".to_owned())?;
        if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("git printed an unreadable tree listing".to_owned());
        }
        entries.push(Entry {
            rel: path.to_owned(),
            oid: oid.to_owned(),
            size,
        });
        if entries.len() > MAX_INSTALL_FILES {
            return Err(format!(
                "refused: more than {MAX_INSTALL_FILES} files: {path} does not fit"
            ));
        }
    }
    Ok(entries)
}

/// A private scratch directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    /// Creates `scoot-look-clone-<pid>-<nanos>-<n>` under `base` with mode
    /// 0700. `mkdir` fails on anything already there (a planted link
    /// included), and the name is retried, so an existing path is never
    /// reused.
    fn new(base: &Path) -> Result<Self, String> {
        use std::os::unix::fs::DirBuilderExt as _;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        for attempt in 0..16u32 {
            let path = base.join(format!(
                "scoot-look-clone-{}-{nanos}-{attempt}",
                std::process::id()
            ));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!(
                        "cannot make a private directory under {}: {error}",
                        base.display()
                    ));
                }
            }
        }
        Err(format!(
            "cannot make a private directory under {}: every name taken",
            base.display()
        ))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Whether the tree under `root` holds more than `cap` bytes (or more than
/// [`SCRATCH_MAX_ENTRIES`] entries). Never follows a link.
fn dir_exceeds(root: &Path, cap: u64) -> bool {
    let mut total: u64 = 0;
    let mut entries = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            entries += 1;
            if entries > SCRATCH_MAX_ENTRIES {
                return true;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
                if total > cap {
                    return true;
                }
            }
        }
    }
    false
}

/// The last lines git said, for an error: control characters become `?`,
/// each line and the whole are truncated. Remote servers write some of
/// these lines, so none reaches the terminal raw.
fn stderr_tail(path: &Path) -> String {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let start = len.saturating_sub(4096);
    let mut raw = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_err() || file.take(4096).read_to_end(&mut raw).is_err()
    {
        return String::new();
    }
    let text = String::from_utf8_lossy(&raw);
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            line.chars()
                .map(|c| if c.is_control() { '?' } else { c })
                .take(200)
                .collect::<String>()
        })
        .filter(|line| !line.trim().is_empty())
        .collect();
    let keep = lines.len().saturating_sub(3);
    lines[keep..].join("; ")
}

#[cfg(test)]
mod tests;
