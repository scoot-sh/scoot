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
//! Two more dotless keys give the cards depth, both optional and both
//! off by default (a look that sets neither paints exactly what it
//! always did): `shadow` (`none`, `soft`, `deep` or `glow`) and `fill`
//! (`solid` or `gradient`). A value outside those is reported and the
//! default stays.
//!
//! Motion is the third group, also optional and off by default: `motion`
//! (`none`, `ease-out` or `spring`), `motion-duration-ms` (0 to 1000),
//! `motion-bounce` (0 to 1) and, in a `[card]` table, `enter` and `exit`
//! (`slide`, `fade`, `scale` or `drop`). With `motion` absent or `none`
//! nothing moves and nothing is built for it. Out-of-range numbers are
//! refused at the grammar and clamped again here, so a hand-edited
//! fragment cannot ask for an hour-long slide.
//!
//! Card styles and typography are the fourth group, optional like the rest
//! and off by default: in `[card]`, `mark` (`none`, `stripe`, `dot` or
//! `badge`), `shape` (`rounded` or `pill`) and `app-accent` (a boolean:
//! each app's cards carry a color hashed from its name); in `[title]`,
//! `weight` (100 to 900) and `spacing` (-2 to 8 px per character) for the
//! summary line. See [`style`].
//!
//! The `font` key names the look's UI family. There is no fontconfig in
//! this stack: the daemon draws with `--font PATH`, else the first usable
//! well-known file, as the bar does. The key is validated and kept (a
//! later ticket may resolve it); v1 documents that it does not pick the
//! file.

use std::path::{Path, PathBuf};

pub mod style;
pub use style::{CardMark, CardShape};

use scootui::color::Color;
use scootui::motion::{Curve, DEFAULT_BOUNCE, DEFAULT_DURATION_MS, Timing};
use scootui::theme::Theme;

/// The default font size, in logical pixels: the contract's 10 pt
/// `popup-size` default (`clamp(1, 256, (pt*4+1)/3)`).
pub const DEFAULT_FONT_SIZE_PX: f32 = 13.0;
/// The default card corner radius, in logical pixels.
pub const DEFAULT_RADIUS: u32 = 8;
/// The default icon theme, when the fragment names none: what the
/// looks resolve to (`Adwaita`).
pub const DEFAULT_ICON_THEME: &str = "Adwaita";

pub use scootui::effects::{FillStyle, ShadowStyle};

/// How a card arrives (`card.enter`) or leaves (`card.exit`). Every style
/// fades the card with the motion; the style adds a move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardMotion {
    /// Slides in from the edge the stack is anchored to.
    Slide,
    /// Fades only.
    Fade,
    /// Grows from a smaller card (shrinks on the way out).
    Scale,
    /// Drops in from above.
    Drop,
}

impl CardMotion {
    /// The style a `card.enter` or `card.exit` value names, if it names one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "slide" => Some(Self::Slide),
            "fade" => Some(Self::Fade),
            "scale" => Some(Self::Scale),
            "drop" => Some(Self::Drop),
            _ => None,
        }
    }
}

/// The look's motion tokens as read from the fragment, before they are
/// clamped into a [`Timing`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionTokens {
    /// `motion`: `none` (the default) moves nothing.
    pub curve: Curve,
    /// `motion-duration-ms`.
    pub duration_ms: u32,
    /// `motion-bounce`, the spring's overshoot.
    pub bounce: f32,
    /// `card.enter`.
    pub enter: CardMotion,
    /// `card.exit`.
    pub exit: CardMotion,
}

impl Default for MotionTokens {
    fn default() -> Self {
        Self {
            curve: Curve::None,
            duration_ms: DEFAULT_DURATION_MS,
            bounce: DEFAULT_BOUNCE,
            enter: CardMotion::Slide,
            exit: CardMotion::Fade,
        }
    }
}

impl MotionTokens {
    /// The clamped timing these tokens ask for: [`Timing::NONE`] for
    /// `motion = none` or a zero duration.
    pub fn timing(&self) -> Timing {
        Timing::new(self.curve, self.duration_ms, self.bounce)
    }
}

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
    /// The depth under each card (`shadow`).
    pub shadow: ShadowStyle,
    /// How the card fill is laid down (`fill`).
    pub fill: FillStyle,
    /// How cards move: arriving, leaving and restacking.
    pub motion: MotionTokens,
    /// The accent marker on each card (`card.mark`).
    pub mark: CardMark,
    /// The card outline (`card.shape`).
    pub shape: CardShape,
    /// Whether each app's cards carry a color from its name
    /// (`card.app-accent`).
    pub app_accent: bool,
    /// The summary's weight (`title.weight`, 100 to 900).
    pub title_weight: u16,
    /// Extra pixels after each summary character (`title.spacing`, -2 to
    /// 8).
    pub title_spacing: i32,
}

impl Look {
    /// The corner radius cards are drawn with: `configured` (the config's
    /// `radius`) for a rounded card, [`style::PILL_RADIUS`] for a pill.
    /// Every paint cuts it back to half the card's own size, so a short
    /// card under a pill look is a true pill. The one place the outline
    /// is chosen: paint, shadow and its corner tables all read it.
    pub fn card_radius(&self, configured: u32) -> u32 {
        match self.shape {
            CardShape::Rounded => configured,
            CardShape::Pill => style::PILL_RADIUS,
        }
    }

    /// How many times a bold summary run is struck
    /// ([`style::title_strikes`]).
    pub fn title_strikes(&self) -> u8 {
        style::title_strikes(self.title_weight)
    }

    /// The summary's letter spacing in logical pixels per character.
    pub fn title_tracking(&self) -> f32 {
        self.title_spacing as f32
    }

    /// The card fill's top and bottom colors ([`FillStyle::stops`] over
    /// `surface`): the light is at the top whatever the look's polarity,
    /// and the shift is small enough that the text keeps its contrast
    /// across the card.
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
            border: Theme::default().dim,
            font_size_px: DEFAULT_FONT_SIZE_PX,
            opacity: 1.0,
            radius: DEFAULT_RADIUS,
            font_family: String::new(),
            icon_theme: DEFAULT_ICON_THEME.to_string(),
            shadow: ShadowStyle::None,
            fill: FillStyle::Solid,
            motion: MotionTokens::default(),
            mark: CardMark::None,
            shape: CardShape::Rounded,
            app_accent: false,
            title_weight: style::DEFAULT_TITLE_WEIGHT,
            title_spacing: 0,
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

/// The look state's directory: `$SCOOT_LOOK_STATE` (a private session's),
/// else `$XDG_STATE_HOME/scoot`, else `~/.local/state/scoot`. `None` when
/// no home is usable: there is then no state link, only the defaults.
/// The daemon resolves this once and hands it to every reload, so tests
/// pass their own.
pub fn state_dir() -> Option<PathBuf> {
    scoot_look::state::state_dir_from_env(None)
}

/// Re-reads the look layer for a reload: `Ok(None)` when no layer is
/// there (no state dir, or nothing at its path), `Err` for a
/// present-but-bad fragment (unreadable, or TOML that does not parse at
/// all), so the caller keeps the last good look. A refused key still
/// keeps its default with a warning, as at startup: only a fragment that
/// is not a table at all refuses the reload. `state` is the look state's
/// directory ([`state_dir`]); tests pass their own.
pub fn reload_layer(state: Option<&Path>) -> Result<Option<(Look, Vec<String>)>, String> {
    let Some(dir) = state else {
        return Ok(None);
    };
    let path = dir.join("look").join("notify.toml");
    let text = match scoot_look::state::read_capped(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot read {} ({error}): keeping the running look",
                path.display()
            ));
        }
    };
    let table: toml::Table = text.parse().map_err(|error| {
        format!(
            "{} does not parse ({error}): keeping the running look",
            path.display()
        )
    })?;
    let mut look = Look::default();
    let mut refused = Vec::new();
    apply_table(&mut look, &table, &mut refused);
    let mut notes = Vec::new();
    for note in refused {
        notes.push(format!("{}: {note}", path.display()));
    }
    Ok(Some((look, notes)))
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
    apply_table(&mut look, &table, &mut notes);
    (look, notes)
}

/// Merges one parsed fragment table over `look`, noting refusals.
fn apply_table(look: &mut Look, table: &toml::Table, notes: &mut Vec<String>) {
    for (key, value) in table {
        if key == "colors" {
            let Some(colors) = value.as_table() else {
                notes.push(String::from("`colors` is not a table: skipped"));
                continue;
            };
            for (name, color) in colors {
                apply(look_color(look, name), "colors", name, color, notes);
            }
            continue;
        }
        if key == "icons" {
            let Some(icons) = value.as_table() else {
                notes.push(String::from("`icons` is not a table: skipped"));
                continue;
            };
            for (name, icon) in icons {
                apply(look_icons(look, name), "icons", name, icon, notes);
            }
            continue;
        }
        if key == "card" {
            let Some(card) = value.as_table() else {
                notes.push(String::from("`card` is not a table: skipped"));
                continue;
            };
            for (name, style) in card {
                apply(look_card(look, name), "card", name, style, notes);
            }
            continue;
        }
        if key == "title" {
            let Some(title) = value.as_table() else {
                notes.push(String::from("`title` is not a table: skipped"));
                continue;
            };
            for (name, style) in title {
                apply(look_title(look, name), "title", name, style, notes);
            }
            continue;
        }
        apply(look_dotless(look, key), "", key, value, notes);
    }
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
    Shadow(&'a mut ShadowStyle),
    Fill(&'a mut FillStyle),
    Motion(&'a mut Curve),
    MotionMs(&'a mut u32),
    MotionBounce(&'a mut f32),
    CardMotion(&'a mut CardMotion),
    Mark(&'a mut CardMark),
    Shape(&'a mut CardShape),
    Flag(&'a mut bool),
    TitleWeight(&'a mut u16),
    TitleSpacing(&'a mut i32),
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
            Slot::Shadow(slot) => match value.as_str().and_then(ShadowStyle::parse) {
                Some(style) => *slot = style,
                None => notes.push(format!(
                    "`{dotted}` is not a shadow style (none, soft, deep or glow): default applies"
                )),
            },
            Slot::Fill(slot) => match value.as_str().and_then(FillStyle::parse) {
                Some(style) => *slot = style,
                None => notes.push(format!(
                    "`{dotted}` is not a fill style (solid or gradient): default applies"
                )),
            },
            Slot::Motion(slot) => match value.as_str().and_then(Curve::parse) {
                Some(curve) => *slot = curve,
                None => notes.push(format!(
                    "`{dotted}` is not a motion (none, ease-out or spring): default applies"
                )),
            },
            Slot::MotionMs(slot) => match value.as_integer() {
                // The grammar bounds it; the clamp keeps a caller that
                // skipped the grammar from asking for an unbounded wait.
                Some(ms) => *slot = ms.clamp(0, i64::from(scootui::motion::MAX_DURATION_MS)) as u32,
                None => notes.push(format!(
                    "`{dotted}` is not a duration in milliseconds: default applies"
                )),
            },
            Slot::MotionBounce(slot) => {
                let bounce = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|int| int as f64));
                match bounce {
                    Some(bounce) if bounce.is_finite() => *slot = bounce.clamp(0.0, 1.0) as f32,
                    _ => notes.push(format!("`{dotted}` is not a bounce: default applies")),
                }
            }
            Slot::CardMotion(slot) => match value.as_str().and_then(CardMotion::parse) {
                Some(style) => *slot = style,
                None => notes.push(format!(
                    "`{dotted}` is not a card motion (slide, fade, scale or drop): default applies"
                )),
            },
            Slot::Mark(slot) => match value.as_str().and_then(CardMark::parse) {
                Some(mark) => *slot = mark,
                None => notes.push(format!(
                    "`{dotted}` is not a card mark (none, stripe, dot or badge): default applies"
                )),
            },
            Slot::Shape(slot) => match value.as_str().and_then(CardShape::parse) {
                Some(shape) => *slot = shape,
                None => notes.push(format!(
                    "`{dotted}` is not a card shape (rounded or pill): default applies"
                )),
            },
            Slot::Flag(slot) => match value.as_bool() {
                Some(flag) => *slot = flag,
                None => notes.push(format!("`{dotted}` is not true or false: default applies")),
            },
            // The grammar bounds both; the clamps keep a caller that
            // skipped it from asking for an absurd weight or spacing.
            Slot::TitleWeight(slot) => match value.as_integer() {
                Some(weight) => *slot = weight.clamp(100, 900) as u16,
                None => notes.push(format!("`{dotted}` is not a weight: default applies")),
            },
            Slot::TitleSpacing(slot) => match value.as_integer() {
                Some(spacing) => *slot = spacing.clamp(-2, 8) as i32,
                None => notes.push(format!(
                    "`{dotted}` is not a spacing in pixels: default applies"
                )),
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

/// The slot for a dotless top-level key, if it is one of the nine.
fn look_dotless<'a>(look: &'a mut Look, key: &str) -> Option<Slot<'a>> {
    match key {
        "font" => Some(Slot::FontFamily(&mut look.font_family)),
        "font-size" => Some(Slot::FontSize(&mut look.font_size_px)),
        "opacity" => Some(Slot::Opacity(&mut look.opacity)),
        "radius" => Some(Slot::Radius(&mut look.radius)),
        "shadow" => Some(Slot::Shadow(&mut look.shadow)),
        "fill" => Some(Slot::Fill(&mut look.fill)),
        "motion" => Some(Slot::Motion(&mut look.motion.curve)),
        "motion-duration-ms" => Some(Slot::MotionMs(&mut look.motion.duration_ms)),
        "motion-bounce" => Some(Slot::MotionBounce(&mut look.motion.bounce)),
        _ => None,
    }
}

/// The slot for a `[card]` key: `enter`, `exit`, `mark`, `shape` and
/// `app-accent`.
fn look_card<'a>(look: &'a mut Look, name: &str) -> Option<Slot<'a>> {
    match name {
        "enter" => Some(Slot::CardMotion(&mut look.motion.enter)),
        "exit" => Some(Slot::CardMotion(&mut look.motion.exit)),
        "mark" => Some(Slot::Mark(&mut look.mark)),
        "shape" => Some(Slot::Shape(&mut look.shape)),
        "app-accent" => Some(Slot::Flag(&mut look.app_accent)),
        _ => None,
    }
}

/// The slot for a `[title]` key: `weight` and `spacing`.
fn look_title<'a>(look: &'a mut Look, name: &str) -> Option<Slot<'a>> {
    match name {
        "weight" => Some(Slot::TitleWeight(&mut look.title_weight)),
        "spacing" => Some(Slot::TitleSpacing(&mut look.title_spacing)),
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
