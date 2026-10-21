//! `scoot look wiring`: the one-time non-Nix setup each look needs,
//! checked against the live files.
//!
//! A swap renders every fragment but wires none of them into the user's
//! own files: the launcher flags, the lock merger, the foot include, the
//! mako include, the wob command, the fcitx5 theme link and the
//! Helix/btop/starship pointers each need one manual step, once. This
//! command checks each step for the active look and prints the exact
//! command or line that is still missing, with real paths (never `~`).
//! Read-only: it writes nothing, starts nothing, and signals no daemon.
//!
//! Each item reports one status:
//!
//! - `wired`: the file already points at the look. Nothing to do.
//! - `missing`: the step is undone; the detail names the exact fix.
//! - `manual`: no file records the step (wob, nightlight, fuzzel,
//!   swaylock), so the detail gives the command to run instead.
//! - `absent`: the look carries no leaf for that app (no `[terminal]`
//!   means no foot wiring, a look without an `apps/` leaf stays silent
//!   about it). Nothing to do.
//!
//! Exit 0 means the report printed, even with missing items (missing
//! wiring is the report, not a failure). Exit 1 means there is no active
//! look or the state cannot be read; exit 2 is a usage error.

use std::path::Path;

use super::terminals::presence_in;
use scoot_look::catalog::{config_home, data_home};
use scoot_look::state::{LINK_NAME, Record, RecordRead, read_record};

/// One wiring item: the app, its status, and the detail (what is wired
/// or the exact missing step).
#[derive(Debug, Clone, PartialEq)]
pub struct WiringItem {
    pub app: &'static str,
    pub status: WiringStatus,
    pub detail: String,
}

/// The shape of one wiring item. `wired` and `absent` need no action;
/// `missing` and `manual` name the exact step in [`WiringItem::detail`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WiringStatus {
    Wired,
    Missing,
    Manual,
    Absent,
}

impl WiringStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WiringStatus::Wired => "wired",
            WiringStatus::Missing => "missing",
            WiringStatus::Manual => "manual",
            WiringStatus::Absent => "absent",
        }
    }
}

/// The resolved directories one wiring check runs against. All explicit,
/// so tests pin them without touching the process environment.
pub struct WiringInput<'a> {
    /// The look state dir (holds `look.json` and the `look` link).
    pub state_dir: &'a Path,
    /// The recorded choice the check reports for.
    pub record: &'a Record,
    /// The rendered look the `look` link points at.
    pub rendered: &'a Path,
    /// The user's config home (`None` when no home resolves).
    pub config_home: Option<&'a Path>,
    /// The user's data home (`None` when no home resolves).
    pub data_home: Option<&'a Path>,
    /// The starship config location (`STARSHIP_CONFIG` when set and
    /// non-empty, else `None`): threaded in like every other path, so no
    /// check ever reads the process environment (a concurrent `cargo test`
    /// shares one process; an env read races a sibling test's write).
    pub starship_config: Option<&'a Path>,
}

/// Checks every one-time wiring step for the active look. Read-only.
pub fn check_wiring(input: &WiringInput<'_>) -> Vec<WiringItem> {
    let link_look = input.state_dir.join("look");
    let mut items = Vec::new();
    let presence = presence_in(input.rendered);
    items.push(foot_item(input, &link_look));
    items.push(mako_item(input, &link_look));
    items.push(wob_item(input, &link_look));
    items.push(fcitx5_item(input, &link_look));
    items.push(nightlight_item(&link_look));
    if presence.helix {
        items.push(helix_item(input, &link_look));
    } else {
        items.push(absent("helix", "the look carries no Helix theme"));
    }
    if presence.btop {
        items.push(btop_item(input, &link_look));
    } else {
        items.push(absent("btop", "the look carries no btop theme"));
    }
    if presence.starship {
        items.push(starship_item(input, &link_look));
    } else {
        items.push(absent("starship", "the look carries no starship config"));
    }
    items.push(fuzzel_item(input, &link_look));
    items.push(swaylock_item(input, &link_look));
    items
}

/// What [`resolve_input`] resolved: the recorded choice, the rendered
/// directory the link points at, the config and data homes (either may be
/// missing), and the starship config location (`STARSHIP_CONFIG` when set
/// and non-empty).
pub type ResolvedInput = (
    Record,
    std::path::PathBuf,
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
);
/// Resolves the live environment (state, record, link, config and data
/// homes, the starship config location) into a [`WiringInput`]. Refuses
/// when no look is applied yet or the link points nowhere. The single
/// process-environment read lives here (production only); tests construct
/// [`WiringInput`] directly and never touch the environment.
pub fn resolve_input(state_dir: &Path) -> Result<ResolvedInput, String> {
    let record = match read_record(state_dir) {
        RecordRead::Present(record) => record,
        RecordRead::Missing => {
            return Err(
                "no look applied yet: `scoot look apply NAME` records a choice first".to_owned(),
            );
        }
        RecordRead::Corrupt(reason) => {
            return Err(format!("the recorded choice is corrupt: {reason}"));
        }
    };
    let rendered = std::fs::read_link(state_dir.join(LINK_NAME)).map_err(|_| {
        format!(
            "the look link is missing: `scoot look apply {}` repairs it",
            record.name
        )
    })?;
    let config = {
        let xdg = std::env::var_os("XDG_CONFIG_HOME");
        let home = std::env::var_os("HOME");
        config_home(xdg.as_deref(), home.as_deref())
    };
    let data = {
        let xdg = std::env::var_os("XDG_DATA_HOME");
        let home = std::env::var_os("HOME");
        data_home(xdg.as_deref(), home.as_deref())
    };
    let starship = std::env::var_os("STARSHIP_CONFIG")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from);
    Ok((record, rendered, config, data, starship))
}

fn absent(app: &'static str, detail: &str) -> WiringItem {
    WiringItem {
        app,
        status: WiringStatus::Absent,
        detail: detail.to_owned(),
    }
}

fn foot_item(input: &WiringInput<'_>, link_look: &Path) -> WiringItem {
    let fragment = link_look.join("foot.ini");
    if !input.rendered.join("foot.ini").is_file() {
        return absent("foot", "the look has no [terminal] hues");
    }
    match input.config_home {
        None => WiringItem {
            app: "foot",
            status: WiringStatus::Missing,
            detail: format!(
                "no config home resolves (set XDG_CONFIG_HOME or HOME), then add `include={}` to foot.ini",
                fragment.display()
            ),
        },
        Some(config) => {
            let user = config.join("foot/foot.ini");
            if foot_include_present(&user) {
                WiringItem {
                    app: "foot",
                    status: WiringStatus::Wired,
                    detail: format!("{} includes the look fragment", user.display()),
                }
            } else {
                WiringItem {
                    app: "foot",
                    status: WiringStatus::Missing,
                    detail: format!("add `include={}` to {}", fragment.display(), user.display()),
                }
            }
        }
    }
}

/// Whether the user's `foot.ini` already includes a look fragment: any
/// uncommented `include` line naming `look/foot.ini`. Shared with the
/// apply-time note, which takes the same file.
pub(crate) fn foot_include_present(user: &Path) -> bool {
    include_present(user, "look/foot.ini")
}

fn mako_item(input: &WiringInput<'_>, link_look: &Path) -> WiringItem {
    let fragment = link_look.join("mako");
    if !input.rendered.join("mako").is_file() {
        return absent("mako", "the look names no notification colors");
    }
    match input.config_home {
        None => WiringItem {
            app: "mako",
            status: WiringStatus::Missing,
            detail: format!(
                "no config home resolves (set XDG_CONFIG_HOME or HOME), then add `include={}` to mako/config",
                fragment.display()
            ),
        },
        Some(config) => {
            let user = config.join("mako/config");
            if include_present(&user, "look/mako") {
                WiringItem {
                    app: "mako",
                    status: WiringStatus::Wired,
                    detail: format!("{} includes the look fragment", user.display()),
                }
            } else {
                WiringItem {
                    app: "mako",
                    status: WiringStatus::Missing,
                    detail: format!(
                        "add `include={}` to {}, then `makoctl reload`",
                        fragment.display(),
                        user.display()
                    ),
                }
            }
        }
    }
}

/// Whether the user's config file already includes a look fragment: any
/// uncommented `include` line (bare or `include=`) naming `needle`.
fn include_present(user: &Path, needle: &str) -> bool {
    let text = match scoot_look::state::read_capped_follow(user) {
        Ok(text) => text,
        Err(_) => return false,
    };
    text.lines().any(|line| {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            return false;
        }
        let Some(rest) = line.strip_prefix("include") else {
            return false;
        };
        if !rest.is_empty() && !rest.starts_with(['=', ' ', '\t']) {
            return false;
        }
        rest.contains(needle)
    })
}

fn wob_item(input: &WiringInput<'_>, link_look: &Path) -> WiringItem {
    if !input.rendered.join("wob.ini").is_file() {
        return absent("wob", "the look names no OSD colors");
    }
    WiringItem {
        app: "wob",
        status: WiringStatus::Manual,
        detail: format!(
            "run `wob -c {}` (it holds no state; the unit restarts into the new colors)",
            link_look.join("wob.ini").display()
        ),
    }
}

fn fcitx5_item(input: &WiringInput<'_>, link_look: &Path) -> WiringItem {
    let fragment = link_look.join("fcitx5/theme.conf");
    if !input.rendered.join("fcitx5/theme.conf").is_file() {
        return absent("fcitx5", "the look names no input-method theme");
    }
    match input.data_home {
        None => WiringItem {
            app: "fcitx5",
            status: WiringStatus::Missing,
            detail: format!(
                "no data home resolves (set XDG_DATA_HOME or HOME), then link the theme at {}",
                fragment.display()
            ),
        },
        Some(data) => {
            let theme = data.join("fcitx5/themes/scoot-look/theme.conf");
            if theme.is_file() {
                WiringItem {
                    app: "fcitx5",
                    status: WiringStatus::Wired,
                    detail: format!(
                        "{} exists (keep Theme=scoot-look in classicui.conf)",
                        theme.display()
                    ),
                }
            } else {
                WiringItem {
                    app: "fcitx5",
                    status: WiringStatus::Missing,
                    detail: format!(
                        "link {} at {} and keep Theme=scoot-look in classicui.conf",
                        fragment.display(),
                        theme.display()
                    ),
                }
            }
        }
    }
}

fn nightlight_item(link_look: &Path) -> WiringItem {
    WiringItem {
        app: "nightlight",
        status: WiringStatus::Manual,
        detail: format!(
            "read `{}` (`temperature=` in Kelvin) in your warmer's startup",
            link_look.join("nightlight").display()
        ),
    }
}

fn helix_item(input: &WiringInput<'_>, link_look: &Path) -> WiringItem {
    let fragment = link_look.join("apps/helix.toml");
    match input.config_home {
        None => WiringItem {
            app: "helix",
            status: WiringStatus::Missing,
            detail: format!(
                "no config home resolves (set XDG_CONFIG_HOME or HOME), then point the theme at {}",
                fragment.display()
            ),
        },
        Some(config) => {
            let theme = config.join("helix/themes/scoot-look.toml");
            if theme.is_file() {
                WiringItem {
                    app: "helix",
                    status: WiringStatus::Wired,
                    detail: format!(
                        "{} exists (theme = \"scoot-look\" in config.toml)",
                        theme.display()
                    ),
                }
            } else {
                WiringItem {
                    app: "helix",
                    status: WiringStatus::Missing,
                    detail: format!(
                        "point {} at {} with theme = \"scoot-look\" in config.toml",
                        theme.display(),
                        fragment.display()
                    ),
                }
            }
        }
    }
}

fn btop_item(input: &WiringInput<'_>, link_look: &Path) -> WiringItem {
    let fragment = link_look.join("apps/btop.theme");
    match input.config_home {
        None => WiringItem {
            app: "btop",
            status: WiringStatus::Missing,
            detail: format!(
                "no config home resolves (set XDG_CONFIG_HOME or HOME), then point the theme at {}",
                fragment.display()
            ),
        },
        Some(config) => {
            let theme = config.join("btop/themes/scoot-look.theme");
            if theme.is_file() {
                WiringItem {
                    app: "btop",
                    status: WiringStatus::Wired,
                    detail: format!(
                        "{} exists (color_theme = \"scoot-look\" in btop.conf)",
                        theme.display()
                    ),
                }
            } else {
                WiringItem {
                    app: "btop",
                    status: WiringStatus::Missing,
                    detail: format!(
                        "point {} at {} with color_theme = \"scoot-look\" in btop.conf",
                        theme.display(),
                        fragment.display()
                    ),
                }
            }
        }
    }
}

fn starship_item(input: &WiringInput<'_>, link_look: &Path) -> WiringItem {
    let fragment = link_look.join("apps/starship.toml");
    if let Some(pointed) = input.starship_config {
        return WiringItem {
            app: "starship",
            status: WiringStatus::Wired,
            detail: format!("STARSHIP_CONFIG points at {}", pointed.display()),
        };
    }
    match input.config_home {
        None => WiringItem {
            app: "starship",
            status: WiringStatus::Missing,
            detail: format!(
                "point STARSHIP_CONFIG or ~/.config/starship.toml at {}",
                fragment.display()
            ),
        },
        Some(config) => {
            let pointed = config.join("starship.toml");
            if pointed.is_file() {
                WiringItem {
                    app: "starship",
                    status: WiringStatus::Wired,
                    detail: format!("{} exists", pointed.display()),
                }
            } else {
                WiringItem {
                    app: "starship",
                    status: WiringStatus::Missing,
                    detail: format!(
                        "point STARSHIP_CONFIG or {} at {}",
                        pointed.display(),
                        fragment.display()
                    ),
                }
            }
        }
    }
}

fn fuzzel_item(input: &WiringInput<'_>, link_look: &Path) -> WiringItem {
    if !input.rendered.join("fuzzel.ini").is_file() {
        return absent("fuzzel", "the look names no launcher colors");
    }
    WiringItem {
        app: "fuzzel",
        status: WiringStatus::Manual,
        detail: format!(
            "pass the look's seven colors as flags: `flags=$(awk '/^\\[colors\\]/{{s=1;next}} /^\\[/{{s=0}} s&&/=/ {{k=$0;sub(/=.*/,\"\",k);v=$0;sub(/[^=]*=/,\"\",v); printf \" --\"k\"-color=\"v}}' {0}/fuzzel.ini); fuzzel $flags`",
            link_look.display(),
        ),
    }
}

fn swaylock_item(input: &WiringInput<'_>, link_look: &Path) -> WiringItem {
    if !input.rendered.join("swaylock").is_file() {
        return absent("swaylock", "the look names no locker colors");
    }
    let user = match input.config_home {
        Some(config) => config.join("swaylock/config").display().to_string(),
        None => "~/.config/swaylock/config".to_owned(),
    };
    WiringItem {
        app: "swaylock",
        status: WiringStatus::Manual,
        detail: format!(
            "merge the look's lines first, yours after: `cat {0}/swaylock {user} > \"$XDG_RUNTIME_DIR/scoot-swaylock-config\"; swaylock -C \"$XDG_RUNTIME_DIR/scoot-swaylock-config\"`",
            link_look.display(),
        ),
    }
}

#[cfg(test)]
mod tests;
