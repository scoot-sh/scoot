//! The look layer: `<state>/look/notify.toml`, read at start and on
//! re-theme.
//!
//! The fragment carries the thirteen shell-app color roles plus the
//! dotless `font`, `font-size`, `opacity` and `radius` (rendered by
//! `scoot-look`, overridden per key through `[apps.scootnotify]`).
//! Every key passes the allowlist and the value grammar
//! (`scoot_look::allowlist::check_app_value`); a refused key keeps its
//! built-in default and is reported, never wedging the daemon. A missing
//! or dangling layer is "no look", not an error.
//!
//! Onto the six-token [`Theme`](scootui::theme::Theme) the card paints
//! read:
//!
//! | Element | Role | Field |
//! |---|---|---|
//! | Card fill | `surface` (default `background`) | [`Look::surface`] |
//! | Summary, body | `foreground` | `theme.foreground` |
//! | App name, age, "+N more" | `dim` | `theme.dim` |
//! | Border, normal | `focus` (default `accent`) | [`Look::focus`] |
//! | Border, low | `border` (default `dim`) | [`Look::border`] |
//! | Border, critical | `urgent` | `theme.urgent` |
//! | Progress bar | `selection` (default `accent`) | `theme.accent` (see below) |
//! | Action button outline / hovered fill | `focus` / `hover` | `Look::focus` / `theme.hover` |
//! | Link text (underlined) | `accent` | `theme.accent` |
//!
//! `accent-fg` is allowlisted and grammar-checked but draws nothing (the
//! selected-ink role has no card reader; `selection-text` arrives
//! resolved, as in the panel). Accepting it keeps the fragment
//! forward-compatible: a role the cards grow into never refused
//! yesterday's look.
//!
//! Two roles share one field where the six tokens run out: the progress
//! bar and link text follow `selection` when a look overrides it apart
//! from `accent`, and the button hover fill follows `match` when that is
//! overridden apart from `hover`. The defaults coincide (`selection`
//! defaults to `accent`, `match` to `hover`), and no shipped look
//! overrides either apart, so the shipped looks draw exactly what the
//! table says (pinned by the goldens in the popup tests).
//!
//! The `font` key names the look's UI family. There is no fontconfig in
//! this stack: the daemon draws with `--font PATH`, else the first usable
//! well-known file, as the bar does. The key is validated and kept (a
//! later ticket may resolve it); v1 documents that it does not pick the
//! file.

use std::path::Path;

use scootui::color::Color;
use scootui::theme::Theme;

/// The default font size, in logical pixels: the contract's 10 pt
/// `popup-size` default (`clamp(1, 256, (pt*4+1)/3)`).
pub const DEFAULT_FONT_SIZE_PX: f32 = 13.0;
/// The default card corner radius, in logical pixels.
pub const DEFAULT_RADIUS: u32 = 8;
/// The default icon theme, when the fragment names none: what the
/// looks resolve to (`Adwaita`).
pub const DEFAULT_ICON_THEME: &str = "Adwaita";

/// The cards' resolved look: the widget theme plus the cards' own fill,
/// borders, size, opacity and radius.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// The card theme (`background` carries `selection-text`'s default;
    /// see the module docs).
    pub theme: Theme,
    /// The card fill (`surface`).
    pub surface: Color,
    /// The normal-urgency border and button outline (`focus`).
    pub focus: Color,
    /// The low-urgency border (`border`).
    pub border: Color,
    /// The text size in logical pixels (`font-size`, 1 to 256).
    pub font_size_px: f32,
    /// The card fill's alpha (`opacity`, 0 to 1).
    pub opacity: f32,
    /// The card corner radius in logical pixels (`radius`, 0 to 64).
    pub radius: u32,
    /// The look's UI family (`font`): validated, informational in v1.
    pub font_family: String,
    /// The icon theme names resolve in (`[icons] theme`): the look's
    /// theme, `Adwaita` by default.
    pub icon_theme: String,
}

impl Default for Look {
    fn default() -> Self {
        let theme = Theme::default();
        Self {
            theme,
            surface: Theme::default().background,
            focus: Theme::default().accent,
            border: Theme::default().dim,
            font_size_px: DEFAULT_FONT_SIZE_PX,
            opacity: 1.0,
            radius: DEFAULT_RADIUS,
            font_family: String::new(),
            icon_theme: DEFAULT_ICON_THEME.to_string(),
        }
    }
}

/// Reads the look layer: `<state>/look/notify.toml`, where `<state>` is
/// `explicit`, then `$SCOOT_LOOK_STATE`, then `$XDG_STATE_HOME/scoot`,
/// then `~/.local/state/scoot`. Returns the resolved look plus one note
/// per fallback (missing layer, refused key), for the startup warning.
/// Never fails: without a layer every key is its built-in default.
pub fn load(explicit: Option<&Path>) -> (Look, Vec<String>) {
    let mut notes = Vec::new();
    let Some(dir) = scoot_look::state::state_dir_from_env(explicit) else {
        notes.push(String::from("no look state dir: no look layer"));
        return (Look::default(), notes);
    };
    let path = dir.join("look").join("notify.toml");
    let text = match scoot_look::state::read_capped(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            notes.push(format!("no {}: no look layer", path.display()));
            return (Look::default(), notes);
        }
        Err(error) => {
            notes.push(format!(
                "cannot read {} ({error}): no look layer",
                path.display()
            ));
            return (Look::default(), notes);
        }
    };
    let (look, refused) = parse(&text);
    for note in refused {
        notes.push(format!("{}: {note}", path.display()));
    }
    (look, notes)
}

/// Parses a `notify.toml` fragment: the allowlisted keys over the
/// defaults, plus one note per refused key. Pure over its input, so tests
/// never touch the filesystem.
pub fn parse(text: &str) -> (Look, Vec<String>) {
    let mut look = Look::default();
    let mut notes = Vec::new();
    let table: toml::Table = match text.parse() {
        Ok(table) => table,
        Err(error) => {
            notes.push(format!("does not parse ({error}): built-in defaults apply"));
            return (look, notes);
        }
    };
    for (key, value) in &table {
        if key == "colors" {
            let Some(colors) = value.as_table() else {
                notes.push(String::from("`colors` is not a table: skipped"));
                continue;
            };
            for (name, color) in colors {
                apply(
                    look_color(&mut look, name),
                    "colors",
                    name,
                    color,
                    &mut notes,
                );
            }
            continue;
        }
        if key == "icons" {
            let Some(icons) = value.as_table() else {
                notes.push(String::from("`icons` is not a table: skipped"));
                continue;
            };
            for (name, icon) in icons {
                apply(look_icons(&mut look, name), "icons", name, icon, &mut notes);
            }
            continue;
        }
        apply(look_dotless(&mut look, key), "", key, value, &mut notes);
    }
    (look, notes)
}

/// Applies one fragment value through the allowlist and the grammar: a
/// refused key keeps its default and is reported.
fn apply(
    slot: Option<Slot<'_>>,
    section: &str,
    key: &str,
    value: &toml::Value,
    notes: &mut Vec<String>,
) {
    let dotted = if section.is_empty() {
        key.to_owned()
    } else {
        format!("{section}.{key}")
    };
    let Some(slot) = slot else {
        notes.push(format!("`{dotted}` is not a notifier look key: skipped"));
        return;
    };
    if let Some(refused) = scoot_look::allowlist::check_app_value("scootnotify", &dotted, value) {
        notes.push(format!("`{dotted}` refused ({refused}): default applies"));
        return;
    }
    slot.set(value, &dotted, notes);
}

/// One writable look key: what `apply` writes after the grammar passed.
/// The typed read is defensive (`Color::parse`, numeric casts): the
/// grammar already constrained the shape, so a failure here names the key
/// and keeps the default rather than panicking.
enum Slot<'a> {
    Color(&'a mut Color),
    FontSize(&'a mut f32),
    Opacity(&'a mut f32),
    Radius(&'a mut u32),
    FontFamily(&'a mut String),
    IconTheme(&'a mut String),
    /// An allowlisted key the cards draw nothing with (`accent-fg`):
    /// validated above, then dropped.
    Ignore,
}

impl Slot<'_> {
    fn set(self, value: &toml::Value, dotted: &str, notes: &mut Vec<String>) {
        match self {
            Slot::Ignore => {
                let _ = (value, dotted, notes);
            }
            Slot::Color(slot) => match value.as_str().and_then(|text| Color::parse(text).ok()) {
                Some(color) => *slot = color,
                None => notes.push(format!("`{dotted}` is not a color: default applies")),
            },
            Slot::FontSize(slot) => match value.as_integer() {
                Some(size) => *slot = size as f32,
                None => notes.push(format!("`{dotted}` is not a size: default applies")),
            },
            Slot::Opacity(slot) => {
                let alpha = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|int| int as f64));
                match alpha {
                    Some(alpha) => *slot = alpha as f32,
                    None => notes.push(format!("`{dotted}` is not an opacity: default applies")),
                }
            }
            Slot::Radius(slot) => match value.as_integer() {
                Some(radius) => *slot = radius.max(0) as u32,
                None => notes.push(format!("`{dotted}` is not a radius: default applies")),
            },
            Slot::FontFamily(slot) => match value.as_str() {
                Some(family) => *slot = family.to_owned(),
                None => notes.push(format!("`{dotted}` is not a font name: default applies")),
            },
            Slot::IconTheme(slot) => match value.as_str() {
                Some(theme) if is_icon_theme(theme) => *slot = theme.to_owned(),
                _ => notes.push(format!("`{dotted}` is not an icon theme: default applies")),
            },
        }
    }
}

/// The color slot for a `[colors]` name, if it is one of the thirteen.
fn look_color<'a>(look: &'a mut Look, name: &str) -> Option<Slot<'a>> {
    match name {
        "background" => Some(Slot::Color(&mut look.theme.background)),
        "foreground" => Some(Slot::Color(&mut look.theme.foreground)),
        "dim" => Some(Slot::Color(&mut look.theme.dim)),
        "accent" => Some(Slot::Color(&mut look.theme.accent)),
        "urgent" => Some(Slot::Color(&mut look.theme.urgent)),
        "surface" => Some(Slot::Color(&mut look.surface)),
        "hover" => Some(Slot::Color(&mut look.theme.hover)),
        "focus" => Some(Slot::Color(&mut look.focus)),
        "border" => Some(Slot::Color(&mut look.border)),
        "accent-fg" => Some(Slot::Ignore),
        "selection" => Some(Slot::Color(&mut look.theme.accent)),
        "selection-text" => Some(Slot::Color(&mut look.theme.background)),
        "match" => Some(Slot::Color(&mut look.theme.hover)),
        _ => None,
    }
}

/// The slot for a dotless top-level key, if it is one of the four.
fn look_dotless<'a>(look: &'a mut Look, key: &str) -> Option<Slot<'a>> {
    match key {
        "font" => Some(Slot::FontFamily(&mut look.font_family)),
        "font-size" => Some(Slot::FontSize(&mut look.font_size_px)),
        "opacity" => Some(Slot::Opacity(&mut look.opacity)),
        "radius" => Some(Slot::Radius(&mut look.radius)),
        _ => None,
    }
}

/// The slot for an `[icons]` key: only `theme` today.
fn look_icons<'a>(look: &'a mut Look, name: &str) -> Option<Slot<'a>> {
    match name {
        "theme" => Some(Slot::IconTheme(&mut look.icon_theme)),
        _ => None,
    }
}

/// Whether `theme` is a usable icon theme name: 1 to 64 ASCII letters,
/// digits and `._+-`, never starting with a dot (no traversal, no
/// absolute path).
fn is_icon_theme(theme: &str) -> bool {
    if theme.is_empty() || theme.len() > 64 || theme.starts_with('.') {
        return false;
    }
    theme.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_' || byte == b'+' || byte == b'-'
    })
}

#[cfg(test)]
mod tests;
