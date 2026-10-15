//! `extends` resolution (design section 3.1): over *source* looks only,
//! cycle-checked, depth at most 8, never to an installed look.
//!
//! Origin is by directory: the caller hands in each candidate with the
//! [`Origin`] its directory records, so a base that lives in
//! `installed-looks/` is refused no matter what its manifest claims.

use std::collections::BTreeMap;

use crate::grammar::GrammarError;
use crate::resolved::{Partial, ResolvedLook};
use crate::schema::{MAX_EXTENDS_DEPTH, Origin, RawLook};

/// Resolve `name` against `sources`, following `extends` and layering each
/// child over its base. Refuses unknown names, cycles, chains deeper than 8,
/// installed bases, and chains that leave a required token unset.
pub fn resolve(
    sources: &BTreeMap<String, (RawLook, Origin)>,
    name: &str,
) -> Result<ResolvedLook, GrammarError> {
    let mut chain = Vec::new();
    let mut partial = Partial::default();
    resolve_into(sources, name, &mut chain, &mut partial)?;
    let mut resolved = partial.finish(name)?;
    resolved.fill_wallpaper_default();
    Ok(resolved)
}

fn resolve_into(
    sources: &BTreeMap<String, (RawLook, Origin)>,
    name: &str,
    chain: &mut Vec<String>,
    partial: &mut Partial,
) -> Result<(), GrammarError> {
    if chain.len() > MAX_EXTENDS_DEPTH as usize {
        return Err(GrammarError::new(
            "extends",
            format!(
                "extends chain deeper than {MAX_EXTENDS_DEPTH}: {}",
                chain.join(" -> ")
            ),
        ));
    }
    if chain.contains(&name.to_string()) {
        return Err(GrammarError::new(
            "extends",
            format!("extends cycle: {} -> {name}", chain.join(" -> ")),
        ));
    }
    let (raw, origin) = sources
        .get(name)
        .ok_or_else(|| GrammarError::new("extends", format!("unknown base look `{name}`")))?;
    if *origin == Origin::Installed {
        return Err(GrammarError::new(
            "extends",
            format!("`{name}` is an installed look and can be applied but never extended"),
        ));
    }
    chain.push(name.to_string());
    if let Some(base) = raw.extends.clone() {
        resolve_into(sources, &base, chain, partial)?;
    }
    chain.pop();
    partial.apply(raw);
    Ok(())
}

#[cfg(test)]
mod tests;
