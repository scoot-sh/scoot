//! A resolved look: every token concrete, every default applied.
//!
//! [`resolve`](crate::resolve::resolve) layers a source look over its
//! `extends` chain (child wins, [`RawFonts`] `files`/`nix` carriers and
//! [`apps`](crate::schema::RawLook::apps) merge per key, `[wallpaper]` swaps
//! as one unit), then fills the defaults the schema promises: optional
//! palette roles from required ones, font sizes from `size`, opacities from
//! `1.0`, `ring-inactive-width` from `ring-width`, `cursor.accent` from
//! `focus`, `wallpaper.fill` from `background`.

use std::collections::BTreeMap;

use crate::grammar::GrammarError;
use crate::schema::{Effects, Icons, Night, Polarity, RawLook, Terminal, Wallpaper};

/// A look with every token concrete: the input to contrast, rendering and
/// every loader.
#[derive(Debug, Clone)]
pub struct ResolvedLook {
    pub name: String,
    pub title: String,
    pub description: String,
    pub polarity: Polarity,
    pub license: Option<String>,
    pub credits: Vec<String>,
    pub preview: Option<String>,
    pub palette: Palette,
    pub terminal: Option<Vec<String>>,
    pub fonts: Fonts,
    pub opacity: Opacity,
    pub shape: Shape,
    /// The depth tokens; a key is `None` until the look sets it.
    pub effects: Effects,
    pub cursor: Cursor,
    pub wallpaper: Option<Wallpaper>,
    pub icons_theme: String,
    pub night_temperature: Option<i64>,
    pub apps: BTreeMap<String, BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Clone)]
pub struct Palette {
    pub background: String,
    pub foreground: String,
    pub dim: String,
    pub accent: String,
    pub urgent: String,
    pub surface: String,
    pub hover: String,
    pub focus: String,
    pub border: String,
    pub accent_fg: String,
    pub selection: String,
    pub selection_text: String,
    pub r#match: String,
}

#[derive(Debug, Clone)]
pub struct Fonts {
    pub ui: String,
    pub sans: String,
    pub mono: String,
    pub size: f64,
    pub popup_size: f64,
    pub app_size: f64,
    pub terminal_size: f64,
    pub files: BTreeMap<String, String>,
    pub nix: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Opacity {
    pub bar: f64,
    pub popups: f64,
    pub terminal: f64,
}

#[derive(Debug, Clone)]
pub struct Shape {
    pub radius: i64,
    pub ring_width: i64,
    pub ring_inactive_width: i64,
}

#[derive(Debug, Clone)]
pub struct Cursor {
    pub theme: String,
    pub size: i64,
    /// `#rrggbb` or `"none"`.
    pub accent: String,
    pub body: Option<String>,
}

/// Points to pixels by the bar's rule (`scootbar.nix:97-100`):
/// `pixels = clamp(1, 256, (pt * 4 + 1) / 3)`.
pub fn pt_to_px(pt: f64) -> u32 {
    ((pt * 4.0 + 1.0) / 3.0).floor().clamp(1.0, 256.0) as u32
}

#[derive(Debug, Default)]
pub(crate) struct Partial {
    pub title: Option<String>,
    pub description: Option<String>,
    pub polarity: Option<Polarity>,
    pub license: Option<String>,
    pub credits: Vec<String>,
    pub preview: Option<String>,
    pub palette: RawPaletteAccum,
    pub terminal: Option<Terminal>,
    pub fonts: RawFontsAccum,
    pub opacity: OpacityAccum,
    pub shape: RawShapeAccum,
    pub effects: Effects,
    pub cursor: RawCursorAccum,
    pub wallpaper: Option<Wallpaper>,
    pub icons: Option<Icons>,
    pub night: Option<Night>,
    pub apps: BTreeMap<String, BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Default)]
pub(crate) struct RawPaletteAccum {
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub dim: Option<String>,
    pub accent: Option<String>,
    pub urgent: Option<String>,
    pub surface: Option<String>,
    pub hover: Option<String>,
    pub focus: Option<String>,
    pub border: Option<String>,
    pub accent_fg: Option<String>,
    pub selection: Option<String>,
    pub selection_text: Option<String>,
    pub r#match: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct RawFontsAccum {
    pub ui: Option<String>,
    pub sans: Option<String>,
    pub mono: Option<String>,
    pub size: Option<f64>,
    pub popup_size: Option<f64>,
    pub app_size: Option<f64>,
    pub terminal_size: Option<f64>,
    pub files: BTreeMap<String, String>,
    pub nix: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
pub(crate) struct OpacityAccum {
    pub bar: Option<f64>,
    pub popups: Option<f64>,
    pub terminal: Option<f64>,
}

#[derive(Debug, Default)]
pub(crate) struct RawShapeAccum {
    pub radius: Option<i64>,
    pub ring_width: Option<i64>,
    pub ring_inactive_width: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct RawCursorAccum {
    pub theme: Option<String>,
    pub size: Option<i64>,
    pub accent: Option<String>,
    pub body: Option<String>,
}

impl Partial {
    pub(crate) fn apply(&mut self, raw: &RawLook) {
        if raw.title.is_some() {
            self.title = raw.title.clone();
        }
        if raw.description.is_some() {
            self.description = raw.description.clone();
        }
        if raw.polarity.is_some() {
            self.polarity = raw.polarity;
        }
        if raw.license.is_some() {
            self.license = raw.license.clone();
        }
        if !raw.credits.is_empty() {
            self.credits = raw.credits.clone();
        }
        if raw.preview.is_some() {
            self.preview = raw.preview.clone();
        }
        let p = &raw.palette;
        macro_rules! opt {
            ($accum:expr, $field:ident) => {
                if p.$field.is_some() {
                    $accum.$field = p.$field.clone();
                }
            };
        }
        opt!(self.palette, background);
        opt!(self.palette, foreground);
        opt!(self.palette, dim);
        opt!(self.palette, accent);
        opt!(self.palette, urgent);
        opt!(self.palette, surface);
        opt!(self.palette, hover);
        opt!(self.palette, focus);
        opt!(self.palette, border);
        opt!(self.palette, accent_fg);
        opt!(self.palette, selection);
        opt!(self.palette, selection_text);
        opt!(self.palette, r#match);
        if raw.terminal.is_some() {
            self.terminal = raw.terminal.clone();
        }
        let f = &raw.fonts;
        if f.ui.is_some() {
            self.fonts.ui = f.ui.clone();
        }
        if f.sans.is_some() {
            self.fonts.sans = f.sans.clone();
        }
        if f.mono.is_some() {
            self.fonts.mono = f.mono.clone();
        }
        if f.size.is_some() {
            self.fonts.size = f.size;
        }
        if f.popup_size.is_some() {
            self.fonts.popup_size = f.popup_size;
        }
        if f.app_size.is_some() {
            self.fonts.app_size = f.app_size;
        }
        if f.terminal_size.is_some() {
            self.fonts.terminal_size = f.terminal_size;
        }
        self.fonts.files.extend(f.files.clone());
        self.fonts.nix.extend(f.nix.clone());
        if raw.opacity.bar.is_some() {
            self.opacity.bar = raw.opacity.bar;
        }
        if raw.opacity.popups.is_some() {
            self.opacity.popups = raw.opacity.popups;
        }
        if raw.opacity.terminal.is_some() {
            self.opacity.terminal = raw.opacity.terminal;
        }
        if raw.shape.radius.is_some() {
            self.shape.radius = raw.shape.radius;
        }
        if raw.shape.ring_width.is_some() {
            self.shape.ring_width = raw.shape.ring_width;
        }
        if raw.shape.ring_inactive_width.is_some() {
            self.shape.ring_inactive_width = raw.shape.ring_inactive_width;
        }
        if raw.effects.shadow.is_some() {
            self.effects.shadow = raw.effects.shadow.clone();
        }
        if raw.effects.fill.is_some() {
            self.effects.fill = raw.effects.fill.clone();
        }
        if raw.cursor.theme.is_some() {
            self.cursor.theme = raw.cursor.theme.clone();
        }
        if raw.cursor.size.is_some() {
            self.cursor.size = raw.cursor.size;
        }
        if raw.cursor.accent.is_some() {
            self.cursor.accent = raw.cursor.accent.clone();
        }
        if raw.cursor.body.is_some() {
            self.cursor.body = raw.cursor.body.clone();
        }
        // `[wallpaper]` is the one unit: a child that sets any of it
        // replaces the whole section, so an image and a color can never mix
        // across the chain.
        if raw.wallpaper.is_some() {
            self.wallpaper = raw.wallpaper.clone();
        }
        if raw.icons.is_some() {
            self.icons = raw.icons.clone();
        }
        if raw.night.is_some() {
            self.night = raw.night.clone();
        }
        for (app, table) in &raw.apps {
            self.apps
                .entry(app.clone())
                .or_default()
                .extend(table.clone());
        }
    }

    pub(crate) fn finish(self, name: &str) -> Result<ResolvedLook, GrammarError> {
        let required = |field: &str, value: Option<String>| {
            value.ok_or_else(|| {
                GrammarError::new(
                    field,
                    format!("required token `{field}` is missing after extends"),
                )
            })
        };
        let background = required("palette.background", self.palette.background)?;
        let foreground = required("palette.foreground", self.palette.foreground)?;
        let dim = required("palette.dim", self.palette.dim)?;
        let accent = required("palette.accent", self.palette.accent)?;
        let urgent = required("palette.urgent", self.palette.urgent)?;
        let focus = self.palette.focus.clone().unwrap_or_else(|| accent.clone());
        let accent_fg = self
            .palette
            .accent_fg
            .clone()
            .unwrap_or_else(|| background.clone());
        let size = self.fonts.size.ok_or_else(|| {
            GrammarError::new(
                "fonts.size",
                "required token `fonts.size` is missing after extends",
            )
        })?;
        let ring_width = self.shape.ring_width.ok_or_else(|| {
            GrammarError::new(
                "shape.ring-width",
                "required token `shape.ring-width` is missing after extends",
            )
        })?;
        Ok(ResolvedLook {
            name: name.to_string(),
            title: required("title", self.title)?,
            description: required("description", self.description)?,
            polarity: self.polarity.ok_or_else(|| {
                GrammarError::new(
                    "polarity",
                    "required token `polarity` is missing after extends",
                )
            })?,
            license: self.license,
            credits: self.credits,
            preview: self.preview,
            palette: Palette {
                surface: self
                    .palette
                    .surface
                    .clone()
                    .unwrap_or_else(|| background.clone()),
                hover: self.palette.hover.clone().unwrap_or_else(|| accent.clone()),
                focus: focus.clone(),
                border: self.palette.border.clone().unwrap_or_else(|| dim.clone()),
                accent_fg: accent_fg.clone(),
                selection: self
                    .palette
                    .selection
                    .clone()
                    .unwrap_or_else(|| accent.clone()),
                selection_text: self
                    .palette
                    .selection_text
                    .clone()
                    .unwrap_or_else(|| accent_fg.clone()),
                r#match: self.palette.r#match.clone().unwrap_or_else(|| {
                    self.palette.hover.clone().unwrap_or_else(|| accent.clone())
                }),
                background,
                foreground,
                dim,
                accent,
                urgent,
            },
            terminal: self.terminal.map(|t| t.colors),
            fonts: Fonts {
                ui: required("fonts.ui", self.fonts.ui)?,
                sans: required("fonts.sans", self.fonts.sans)?,
                mono: required("fonts.mono", self.fonts.mono)?,
                popup_size: self.fonts.popup_size.unwrap_or(size),
                app_size: self.fonts.app_size.unwrap_or(12.0),
                terminal_size: self.fonts.terminal_size.unwrap_or(size),
                files: self.fonts.files,
                nix: self.fonts.nix,
                size,
            },
            opacity: Opacity {
                bar: self.opacity.bar.unwrap_or(1.0),
                popups: self.opacity.popups.unwrap_or(1.0),
                terminal: self.opacity.terminal.unwrap_or(1.0),
            },
            shape: Shape {
                radius: self.shape.radius.unwrap_or(0),
                ring_inactive_width: self.shape.ring_inactive_width.unwrap_or(ring_width),
                ring_width,
            },
            effects: self.effects,
            cursor: Cursor {
                theme: required("cursor.theme", self.cursor.theme)?,
                size: self.cursor.size.unwrap_or(24),
                accent: self.cursor.accent.clone().unwrap_or_else(|| focus.clone()),
                body: self.cursor.body,
            },
            wallpaper: self.wallpaper,
            icons_theme: self
                .icons
                .map(|i| i.theme)
                .unwrap_or_else(|| "Adwaita".to_string()),
            night_temperature: self.night.map(|n| n.temperature),
            apps: self.apps,
        })
    }
}

impl ResolvedLook {
    /// The wallpaper letterbox color defaults to the background (section 4.3:
    /// values a look derives inside itself at render time are plain keys by
    /// then). Runs after [`Partial::finish`], which cannot see the palette
    /// while filling the wallpaper.
    pub(crate) fn fill_wallpaper_default(&mut self) {
        if let Some(w) = &mut self.wallpaper
            && w.fill.is_none()
        {
            w.fill = Some(self.palette.background.clone());
        }
    }
}

#[cfg(test)]
mod tests;
