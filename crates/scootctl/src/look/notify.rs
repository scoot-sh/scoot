//! Best-effort desktop notifications for look swaps triggered without a
//! terminal (a keyboard shortcut running `scoot look next`).
//!
//! A successful swap stays silent; a degraded one (a font fell back, a look
//! was skipped) or a failure posts one short plain notification through the
//! user's own notification daemon (`notify-send`, which speaks
//! `org.freedesktop.Notifications` to scootnotify or mako, whichever runs).
//! No new daemon: when no service runs (or no `notify-send` is on PATH)
//! this is a silent no-op, and it never fails the swap.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long a notification may take before it stops being best-effort: a
/// hung bus must never wedge a swap that holds no lock by now but still
/// keeps the user's keypress waiting.
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(2);

/// Posts `summary`/`body` through `notify-send`, best-effort. Always
/// returns `()`: a missing binary, a hung daemon, or no bus all stay
/// silent, so calling this can never fail a swap.
pub fn notify(summary: &str, body: &str) {
    let Some(binary) = find_notify_send() else {
        return;
    };
    let mut child = match std::process::Command::new(&binary)
        .args(["--app-name=scoot", summary, body])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return,
    };
    let deadline = Instant::now() + NOTIFY_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => {
                if Instant::now() >= deadline {
                    child.kill().ok();
                    child.wait().ok();
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// `notify-send` on `PATH` (or beside this binary, the way the effectful
/// checks find their helpers). `None` means do not notify.
fn find_notify_send() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
    {
        let sibling = bin.join("notify-send");
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    super::toolkit::find_on_path("notify-send")
}

/// A short plain body for a degraded swap: the skipped looks and notes,
/// capped so a long chain stays one notification.
pub fn degraded_body(skipped: &[String], notes: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for skip in skipped {
        parts.push(format!("skipped: {skip}"));
    }
    for note in notes {
        parts.push(note.clone());
    }
    let mut body = parts.join("; ");
    const MAX: usize = 400;
    if body.len() > MAX {
        body.truncate(MAX);
        body.push('…');
    }
    body
}
