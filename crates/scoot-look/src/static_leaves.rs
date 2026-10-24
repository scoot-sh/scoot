//! The terminal static leaves (ticket `look-leaves-terminal`): the look's
//! own Helix theme, btop theme, starship config and Neovim colorscheme,
//! copied verbatim from `apps/` in the source look into the rendered look.
//!
//! Unlike every other fragment these are never rendered from tokens: the
//! tool copies the author's file byte for byte. That is also why they are
//! trust-gated: the formats are not generated, so a hostile file could
//! carry anything (starship custom commands run a shell, Lua runs in the
//! editor), and an installed
//! (someone else's) look drops them unless `--trust` is given. The check
//! here proves only shape (size, text, well-formed TOML for the TOML
//! leaves, btop theme lines, a `colors_name`-setting Lua colorscheme):
//! presence, provenance
//! (the `extends` walk) and trust live with the callers, who own the
//! filesystem.
//!
//! The file names are the design's section 8.3 (`apps/helix.toml`,
//! `apps/btop.theme`, `apps/starship.toml`, `apps/nvim.lua`), kept as
//! `apps/*` inside the rendered look (the fragment-names table).
//! `apps/foot.ini` and `apps/regreet.css` are other tickets' (foot
//! generates from `[terminal]`; the greeter is static at build time).

use crate::grammar::GrammarError;

/// One static leaf: the path inside the source look and the same path
/// inside the rendered look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticLeaf {
    /// `apps/helix.toml`, relative to the source look's directory.
    pub path: &'static str,
    /// How the bytes are checked.
    pub kind: StaticKind,
}

/// The check a leaf's bytes must pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticKind {
    /// Parses as TOML (the Helix theme, the starship config).
    Toml,
    /// btop's `theme[name]="value"` lines (checked against its 1.4.7
    /// theme reader: `#` comments, `theme[` entries).
    BtopTheme,
    /// A Neovim colorscheme: Lua source setting `colors_name` (so a
    /// wrong file at the path refuses instead of landing silently),
    /// with no carriage returns. The watcher block inside re-sources
    /// the file on change; the check does not run it.
    Lua,
}

/// Every static leaf this ticket owns, in render order.
pub const STATIC_LEAVES: &[StaticLeaf] = &[
    StaticLeaf {
        path: "apps/helix.toml",
        kind: StaticKind::Toml,
    },
    StaticLeaf {
        path: "apps/btop.theme",
        kind: StaticKind::BtopTheme,
    },
    StaticLeaf {
        path: "apps/starship.toml",
        kind: StaticKind::Toml,
    },
    StaticLeaf {
        path: "apps/nvim.lua",
        kind: StaticKind::Lua,
    },
];

/// The leaf for a rendered relative path, if one owns it.
pub fn leaf_for(path: &str) -> Option<StaticLeaf> {
    STATIC_LEAVES.iter().find(|leaf| leaf.path == path).copied()
}

/// The largest static leaf accepted: the same 64 KiB as every other
/// look file ([`MAX_FILE`](crate::state::MAX_FILE)). Theme files are
/// small (the shipped ones are under 3 KiB); anything larger is not a
/// theme but a mistake.
pub const MAX_STATIC_BYTES: u64 = crate::state::MAX_FILE;

/// Checks one leaf's bytes: the size cap, text without NUL, then the
/// kind's shape. A hand-built caller can bypass the parse gate, so this
/// runs on the bytes about to be written, not only at validation.
pub fn check_static_leaf(path: &str, bytes: &[u8]) -> Result<(), GrammarError> {
    let leaf = leaf_for(path).ok_or_else(|| GrammarError::new(path, "not a static leaf"))?;
    if bytes.len() as u64 > MAX_STATIC_BYTES {
        return Err(GrammarError::new(
            path,
            format!("larger than {MAX_STATIC_BYTES} bytes"),
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| GrammarError::new(path, "file is not valid UTF-8"))?;
    if text.contains('\0') {
        return Err(GrammarError::new(path, "NUL is refused everywhere"));
    }
    match leaf.kind {
        StaticKind::Toml => {
            text.parse::<toml::Table>().map_err(|error| {
                GrammarError::new(path, format!("does not parse as TOML: {error}"))
            })?;
        }
        StaticKind::BtopTheme => check_btop_theme(path, text)?,
        StaticKind::Lua => check_lua_colorscheme(path, text)?,
    }
    Ok(())
}

/// Checks a Neovim colorscheme: Lua source with no carriage returns
/// (a stray CR would land verbatim in the render) that sets
/// `colors_name`, so a wrong file at the path refuses instead of
/// landing as a colorscheme that never names itself.
fn check_lua_colorscheme(path: &str, text: &str) -> Result<(), GrammarError> {
    if text.contains('\r') {
        return Err(GrammarError::new(
            path,
            "carriage returns are refused in a Neovim colorscheme",
        ));
    }
    if !text.contains("colors_name") {
        return Err(GrammarError::new(
            path,
            "a Neovim colorscheme sets colors_name",
        ));
    }
    Ok(())
}
/// Checks a btop theme: blank lines and `#` comments aside, every line is
/// `theme[<name>]=<value>` on one line, with a non-empty name and value.
/// At least one entry is required: a comments-only file is not a theme.
fn check_btop_theme(path: &str, text: &str) -> Result<(), GrammarError> {
    // A carriage return would write a broken line into the render
    // (`lines` below strips it silently, so check first).
    if text.contains('\r') {
        return Err(GrammarError::new(
            path,
            "carriage returns are refused in a line-based theme",
        ));
    }
    let mut entries = 0;
    for (index, raw) in text.lines().enumerate() {
        if raw.chars().any(|c| c.is_control() && c != '\t') {
            return Err(GrammarError::new(
                path,
                format!("line {} carries a control character", index + 1),
            ));
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("theme[") else {
            return Err(GrammarError::new(
                path,
                format!("line {} is not a theme entry: {line}", index + 1),
            ));
        };
        let Some((name, value)) = rest.split_once('=') else {
            return Err(GrammarError::new(
                path,
                format!("line {} is not a theme entry: {line}", index + 1),
            ));
        };
        if !name.ends_with(']') || name.len() < 2 || value.is_empty() {
            return Err(GrammarError::new(
                path,
                format!("line {} is not a theme entry: {line}", index + 1),
            ));
        }
        entries += 1;
    }
    if entries == 0 {
        return Err(GrammarError::new(path, "no theme entries: only comments"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
