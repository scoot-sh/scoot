//! A module's icon from its config keys: `icon` (a glyph), `icon-path`
//! (SVG path data, with `icon-viewbox`) or `icon-image` (a PNG, in a build
//! with the `icon-image` feature). At most one; each refusal names the key.
//! The clock and the `button` modules take the same three keys through
//! [`read`] (the modules that follow, volume, battery, will); volume adds
//! its static icon through [`volume`], battery and brightness add one glyph
//! or one per level through [`battery`] and [`brightness`], and the network,
//! bluetooth and media modules add their static icons plus one glyph per
//! state through [`network`], [`bluetooth`] and [`media`].

#[cfg(any(feature = "clock", feature = "icon-image"))]
use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "battery")]
use crate::modules::battery::BatteryIcon;
#[cfg(feature = "brightness")]
use crate::modules::brightness::BrightnessIcon;
#[cfg(feature = "network")]
use crate::modules::network::WifiIcon;

#[cfg(feature = "clock")]
use super::{ClockFile, Error};
use crate::icon::path::{Vector, ViewBox};
use crate::icon::{Art, Icon};

/// A table's icon keys as the file gave them.
pub(super) struct Keys<'a> {
    pub icon: Option<&'a str>,
    pub icon_path: Option<&'a str>,
    pub icon_viewbox: Option<&'a str>,
    /// The key's presence is all a build without the `icon-image` feature
    /// needs (it refuses it, saying what is missing).
    pub icon_image: Option<&'a str>,
}

/// The clock's icon, if its section gives one.
#[cfg(feature = "clock")]
pub(super) fn clock(path: &Path, file: &ClockFile) -> Result<Option<Icon>, Error> {
    #[cfg(feature = "icon-image")]
    let image = file.icon_image.as_deref();
    #[cfg(not(feature = "icon-image"))]
    let image = file.icon_image.as_ref().map(|_| "");
    read(
        "clock",
        "a clock",
        &Keys {
            icon: file.icon.as_deref(),
            icon_path: file.icon_path.as_deref(),
            icon_viewbox: file.icon_viewbox.as_deref(),
            icon_image: image,
        },
    )
    .map_err(|(key, message)| Error::Named {
        path: path.to_owned(),
        key,
        message,
    })
}

/// The volume or microphone table's static icon, if its section gives
/// one: shown for every level instead of the built-in ones. `prefix` is
/// the table's name (`volume`, `microphone`).
#[cfg(any(feature = "volume", feature = "microphone"))]
pub(super) fn volume(
    prefix: &str,
    table: &super::VolumeFile,
) -> Result<Option<Icon>, (String, String)> {
    #[cfg(feature = "icon-image")]
    let image = table.icon_image.as_deref();
    #[cfg(not(feature = "icon-image"))]
    let image = table.icon_image.as_ref().map(|_| "");
    #[cfg(feature = "microphone")]
    let what = if prefix == crate::modules::microphone::ID {
        "a microphone"
    } else {
        "a volume"
    };
    #[cfg(not(feature = "microphone"))]
    let what = "a volume";
    read(
        prefix,
        what,
        &Keys {
            icon: table.icon.as_deref(),
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: image,
        },
    )
}

/// The network table's icons, if its section gives any: the static icon
/// (shown for every state instead of the per-state ones below) and one
/// glyph per state (`icon-ethernet`, `icon-wifi`, `icon-vpn`,
/// `icon-offline`), each falling back to the static one. `icon-wifi`
/// takes one glyph for every signal level, or four (weakest to
/// strongest), picked by the level. `Err` is the dotted key at fault and
/// why.
#[cfg(feature = "network")]
pub(super) fn network(table: &super::NetworkFile) -> Result<NetworkIcons, (String, String)> {
    #[cfg(feature = "icon-image")]
    let image = table.icon_image.as_deref();
    #[cfg(not(feature = "icon-image"))]
    let image = table.icon_image.as_ref().map(|_| "");
    let icon = read(
        "network",
        "a network",
        &Keys {
            icon: table.icon.as_deref(),
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: image,
        },
    )?;
    // One glyph each, like `icon` itself: a per-state path or picture
    // would need three keys per state, and the bar draws the static one
    // wherever a state has none. `icon-wifi` alone takes four glyphs as
    // well as one (a static path or picture has no levels).
    let glyph = |name: &str, text: Option<&str>| glyph("network", name, text);
    Ok(NetworkIcons {
        icon,
        ethernet: glyph("icon-ethernet", table.icon_ethernet.as_deref())?,
        wifi: wifi_icon(table.icon_wifi.as_ref())?,
        vpn: glyph("icon-vpn", table.icon_vpn.as_deref())?,
        offline: glyph("icon-offline", table.icon_offline.as_deref())?,
    })
}

/// The network table's WiFi icon: one glyph for every signal level, or
/// four (weakest to strongest), picked by the level. `Err` names the key.
#[cfg(feature = "network")]
fn wifi_icon(value: Option<&toml::Value>) -> Result<Option<WifiIcon>, (String, String)> {
    const KEY: &str = "network.icon-wifi";
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        toml::Value::String(text) => parse_glyph(text)
            .map(|c| Some(WifiIcon::One(c)))
            .map_err(|message| (KEY.to_owned(), message)),
        toml::Value::Array(items) => {
            if items.len() != 4 {
                return Err((
                    KEY.to_owned(),
                    format!(
                        "takes 4 glyphs (weakest to strongest, one each), not {}",
                        items.len()
                    ),
                ));
            }
            let mut levels = ['\0'; 4];
            for (n, item) in items.iter().enumerate() {
                let Some(text) = item.as_str() else {
                    return Err((
                        KEY.to_owned(),
                        format!(
                            "takes 4 glyphs (weakest to strongest, one each): entry {} is {}, not a glyph",
                            n + 1,
                            kind(item),
                        ),
                    ));
                };
                levels[n] = parse_glyph(text)
                    .map_err(|message| (KEY.to_owned(), format!("entry {} {message}", n + 1)))?;
            }
            Ok(Some(WifiIcon::Levels(levels)))
        }
        _ => Err((
            KEY.to_owned(),
            format!(
                "takes one glyph, or 4 of them (weakest to strongest): not {}",
                kind(value),
            ),
        )),
    }
}

/// What a TOML value is, for a refusal that names the key.
#[cfg(any(feature = "network", feature = "battery", feature = "brightness"))]
fn kind(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::String(_) => "a string",
        toml::Value::Integer(_) => "a number",
        toml::Value::Float(_) => "a number",
        toml::Value::Boolean(_) => "a boolean",
        toml::Value::Datetime(_) => "a date",
        toml::Value::Array(_) => "an array",
        toml::Value::Table(_) => "a table",
    }
}

/// One per-state glyph: `None` where the key is absent, else exactly one
/// glyph. `Err` names the dotted key (`prefix.name`) and why.
#[cfg(any(
    feature = "network",
    feature = "battery",
    feature = "bluetooth",
    feature = "media"
))]
fn glyph(prefix: &str, name: &str, text: Option<&str>) -> Result<Option<Icon>, (String, String)> {
    match text {
        None => Ok(None),
        Some(text) => parse_glyph(text)
            .map(|c| Some(Icon::Glyph(c)))
            .map_err(|message| (format!("{prefix}.{name}"), message)),
    }
}

/// One glyph, or `N` of them in `what` order: what a level-picked key
/// (`battery.icon`, `brightness.icon`) takes. `Err` names the dotted key
/// (`prefix.name`) and why, as the network's WiFi icon does.
#[cfg(any(feature = "battery", feature = "brightness"))]
fn one_or_levels<const N: usize>(
    prefix: &str,
    name: &str,
    what: &str,
    value: Option<&toml::Value>,
) -> Result<Option<OneOrLevels<N>>, (String, String)> {
    let key = format!("{prefix}.{name}");
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        toml::Value::String(text) => parse_glyph(text)
            .map(|c| Some(OneOrLevels::One(c)))
            .map_err(|message| (key, message)),
        toml::Value::Array(items) => {
            if items.len() != N {
                return Err((
                    key,
                    format!("takes {N} glyphs ({what}, one each), not {}", items.len()),
                ));
            }
            let mut levels = ['\0'; N];
            for (n, item) in items.iter().enumerate() {
                let Some(text) = item.as_str() else {
                    return Err((
                        key,
                        format!(
                            "takes {N} glyphs ({what}, one each): entry {} is {}, not a glyph",
                            n + 1,
                            kind(item),
                        ),
                    ));
                };
                levels[n] = parse_glyph(text)
                    .map_err(|message| (key.clone(), format!("entry {} {message}", n + 1)))?;
            }
            Ok(Some(OneOrLevels::Levels(levels)))
        }
        _ => Err((
            key,
            format!(
                "takes one glyph, or {N} of them ({what}): not {}",
                kind(value)
            ),
        )),
    }
}

/// What [`one_or_levels`] read: one glyph for every level, or one per
/// level.
#[cfg(any(feature = "battery", feature = "brightness"))]
enum OneOrLevels<const N: usize> {
    One(char),
    Levels([char; N]),
}

/// The battery table's icons, if its section gives any: `icon` (one
/// glyph for every level, or five, empty to full, picked by the level),
/// a static path or picture instead of it, and one glyph each for
/// charging and full, each falling back to the level's glyph. `Err` is
/// the dotted key at fault and why.
#[cfg(feature = "battery")]
pub(super) fn battery(table: &super::BatteryFile) -> Result<BatteryIcons, (String, String)> {
    #[cfg(feature = "icon-image")]
    let image = table.icon_image.as_deref();
    #[cfg(not(feature = "icon-image"))]
    let image = table.icon_image.as_ref().map(|_| "");
    // A string `icon` parses with the static keys (a second static key
    // beside it is refused as usual); an array takes the levels, and a
    // static path or picture beside that is two icons.
    let single = table.icon.as_ref().and_then(|value| value.as_str());
    let still = read(
        "battery",
        "a battery",
        &Keys {
            icon: single,
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: image,
        },
    )?;
    let glyphs = one_or_levels::<5>("battery", "icon", "empty to full", table.icon.as_ref())?;
    let icon = match (glyphs, still) {
        (None, None) => None,
        (None, Some(Icon::Art(art))) => Some(BatteryIcon::Art(art)),
        (None, Some(Icon::Glyph(c))) | (Some(OneOrLevels::One(c)), _) => Some(BatteryIcon::One(c)),
        (Some(OneOrLevels::Levels(levels)), None) => Some(BatteryIcon::Levels(levels)),
        (Some(OneOrLevels::Levels(_)), Some(_)) => {
            let clash = if table.icon_path.is_some() {
                "battery.icon-path"
            } else {
                "battery.icon-image"
            };
            return Err((
                clash.to_owned(),
                "a battery shows one icon: it cannot be set with battery.icon".to_owned(),
            ));
        }
    };
    Ok(BatteryIcons {
        icon,
        charging: glyph("battery", "icon-charging", table.icon_charging.as_deref())?,
        full: glyph("battery", "icon-full", table.icon_full.as_deref())?,
    })
}

/// What [`battery`] read: the level icon (one glyph, five, or a static
/// path or picture) and the charging and full glyphs.
#[cfg(feature = "battery")]
pub(super) struct BatteryIcons {
    pub icon: Option<BatteryIcon>,
    pub charging: Option<Icon>,
    pub full: Option<Icon>,
}

/// The brightness table's icons, if its section gives any: `icon` (one
/// glyph for every level, or four, dim to bright, picked by the level),
/// or a static path or picture instead of it. `Err` is the dotted key
/// at fault and why.
#[cfg(feature = "brightness")]
pub(super) fn brightness(
    table: &super::BrightnessFile,
) -> Result<BrightnessIcons, (String, String)> {
    #[cfg(feature = "icon-image")]
    let image = table.icon_image.as_deref();
    #[cfg(not(feature = "icon-image"))]
    let image = table.icon_image.as_ref().map(|_| "");
    // As the battery's: a string `icon` parses with the static keys, an
    // array takes the levels, and a static path or picture beside that
    // is two icons.
    let single = table.icon.as_ref().and_then(|value| value.as_str());
    let still = read(
        "brightness",
        "a brightness",
        &Keys {
            icon: single,
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: image,
        },
    )?;
    let glyphs = one_or_levels::<4>("brightness", "icon", "dim to bright", table.icon.as_ref())?;
    let icon = match (glyphs, still) {
        (None, None) => None,
        (None, Some(Icon::Art(art))) => Some(BrightnessIcon::Art(art)),
        (None, Some(Icon::Glyph(c))) | (Some(OneOrLevels::One(c)), _) => {
            Some(BrightnessIcon::One(c))
        }
        (Some(OneOrLevels::Levels(levels)), None) => Some(BrightnessIcon::Levels(levels)),
        (Some(OneOrLevels::Levels(_)), Some(_)) => {
            let clash = if table.icon_path.is_some() {
                "brightness.icon-path"
            } else {
                "brightness.icon-image"
            };
            return Err((
                clash.to_owned(),
                "a brightness shows one icon: it cannot be set with brightness.icon".to_owned(),
            ));
        }
    };
    Ok(BrightnessIcons { icon })
}

/// What [`brightness`] read: the level icon (one glyph, four, or a
/// static path or picture).
#[cfg(feature = "brightness")]
pub(super) struct BrightnessIcons {
    pub icon: Option<BrightnessIcon>,
}

/// The bluetooth table's icons, if its section gives any: the static
/// icon (shown for every state instead of the per-state ones below) and
/// one glyph per state (`icon-off`, `icon-on`, `icon-connected`), each
/// falling back to the static one. `Err` is the dotted key at fault and
/// why.
#[cfg(feature = "bluetooth")]
pub(super) fn bluetooth(table: &super::BluetoothFile) -> Result<BluetoothIcons, (String, String)> {
    #[cfg(feature = "icon-image")]
    let image = table.icon_image.as_deref();
    #[cfg(not(feature = "icon-image"))]
    let image = table.icon_image.as_ref().map(|_| "");
    let icon = read(
        "bluetooth",
        "a bluetooth",
        &Keys {
            icon: table.icon.as_deref(),
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: image,
        },
    )?;
    // One glyph each, like `icon` itself: a per-state path or picture
    // would need three keys per state, and the bar draws the static one
    // wherever a state has none.
    Ok(BluetoothIcons {
        icon,
        off: glyph("bluetooth", "icon-off", table.icon_off.as_deref())?,
        on: glyph("bluetooth", "icon-on", table.icon_on.as_deref())?,
        connected: glyph(
            "bluetooth",
            "icon-connected",
            table.icon_connected.as_deref(),
        )?,
    })
}

/// What [`bluetooth`] read: the static icon and the per-state glyphs.
#[cfg(feature = "bluetooth")]
pub(super) struct BluetoothIcons {
    pub icon: Option<Icon>,
    pub off: Option<Icon>,
    pub on: Option<Icon>,
    pub connected: Option<Icon>,
}

/// The media table's icons, if its section gives any: the static icon
/// (shown for both states instead of the per-state ones below, and
/// instead of the built-in play and pause vectors) and one glyph per
/// state (`icon-playing`, `icon-paused`), each falling back to the
/// static one. `Err` is the dotted key at fault and why.
#[cfg(feature = "media")]
pub(super) fn media(table: &super::MediaFile) -> Result<MediaIcons, (String, String)> {
    #[cfg(feature = "icon-image")]
    let image = table.icon_image.as_deref();
    #[cfg(not(feature = "icon-image"))]
    let image = table.icon_image.as_ref().map(|_| "");
    let icon = read(
        "media",
        "a media",
        &Keys {
            icon: table.icon.as_deref(),
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: image,
        },
    )?;
    // One glyph each, like `icon` itself; a stopped player shows nothing,
    // so there is no `icon-stopped` (see the module).
    Ok(MediaIcons {
        icon,
        playing: glyph("media", "icon-playing", table.icon_playing.as_deref())?,
        paused: glyph("media", "icon-paused", table.icon_paused.as_deref())?,
    })
}

/// What [`media`] read: the static icon and the per-state glyphs.
#[cfg(feature = "media")]
pub(super) struct MediaIcons {
    pub icon: Option<Icon>,
    pub playing: Option<Icon>,
    pub paused: Option<Icon>,
}

/// The window-title table's icon, if its section gives one: a single
/// static glyph, path or picture, drawn before the title whenever a
/// window is focused.
#[cfg(feature = "window-title")]
pub(super) fn window_title(
    table: &super::WindowTitleFile,
) -> Result<Option<Icon>, (String, String)> {
    #[cfg(feature = "icon-image")]
    let image = table.icon_image.as_deref();
    #[cfg(not(feature = "icon-image"))]
    let image = table.icon_image.as_ref().map(|_| "");
    read(
        "window-title",
        "a window title",
        &Keys {
            icon: table.icon.as_deref(),
            icon_path: table.icon_path.as_deref(),
            icon_viewbox: table.icon_viewbox.as_deref(),
            icon_image: image,
        },
    )
}

/// What [`network`] read: the static icon, the per-state glyphs, and
/// the WiFi icon (one glyph, or one per signal level).
#[cfg(feature = "network")]
pub(super) struct NetworkIcons {
    pub icon: Option<Icon>,
    pub ethernet: Option<Icon>,
    pub wifi: Option<crate::modules::network::WifiIcon>,
    pub vpn: Option<Icon>,
    pub offline: Option<Icon>,
}

/// The icon the keys of table `prefix` (`clock`, `button.launcher`) name,
/// if any. `what` is how a message calls the module (`a clock`). `Err` is
/// the dotted key at fault and why.
pub(super) fn read(
    prefix: &str,
    what: &str,
    keys: &Keys<'_>,
) -> Result<Option<Icon>, (String, String)> {
    let key = |name: &str| format!("{prefix}.{name}");
    let given = [
        ("icon", keys.icon.is_some()),
        ("icon-path", keys.icon_path.is_some()),
        ("icon-image", keys.icon_image.is_some()),
    ];
    let mut set = given
        .iter()
        .filter(|(_, is_set)| *is_set)
        .map(|(name, _)| *name);
    if let (Some(first), Some(second)) = (set.next(), set.next()) {
        return Err((
            key(second),
            format!(
                "{what} shows one icon: it cannot be set with {}",
                key(first)
            ),
        ));
    }
    #[cfg(not(feature = "icon-image"))]
    if keys.icon_image.is_some() {
        return Err((
            key("icon-image"),
            format!(
                "needs a build with the `icon-image` Cargo feature (`cargo build --features \
                 icon-image`, or `icon-image` in programs.scootbar.features); this one has no \
                 PNG decoder. Use {} for an icon that needs none",
                key("icon-path")
            ),
        ));
    }
    if keys.icon_viewbox.is_some() && keys.icon_path.is_none() {
        let with = if keys.icon.is_some() {
            format!("{} is a glyph and has no viewbox", key("icon"))
        } else if keys.icon_image.is_some() {
            format!("{} is a picture and has none", key("icon-image"))
        } else {
            "no icon-path is set".to_owned()
        };
        return Err((
            key("icon-viewbox"),
            format!("is the viewbox of {}, and {with}", key("icon-path")),
        ));
    }
    if let Some(text) = keys.icon {
        return parse_glyph(text)
            .map(|c| Some(Icon::Glyph(c)))
            .map_err(|message| (key("icon"), message));
    }
    if let Some(d) = keys.icon_path {
        let view = match keys.icon_viewbox {
            None => ViewBox::default(),
            Some(text) => ViewBox::parse(text).map_err(|error| {
                (
                    key("icon-viewbox"),
                    format!(
                        "takes four numbers, `min-x min-y width height` (such as \"0 0 24 24\"): {error}"
                    ),
                )
            })?,
        };
        let vector = Vector::parse(d, view).map_err(|error| {
            (
                key("icon-path"),
                format!("not usable SVG path data: {error}"),
            )
        })?;
        return Ok(Some(Icon::Art(Art::Vector(Arc::new(vector)))));
    }
    #[cfg(feature = "icon-image")]
    if let Some(text) = keys.icon_image {
        let file = Path::new(text);
        if !file.is_absolute() {
            return Err((
                key("icon-image"),
                format!(
                    "takes an absolute path to a PNG file, not `{}`",
                    text.escape_debug()
                ),
            ));
        }
        let image = crate::icon::image::load(file).map_err(|error| {
            (
                key("icon-image"),
                format!("cannot use {}: {error}", file.display()),
            )
        })?;
        return Ok(Some(Icon::Art(Art::Image(Arc::new(image)))));
    }
    Ok(None)
}

/// A glyph icon: exactly one Unicode character (a symbol font's glyph is
/// one private-use codepoint). Not shaped, so a sequence of several (an
/// emoji with a modifier) is refused rather than drawn as separate boxes.
fn parse_glyph(text: &str) -> Result<char, String> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() => Ok(c),
        _ => Err(format!(
            "takes exactly one character (a glyph from a symbol font), not `{}`",
            text.escape_debug()
        )),
    }
}
