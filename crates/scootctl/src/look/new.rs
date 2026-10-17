//! `scoot look new NAME --from BASE`: start a user-authored look.
//!
//! Writes `$XDG_DATA_HOME/scoot/user-looks/NAME/look.toml` with
//! `extends = "BASE"`, so the new look inherits every token and overrides
//! from there. BASE names a *source* look — shipped, declared or
//! user-authored (a look made with `new` chains onto the next one) — never
//! a rendered look and never an installed one. Works with no Nix: a plain
//! binary install carries the same catalog directories as plain files.

use std::path::{Path, PathBuf};

use scoot_look::catalog::{Catalog, data_home};

/// Creates the source look `name` extending `base`: validates both names,
/// resolves the base through the catalog (refusing unknown and installed
/// bases), refuses a name the catalog or the directory already holds, and
/// writes the manifest. Returns the new directory.
pub fn create_new(name: &str, base: &str, catalog: &Catalog) -> Result<PathBuf, String> {
    let parent = user_looks_dir()?;
    create_new_into(name, base, catalog, &parent)
}

/// [`create_new`] with the parent directory explicit, so tests pin it
/// without touching the process environment.
pub fn create_new_into(
    name: &str,
    base: &str,
    catalog: &Catalog,
    parent: &Path,
) -> Result<PathBuf, String> {
    if !scoot_look::grammar::is_name(name) {
        return Err(format!("`{name}` is not a look name ([a-z0-9-]{{1,32}})"));
    }
    if !scoot_look::grammar::is_name(base) {
        return Err(format!("`{base}` is not a look name ([a-z0-9-]{{1,32}})"));
    }
    let entry = catalog.find(base).ok_or_else(|| {
        let mut names: Vec<&str> = catalog
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        names.sort();
        if names.is_empty() {
            format!(
                "unknown base look `{base}`: {}",
                super::empty_catalog_hint()
            )
        } else {
            format!(
                "unknown base look `{base}` (available: {})",
                names.join(", ")
            )
        }
    })?;
    if !entry.origin.extendable() {
        return Err(format!(
            "`{base}` is an installed look and can be applied but never extended"
        ));
    }
    if catalog.find(name).is_some() {
        return Err(format!(
            "`{name}` is already a look: pick another name (nothing was written)"
        ));
    }
    let dir = parent.join(name);
    if dir.exists() {
        return Err(format!(
            "`{}` already exists: pick another name (nothing was written)",
            dir.display()
        ));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let manifest = new_manifest(name, base);
    // The dir is ours alone (just created); a failure here removes it, so
    // a half-made look never lingers in the catalog.
    if let Err(error) = std::fs::write(dir.join("look.toml"), manifest) {
        std::fs::remove_dir_all(&dir).ok();
        return Err(format!(
            "cannot write {}: {error}",
            dir.join("look.toml").display()
        ));
    }
    Ok(dir)
}

#[cfg(test)]
mod tests;

/// The user-looks directory: `$XDG_DATA_HOME/scoot/user-looks`, else
/// `~/.local/share/scoot/user-looks`. `None` only when no home resolves.
pub fn user_looks_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_DATA_HOME");
    let home = std::env::var_os("HOME");
    data_home(xdg.as_deref(), home.as_deref())
        .map(|base| base.join("scoot/user-looks"))
        .ok_or_else(|| "no home resolves: set XDG_DATA_HOME or HOME".to_owned())
}

/// The starting manifest: the name, a title to rename, and the base. Every
/// other token inherits from the base until the author overrides it below
/// (colors are `#rrggbb`; the full token list is the look format
/// reference). The description inherits too, until the look earns its own.
fn new_manifest(name: &str, base: &str) -> String {
    format!(
        "\
schema = 1
name = \"{name}\"
title = \"{name}\"
extends = \"{base}\"

# Override any token from {base} below. Examples:
#
# [palette]
# accent = \"#FFA45C\"
#
# [fonts]
# size = 11
"
    )
}
