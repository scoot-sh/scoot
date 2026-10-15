//! Pure renderers for the third-party daemon leaves (design section 3.3,
//! ticket `look-leaves-daemons`): mako, wob, the fcitx5 classic UI theme
//! and the night light, from a resolved look.
//!
//! Each renderer re-checks every value it interpolates: the crate never
//! trusts the tool, not even itself, so a hand-built `ResolvedLook` past
//! the parse gate still refuses here instead of writing a hostile line.
//! All three config formats are line-based with keys that run code
//! elsewhere (mako's `exec` actions and `include`, fuzzel-style launch
//! prefixes, foot's `shell`), so the value grammar plus each format's own
//! quoting carry the trust boundary (design section 3.4). Every value
//! below is a `#rrggbb` color the grammar checked and the renderer
//! re-checks; nothing else user-controlled is interpolated.
//!
//! What each fragment is:
//!
//! - `mako`: the look's notification colors as a mako config (mako 1.11.0:
//!   `key=value` lines, `#` comments, criteria sections). The user's own
//!   mako config names it first with `include=` (an absolute path or
//!   `~/`-prefixed: the only shapes mako accepts, checked against its
//!   1.11.0 `config.c`), then sets its own keys, which win: later
//!   assignments override earlier ones, and the included file parses
//!   inline. `makoctl reload` re-parses and restyles on screen.
//! - `wob.ini`: a complete wob config (wob 0.16 reads one file from `-c`,
//!   no include mechanism, colors as `RRGGBBAA` without `#`). No `timeout`
//!   line: the hide timeout is desktop configuration, not a look token,
//!   so the launcher prepends it ahead of every section (a duplicate key
//!   later in the file would land inside `[style.muted]`, which refuses
//!   unknown keys). A bare `wob -c` of the fragment gets wob's own 1000 ms
//!   default.
//! - `fcitx5/theme.conf`: the look as an fcitx5 classic UI theme
//!   (`scoot-look`, a name no system theme takes), colors as `#rrggbb`
//!   (what fcitx5's `Color::setFromString` parses). A partial theme: every
//!   key it leaves out keeps fcitx5's own default. Applied with D-Bus
//!   `ReloadAddonConfig "classicui"` (`fcitx5-remote -r` does not restyle
//!   it). The `classicui.conf` pointing `Theme=`/`DarkTheme=` at it is
//!   static and lives beside the theme, not in the render.
//! - `nightlight`: one `temperature=` line (1000 to 10000 Kelvin) for the
//!   unit wrapper to read at start; no look temperature means no file.
//!
//! There are no `[apps.mako]`, `[apps.wob]` or `[apps.fcitx5]` overrides
//! in v1: the user's per-key settings live in their own files and Nix
//! options (`notif.settings`-style) and win by file order, so a look
//! never needs to name them.

use crate::grammar::{GrammarError, is_hex_color};
use crate::resolved::ResolvedLook;

/// One `#rrggbb` color, re-checked at render time. The parse gate already
/// refused anything else; a library caller can hand-build a `ResolvedLook`
/// past it, so the renderer checks again instead of writing the value raw
/// into a line-based format.
fn hex<'a>(color: &'a str, field: &str) -> Result<&'a str, GrammarError> {
    if is_hex_color(color) {
        Ok(color)
    } else {
        Err(GrammarError::new(
            field,
            "must be exactly #rrggbb to render into a leaf config",
        ))
    }
}

/// One `#rrggbb` color as wob's `RRGGBBAA` (opaque): the `#` off, `ff` on.
/// wob's `wob_color_from_rgba_string` takes 6 or 8 bare hex digits.
fn hex_a(color: &str, field: &str) -> Result<String, GrammarError> {
    Ok(format!("{}ff", &hex(color, field)?[1..]))
}

/// The look name inside a `#` comment: quoted the way the TOML renderer
/// quotes, so a newline in a hand-built name stays an escaped two
/// characters on one line, never a second line (which in mako could open
/// a criteria section).
fn comment_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('"');
    for c in name.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn header(name: &str, what: &str) -> String {
    format!(
        "# Rendered from look {} by scoot-look ({what}). Do not edit: re-rendered on every apply.\n",
        comment_name(name)
    )
}

/// Render the mako fragment: the notification colors as a mako config.
/// Mirrors the mapping the Nix module used to inline (layer, the four
/// global colors, do-not-disturb, the critical border); the user's own
/// `settings` merge after the `include=`, so they keep winning per key.
pub fn render_mako(look: &ResolvedLook) -> Result<String, GrammarError> {
    let p = &look.palette;
    let background = hex(&p.background, "palette.background")?;
    let foreground = hex(&p.foreground, "palette.foreground")?;
    let border = hex(&p.focus, "palette.focus")?;
    let accent = hex(&p.accent, "palette.accent")?;
    let urgent = hex(&p.urgent, "palette.urgent")?;
    let mut out = header(&look.name, "mako");
    out.push_str("layer=overlay\n");
    out.push_str(&format!("background-color={background}\n"));
    out.push_str(&format!("text-color={foreground}\n"));
    out.push_str(&format!("border-color={border}\n"));
    out.push_str(&format!("progress-color=over {accent}\n"));
    out.push_str("\n[mode=do-not-disturb]\n");
    out.push_str("invisible=1\n");
    out.push_str("\n[urgency=critical]\n");
    out.push_str(&format!("border-color={urgent}\n"));
    Ok(out)
}

/// The desktop OSD geometry every wob config carries: the values the Nix
/// module always wrote (400x40 at the bottom, 48 px margin, cap 100, no
/// text wrap). Constants, identical on Nix and non-Nix.
const WOB_GEOMETRY: &[(&str, &str)] = &[
    ("max", "100"),
    ("width", "400"),
    ("height", "40"),
    ("anchor", "bottom"),
    ("margin", "48"),
    ("overflow_mode", "nowrap"),
];

/// Render the wob fragment: a complete `wob.ini` (geometry plus the
/// look's colors and the muted style), without a `timeout` line (see the
/// module docs for why the launcher owns that line).
pub fn render_wob(look: &ResolvedLook) -> Result<String, GrammarError> {
    let p = &look.palette;
    let background = hex_a(&p.background, "palette.background")?;
    let bar = hex_a(&p.accent, "palette.accent")?;
    let border = hex_a(&p.focus, "palette.focus")?;
    let overflow_bar = hex_a(&p.urgent, "palette.urgent")?;
    let muted_bar = hex_a(&p.dim, "palette.dim")?;
    let mut out = header(&look.name, "wob.ini");
    for (key, value) in WOB_GEOMETRY {
        out.push_str(&format!("{key} = {value}\n"));
    }
    out.push_str(&format!("background_color = {background}\n"));
    out.push_str(&format!("bar_color = {bar}\n"));
    out.push_str(&format!("border_color = {border}\n"));
    out.push_str(&format!("overflow_background_color = {background}\n"));
    out.push_str(&format!("overflow_bar_color = {overflow_bar}\n"));
    out.push_str(&format!("overflow_border_color = {overflow_bar}\n"));
    out.push_str("\n[style.muted]\n");
    out.push_str(&format!("background_color = {background}\n"));
    out.push_str(&format!("bar_color = {muted_bar}\n"));
    out.push_str(&format!("border_color = {muted_bar}\n"));
    Ok(out)
}

/// Render the fcitx5 classic UI theme: the look's palette as the
/// `scoot-look` theme. Mirrors the mapping the Nix module used to
/// inline; the faces stay `Sans` at fcitx5's own sizes (look constants,
/// not tokens: no behavior change for existing users).
pub fn render_fcitx5_theme(look: &ResolvedLook) -> Result<String, GrammarError> {
    let p = &look.palette;
    let background = hex(&p.background, "palette.background")?;
    let foreground = hex(&p.foreground, "palette.foreground")?;
    let accent = hex(&p.accent, "palette.accent")?;
    let border = hex(&p.focus, "palette.focus")?;
    let mut out = header(&look.name, "fcitx5 classic UI theme");
    out.push_str(
        "[Metadata]\n\
         Name=Scoot Look\n\
         Version=1.0\n\
         Author=scoot\n\
         Description=The desktop look's palette as an fcitx5 classic UI theme.\n\
         ScaleWithDPI=True\n\
         \n\
         [InputPanel]\n\
         Font=Sans 11\n",
    );
    out.push_str(&format!("NormalColor={foreground}\n"));
    out.push_str(&format!("HighlightColor={accent}\n"));
    out.push_str(&format!("HighlightBackgroundColor={background}\n"));
    out.push_str(&format!("HighlightCandidateColor={background}\n"));
    out.push_str("\n[InputPanel/Background]\n");
    out.push_str(&format!("Color={background}\n"));
    out.push_str(&format!("BorderColor={border}\n"));
    out.push_str("\n[InputPanel/Highlight]\n");
    out.push_str(&format!("Color={accent}\n"));
    out.push_str("\n[Menu]\nFont=Sans 10\n");
    out.push_str(&format!("NormalColor={foreground}\n"));
    out.push_str("\n[Menu/Background]\n");
    out.push_str(&format!("Color={background}\n"));
    out.push_str("\n[Menu/Highlight]\n");
    out.push_str(&format!("Color={accent}\n"));
    Ok(out)
}

/// Render the night light fragment: the look's night temperature for the
/// unit wrapper to read at start. The range mirrors the option's (1000 to
/// 10000 Kelvin); a look without `[night]` has nothing to write.
pub fn render_nightlight(name: &str, temperature: i64) -> Result<String, GrammarError> {
    if !(1000..=10000).contains(&temperature) {
        return Err(GrammarError::new(
            "night.temperature",
            "must be 1000 to 10000 Kelvin to render into the nightlight fragment",
        ));
    }
    let mut out = header(name, "nightlight");
    out.push_str(&format!("temperature={temperature}\n"));
    Ok(out)
}

/// Every daemon leaf file a rendered look carries, as (relative path,
/// contents): `mako`, `wob.ini`, `fcitx5/theme.conf`, plus `nightlight`
/// when the look names a temperature. The first-party fragments stay with
/// their own renderers (they need [`RenderInputs`](crate::render::RenderInputs));
/// these need only the look. Both writers (`scoot look build` and the
/// apply cache) go through here, so validate renders what apply renders.
pub fn render_leaf_files(look: &ResolvedLook) -> Result<Vec<(String, String)>, GrammarError> {
    let mut files = vec![
        ("mako".to_string(), render_mako(look)?),
        ("wob.ini".to_string(), render_wob(look)?),
        ("fcitx5/theme.conf".to_string(), render_fcitx5_theme(look)?),
    ];
    if let Some(temperature) = look.night_temperature {
        files.push((
            "nightlight".to_string(),
            render_nightlight(&look.name, temperature)?,
        ));
    }
    Ok(files)
}

#[cfg(test)]
mod tests;
