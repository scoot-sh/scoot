//! `scoot look target add PATH [--trust]`: adds a pack of look targets
//! (a directory someone else wrote) to the user's own
//! `<config>/scoot/look-targets/`.
//!
//! A pack is not a look. It never rides inside one, so installed looks
//! still cannot supply templates, outputs or reload commands (the parser
//! refuses those, unchanged). It is a directory the user names on the
//! command line, and the user owns what lands. Because a target's
//! reload runs on every swap, the add is gated like `look install`'s
//! `--trust`: without it the tool prints each target (what it writes,
//! the reload command on its own line, the whole template) and adds
//! nothing; with it, the same review prints first and the targets land.
//!
//! Nothing here executes anything: files are read, checked and copied.
//!
//! The source is foreign, so it is gated harder than the user's own
//! `look-targets/` directory is: no symlinks anywhere (a symlinked
//! target directory, manifest or template refuses), no hard links, no
//! file beyond `target.toml` and its template, and caps on the target
//! count and total size. Every file is read exactly once, through the
//! same `O_NOFOLLOW` reader the look inputs use; the bytes reviewed are
//! the bytes written. Adding is all or nothing: every target is checked
//! (against the pack and against what is already there) before the
//! first one lands, and targets stage in a sibling of `look-targets/`
//! (a stray directory inside it would make every later apply refuse).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use scoot_look::{
    MAX_TEMPLATE_BYTES, RawTarget, Reload, check_target_name_matches_dir, check_target_path,
    parse_target_toml, reload_timeout, render_template, target_tokens,
};

use super::apply;
use super::targets::{check_output_inside_config, discover_in, targets_root};
use crate::output;

/// The most targets one pack may carry. The starter pack has seven.
pub const MAX_PACK_TARGETS: usize = 32;

/// The most bytes one pack may carry in total (manifests and templates).
pub const MAX_PACK_BYTES: usize = 4 * 1024 * 1024;

/// One target read from a pack, ready to review and to write.
#[derive(Debug, Clone)]
pub struct PackTarget {
    /// The target name (matches its directory).
    pub name: String,
    /// The output path, relative to the config home.
    pub output: String,
    /// The template's file name inside the target directory.
    pub template_file: String,
    /// The pack's reload, if it declares one.
    pub reload: Option<Reload>,
    /// The exact `target.toml` bytes read from the pack.
    pub toml_bytes: Vec<u8>,
    /// The exact template bytes read from the pack.
    pub template_bytes: Vec<u8>,
    /// The template as text (validated UTF-8), for the review.
    pub template_text: String,
}

/// Reads and checks a pack: a target directory (it holds `target.toml`)
/// or a directory of target directories. `known` is the union of token
/// names across the catalog's looks (`None` when there are no looks to
/// check against): a token no look spells refuses here instead of
/// refusing every later swap. `config_home` is where the targets will
/// live, for the output containment check.
pub fn read_pack(
    src: &Path,
    config_home: &Path,
    known: Option<&BTreeMap<String, String>>,
) -> Result<Vec<PackTarget>, String> {
    let meta = std::fs::symlink_metadata(src)
        .map_err(|error| format!("cannot stat {}: {error}", src.display()))?;
    if meta.is_symlink() || !meta.is_dir() {
        return Err(format!(
            "refused: {} is not a plain directory (a symlink or a file)",
            src.display()
        ));
    }
    if src.join("look.toml").exists() {
        return Err(format!(
            "refused: {} is a look, not a target pack: use `scoot look install` for looks",
            src.display()
        ));
    }
    let mut budget = MAX_PACK_BYTES;
    let mut targets = Vec::new();
    if src.join("target.toml").exists() {
        let canonical = std::fs::canonicalize(src)
            .map_err(|error| format!("cannot resolve {}: {error}", src.display()))?;
        let name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} has no directory name", src.display()))?;
        targets.push(read_target(src, &name, config_home, known, &mut budget)?);
    } else {
        let mut entries: Vec<_> = std::fs::read_dir(src)
            .map_err(|error| format!("cannot list {}: {error}", src.display()))?
            .collect::<Result<_, _>>()
            .map_err(|error| format!("cannot list {}: {error}", src.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if meta.is_dir() {
                if targets.len() >= MAX_PACK_TARGETS {
                    return Err(format!(
                        "refused: the pack carries more than {MAX_PACK_TARGETS} targets"
                    ));
                }
                targets.push(read_target(
                    &path,
                    &file_name,
                    config_home,
                    known,
                    &mut budget,
                )?);
            } else if meta.is_file() && is_pack_doc(&file_name) {
                // A README or LICENSE beside the targets: never copied.
            } else {
                return Err(format!(
                    "refused: {} is not a target directory (a pack holds target directories, \
                     and optionally README or LICENSE files)",
                    path.display()
                ));
            }
        }
    }
    if targets.is_empty() {
        return Err(format!(
            "refused: {} holds no targets (a target directory has a target.toml)",
            src.display()
        ));
    }
    let mut seen = BTreeSet::new();
    for target in &targets {
        if !seen.insert(target.output.to_lowercase()) {
            return Err(format!(
                "refused: target `{}` writes the same output as another target in the pack: `{}`",
                target.name, target.output
            ));
        }
    }
    Ok(targets)
}

/// A README or LICENSE file beside the target directories.
fn is_pack_doc(name: &str) -> bool {
    ["README", "LICENSE", "COPYING"]
        .iter()
        .any(|stem| name.starts_with(stem))
}

/// Reads one target directory: exactly `target.toml` plus the template it
/// names, both plain single-link files.
fn read_target(
    dir: &Path,
    dir_name: &str,
    config_home: &Path,
    known: Option<&BTreeMap<String, String>>,
    budget: &mut usize,
) -> Result<PackTarget, String> {
    let manifest = dir.join("target.toml");
    let toml_bytes = apply::read_regular(&manifest).map_err(|error| {
        format!(
            "target `{dir_name}`: cannot read {}: {error}",
            manifest.display()
        )
    })?;
    take_budget(budget, toml_bytes.len(), dir_name)?;
    let raw: RawTarget = parse_target_toml(&toml_bytes).map_err(|errors| {
        format!(
            "target `{dir_name}`: invalid target.toml: {}",
            errors
                .iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    if let Some(error) = check_target_name_matches_dir(&raw, dir_name) {
        return Err(format!("target `{dir_name}`: invalid target.toml: {error}"));
    }
    if raw.template.contains('/') {
        return Err(format!(
            "target `{}`: template `{}` must be a plain file name in the target directory",
            raw.name, raw.template
        ));
    }
    // Lexical grammar always; containment (including symlinks on the way
    // out) once the config home exists to resolve against.
    if let Some(error) = check_target_path("output", &raw.output) {
        return Err(format!(
            "target `{}`: invalid output `{}`: {error}",
            raw.name, raw.output
        ));
    }
    if config_home.exists() {
        check_output_inside_config(config_home, &raw.output).map_err(|error| {
            format!(
                "target `{}`: invalid output `{}`: {error}",
                raw.name, raw.output
            )
        })?;
    }
    // Nothing else may ride along: a stray file is data the user never
    // reviewed, and macOS tar leaves `._*` files that would land.
    for entry in
        std::fs::read_dir(dir).map_err(|error| format!("cannot list {}: {error}", dir.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != "target.toml" && name != raw.template {
            return Err(format!(
                "target `{}`: unexpected file `{name}` (a target is target.toml plus its template)",
                raw.name
            ));
        }
    }
    let template_path = dir.join(&raw.template);
    let template_bytes = apply::read_regular(&template_path).map_err(|error| {
        format!(
            "target `{}`: cannot read {}: {error}",
            raw.name,
            template_path.display()
        )
    })?;
    if template_bytes.len() > MAX_TEMPLATE_BYTES {
        return Err(format!(
            "target `{}`: template larger than {MAX_TEMPLATE_BYTES} bytes",
            raw.name
        ));
    }
    take_budget(budget, template_bytes.len(), &raw.name)?;
    let template_text = String::from_utf8(template_bytes.clone())
        .map_err(|_| format!("target `{}`: template is not valid UTF-8", raw.name))?;
    if let Some(known) = known {
        render_template(&template_text, known)
            .map_err(|error| format!("target `{}`: template refuses: {error}", raw.name))?;
    }
    Ok(PackTarget {
        name: raw.name,
        output: raw.output,
        template_file: raw.template,
        reload: raw.reload,
        toml_bytes,
        template_bytes,
        template_text,
    })
}

fn take_budget(budget: &mut usize, bytes: usize, name: &str) -> Result<(), String> {
    *budget = budget.checked_sub(bytes).ok_or_else(|| {
        format!("refused: the pack is larger than {MAX_PACK_BYTES} bytes (at target `{name}`)")
    })?;
    Ok(())
}

/// The union of token names across every look in the catalog, for the
/// add-time check that a template names tokens some look spells.
/// `None` when no look resolves.
pub fn catalog_tokens() -> Option<BTreeMap<String, String>> {
    let catalog = scoot_look::catalog::discover();
    let sources = catalog.sources();
    let mut union = BTreeMap::new();
    for name in sources.keys() {
        if let Ok(resolved) = scoot_look::resolve(&sources, name) {
            for (key, value) in target_tokens(&resolved) {
                union.entry(key).or_insert(value);
            }
        }
    }
    if union.is_empty() { None } else { Some(union) }
}

/// Escapes what could rewrite the terminal or spoof the review: every
/// control character except newline and tab (ESC, CR, DEL, C1) and the
/// invisible Unicode format characters (zero-width, bidi overrides,
/// BOM), shown as `\u{..}`.
pub fn escape_for_terminal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let hidden = (c.is_control() && c != '\n' && c != '\t')
            || matches!(c as u32, 0x200B..=0x200F | 0x2028..=0x202E | 0x2066..=0x2069 | 0xFEFF);
        if hidden {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// The review for one target: what it writes, the reload on its own
/// line, then the whole template between rules. Terminal-safe.
pub fn review_text(target: &PackTarget) -> String {
    let mut out = format!("target {}\n", target.name);
    out.push_str(&format!(
        "  writes:   targets/{} in the active look on every swap\n",
        target.output
    ));
    match &target.reload {
        Some(reload) => {
            let argv: Vec<String> = reload.argv.iter().map(|a| escape_for_terminal(a)).collect();
            out.push_str(&format!(
                "  RELOAD:   {} (runs after every swap, up to {}s)\n",
                argv.join(" "),
                reload_timeout(reload)
            ));
        }
        None => out.push_str("  reload:   none\n"),
    }
    out.push_str(&format!(
        "  template: {} ({} bytes)\n  ----\n",
        escape_for_terminal(&target.template_file),
        target.template_bytes.len()
    ));
    for line in escape_for_terminal(&target.template_text).lines() {
        out.push_str("  | ");
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("  ----\n");
    out
}

/// Refuses when the pack collides with what is already there, then
/// returns normally. Run before the first write: a name that exists
/// (including a Nix-declared symlink), or an output an existing target
/// already writes, refuses the whole add.
pub fn check_collisions(targets: &[PackTarget], config_home: &Path) -> Result<(), String> {
    let existing = discover_in(config_home).map_err(|error| {
        format!("the existing look-targets do not load, fix them first: {error}")
    })?;
    let root = targets_root(config_home);
    let outputs: BTreeSet<String> = existing.iter().map(|d| d.output.to_lowercase()).collect();
    for target in targets {
        if std::fs::symlink_metadata(root.join(&target.name)).is_ok() {
            return Err(format!(
                "refused: a target named `{}` already exists in {}: remove it first, nothing was added",
                target.name,
                root.display()
            ));
        }
        if outputs.contains(&target.output.to_lowercase()) {
            return Err(format!(
                "refused: target `{}` writes `{}`, which an existing target already writes: \
                 nothing was added",
                target.name, target.output
            ));
        }
    }
    Ok(())
}

/// Removes a staging directory on drop (success and every error path).
struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes the reviewed bytes into `look-targets/`, all or nothing:
/// stages every target in a sibling directory first, then renames each
/// into place, undoing the renames made so far if one fails.
pub fn add_into(targets: &[PackTarget], config_home: &Path) -> Result<(), String> {
    let root = targets_root(config_home);
    let parent = root
        .parent()
        .ok_or_else(|| "the targets root has no parent".to_owned())?
        .to_owned();
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("cannot create {}: {error}", root.display()))?;
    // The config home now exists: the full containment check, symlinks
    // on the output path included.
    for target in targets {
        check_output_inside_config(config_home, &target.output).map_err(|error| {
            format!(
                "target `{}`: invalid output `{}`: {error}",
                target.name, target.output
            )
        })?;
    }
    check_collisions(targets, config_home)?;
    let staging_dir = parent.join(format!(".look-targets-add-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging_dir);
    std::fs::create_dir(&staging_dir)
        .map_err(|error| format!("cannot stage in {}: {error}", staging_dir.display()))?;
    let _staging = Staging(staging_dir.clone());
    for target in targets {
        let dir = staging_dir.join(&target.name);
        std::fs::create_dir(&dir)
            .map_err(|error| format!("cannot stage target `{}`: {error}", target.name))?;
        std::fs::write(dir.join("target.toml"), &target.toml_bytes)
            .map_err(|error| format!("cannot stage target `{}`: {error}", target.name))?;
        std::fs::write(dir.join(&target.template_file), &target.template_bytes)
            .map_err(|error| format!("cannot stage target `{}`: {error}", target.name))?;
    }
    let mut moved: Vec<PathBuf> = Vec::new();
    for target in targets {
        let dest = root.join(&target.name);
        if let Err(error) = std::fs::rename(staging_dir.join(&target.name), &dest) {
            for done in &moved {
                let _ = std::fs::remove_dir_all(done);
            }
            return Err(format!(
                "cannot add target `{}`: {error}: nothing was added",
                target.name
            ));
        }
        moved.push(dest);
    }
    Ok(())
}

/// Runs `scoot look target add`: review, then add under `--trust`.
pub fn run(look: &super::Look, path: &Path, trust: bool) -> Result<(), String> {
    let config_home = super::install::config_home_dir()?;
    let known = catalog_tokens();
    let targets = read_pack(path, &config_home, known.as_ref())?;
    // Collisions are reported before the review asks for trust, so the
    // user is not asked to approve something that cannot land.
    if config_home.exists() {
        check_collisions(&targets, &config_home)?;
    }
    if look.json {
        let doc = serde_json::json!({
            "added": trust,
            "targets": targets.iter().map(|t| serde_json::json!({
                "name": t.name,
                "output": t.output,
                "reload": t.reload.as_ref().map(|r| serde_json::json!({
                    "argv": r.argv,
                    "timeout_secs": reload_timeout(r),
                })),
                "template_file": t.template_file,
                "template": t.template_text,
            })).collect::<Vec<_>>(),
            "notes": if known.is_none() {
                vec!["token names were not checked: no looks in the catalog"]
            } else {
                Vec::new()
            },
        });
        output::print_line(&serde_json::to_string_pretty(&doc).unwrap_or_default())
            .map_err(|error| error.to_string())?;
    } else {
        for target in &targets {
            for line in review_text(target).lines() {
                output::print_line(line).map_err(|error| error.to_string())?;
            }
        }
        if known.is_none() {
            output::warn(format_args!(
                "note: token names were not checked: no looks in the catalog"
            ));
        }
    }
    if !trust {
        return Err(
            "nothing added: these templates render into your desktop's config and any reload \
             runs on every swap; read the review above and rerun with --trust to add them"
                .to_owned(),
        );
    }
    add_into(&targets, &config_home)?;
    if !look.json {
        let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
        output::print_line(&format!(
            "added: {} to {}",
            names.join(", "),
            targets_root(&config_home).display()
        ))
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
