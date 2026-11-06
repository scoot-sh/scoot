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
//!   fills the disk, by at most one interval's writes. Each blob read is
//!   bounded too, and git's own words reach the user only escaped and
//!   truncated.
//! - **No orphans.** Git leads its own process group; a deadline, the size
//!   cap or an error kills the whole group (git, `ssh`, the remote
//!   helper). Because the terminal's Ctrl-C reaches only scoot's group,
//!   a small `sh` watchdog in a third group holds the other end of a pipe
//!   scoot keeps open: when scoot ends by any means, a signal included, the
//!   pipe closes and the watchdog kills the registered git group and
//!   removes the temp directory. Without `sh` there is no watchdog, and a
//!   killed scoot leaves the 0700 `scoot-look-clone-*` directory.
//! - **ssh.** The user's ssh configuration applies, except that scoot
//!   forces `BatchMode`, a connect timeout, keepalives, and no agent, X11
//!   or port forwarding, so a `Host * ForwardAgent yes` cannot hand the
//!   agent to whatever host a pasted URL names.
//! - **Version.** `git --version` is checked once; older than 2.32 (which
//!   added `GIT_CONFIG_GLOBAL`) refuses.

mod scratch;
mod url;
pub use url::{Source, classify};

use std::ffi::OsString;
use std::fs::File;
use std::os::unix::process::CommandExt as _;
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
use scratch::{Scratch, dir_exceeds, stderr_summary};
pub use url::GitUrl;
use url::printable;

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
    poll: Duration,
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
            poll: POLL,
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
            poll: POLL,
        }
    }

    /// A slower poll, so a test can finish a run between two looks.
    #[cfg(test)]
    pub fn with_poll(mut self, poll: Duration) -> Self {
        self.poll = poll;
        self
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
                 -o ServerAliveCountMax=2 -o ClearAllForwardings=yes -o ForwardAgent=no \
                 -o ForwardX11=no"
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
        self.check_version()?;
        let mut scratch = Scratch::new(scratch_base)?;
        let repo = scratch.path().join("repo");
        self.run(
            &mut scratch,
            &self.clone_args(url, &repo),
            None,
            self.clone_timeout,
        )?;
        self.collect(&mut scratch, &repo)
    }

    /// Lists the tip's tree, gates the listing, then reads each blob.
    fn collect(&self, scratch: &mut Scratch, repo: &Path) -> Result<Vec<LookFile>, String> {
        let listing_path = scratch.path().join("listing");
        let mut args = self.repo_args(repo);
        args.extend(["ls-tree", "-r", "-z", "-l", "--full-tree", "HEAD"].map(OsString::from));
        self.run(
            scratch,
            &args,
            Some(&listing_path),
            Duration::from_secs(LOCAL_SECS),
        )
        .map_err(|error| {
            if error.contains("Not a valid object name") {
                format!("{error} (an empty repository has no look to install)")
            } else {
                error
            }
        })?;
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

    /// Refuses a git older than [`MIN_GIT`] (or one that will not say), since
    /// the config isolation below needs `GIT_CONFIG_GLOBAL`.
    fn check_version(&self) -> Result<(), String> {
        let out = Command::new(&self.bin)
            .env_clear()
            .envs(self.env(|name| std::env::var_os(name)))
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "cannot install from a git URL: no `git` on PATH".to_owned()
                } else {
                    format!("cannot run `{}`: {error}", self.bin.display())
                }
            })?;
        let said = String::from_utf8_lossy(&out.stdout);
        match parse_git_version(&said) {
            Some(version) if version >= MIN_GIT => Ok(()),
            Some((major, minor)) => Err(format!(
                "git {major}.{minor} is too old: `scoot look install GIT-URL` needs git {}.{} or \
                 later (or clone the repository yourself and install the directory)",
                MIN_GIT.0, MIN_GIT.1
            )),
            None => Err(format!(
                "cannot tell which git this is (`git --version` said `{}`): needs git {}.{} or later",
                printable(said.trim(), 60),
                MIN_GIT.0,
                MIN_GIT.1
            )),
        }
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
        scratch: &mut Scratch,
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
            // Git leads its own process group, so one signal reaches the
            // ssh or remote helper it starts too; the scratch watchdog
            // stops that group if scoot is killed first.
            .process_group(0)
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "cannot install from a git URL: no `git` on PATH".to_owned()
                } else {
                    format!("cannot run `{}`: {error}", self.bin.display())
                }
            })?;
        scratch.register_group(child.id());
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    stop(&mut child, scratch);
                    return Err(format!("cannot wait on git: {error}"));
                }
            }
            if Instant::now() >= deadline {
                stop(&mut child, scratch);
                return Err(format!(
                    "git did not finish in {}s and was stopped",
                    timeout.as_secs()
                ));
            }
            if dir_exceeds(scratch.path(), self.scratch_cap) {
                stop(&mut child, scratch);
                return Err(self.over_cap());
            }
            std::thread::sleep(self.poll);
        };
        // Git is gone; its group id may be reused, so the watchdog must
        // not signal it. A helper that outlived git (a detached ssh) is
        // killed here with the group.
        kill_group(child.id());
        scratch.register_group(0);
        // A run that finished inside one poll interval was never measured
        // by the loop: the cap holds for it too.
        if dir_exceeds(scratch.path(), self.scratch_cap) {
            return Err(self.over_cap());
        }
        if status.success() {
            return Ok(());
        }
        let said = stderr_summary(&stderr_path);
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

/// Kills the whole process group of `child` (git leads it), then reaps
/// git, and tells the watchdog the group is gone.
fn stop(child: &mut std::process::Child, scratch: &mut Scratch) {
    kill_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
    scratch.register_group(0);
}

/// SIGKILLs process group `group`. Failures are ignored on purpose: the
/// group may already be gone.
fn kill_group(group: u32) {
    if let Some(pgid) = i32::try_from(group)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process_group(pgid, rustix::process::Signal::KILL);
    }
}

/// The oldest git whose `GIT_CONFIG_GLOBAL` isolation scoot relies on.
const MIN_GIT: (u32, u32) = (2, 32);

/// `(major, minor)` from `git --version` output: "git version 2.39.2",
/// "git version 2.39.2 (Apple Git-143)", "git version 2.43.0.windows.1".
fn parse_git_version(output: &str) -> Option<(u32, u32)> {
    let version = output.trim().strip_prefix("git version ")?;
    let mut parts = version.split(|c: char| !c.is_ascii_digit());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
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
        if let Some(bad) = path.chars().find(|c| url::is_unsafe_char(*c)) {
            return Err(format!(
                "refused: a path in the repository holds {}: {}",
                if bad.is_control() {
                    "a control character"
                } else {
                    "a bidi or format character"
                },
                printable(path, 80)
            ));
        }
        let shown = printable(path, 120);
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
                    "refused: {shown} is a symlink: installed looks carry regular files only"
                ));
            }
            ("160000", _) | (_, "commit") => {
                return Err(format!(
                    "refused: {shown} is a submodule: installed looks carry regular files only"
                ));
            }
            _ => {
                return Err(format!(
                    "refused: {shown} is not a regular file: installed looks carry regular files only"
                ));
            }
        }
        if let Some(error) = check_relative_path("file", path) {
            return Err(format!("refused: {shown}: {error}"));
        }
        if !permitted_rel(path) {
            return Err(not_a_look_file(&shown));
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
                "refused: more than {MAX_INSTALL_FILES} files: {shown} does not fit"
            ));
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests;
