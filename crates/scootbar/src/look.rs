//! The look layer: `<state>/look/bar.toml` merged under the user's file.
//!
//! Precedence is flags, then the user's own file, then the look layer, then
//! the built-in defaults. The merge happens on raw TOML keys, before any
//! derivation: every key merges per key, except one derived pair (the
//! user's `hover` if set; else, if the user set `accent`, the look's
//! `hover` is dropped so the bar derives hover from the user's accent;
//! else the look's `hover`). The flags rung (`--background`,
//! `--foreground`) stays above both: it overlays after the merge.
//!
//! The layer holds only what `scoot-look`'s scootbar allowlist holds,
//! plus the renderer's own `bar.font` and `bar.fallback-fonts` (resolved
//! file paths, never author overrides, which `scoot-look` refuses to
//! spell). Anything else in the layer refuses the whole layer; a key
//! whose feature is not compiled in is dropped from the layer with a
//! note, while the user's own file keeps `deny_unknown_fields`.
//!
//! A scoped reload (`scope = "look"`) merges the new layer under a
//! snapshot of the user file's raw table from the last successful plain
//! load, so a swap never applies pending edits. A plain reload re-reads
//! both; a layer that went missing while it was active degrades to no
//! layer with a note, so the user's edits still apply. Startup with a
//! missing or broken layer starts without one and says so; it never
//! fails over a look.
//!
//! All of this runs at start-up and on reloads only: nothing here touches
//! a per-frame path.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::config::{self, Config};

/// Which build-gated keys the layer may carry: one flag per cargo feature
/// that gates a config section. The daemon passes [`Features::compiled`];
/// tests pass their own.
#[derive(Debug, Clone, Copy)]
pub struct Features {
    pub popup: bool,
    pub clock: bool,
    pub workspaces: bool,
    pub window_title: bool,
    pub volume: bool,
    pub microphone: bool,
    pub network: bool,
    pub brightness: bool,
    pub battery: bool,
    pub tray: bool,
    pub media: bool,
    pub bluetooth: bool,
    pub power: bool,
}

impl Features {
    /// The features of this binary.
    pub fn compiled() -> Self {
        Self {
            popup: cfg!(feature = "popup"),
            clock: cfg!(feature = "clock"),
            workspaces: cfg!(feature = "workspaces"),
            window_title: cfg!(feature = "window-title"),
            volume: cfg!(feature = "volume"),
            microphone: cfg!(feature = "microphone"),
            network: cfg!(feature = "network"),
            brightness: cfg!(feature = "brightness"),
            battery: cfg!(feature = "battery"),
            tray: cfg!(feature = "tray"),
            media: cfg!(feature = "media"),
            bluetooth: cfg!(feature = "bluetooth"),
            power: cfg!(feature = "power"),
        }
    }

    #[cfg(test)]
    const fn all() -> Self {
        Self {
            popup: true,
            clock: true,
            workspaces: true,
            window_title: true,
            volume: true,
            microphone: true,
            network: true,
            brightness: true,
            battery: true,
            tray: true,
            media: true,
            bluetooth: true,
            power: true,
        }
    }
}

/// The daemon's look state: a snapshot of the user file's raw table from
/// the last successful plain load, and whether that load merged a layer.
/// A scoped reload merges the new layer under the snapshot; it never
/// re-reads the file, so pending edits wait for a plain reload.
#[derive(Debug)]
pub struct LookState {
    snapshot: toml::Value,
    active: bool,
}

impl LookState {
    pub(crate) fn new(snapshot: toml::Value, active: bool) -> Self {
        Self { snapshot, active }
    }
}

/// What a reload computed: the merged config, its notes, and the look
/// state to commit. The caller commits `look` only after the whole reload
/// (the flags overlay, the apply) succeeds, so a refused reload leaves
/// the snapshot exactly as it was: a later scoped reload never merges a
/// new layer under edits that never applied.
#[derive(Debug)]
pub struct Reloaded {
    pub config: Config,
    pub notes: Vec<String>,
    pub look: LookState,
}

/// What starting with the look layer gives: the merged config, the file a
/// reload re-reads, and the snapshot plus notes.
#[derive(Debug)]
pub struct Startup {
    pub config: Config,
    pub file: Option<PathBuf>,
    pub from_file: bool,
    pub look: LookState,
    pub notes: Vec<String>,
}

/// The look state's directory: `$SCOOT_LOOK_STATE` (a private session's),
/// else `$XDG_STATE_HOME/scoot`, else `~/.local/state/scoot`. `None` when
/// no home is usable: there is then no state link, only a fixed layer.
/// The rule lives once in `scoot-look`'s `state` module (shared with scoot
/// and the future `look-apply-cli`; scootbg takes no look state); this is
/// the bar's call site, not a second copy.
pub fn state_dir() -> Option<PathBuf> {
    state_dir_from(
        std::env::var_os(scoot_look::state::STATE_ENV).as_deref(),
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// [`state_dir`] over explicit values, so tests never touch the
/// environment (whose mutation is `unsafe`, which this crate forbids).
/// Delegates to the shared rule, like [`state_dir`].
fn state_dir_from(
    session: Option<&OsStr>,
    xdg: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    scoot_look::state::resolve_state_dir(None, session, xdg, home)
}

/// Where the layer comes from: the user's file names a fixed one, or the
/// state link does.
enum Origin {
    /// `[look] layer` in the user's file: the bar-only path, no scoot.
    Fixed(PathBuf),
    /// `<state>/look/bar.toml`: the live choice.
    State(PathBuf),
}

/// The layer read: absent, dangling, broken, or a table.
enum Layer {
    /// No layer, silently: no state dir, or no link where one would be.
    Absent,
    /// A dangling state link: no layer, but worth a warning, since a live
    /// choice pointed here once.
    Dangling(PathBuf),
    /// Refused, naming why.
    Broken(String),
    /// A table to merge.
    Table(toml::Value),
}

/// Reads the file `explicit` names, or the default file, merged with the
/// look layer. Absent is the defaults; a malformed user file is a loud
/// error, exactly as without a layer. A missing or broken layer is never
/// fatal here: the bar starts without one and says so.
///
/// `state` is the look state's directory ([`state_dir`]): the state link
/// resolves under it. Tests pass their own.
pub fn load_startup(
    explicit: Option<&Path>,
    state: Option<&Path>,
) -> Result<Startup, config::Error> {
    let empty = || toml::Value::Table(toml::map::Map::new());
    // The user's file alone first, so its errors keep their positions:
    // the merged table can only fail where the layer changed it.
    let user = config::load_startup(explicit)?;
    let config::Startup {
        config: alone,
        file,
        from_file,
    } = user;
    let snapshot = match file.as_deref() {
        Some(path) if from_file => config::read_value(path)?,
        _ => empty(),
    };
    let features = Features::compiled();
    let mut notes = Vec::new();
    let (config, active) =
        match start_layer(&snapshot, file.as_deref(), &features, state, &mut notes) {
            Ok(Some(config)) => (config, true),
            Ok(None) => (alone, false),
            Err(broken) => {
                notes.push(broken);
                (alone, false)
            }
        };
    Ok(Startup {
        config,
        file,
        from_file,
        look: LookState::new(snapshot, active),
        notes,
    })
}

/// Merges the layer for start-up, with the font probe: a layer font that
/// fails to load drops the layer's font keys (the bar falls back to its
/// well-known paths) instead of refusing the start. Returns `Ok(None)`
/// when there is no layer, `Err` when it is broken.
fn start_layer(
    user: &toml::Value,
    file: Option<&Path>,
    features: &Features,
    state: Option<&Path>,
    notes: &mut Vec<String>,
) -> Result<Option<Config>, String> {
    let origin = match resolve_origin(user, file, state)? {
        None => return Ok(None),
        Some(origin) => origin,
    };
    let layer = match read_layer(&origin) {
        Layer::Absent => return Ok(None),
        Layer::Dangling(path) => {
            notes.push(format!(
                "look layer: {} is a dangling link, starting without a look layer",
                path.display()
            ));
            return Ok(None);
        }
        Layer::Broken(reason) => return Err(reason),
        Layer::Table(layer) => layer,
    };
    let (mut merged, staged, mut layer_notes) = merge(user, &layer, features, &origin)?;
    notes.append(&mut layer_notes);
    let path = layer_path(&origin);
    let mut config = config::config_from_value(merged.clone(), &path)
        .map_err(|error| format!("look layer: {error}"))?;
    // A layer font that fails to load drops the layer's font keys: the
    // bar falls back to its well-known paths, and the start is not lost
    // over a look. (A reload refuses instead, with the running bar kept.)
    if staged_contributed_font(&staged, user)
        && something_placed(&config)
        && let Err(error) = crate::font::text(config.font.as_deref(), &config.fallback_fonts)
    {
        notes.push(format!(
            "look layer: its font fails to load ({error}), the layer's font is dropped"
        ));
        merged = drop_layer_fonts(merged, &staged, user);
        config = config::config_from_value(merged, &path)
            .map_err(|error| format!("look layer: {error}"))?;
    }
    Ok(Some(config))
}

/// Re-reads the user file and the layer, and merges them: the plain
/// reload. Returns the merged config with the snapshot to commit; the
/// caller commits it only after the apply succeeds. Any failure leaves
/// the running bar (and the snapshot) exactly as they were.
///
/// A layer that is gone while it was active degrades to no layer with a
/// loud note (as at startup) and clears `active`, so the user's edits
/// still apply. Only the look-scoped reload refuses a missing layer.
pub fn reload_user(
    file: Option<&Path>,
    look: &LookState,
    features: &Features,
    state: Option<&Path>,
) -> Result<Reloaded, String> {
    let mut notes = Vec::new();
    let Some(path) = file else {
        return Err(
            "no config file to reload: no usable config home, start with `--config PATH`"
                .to_owned(),
        );
    };
    let bytes = config::read_bytes(path).map_err(|error| error.to_string())?;
    let alone = config::parse_user(&bytes, path).map_err(|error| error.to_string())?;
    let user: toml::Value = toml::from_slice(&bytes)
        .map_err(|error| config::Error::parse_error(path.to_owned(), error.to_string()))
        .map_err(|error| error.to_string())?;
    let ready =
        |config: Config, notes: Vec<String>, snapshot: toml::Value, active: bool| Reloaded {
            config,
            notes,
            look: LookState::new(snapshot, active),
        };
    let origin = match resolve_origin(&user, file, state)? {
        None => {
            return Ok(ready(alone, notes, user, false));
        }
        Some(origin) => origin,
    };
    let layer = match read_layer(&origin) {
        Layer::Absent if look.active => {
            notes.push(format!(
                "look layer {} is gone, reloading without a look layer",
                layer_path(&origin).display()
            ));
            return Ok(ready(alone, notes, user, false));
        }
        Layer::Absent => {
            return Ok(ready(alone, notes, user, false));
        }
        Layer::Dangling(path) if look.active => {
            notes.push(format!(
                "look layer: {} is a dangling link, reloading without a look layer",
                path.display()
            ));
            return Ok(ready(alone, notes, user, false));
        }
        Layer::Dangling(path) => {
            notes.push(format!(
                "look layer: {} is a dangling link, reloading without a look layer",
                path.display()
            ));
            return Ok(ready(alone, notes, user, false));
        }
        Layer::Broken(reason) => return Err(reason),
        Layer::Table(layer) => layer,
    };
    let (merged, _, mut layer_notes) = merge(&user, &layer, features, &origin)?;
    notes.append(&mut layer_notes);
    let path = layer_path(&origin);
    let config =
        config::config_from_value(merged, &path).map_err(|error| format!("look layer: {error}"))?;
    Ok(ready(config, notes, user, true))
}

/// Merges the new layer under the snapshot: the look-scoped reload. The
/// user file is never re-read, so pending edits wait for a plain reload.
/// Returns the merged config with the state to commit; the caller commits
/// it only after the apply succeeds. A missing or broken layer refuses;
/// nothing moves.
pub fn reload_look(
    file: Option<&Path>,
    look: &LookState,
    features: &Features,
    state: Option<&Path>,
) -> Result<Reloaded, String> {
    let mut notes = Vec::new();
    let origin = match resolve_origin(&look.snapshot, file, state)? {
        None => {
            return Err("no look layer is configured, refusing the look-scoped reload".to_owned());
        }
        Some(origin) => origin,
    };
    let layer = match read_layer(&origin) {
        Layer::Absent => {
            return Err(format!(
                "look layer {} is missing, refusing: the running bar stands",
                layer_path(&origin).display()
            ));
        }
        Layer::Dangling(path) => {
            return Err(format!(
                "look layer {} is a dangling link, refusing: the running bar stands",
                path.display()
            ));
        }
        Layer::Broken(reason) => return Err(reason),
        Layer::Table(layer) => layer,
    };
    let (merged, _, mut layer_notes) = merge(&look.snapshot, &layer, features, &origin)?;
    notes.append(&mut layer_notes);
    let path = layer_path(&origin);
    let config =
        config::config_from_value(merged, &path).map_err(|error| format!("look layer: {error}"))?;
    Ok(Reloaded {
        config,
        notes,
        look: LookState::new(look.snapshot.clone(), true),
    })
}

/// Resolves where the layer comes from: `[look] layer` when the user's
/// file (or snapshot) names one, else the state link. A relative fixed
/// path resolves against the user file's directory.
fn resolve_origin(
    user: &toml::Value,
    file: Option<&Path>,
    state: Option<&Path>,
) -> Result<Option<Origin>, String> {
    if let Some(fixed) = user.get("look").and_then(|look| look.get("layer")) {
        let Some(path) = fixed.as_str() else {
            return Err("`look.layer` takes a path to a look layer file".to_owned());
        };
        if path.is_empty() || path.contains('\0') {
            return Err(format!(
                "`look.layer` takes a path to a look layer file, not `{path}`"
            ));
        }
        let path = PathBuf::from(path);
        let resolved = if path.is_absolute() {
            path
        } else {
            match file.and_then(|file| file.parent()) {
                Some(parent) if !parent.as_os_str().is_empty() => parent.join(path),
                _ => {
                    return Err(
                        "`look.layer` is relative but no config file directory resolves it"
                            .to_owned(),
                    );
                }
            }
        };
        return Ok(Some(Origin::Fixed(resolved)));
    }
    let Some(dir) = state else {
        return Ok(None);
    };
    Ok(Some(Origin::State(dir.join("look").join("bar.toml"))))
}

/// The layer's path, for messages.
fn layer_path(origin: &Origin) -> PathBuf {
    match origin {
        Origin::Fixed(path) | Origin::State(path) => path.clone(),
    }
}

/// Reads the layer: absent links are no layer, anything else that fails
/// is broken. A fixed layer is explicit configuration, so even a missing
/// file is broken, never absent.
fn read_layer(origin: &Origin) -> Layer {
    let path = layer_path(origin);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match origin {
            Origin::Fixed(_) => Layer::Broken(format!("look layer {}: {error}", path.display())),
            Origin::State(_) => {
                // A dangling link reads as missing: tell it apart, since
                // a live choice pointed here once.
                let dangling = path
                    .parent()
                    .and_then(|link| std::fs::symlink_metadata(link).ok())
                    .is_some_and(|meta| meta.file_type().is_symlink());
                if dangling {
                    Layer::Dangling(path)
                } else {
                    Layer::Absent
                }
            }
        },
        Err(error) => Layer::Broken(format!("look layer {}: {error}", path.display())),
        Ok(_) => match config::read_value(&path) {
            Err(error) => {
                // A dangling state link reads as missing: warn at start,
                // refuse the reload, but never fail the start over it.
                if matches!(origin, Origin::State(_))
                    && matches!(error, config::Error::Read { .. })
                    && std::fs::metadata(&path).is_err()
                {
                    Layer::Dangling(path)
                } else {
                    Layer::Broken(format!("look layer: {error}"))
                }
            }
            Ok(value) => match &value {
                toml::Value::Table(_) => Layer::Table(value),
                _ => Layer::Broken(format!("look layer {}: not a TOML table", path.display())),
            },
        },
    }
}

/// What the layer staged, as dotted `(section, key)` pairs: the font
/// probe reads it back.
type StagedKeys = Vec<(String, String)>;

/// Whether `dotted` needs a feature this build lacks, naming it.
fn gated_off(dotted: &str, features: &Features) -> Option<&'static str> {
    let (section, _) = dotted.split_once('.').unwrap_or((dotted, ""));
    match section {
        "bar" if dotted == "bar.popup-radius" && !features.popup => Some("popup"),
        "workspaces" if !features.workspaces => Some("workspaces"),
        "window-title" if !features.window_title => Some("window-title"),
        "clock" if !features.clock => Some("clock"),
        "volume" if !features.volume => Some("volume"),
        "microphone" if !features.microphone => Some("microphone"),
        "network" if !features.network => Some("network"),
        "brightness" if !features.brightness => Some("brightness"),
        "battery" if !features.battery => Some("battery"),
        "tray" if !features.tray => Some("tray"),
        "media" if !features.media => Some("media"),
        "bluetooth" if !features.bluetooth => Some("bluetooth"),
        "power" if !features.power => Some("power"),
        _ => None,
    }
}

/// A font file path the renderer wrote: absolute (relative would resolve
/// against the daemon's working directory, which a unit leaves
/// unpredictable), plain text, no NUL or control.
fn check_font_path(key: &str, value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 4096
        || value.contains('\0')
        || value.chars().any(|c| c.is_control())
    {
        return Some(format!("`{key}` takes a font file path"));
    }
    if !value.starts_with('/') {
        return Some(format!(
            "`{key}` takes an absolute font file path, not `{value}`"
        ));
    }
    None
}

/// Merges the layer under the user table on raw keys: a user key wins,
/// else the layer's, else the default. Returns the merged table, what the
/// layer staged (for the font probe), and notes for dropped keys.
///
/// A forbidden key or a bad value refuses the whole layer, naming it. A
/// key whose feature is not compiled in is dropped with a note.
fn merge(
    user: &toml::Value,
    layer: &toml::Value,
    features: &Features,
    origin: &Origin,
) -> Result<(toml::Value, StagedKeys, Vec<String>), String> {
    let path = layer_path(origin);
    let failed = |reason: String| format!("look layer {}: {reason}", path.display());
    let tables = layer
        .as_table()
        .ok_or_else(|| failed("not a TOML table".to_owned()))?;
    let mut staged: Vec<(String, String, toml::Value)> = Vec::new();
    let mut notes = Vec::new();
    for (section, table) in tables {
        // Only tables hold look keys: a top-level list (`left = [...]`)
        // or scalar is a forbidden key, not a malformed table.
        let table = table
            .as_table()
            .ok_or_else(|| failed(format!("key `{section}` is not a look key")))?;
        for (key, value) in table {
            let dotted = format!("{section}.{key}");
            // The renderer's own font paths: resolved file paths, never
            // author overrides (which `scoot-look` refuses to spell).
            if dotted == "bar.font" {
                let Some(text) = value.as_str() else {
                    return Err(failed("`bar.font` takes a font file path".to_owned()));
                };
                if let Some(reason) = check_font_path(&dotted, text) {
                    return Err(failed(reason));
                }
                staged.push((section.clone(), key.clone(), value.clone()));
                continue;
            }
            if dotted == "bar.fallback-fonts" {
                let Some(items) = value.as_array() else {
                    return Err(failed(
                        "`bar.fallback-fonts` takes at most 2 font files".to_owned(),
                    ));
                };
                if items.len() > crate::text::MAX_FALLBACKS {
                    return Err(failed(format!(
                        "`bar.fallback-fonts` takes at most {} font files, not {}",
                        crate::text::MAX_FALLBACKS,
                        items.len()
                    )));
                }
                for item in items {
                    match item.as_str() {
                        Some(text) if check_font_path(&dotted, text).is_none() => {}
                        _ => {
                            return Err(failed(
                                "`bar.fallback-fonts` takes at most 2 font files".to_owned(),
                            ));
                        }
                    }
                }
                staged.push((section.clone(), key.clone(), value.clone()));
                continue;
            }
            if !scoot_look::allowlist::allows("scootbar", &dotted) {
                return Err(failed(format!("key `{dotted}` is not a look key")));
            }
            if let Some(error) = scoot_look::allowlist::check_app_value("scootbar", &dotted, value)
            {
                return Err(failed(error.to_string()));
            }
            if let Some(feature) = gated_off(&dotted, features) {
                notes.push(format!(
                    "look layer: `{dotted}` dropped (this build has no `{feature}`)"
                ));
                continue;
            }
            staged.push((section.clone(), key.clone(), value.clone()));
        }
    }
    let mut merged = user.clone();
    let root = merged
        .as_table_mut()
        .ok_or_else(|| failed("the user config is not a TOML table".to_owned()))?;
    for (section, key, value) in &staged {
        let table = root
            .entry(section.clone())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if let Some(table) = table.as_table_mut() {
            table.entry(key.clone()).or_insert(value.clone());
        }
    }
    // The derived pair: the user's `hover` if set; else, if the user set
    // `accent`, the look's `hover` is dropped so the bar derives hover
    // from the user's accent; else the look's `hover` stands.
    let user_hover = user
        .get("colors")
        .and_then(|colors| colors.get("hover"))
        .is_some();
    let user_accent = user
        .get("colors")
        .and_then(|colors| colors.get("accent"))
        .is_some();
    if !user_hover
        && user_accent
        && let Some(colors) = root.get_mut("colors")
        && let Some(colors) = colors.as_table_mut()
    {
        colors.remove("hover");
    }
    let staged_keys = staged
        .into_iter()
        .map(|(section, key, _)| (section, key))
        .collect();
    Ok((merged, staged_keys, notes))
}

/// Whether the staged layer keys contributed a font the user file lacks:
/// only then can the font load be the layer's to answer for.
fn staged_contributed_font(staged: &[(String, String)], user: &toml::Value) -> bool {
    staged.iter().any(|(section, key)| {
        section == "bar"
            && (key == "font" || key == "fallback-fonts")
            && user.get("bar").and_then(|bar| bar.get(key)).is_none()
    })
}

/// Drops the layer's font keys from a merged table (the font probe
/// failed): the bar falls back to its well-known paths.
fn drop_layer_fonts(
    mut merged: toml::Value,
    staged: &[(String, String)],
    user: &toml::Value,
) -> toml::Value {
    if let Some(bar) = merged.get_mut("bar")
        && let Some(bar) = bar.as_table_mut()
    {
        for (section, key) in staged {
            if section == "bar"
                && (key == "font" || key == "fallback-fonts")
                && user.get("bar").and_then(|bar| bar.get(key)).is_none()
            {
                bar.remove(key);
            }
        }
    }
    merged
}

/// Whether any module is placed: with none, no font loads either way.
fn something_placed(config: &Config) -> bool {
    config
        .outputs
        .to_start(&config.layout)
        .placed()
        .next()
        .is_some()
}

#[cfg(test)]
mod tests;
