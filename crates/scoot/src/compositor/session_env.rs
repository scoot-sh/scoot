//! The session-identity environment this compositor exports to its children.
//!
//! A Wayland session owes the programs inside it answers no protocol carries:
//! `xdg-desktop-portal` picks its backend by matching `XDG_CURRENT_DESKTOP`
//! against its backends' `portals.conf` entries, and without it a browser has
//! no screen sharing on Wayland and degraded file choosers (see
//! `docs/backlog/resolved/session-environment-and-portals-done.md` for the
//! full record). The cursor pair (`XCURSOR_THEME`/`XCURSOR_SIZE`, set beside
//! these in [`crate::compositor::run`]) is the precedent: a client that loads
//! a theme itself must pick the same one scoot drew.
//!
//! Three variables, two ownership rules:
//!
//! - `XDG_CURRENT_DESKTOP=scoot`, set **unconditionally**. Who reads it is a
//!   scoot child talking to scoot's compositor, so it must name the
//!   compositor the child actually talks to. Under `--nested` (or in a
//!   container) an inherited host value would key portal lookup and backend
//!   behaviour to the wrong compositor -- the host's `scoot-portals.conf`
//!   would never be consulted, and a host-keyed backend could address the
//!   host session instead of this one. niri sets its own name the same way.
//! - `XDG_SESSION_TYPE=wayland` and `XDG_SESSION_DESKTOP=scoot`, set **only
//!   when unset or empty**. Both are logind-owned on a seat (`pam_systemd`
//!   sets the first and reads the second as its `desktop=` input), and a
//!   compositor overwriting logind's values is at best redundant. Where no
//!   logind exists -- `--nested`, a container -- nothing sets them, which is
//!   exactly the vacuum these fill (niri sets the first unconditionally "for
//!   xdg-autostart and Qt apps"; scoot fills rather than overwrites, so a
//!   logind seat keeps its owner's values). An empty string counts as unset:
//!   no reader treats it as meaningful, and Qt checks emptiness.
//!
//! Both application sites -- [`crate::compositor::run`]'s `set_var` block
//! (the process environment, inherited by everything spawned later) and
//! [`crate::compositor::State::spawn`] (each child, explicitly) -- go through
//! [`resolve`], so there is one decision site. Resolving per child rather
//! than relying on inheritance is deliberate: it documents the contract at
//! the spawn site the way `WAYLAND_DISPLAY` already is, and it keeps a child
//! correct even if the process environment was never settled (as in tests).
//!
//! What this module does *not* do, and why:
//!
//! - No D-Bus activation-environment propagation. The portal is D-Bus
//!   activated and inherits the bus's environment, not scoot's children's,
//!   so a session script runs `dbus-update-activation-environment --systemd
//!   WAYLAND_DISPLAY XDG_CURRENT_DESKTOP ...` (see `docs/configuration.md`).
//!   That is the script's job -- "config is state, the script is behavior" --
//!   not the compositor's: scoot has no `--session` concept and ships no
//!   session script, and spawning bus tools from the compositor buys a hang
//!   (a wedged bus stalls startup with no timeout), a zombie (the reaper
//!   tracks only `spawn` pids), and per-system `--systemd`-or-not logic, all
//!   for a line the session author writes once in the right context.
//! - No unsetting on session end. The process environment dies with the
//!   process, and scoot never writes the activation environment, so there is
//!   nothing to retract (niri unsets from the systemd manager precisely
//!   because it wrote there). A surviving child keeps a stale
//!   `WAYLAND_DISPLAY` pointing at a dead socket -- pre-existing, and the
//!   same for these variables.

/// The desktop name scoot exports and its `scoot-portals.conf` is keyed on.
pub const DESKTOP_NAME: &str = "scoot";

/// The value [`SESSION_TYPE`] takes when nothing already set it.
pub const SESSION_TYPE_VALUE: &str = "wayland";

/// `xdg-desktop-portal` matches this against its backends' `portals.conf`.
pub const CURRENT_DESKTOP: &str = "XDG_CURRENT_DESKTOP";

/// Owned by logind on a seat; filled here only where no logind exists.
pub const SESSION_TYPE: &str = "XDG_SESSION_TYPE";

/// Owned by logind on a seat (its `desktop=` input); same fill rule.
pub const SESSION_DESKTOP: &str = "XDG_SESSION_DESKTOP";

/// The session-identity environment settled for one process or child.
///
/// Borrows the already-set values where they are kept, so no allocation;
/// defaults are `'static` and coerce to any lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionEnv<'a> {
    /// Always [`DESKTOP_NAME`]: the child talks to this compositor.
    pub current_desktop: &'a str,
    /// The inherited value where meaningful, else [`SESSION_TYPE_VALUE`].
    pub session_type: &'a str,
    /// The inherited value where meaningful, else [`DESKTOP_NAME`].
    pub session_desktop: &'a str,
}

/// Settles the session environment against the values this process started
/// with (`None` where unset). Pure, so the ownership rules above are
/// unit-testable without touching the process-global environment -- which
/// every other test in this binary shares under `cargo test`.
pub fn resolve<'a>(
    current_desktop: Option<&'a str>,
    session_type: Option<&'a str>,
    session_desktop: Option<&'a str>,
) -> SessionEnv<'a> {
    // `current_desktop` is deliberately unread: the unconditional rule means
    // no inherited value survives, whatever it was. Named (not `_`) so a
    // future reader sees the omission is the decision, not an oversight.
    let _ = current_desktop;
    SessionEnv {
        current_desktop: DESKTOP_NAME,
        session_type: meaningful(session_type).unwrap_or(SESSION_TYPE_VALUE),
        session_desktop: meaningful(session_desktop).unwrap_or(DESKTOP_NAME),
    }
}

/// A value worth keeping: present and not empty.
fn meaningful(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_desktop_is_always_scoot() {
        for inherited in [None, Some(""), Some("scoot"), Some("gnome"), Some("sway")] {
            assert_eq!(
                resolve(inherited, None, None).current_desktop,
                DESKTOP_NAME,
                "an inherited XDG_CURRENT_DESKTOP of {inherited:?} survived"
            );
        }
    }

    #[test]
    fn session_type_is_kept_where_set_and_filled_where_not() {
        // logind's (or the host session's) value is never overwritten ...
        for kept in ["wayland", "x11", "tty"] {
            assert_eq!(
                resolve(None, Some(kept), None).session_type,
                kept,
                "an inherited XDG_SESSION_TYPE of {kept:?} was clobbered"
            );
        }
        // ... while the no-logind vacuum (--nested, container) gets one.
        for missing in [None, Some("")] {
            assert_eq!(
                resolve(None, missing, None).session_type,
                SESSION_TYPE_VALUE,
                "an XDG_SESSION_TYPE of {missing:?} was left empty"
            );
        }
    }

    #[test]
    fn session_desktop_is_kept_where_set_and_filled_where_not() {
        for kept in ["scoot", "gnome", "sway"] {
            assert_eq!(
                resolve(None, None, Some(kept)).session_desktop,
                kept,
                "an inherited XDG_SESSION_DESKTOP of {kept:?} was clobbered"
            );
        }
        for missing in [None, Some("")] {
            assert_eq!(
                resolve(None, None, missing).session_desktop,
                DESKTOP_NAME,
                "an XDG_SESSION_DESKTOP of {missing:?} was left empty"
            );
        }
    }

    #[test]
    fn kept_values_are_borrowed_not_copied() {
        // The no-allocation claim the hot-path bar cares about, pinned: a
        // kept value must be the very string handed in, not a copy of it.
        let session_type = String::from("x11");
        let session_desktop = String::from("sway");
        let resolved = resolve(None, Some(&session_type), Some(&session_desktop));
        assert!(
            std::ptr::eq(resolved.session_type, session_type.as_str()),
            "a kept XDG_SESSION_TYPE was copied"
        );
        assert!(
            std::ptr::eq(resolved.session_desktop, session_desktop.as_str()),
            "a kept XDG_SESSION_DESKTOP was copied"
        );
    }
}
