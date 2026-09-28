//! `scootbg apply-config`: the one command scoot runs
//! (docs/scootbg/backlog/resolved/scoot-integration-done.md). The daemon's half, what a
//! section does once it arrives, is `daemon::config`; this is getting it
//! there.
//!
//! 1. **A daemon answers:** the section goes to it, behind a `version`
//!    request on the same connection, so a daemon from another build is
//!    reported (a different version: a warning; a different protocol, or a
//!    daemon too old to know `apply-config`: an error). The reply comes once
//!    every output shows what it should.
//! 2. **None answers, and the display's lock is free** (no daemon alive,
//!    none starting): with an empty section, the clear and the fingerprint
//!    are written to the profile's state file while the lock is held, and
//!    no daemon is started. Otherwise a daemon is started, detached, with
//!    the section as its starting point ([`serve`]), and step 1 is repeated
//!    until it answers.
//! 3. **None answers, but the lock is held:** a daemon is starting (its
//!    socket is bound just after its lock, and connections wait in its
//!    backlog until it serves), so step 1 is retried.
//!
//! Finding or starting a daemon is bounded by [`START_WAIT`] (tries every
//! [`RETRY`] for the first [`FAST`], then every [`SLOW_RETRY`]), the reply
//! by the client's usual timeout; a daemon is started at most once per
//! run.
//!
//! **Detaching.** The daemon is this binary again: its path while that is
//! still this file, else `/proc/self/exe` itself, so a package upgraded
//! meanwhile never runs a different build ([`start`]); spawned with
//! `apply-config --serve`: stdin and stdout
//! `/dev/null`, stderr this command's (scoot's log, when scoot runs it),
//! working directory `/`. That process calls `setsid(2)` first, so it has a
//! session and process group of its own and no controlling terminal: a
//! signal meant for this command's group (a terminal's hang-up or Ctrl-C, a
//! supervisor or `timeout(1)` stopping its job) does not take the wallpaper
//! with it. `setsid` rather than the classic double fork: `fork` is not
//! reachable from safe Rust, and it is not needed, since this command
//! itself is the short-lived intermediate a double fork makes. It exits
//! without waiting for the daemon, which is then reparented (to init, or a
//! subreaper such as the user's systemd), so no zombie is left with scoot.
//! The daemon still ends when the compositor does: its Wayland connection
//! closes.
//!
//! **Races.** Two runs at once (scoot's, and a second reload's; or an
//! `[autostart]` `scootbg daemon`) are settled by the lock: exactly one
//! daemon claims the socket. A started daemon that loses forwards its
//! section to the winner (as this command does too), so the config's values
//! are never dropped; and if the winner goes away without serving (a `{}`
//! being recorded holds the lock and starts nothing; a daemon that fails
//! before binding), it tries once more to be the daemon itself, so the
//! chain is bounded: no process starts another. Which section arrives last is not decided here: two
//! reloads in quick succession race, and scoot (part B) should not start a
//! second `apply-config` while one runs.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::cli::{ApplyOptions, DaemonOptions};
use crate::control::lock_if_free;
use crate::daemon::{self, Start};
use crate::paths::{self, Paths};
use crate::print::warn;
use crate::protocol::PROTOCOL_VERSION;
use crate::section::Section;
use crate::state::{self, Profile, format, saver};

#[cfg(test)]
mod tests;

/// How long to find a daemon, or start one and see it answer. A daemon
/// answers within a few ms of starting; this bounds a wedge.
pub const START_WAIT: Duration = Duration::from_secs(5);

/// How long to wait for a reply once connected: the client's usual bound
/// (`client`), since the reply waits for an image to decode.
pub const REPLY_WAIT: Duration = Duration::from_secs(30);

/// Between tries while a daemon starts: it binds its socket about a
/// millisecond after it is spawned (measured: 5 ms here cost 3 ms of every
/// cold start), and a failed `connect` costs microseconds.
const RETRY: Duration = Duration::from_millis(1);

/// After [`FAST`] of tries every [`RETRY`], every [`SLOW_RETRY`]: a daemon
/// that has not answered by then is slow (a compositor busy at login), and
/// waiting for it need not cost a wakeup a millisecond.
const FAST: Duration = Duration::from_millis(50);
const SLOW_RETRY: Duration = Duration::from_millis(15);

/// The longest reply read.
const MAX_REPLY: u64 = 64 * 1024;

/// Runs `apply-config`; the exit status (0 or 1: usage errors never get
/// here).
pub fn run(options: ApplyOptions) -> u8 {
    let ApplyOptions {
        profile,
        section,
        serve: serving,
    } = options;
    if serving {
        return serve(profile, section);
    }
    match deliver(&profile, &section, MayStart::Once).map_err(Undelivered::message) {
        Ok(()) => 0,
        Err(message) => {
            warn(format_args!("scootbg: apply-config: {message}"));
            1
        }
    }
}

/// The detached daemon (see the module docs): its own session, then the
/// daemon with `section` as its starting point, reporting as `scootbg
/// daemon` does. Losing the race to another daemon, it forwards the
/// section there instead, reporting as `apply-config` does.
fn serve(profile: Profile, section: Section) -> u8 {
    if let Err(error) = rustix::process::setsid() {
        // Only a process group leader cannot, and `deliver` never makes
        // one; carry on attached rather than not at all.
        warn(format_args!(
            "scootbg: note: cannot start a session of its own ({error}); the daemon \
             stays in its caller's"
        ));
    }
    let start = Start::new(section);
    let options = DaemonOptions {
        profile: profile.clone(),
        restore: true,
    };
    // Twice at most: losing the race, the section goes to the winner; if
    // the winner is gone before it answers (a `{}` being recorded holds
    // the lock and starts nothing; a daemon that failed before binding),
    // the lock is free again and this one tries once more to be the
    // daemon itself, rather than start yet another process.
    for last in [false, true] {
        match daemon::run(options.clone(), Some(&start)) {
            daemon::Exit::Stopped => return 0,
            daemon::Exit::Failed(error) if error.lost_the_race() => {
                match deliver(&profile, &start.section, MayStart::Never) {
                    Ok(()) => return 0,
                    Err(Undelivered::NoDaemon) if !last => {}
                    Err(error) => {
                        warn(format_args!("scootbg: apply-config: {}", error.message()));
                        return 1;
                    }
                }
            }
            daemon::Exit::Failed(error) => {
                warn(format_args!("scootbg: {error}"));
                return 1;
            }
        }
    }
    1
}

/// Whether [`deliver`] may start a daemon when none runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MayStart {
    /// At most once (`apply-config` itself).
    Once,
    /// Never: return [`Undelivered::NoDaemon`] instead (a `--serve` that
    /// lost, which then tries to be the daemon itself).
    Never,
}

/// Why [`deliver`] did not get the section to a daemon.
#[derive(Debug)]
enum Undelivered {
    /// No daemon runs, and the lock is free (only with [`MayStart::Never`]).
    NoDaemon,
    Said(String),
}

impl Undelivered {
    fn message(self) -> String {
        match self {
            Self::NoDaemon => {
                "no daemon answered, and none was starting, so the section was not sent".to_owned()
            }
            Self::Said(message) => message,
        }
    }
}

/// Gets `section` to a daemon for this display, starting one when none
/// runs as `may_start` says; see the module docs.
fn deliver(profile: &Profile, section: &Section, may_start: MayStart) -> Result<(), Undelivered> {
    let said = Undelivered::Said;
    let paths = paths::from_env().map_err(|error| said(error.to_string()))?;
    let began = Instant::now();
    let deadline = began + START_WAIT;
    let mut started: Option<Child> = None;
    loop {
        match UnixStream::connect(&paths.socket) {
            Ok(stream) => {
                return match exchange(stream, profile, section) {
                    Err(Failure::Gone) => Err(said(gone(started.as_mut()))),
                    Err(Failure::Said(message)) => Err(said(message)),
                    Ok(()) => Ok(()),
                };
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) => {}
            Err(error) => return Err(said(format!("{}: {error}", paths.socket.display()))),
        }
        // Nothing answers yet.
        if let Some(child) = started.as_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                if !status.success() {
                    return Err(said(format!(
                        "the daemon it started exited ({status}) before answering; \
                         why is above"
                    )));
                }
                // It lost the race and forwarded the section: a daemon
                // runs, and answers soon.
            }
        } else {
            match lock_if_free(&paths.lock).map_err(|error| said(error.to_string()))? {
                Some(_) if may_start == MayStart::Never => return Err(Undelivered::NoDaemon),
                Some(lock) if section.is_empty() => {
                    let dir = state::dir_from_env();
                    let written = record_clear(dir.as_deref(), profile, section);
                    drop(lock);
                    return written.map_err(said);
                }
                Some(lock) => {
                    // Released first: the daemon takes it. Another may win
                    // it meanwhile; ours then forwards to that one.
                    drop(lock);
                    started = Some(
                        start(profile, section)
                            .map_err(|error| said(format!("cannot start the daemon: {error}")))?,
                    );
                }
                // A daemon is starting: its socket is bound next.
                None => {}
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(said(no_answer(&paths, started.is_some())));
        }
        std::thread::sleep(if now - began < FAST {
            RETRY
        } else {
            SLOW_RETRY
        });
    }
}

/// Starts the detached daemon (see the module docs): this very binary.
/// Its path from `current_exe` while that is still this file (the same
/// device and inode as `/proc/self/exe`), so the daemon's process name is
/// `scootbg` for `ps`, `pgrep` and `pkill`; `/proc/self/exe` itself when the
/// path now names another file or none (a package upgraded or removed
/// meanwhile), which runs this build all the same, under the process name
/// `exe`; the path alone without `/proc`.
fn start(profile: &Profile, section: &Section) -> io::Result<Child> {
    use std::os::unix::fs::MetadataExt;
    const SELF: &str = "/proc/self/exe";
    let named = std::env::current_exe();
    let running = std::fs::metadata(SELF);
    let program: std::path::PathBuf = match (named, running) {
        (Ok(path), Ok(running)) => {
            let same = std::fs::metadata(&path)
                .is_ok_and(|m| m.dev() == running.dev() && m.ino() == running.ino());
            if same { path } else { SELF.into() }
        }
        (Err(_), Ok(_)) => SELF.into(),
        (Ok(path), Err(_)) => path,
        (Err(error), Err(_)) => return Err(error),
    };
    let mut command = Command::new(program);
    command
        .arg("apply-config")
        .arg(crate::cli::SERVE)
        .arg(format!("--profile={}", profile.as_str()))
        .arg(section.canonical())
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
}

/// With the lock held and no daemon: records an empty section in
/// `profile`'s state file in `dir` (every choice cleared, the fingerprint
/// set), unless it is recorded already.
fn record_clear(dir: Option<&Path>, profile: &Profile, section: &Section) -> Result<(), String> {
    let Some(dir) = dir else {
        // Nowhere to keep state: a daemon started later restores nothing,
        // which is what an empty section asks for.
        return Ok(());
    };
    let file = dir.join(profile.as_str());
    let fingerprint = section.fingerprint();
    match state::load(&file) {
        Err(error) => {
            return Err(format!(
                "cannot read the state file {}: {error}; the empty section was not recorded \
                 there (fix or remove it)",
                file.display()
            ));
        }
        Ok(Some(parsed)) if parsed.newer => {
            return Err(format!(
                "the state file {} is a newer scootbg's; the empty section was not recorded \
                 there, so as not to write over it",
                file.display()
            ));
        }
        Ok(Some(parsed)) if parsed.record.fingerprint.as_deref() == Some(&fingerprint) => {
            return Ok(());
        }
        Ok(_) => {}
    }
    let mut text = String::with_capacity(128);
    format::encode(
        &mut text,
        profile.as_str(),
        Some(&fingerprint),
        Some(&None),
        &[],
    );
    saver::write_atomic(&file, text.as_bytes())
        .map_err(|error| format!("cannot write the state file {}: {error}", file.display()))
}

/// Why an exchange ended without an answer to act on.
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    /// The connection closed before both replies came.
    Gone,
    /// What to report.
    Said(String),
}

/// `version`, then the section, on one connection; the replies in turn.
fn exchange(mut stream: UnixStream, profile: &Profile, section: &Section) -> Result<(), Failure> {
    let io_failure = |error: io::Error| match error.kind() {
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::UnexpectedEof => Failure::Gone,
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Failure::Said(format!(
            "the daemon did not answer within {} s (the change may still happen)",
            REPLY_WAIT.as_secs()
        )),
        _ => Failure::Said(format!("talking to the daemon: {error}")),
    };
    // One bound for the whole exchange, both replies included.
    let deadline = Instant::now() + REPLY_WAIT;
    let left = || {
        // Never zero, which would mean no timeout at all.
        Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        )
    };
    stream.set_read_timeout(left()).map_err(io_failure)?;
    stream.set_write_timeout(left()).map_err(io_failure)?;
    let mut request = String::from("{\"protocol\":");
    request.push_str(&PROTOCOL_VERSION.to_string());
    request.push_str(",\"type\":\"version\"}\n");
    request.push_str(&section.request_line(profile));
    stream.write_all(request.as_bytes()).map_err(io_failure)?;
    let mut reader = BufReader::new((&stream).take(MAX_REPLY));
    let mut line = String::new();
    let theirs = match read_reply(&mut reader, &mut line).map_err(io_failure)? {
        Reply::Version(version) => version,
        Reply::Error(message) => {
            return Err(Failure::Said(format!(
                "the running daemon refused this scootbg's protocol {PROTOCOL_VERSION}: \
                 {message}; it is from another scootbg build: `scootbg kill` it, then \
                 run apply-config again (scoot does on reload)"
            )));
        }
        Reply::Ok | Reply::Other(_) => return Err(Failure::Said(unreadable(&line))),
        Reply::Closed => return Err(Failure::Gone),
    };
    let ours = env!("CARGO_PKG_VERSION");
    if theirs != ours {
        warn(format_args!(
            "scootbg: apply-config: warning: the running daemon is scootbg {theirs} and this \
             is {ours}; both speak protocol {PROTOCOL_VERSION}, so the section was sent, but \
             it is applied by that build: `scootbg kill` it and run apply-config again to \
             use this one"
        ));
    }
    reader
        .get_ref()
        .get_ref()
        .set_read_timeout(left())
        .map_err(io_failure)?;
    match read_reply(&mut reader, &mut line).map_err(io_failure)? {
        Reply::Ok => Ok(()),
        Reply::Error(message) if message.starts_with("unknown request") => {
            Err(Failure::Said(format!(
                "the running daemon, scootbg {theirs}, predates apply-config ({message}); \
                 `scootbg kill` it, then run apply-config again (scoot does on reload)"
            )))
        }
        Reply::Error(message) => Err(Failure::Said(format!("daemon: {message}"))),
        Reply::Version(_) | Reply::Other(_) => Err(Failure::Said(unreadable(&line))),
        Reply::Closed => Err(Failure::Gone),
    }
}

/// A reply line, as far as `apply-config` cares.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Ok,
    Version(String),
    Error(String),
    Other(String),
    /// End of stream.
    Closed,
}

fn read_reply(reader: &mut impl BufRead, line: &mut String) -> io::Result<Reply> {
    line.clear();
    if reader.read_line(line)? == 0 {
        return Ok(Reply::Closed);
    }
    Ok(classify(line))
}

/// What a reply line says.
fn classify(line: &str) -> Reply {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Reply::Other(line.to_owned());
    };
    match value["type"].as_str() {
        Some("ok") => Reply::Ok,
        Some("version") => match value["version"].as_str() {
            Some(version) => Reply::Version(version.to_owned()),
            None => Reply::Other(line.to_owned()),
        },
        Some("error") => Reply::Error(
            value["message"]
                .as_str()
                .unwrap_or("(no message)")
                .to_owned(),
        ),
        _ => Reply::Other(line.to_owned()),
    }
}

fn unreadable(line: &str) -> String {
    format!("unreadable reply from the daemon: {:?}", line.trim_end())
}

/// The message for a connection closed before its answer.
fn gone(started: Option<&mut Child>) -> String {
    if let Some(child) = started {
        // Give a daemon that failed to connect to the compositor a moment
        // to finish exiting, so its status can be named.
        let until = Instant::now() + Duration::from_millis(200);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if !status.success() => {
                    return format!(
                        "the daemon it started exited ({status}) before answering; why is above"
                    );
                }
                Ok(Some(_)) | Err(_) => break,
                Ok(None) if Instant::now() >= until => break,
                Ok(None) => std::thread::sleep(RETRY),
            }
        }
    }
    "the daemon closed the connection before answering (stopped by `scootbg kill`, or it \
     lost the compositor); the section may not be applied: run apply-config again"
        .to_owned()
}

/// The message when no daemon answered in time.
fn no_answer(paths: &Paths, started: bool) -> String {
    let what = if started {
        "the daemon it started did not answer"
    } else {
        "a daemon holds the lock but did not answer"
    };
    format!(
        "{what} on {} within {} s",
        Path::display(&paths.socket),
        START_WAIT.as_secs()
    )
}
