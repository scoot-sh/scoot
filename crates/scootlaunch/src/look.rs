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
//! | Matched characters | `match` (default `hover`) | `theme.hover` (plain rows; the selected row resolves its own ink through [`Theme::match_on_selection`](scootui::theme::Theme::match_on_selection)) |
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
//! Depth and motion are optional and off by default (`shadow`, `fill`,
//! `motion`, `motion-duration-ms`, `motion-bounce`; the shared `[effects]`
//! of the look, or `[apps.scootlaunch]`): a fragment that sets none paints
//! and behaves as it always did. `shadow` adds the drop shadow (a glow
//! takes the `focus` color), `fill = "gradient"` lights the panel from the
//! top, and a `motion` makes the panel scale and fade in, fade out when
//! dismissed, and slide the selection between rows.
//!
//! The `font` key names the look's UI family and `font-file` the file
//! `scoot look apply` resolved it to (fontconfig, or the look's own font
//! file). There is no fontconfig in this stack: the panel draws with
//! `--font PATH` or the config's `font` file, else `font-file` as given,
//! else the family's file found by name in the usual font directories
//! (`scootui::font::family`; the way a hand-edited fragment, or one applied
//! by an older tool, still works), else the first usable well-known file.
//! There is no bold face: the panel draws none.

use std::path::{Path, PathBuf};

pub mod user;
pub use user::apply_user;

use scootui::color::Color;
use scootui::effects::{FillStyle, ShadowStyle};
use scootui::motion::{Curve, DEFAULT_BOUNCE, MAX_DURATION_MS, Timing};
use scootui::theme::Theme;

/// The default font size, in logical pixels: the contract's 10 pt
/// `popup-size` default (`clamp(1, 256, (pt*4+1)/3)`).
pub const DEFAULT_FONT_SIZE_PX: f32 = 13.0;
/// The default panel corner radius, in logical pixels (scaled with the
/// output, as the border and the font are).
pub const DEFAULT_RADIUS: u32 = 8;
/// How long the panel's motion runs when a look asks for motion and names no
/// duration, in milliseconds: shorter than the notification cards' default
/// because the panel is large and opens on a keypress.
pub const DEFAULT_MOTION_MS: u32 = 160;
/// The default icon theme, when the fragment names none: what the
/// looks resolve to (`Adwaita`).
pub const DEFAULT_ICON_THEME: &str = "Adwaita";

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
    /// The panel corner radius in logical pixels (`radius`, 0 to 64), scaled
    /// by the output's scale when drawn.
    pub radius: u32,
    /// The look's UI family (`font`), looked for as a font file at each open
    /// when [`Look::font_file`] names none or cannot be used.
    pub font_family: String,
    /// The file `scoot look apply` resolved the family to (`font-file`):
    /// read as given, no font directory walked.
    pub font_file: Option<PathBuf>,
    /// The icon theme the panel looks up names in (`[icons] theme`):
    /// the look's theme, `Adwaita` by default.
    pub icon_theme: String,
    /// The depth under the panel (`shadow`).
    pub shadow: ShadowStyle,
    /// How the panel fill is laid down (`fill`).
    pub fill: FillStyle,
    /// The curve the panel's motion follows (`motion`); `none` moves
    /// nothing.
    pub motion: Curve,
    /// How long one motion runs (`motion-duration-ms`, 0 to 1000).
    pub motion_ms: u32,
    /// The spring's overshoot (`motion-bounce`, 0 to 1).
    pub motion_bounce: f32,
}

impl Look {
    /// The clamped timing the motion tokens ask for: [`Timing::NONE`] for
    /// `motion = none` or a zero duration, which paints and costs exactly
    /// what a look without motion does.
    pub fn timing(&self) -> Timing {
        Timing::new(self.motion, self.motion_ms, self.motion_bounce)
    }

    /// The panel fill's top and bottom colors ([`FillStyle::stops`]): the
    /// `surface` twice for a solid fill.
    pub fn fill_stops(&self) -> (Color, Color) {
        self.fill.stops(self.surface)
    }
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
            font_file: None,
            icon_theme: DEFAULT_ICON_THEME.to_string(),
            shadow: ShadowStyle::None,
            fill: FillStyle::Solid,
            motion: Curve::None,
            motion_ms: DEFAULT_MOTION_MS,
            motion_bounce: DEFAULT_BOUNCE,
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
        notes.push(format!("`{dotted}` is not a launcher look key: skipped"));
        return;
    };
    // `font-file` is the tool's own key (`scoot look apply` resolves the
    // family), so it is not in the allowlist an `[apps.scootlaunch]` table may
    // set; it has its own check.
    let refused = if dotted == scoot_look::allowlist::FONT_FILE_KEY {
        scoot_look::allowlist::check_font_file(value)
    } else {
        scoot_look::allowlist::check_app_value("scootlaunch", &dotted, value)
    };
    if let Some(refused) = refused {
        notes.push(format!("`{dotted}` refused ({refused}): value ignored"));
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
    FontFile(&'a mut Option<PathBuf>),
    IconTheme(&'a mut String),
    Shadow(&'a mut ShadowStyle),
    Fill(&'a mut FillStyle),
    Motion(&'a mut Curve),
    MotionMs(&'a mut u32),
    MotionBounce(&'a mut f32),
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
                None => notes.push(format!("`{dotted}` is not a color: value ignored")),
            },
            Slot::FontSize(slot) => match value.as_integer() {
                Some(size) => *slot = size as f32,
                None => notes.push(format!("`{dotted}` is not a size: value ignored")),
            },
            Slot::Opacity(slot) => {
                let alpha = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|int| int as f64));
                match alpha {
                    Some(alpha) => *slot = alpha as f32,
                    None => notes.push(format!("`{dotted}` is not an opacity: value ignored")),
                }
            }
            Slot::Radius(slot) => match value.as_integer() {
                Some(radius) => *slot = radius.max(0) as u32,
                None => notes.push(format!("`{dotted}` is not a radius: value ignored")),
            },
            Slot::FontFamily(slot) => match value.as_str() {
                Some(family) => *slot = family.to_owned(),
                None => notes.push(format!("`{dotted}` is not a font name: value ignored")),
            },
            Slot::FontFile(slot) => match value.as_str() {
                Some(path) => *slot = Some(PathBuf::from(path)),
                None => notes.push(format!("`{dotted}` is not a path: value ignored")),
            },
            Slot::Shadow(slot) => match value.as_str().and_then(ShadowStyle::parse) {
                Some(style) => *slot = style,
                None => notes.push(format!(
                    "`{dotted}` is not a shadow style (none, soft, deep or glow): value ignored"
                )),
            },
            Slot::Fill(slot) => match value.as_str().and_then(FillStyle::parse) {
                Some(style) => *slot = style,
                None => notes.push(format!(
                    "`{dotted}` is not a fill style (solid or gradient): value ignored"
                )),
            },
            Slot::Motion(slot) => match value.as_str().and_then(Curve::parse) {
                Some(curve) => *slot = curve,
                None => notes.push(format!(
                    "`{dotted}` is not a motion (none, ease-out or spring): value ignored"
                )),
            },
            Slot::MotionMs(slot) => match value.as_integer() {
                Some(ms) => *slot = ms.clamp(0, i64::from(MAX_DURATION_MS)) as u32,
                None => notes.push(format!(
                    "`{dotted}` is not a duration in milliseconds: value ignored"
                )),
            },
            Slot::MotionBounce(slot) => {
                let bounce = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|int| int as f64));
                match bounce {
                    Some(bounce) if bounce.is_finite() => *slot = bounce.clamp(0.0, 1.0) as f32,
                    _ => notes.push(format!("`{dotted}` is not a bounce: value ignored")),
                }
            }
            Slot::IconTheme(slot) => match value.as_str() {
                Some(theme) if is_icon_theme(theme) => *slot = theme.to_owned(),
                _ => notes.push(format!("`{dotted}` is not an icon theme: value ignored")),
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

/// The slot for a dotless top-level key, if it is one of the nine.
fn look_dotless<'a>(look: &'a mut Look, key: &str) -> Option<Slot<'a>> {
    match key {
        "font" => Some(Slot::FontFamily(&mut look.font_family)),
        "font-file" => Some(Slot::FontFile(&mut look.font_file)),
        "font-size" => Some(Slot::FontSize(&mut look.font_size_px)),
        "opacity" => Some(Slot::Opacity(&mut look.opacity)),
        "radius" => Some(Slot::Radius(&mut look.radius)),
        "shadow" => Some(Slot::Shadow(&mut look.shadow)),
        "fill" => Some(Slot::Fill(&mut look.fill)),
        "motion" => Some(Slot::Motion(&mut look.motion)),
        "motion-duration-ms" => Some(Slot::MotionMs(&mut look.motion_ms)),
        "motion-bounce" => Some(Slot::MotionBounce(&mut look.motion_bounce)),
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
