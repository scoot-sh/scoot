//! A module's icon from its config keys: `icon` (a glyph), `icon-path`
//! (SVG path data, with `icon-viewbox`) or `icon-image` (a PNG, in a build
//! with the `icon-image` feature). At most one; each refusal names the key.
//! The clock is the only module with one so far; the modules that follow
//! (button, volume, network, battery) take the same three keys through
//! this function.

use std::path::Path;
use std::sync::Arc;

use super::{ClockFile, Error, value};
use crate::icon::path::{Vector, ViewBox};
use crate::icon::{Art, Icon};

/// The clock's icon, if its section gives one.
pub(super) fn clock(path: &Path, file: &ClockFile) -> Result<Option<Icon>, Error> {
    #[cfg(feature = "icon-image")]
    let image = file.icon_image.as_deref();
    #[cfg(not(feature = "icon-image"))]
    let image: Option<&str> = None;
    let given = [
        ("clock.icon", file.icon.is_some()),
        ("clock.icon-path", file.icon_path.is_some()),
        ("clock.icon-image", image.is_some()),
    ];
    let mut set = given
        .iter()
        .filter(|(_, is_set)| *is_set)
        .map(|(key, _)| *key);
    if let (Some(first), Some(second)) = (set.next(), set.next()) {
        return Err(value(
            path,
            second,
            format_args!("a clock shows one icon: it cannot be set with {first}"),
        ));
    }
    if let Some(text) = &file.icon {
        return parse_glyph(text)
            .map(|c| Some(Icon::Glyph(c)))
            .map_err(|message| value(path, "clock.icon", format_args!("{message}")));
    }
    if file.icon_viewbox.is_some() && file.icon_path.is_none() {
        return Err(value(
            path,
            "clock.icon-viewbox",
            format_args!("is the viewbox of clock.icon-path, which is not set"),
        ));
    }
    if let Some(d) = &file.icon_path {
        let view = match &file.icon_viewbox {
            None => ViewBox::default(),
            Some(text) => ViewBox::parse(text).map_err(|error| {
                value(
                    path,
                    "clock.icon-viewbox",
                    format_args!(
                        "takes four numbers, `min-x min-y width height` (such as \"0 0 24 24\"): {error}"
                    ),
                )
            })?,
        };
        let vector = Vector::parse(d, view).map_err(|error| {
            value(
                path,
                "clock.icon-path",
                format_args!("not usable SVG path data: {error}"),
            )
        })?;
        return Ok(Some(Icon::Art(Art::Vector(Arc::new(vector)))));
    }
    #[cfg(feature = "icon-image")]
    if let Some(text) = image {
        let file = Path::new(text);
        if !file.is_absolute() {
            return Err(value(
                path,
                "clock.icon-image",
                format_args!(
                    "takes an absolute path to a PNG file, not `{}`",
                    text.escape_debug()
                ),
            ));
        }
        let image = crate::icon::image::load(file).map_err(|error| {
            value(
                path,
                "clock.icon-image",
                format_args!("cannot use {}: {error}", file.display()),
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
