//! The look layer: `<state>/look/launch.toml`, read at every open.
//!
//! The fragment carries the thirteen shell-app color roles plus the
//! dotless `font`, `font-size`, `opacity` and `radius` (rendered by
//! `scoot-look`, overridden per key through `[apps.scootlaunch]`).
//! Every key passes the allowlist and the value grammar
//! (`scoot_look::allowlist::check_app_value`); a refused key keeps its
//! built-in default and is reported, never wedging the open. A missing or
//! dangling layer is "no look", not an error.
//!
//! Onto the six-token [`Theme`](scootui::theme::Theme) the entry and list
//! paints read:
//!
//! | Element | Role | Field |
//! |---|---|---|
//! | Panel fill | `surface` (default `background`) | [`Look::surface`] |
//! | Input and row text | `foreground` | `theme.foreground` |
//! | Placeholder | `dim` | `theme.dim` |
//! | Matched characters | `match` (default `hover`) | `theme.hover` |
//! | Selected row fill | `selection` (default `accent`) | `theme.accent` |
//! | Selected row text | `selection-text` (default `accent-fg`, default `background`) | `theme.background` |
//! | Panel border | `focus` (default `accent`) | [`Look::focus`] |
//! | Caret, entry focus ring | `accent` | `theme.accent` (see below) |
//!
//! `border` and `accent-fg` are allowlisted and grammar-checked but draw
//! nothing in the panel (the border is `focus`; `selection-text` arrives
//! resolved, so `accent-fg` has no separate reader). Accepting them keeps
//! the fragment forward-compatible: a role the panel grows into never
//! refused yesterday's look.
//!
//! Two roles share one field where the six tokens run out: the caret and
//! the focus ring follow `selection` when a look overrides it apart from
//! `accent`, and the selected-row ink follows `background` when
//! `selection-text` is overridden apart. The defaults coincide
//! (`selection` defaults to `accent`, `selection-text` to `background`),
//! and no shipped look overrides either apart, so the shipped looks draw
//! exactly what the table says (pinned by the goldens below).
//!
//! The `font` key names the look's UI family. There is no fontconfig in
//! this stack: the panel draws with `--font PATH`, else the first usable
//! well-known file, as the bar does. The key is validated and kept (a
//! later ticket may resolve it); v1 documents that it does not pick the
//! file.

use std::path::Path;

use scootui::color::Color;
use scootui::theme::Theme;

/// The default font size, in logical pixels: the contract's 10 pt
/// `popup-size` default (`clamp(1, 256, (pt*4+1)/3)`).
pub const DEFAULT_FONT_SIZE_PX: f32 = 13.0;
/// The default panel corner radius, in device pixels.
pub const DEFAULT_RADIUS: u32 = 8;

/// The panel's resolved look: the widget theme plus the panel's own fill,
/// border, size, opacity and radius.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// The entry and list theme (`background` carries `selection-text`'s
    /// default; see the module docs).
    pub theme: Theme,
    /// The panel fill (`surface`).
    pub surface: Color,
    /// The panel border (`focus`).
    pub focus: Color,
    /// The text size in logical pixels (`font-size`, 1 to 256).
    pub font_size_px: f32,
    /// The panel fill's alpha (`opacity`, 0 to 1).
    pub opacity: f32,
    /// The panel corner radius in device pixels (`radius`, 0 to 64).
    pub radius: u32,
    /// The look's UI family (`font`): validated, informational in v1.
    pub font_family: String,
}

impl Default for Look {
    fn default() -> Self {
        let theme = Theme::default();
        Self {
            theme,
            surface: Theme::default().background,
            focus: Theme::default().accent,
            font_size_px: DEFAULT_FONT_SIZE_PX,
            opacity: 1.0,
            radius: DEFAULT_RADIUS,
            font_family: String::new(),
        }
    }
}

/// Reads the look layer: `<state>/look/launch.toml`, where `<state>` is
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
    let path = dir.join("look").join("launch.toml");
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

/// Parses a `launch.toml` fragment: the allowlisted keys over the
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
        notes.push(format!("`{dotted}` is not a launcher look key: skipped"));
        return;
    };
    if let Some(refused) = scoot_look::allowlist::check_app_value("scootlaunch", &dotted, value) {
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
    /// An allowlisted key the panel draws nothing with (`border`,
    /// `accent-fg`): validated above, then dropped.
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
        "border" | "accent-fg" => Some(Slot::Ignore),
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

#[cfg(test)]
mod tests;
