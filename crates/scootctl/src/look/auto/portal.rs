//! `--follow-portal`: swap when the desktop's light/dark setting flips.
//!
//! The setting is `org.gnome.desktop.interface color-scheme`, the key
//! xdg-desktop-portal-gtk serves as `org.freedesktop.appearance
//! color-scheme`. It lives in the user's dconf database, one binary file,
//! so the path unit fires on every dconf write by anyone. Everything here
//! is therefore built to be a cheap no-op nearly every time it runs.
//!
//! The rule is polarity against polarity, never look name against look
//! name: the setting says dark or light, the active look is dark or light,
//! and the service applies the matching look only when they disagree. That
//! is what ends the loop `look apply` would otherwise start. Applying a
//! look writes `color-scheme` (the toolkit step), which changes the
//! database, which fires the path unit again; the second run finds the
//! active look's polarity equal to the setting's and stops. It does not
//! depend on that write happening or on what it wrote. A look the user
//! chose by hand with the right polarity is left alone, and a manual apply
//! of the opposite polarity writes `color-scheme` itself, so the setting
//! follows the look as it does everywhere else.
//!
//! Under a path unit systemd drops its inotify watches while the service
//! runs (`path_enter_running` calls `path_unwatch`), so a flip during a
//! run is not seen. The run therefore re-reads the setting after every
//! swap, a few times at most.

use std::path::{Path, PathBuf};

use scoot_look::schema::Polarity;

use super::super::toolkit::{COLOR_SCHEME_KEY, Dconf, PREFER_DARK, PREFER_LIGHT};
use super::unit::{Portal, Side};

/// How many times one run re-reads the setting after a swap. A flip
/// during a swap needs one more round; more means something keeps writing
/// the key, and the next trigger takes it from there.
const ROUNDS: usize = 3;

/// The dconf database of the default profile under a config home:
/// `<config>/dconf/user`.
pub fn dconf_db(config_home: &Path) -> PathBuf {
    config_home.join("dconf/user")
}

/// The side a `color-scheme` value asks for. `'default'` is what GNOME's
/// own settings write for Light (the portal reports it as "no preference",
/// which every toolkit renders light), so it reads light. Unset or any
/// other text is no signal at all.
pub fn parse_setting(raw: &str) -> Option<Side> {
    match raw.trim() {
        PREFER_DARK => Some(Side::Dark),
        PREFER_LIGHT | "'default'" => Some(Side::Light),
        _ => None,
    }
}

fn polarity_of_side(side: Side) -> Polarity {
    match side {
        Side::Light => Polarity::Light,
        Side::Dark => Polarity::Dark,
    }
}

fn side_word(side: Side) -> &'static str {
    match side {
        Side::Light => "light",
        Side::Dark => "dark",
    }
}

/// What the run reads and does in the world, behind a trait so a test can
/// run the real toolkit step against a scripted dconf.
pub trait Session {
    /// The polarity of the recorded active look, `None` when there is no
    /// record or the look no longer resolves.
    fn active_polarity(&mut self) -> Option<Polarity>;
    /// The polarity of a named look, `None` when it does not resolve.
    fn polarity_of(&mut self, look: &str) -> Option<Polarity>;
    fn apply(&mut self, look: &str) -> Result<(), String>;
}

/// What one run did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The looks applied, in order.
    pub applied: Vec<String>,
    /// One line for the journal saying why it did or did not act.
    pub note: String,
}

/// One run of the service. Reads the setting first and returns before
/// touching the catalog when there is nothing to follow.
pub fn follow(
    portal: &Portal,
    dconf: &dyn Dconf,
    session: &mut dyn Session,
) -> Result<Outcome, String> {
    if !dconf.available() {
        return Err(
            "no `dconf` on PATH: --follow-portal reads the setting with it (the user manager's PATH \
             must include it)"
                .to_owned(),
        );
    }
    let mut out = Outcome::default();
    for _ in 0..ROUNDS {
        let raw = dconf.read(COLOR_SCHEME_KEY)?;
        let Some(side) = raw.as_deref().and_then(parse_setting) else {
            out.note = match raw {
                None => "color-scheme is not set: nothing to follow".to_owned(),
                Some(raw) => format!("color-scheme is {raw}: not a setting to follow"),
            };
            return Ok(out);
        };
        let want = polarity_of_side(side);
        if session.active_polarity() == Some(want) {
            if out.applied.is_empty() {
                out.note = format!(
                    "color-scheme asks for {}, the active look is {}: nothing to do",
                    side_word(side),
                    want.as_str()
                );
            }
            return Ok(out);
        }
        let target = portal.look_for(side);
        let found = session.polarity_of(target);
        if found != Some(want) {
            // Edited or removed since `auto` checked it. Applying it would
            // write the opposite value and fight the setting.
            out.note = format!(
                "color-scheme asks for {}, but `{target}` is {}: left alone (run `scoot look auto` again)",
                side_word(side),
                found.map_or("not a look that resolves", Polarity::as_str)
            );
            return Ok(out);
        }
        session.apply(target)?;
        out.applied.push(target.to_owned());
        out.note = format!(
            "color-scheme asks for {}: applied {target}",
            side_word(side)
        );
    }
    out.note
        .push_str(" (the setting kept changing: stopped re-reading)");
    Ok(out)
}
