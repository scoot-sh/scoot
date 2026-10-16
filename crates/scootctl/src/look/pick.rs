//! `scoot look pick`: the keyboard picker until scootlaunch lands.
//!
//! The catalog names go to fuzzel's dmenu on stdin, one per line; the
//! choice comes back on stdout and applies through the same transaction a
//! named apply uses. A cancel (no choice) applies nothing and succeeds, so
//! a keybinding can offer a pick without branching. A missing or broken
//! fuzzel fails loudly, never silently: there is no second implementation
//! and no Nix anywhere on this path, only `scoot` and `fuzzel` on PATH.

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The menu on PATH, and its dmenu contract: lines on stdin, the pick on
/// stdout.
const FUZZEL: &str = "fuzzel";
const FUZZEL_ARGS: &[&str] = &["--dmenu", "--prompt=Look: "];

/// Bytes of menu output kept: one chosen name. Fuzzel echoes the pick;
/// anything past this is not a look name (names are short validated
/// tokens) and is dropped before it can grow.
const MENU_OUTPUT_MAX: u64 = 64 * 1024;

/// The Wayland socket `pick` needs fuzzel to dial: an absolute
/// `$WAYLAND_DISPLAY` is the socket itself, anything else hangs off
/// `$XDG_RUNTIME_DIR`. `None` when the display is unset or empty, or a
/// relative display has no runtime dir to hang off.
pub fn wayland_socket() -> Option<PathBuf> {
    let display = std::env::var_os("WAYLAND_DISPLAY")?;
    if display.is_empty() {
        return None;
    }
    let display = Path::new(&display);
    if display.is_absolute() {
        return Some(display.to_owned());
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    if runtime.is_empty() {
        return None;
    }
    Some(Path::new(&runtime).join(display))
}

/// Whether `path` takes a Wayland client right now: a connect succeeds.
/// A stale `$WAYLAND_DISPLAY` (an ssh session, a restarted compositor)
/// leaves no socket, or a plain file where the socket was; either way
/// fuzzel's empty failure would look exactly like a cancel, so the caller
/// refuses before spawning it. The probe's own connection comes and goes
/// before fuzzel starts: the compositor sees one extra client that never
/// sends anything.
pub fn socket_live(path: &Path) -> bool {
    UnixStream::connect(path).is_ok()
}

/// Runs the menu over `names`: the chosen name, or `None` on a cancel.
/// Errors name fuzzel, never the catalog: a missing fuzzel, one that
/// cannot start, or one that fails while choosing.
pub fn menu_choice(names: &[String]) -> Result<Option<String>, String> {
    let mut child = std::process::Command::new(FUZZEL)
        .args(FUZZEL_ARGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Inherited, not nulled: a failing menu's own error (no display,
        // no working renderer) is the diagnosis, alongside ours.
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("needs `{FUZZEL}` on PATH to pick a look: {error}"))?;
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.wait();
        return Err("the look menu gave no stdin pipe".to_owned());
    };
    let mut failed: Option<std::io::Error> = None;
    for name in names {
        if let Err(error) = stdin
            .write_all(name.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
        {
            failed = Some(error);
            break;
        }
    }
    // EOF first, so a menu that stopped reading still exits.
    drop(stdin);
    if let Some(error) = failed {
        let _ = child.wait();
        return Err(format!("the look menu failed: {error}"));
    }
    let Some(stdout) = child.stdout.take() else {
        let _ = child.wait();
        return Err("the look menu gave no stdout pipe".to_owned());
    };
    // Bounded: only the first MENU_OUTPUT_MAX bytes are ever read. Past
    // it the menu is broken or chatty; either way no choice is applied.
    let mut text = String::new();
    if let Err(error) = stdout.take(MENU_OUTPUT_MAX).read_to_string(&mut text) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "the look menu returned text that is not UTF-8: {error}"
        ));
    }
    if text.len() >= MENU_OUTPUT_MAX as usize && child.try_wait().unwrap_or(None).is_none() {
        // Still running with a full cap: blocked writing past it. Stop it
        // rather than wait out output no choice can use.
        let _ = child.kill();
        let _ = child.wait();
        return Err("the look menu returned more than one choice".to_owned());
    }
    let status = child
        .wait()
        .map_err(|error| format!("the look menu failed: {error}"))?;
    menu_outcome(status.success(), &text)
}

/// Shapes the menu's exit and output into a choice: the first line on
/// success; no output is a cancel, whatever the status. Output with a
/// failure is an error, never a pick: a broken menu must be loud, not a
/// truncated choice applied.
fn menu_outcome(success: bool, output: &str) -> Result<Option<String>, String> {
    let choice = output.lines().next().unwrap_or("");
    if choice.is_empty() {
        return Ok(None);
    }
    if !success {
        return Err("the look menu failed while choosing".to_owned());
    }
    Ok(Some(choice.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{menu_outcome, socket_live};

    /// A scratch dir, removed on drop.
    fn scratch(tag: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("scootctl-pick-{tag}-{}-", std::process::id()))
            .tempdir()
            .expect("a scratch dir")
    }

    #[test]
    fn a_choice_is_the_first_line() {
        assert_eq!(
            menu_outcome(true, "look-b\n"),
            Ok(Some("look-b".to_owned()))
        );
    }

    #[test]
    fn no_output_is_a_cancel_whatever_the_status() {
        assert_eq!(menu_outcome(true, ""), Ok(None));
        assert_eq!(menu_outcome(false, ""), Ok(None));
    }

    #[test]
    fn output_with_a_failure_is_loud_never_a_pick() {
        assert!(menu_outcome(false, "look-b\n").is_err());
    }

    #[test]
    fn a_missing_socket_is_not_live() {
        let scratch = scratch("missing");
        assert!(!socket_live(&scratch.path().join("wayland-dead")));
    }

    #[test]
    fn a_plain_file_is_not_a_live_socket() {
        // A stale display can leave a regular file where the socket was;
        // connecting refuses instead of dialing it.
        let scratch = scratch("file");
        let path = scratch.path().join("wayland-file");
        std::fs::write(&path, "not a socket").expect("a plain file");
        assert!(!socket_live(&path));
    }

    #[test]
    fn a_bound_socket_is_live() {
        use std::os::unix::net::UnixListener;
        let scratch = scratch("bound");
        let path = scratch.path().join("wayland-live");
        let held = UnixListener::bind(&path).expect("a bound socket");
        assert!(socket_live(&path));
        drop(held);
    }
}
