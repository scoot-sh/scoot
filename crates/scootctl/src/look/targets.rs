//! User-owned look targets: discovery, rendering and reload running.
//!
//! Targets live in `<config>/scoot/look-targets/<name>/target.toml` plus
//! one template file, all owned by the user. On every apply the tool
//! substitutes the active look's tokens into each template and writes the
//! result into the rendered look under `targets/<output>`, beside the
//! built-in fragments, under the same atomic swap, cache key and rollback
//! rules. `validate` and `build` cover them the same way.
//!
//! Trust boundary: installed looks (someone else's) can never supply
//! templates, output paths or reload commands. That rule needs no code
//! here — a `look.toml` with a `[targets]` table or an `[apps.<target>]`
//! table refuses at parse through the existing allowlist rules (pinned by
//! the `scoot-look` tests) — only these user-owned directories are read.
//!
//! Reloads are the user's own commands, run after a successful swap with
//! no shell interpretation (argv executed directly), a bounded wait, and
//! best-effort reporting: a missing app is a note, never an error, and a
//! failing reload never undoes the swap. Render failures (unknown token,
//! unreadable template, duplicate or escaping output) refuse before
//! anything is recorded or repointed, so one failing target rolls the
//! swap back the way a refused reload does.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use scoot_look::check_target_path;
use scoot_look::{
    MAX_TEMPLATE_BYTES, RawTarget, ResolvedLook, check_target_name_matches_dir, parse_target_toml,
    reload_timeout, render_template, target_tokens,
};

/// One discovered user target: the validated manifest plus the template
/// bytes it renders from.
#[derive(Debug, Clone)]
pub struct TargetDef {
    /// The target name (matches its directory).
    pub name: String,
    /// The output path, relative to the config home (`kitty/colors.conf`).
    pub output: String,
    /// The user's own reload, if declared.
    pub reload: Option<scoot_look::Reload>,
    /// The exact `target.toml` bytes (part of the render cache key).
    pub toml_bytes: Vec<u8>,
    /// The exact template bytes (part of the render cache key).
    pub template_bytes: Vec<u8>,
}

/// One rendered target file: the output path plus its bytes, written to
/// `targets/<output>` in the render.
#[derive(Debug, Clone)]
pub struct TargetFile {
    /// The output path, relative to the config home and to `targets/`.
    pub output: String,
    /// The substituted bytes.
    pub bytes: Vec<u8>,
}

/// The targets root under a config home: `<config>/scoot/look-targets`.
pub fn targets_root(config_home: &Path) -> PathBuf {
    config_home.join("scoot/look-targets")
}

/// Discovers the user's targets under an explicit config home (`None`
/// resolves the live environment, for production; tests pin a scratch
/// dir). A missing targets directory is no targets, never an error. Every
/// other problem — an unreadable or invalid `target.toml`, a missing or
/// unreadable template, a duplicate or escaping output — refuses the
/// whole load loudly, before anything renders: a half-written target
/// never swaps half a desktop.
pub fn discover(config: Option<&Path>) -> Result<Vec<TargetDef>, String> {
    let owned;
    let home = match config {
        Some(dir) => dir,
        None => {
            owned = resolve_home();
            match owned.as_deref() {
                Some(home) => home,
                None => return Ok(Vec::new()),
            }
        }
    };
    discover_in(home)
}

/// [`discover`] over an explicit config home, so tests pin the directory
/// without touching the process environment.
pub fn discover_in(config_home: &Path) -> Result<Vec<TargetDef>, String> {
    let root = targets_root(config_home);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!("cannot list {}: {error}", root.display()));
        }
    };
    let mut names: Vec<_> = entries.collect();
    names.sort_by_key(|entry| entry.as_ref().map(|e| e.file_name()).unwrap_or_default());
    let mut defs = Vec::new();
    for entry in names {
        let entry = entry.map_err(|error| format!("cannot list {}: {error}", root.display()))?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        // A home-manager/stow-style catalog links each target into the
        // store: a symlink to a directory is a target, like a look.
        // Anything else (files, FIFOs) is skipped quietly here and
        // refused loudly by nothing — there is no trust boundary at
        // this root, only the user's own files.
        let dir = if meta.file_type().is_symlink() {
            let canonical = std::fs::canonicalize(&path)
                .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
            if !canonical.is_dir() {
                continue;
            }
            canonical
        } else if meta.is_dir() {
            path.clone()
        } else {
            continue;
        };
        // The name is the entry's own name, never a resolved target's
        // basename — a stowed link keeps its target name.
        let link_name = entry.file_name().to_string_lossy().into_owned();
        defs.push(load_one(&dir, &link_name, config_home)?);
    }
    defs.sort_by(|a, b| a.name.cmp(&b.name));
    // Two targets writing the same output refuse: the second write
    // would silently win, and no wiring could name both. Compared
    // case-insensitively: `Kitty/x` and `kitty/x` are one file on a
    // case-insensitive filesystem and confusing everywhere else. No
    // Unicode normalization step is needed beside it: outputs are
    // ASCII-only by grammar (`check_target_path` refuses the rest), so
    // the NFC and NFD spellings of one file never reach this compare.
    let mut seen = std::collections::BTreeSet::new();
    for def in &defs {
        if !seen.insert(def.output.to_lowercase()) {
            return Err(format!(
                "target `{}` writes the same output as another target: `{}`",
                def.name, def.output
            ));
        }
    }
    Ok(defs)
}

/// Loads one target directory: the manifest, the name match, the
/// config-home containment (including the symlink escape), and the
/// template bytes. Symlinked manifests and templates refuse instead of
/// redirecting the read elsewhere. `link_name` is the entry's own name
/// (a stowed link keeps its target name, never the store dir's).
fn load_one(dir: &Path, link_name: &str, config_home: &Path) -> Result<TargetDef, String> {
    let dir_name = link_name.to_owned();
    let manifest = dir.join("target.toml");
    let toml_bytes = super::apply::read_regular(&manifest).map_err(|error| {
        format!(
            "target `{dir_name}`: cannot read {}: {error}",
            manifest.display()
        )
    })?;
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
    // Targets are user-owned by construction (only this directory is
    // ever read): no license rule applies, and installed looks can never
    // reach this loader — a look.toml smuggling `[targets]` or a
    // target-shaped `[apps.*]` table refuses at parse through the
    // allowlist rules.
    if let Some(error) = check_target_name_matches_dir(&raw, &dir_name) {
        return Err(format!("target `{dir_name}`: invalid target.toml: {error}"));
    }
    check_output_inside_config(config_home, &raw.output).map_err(|error| {
        format!(
            "target `{}`: invalid output `{}`: {error}",
            raw.name, raw.output
        )
    })?;
    let template_path = dir.join(&raw.template);
    // The template must stay inside its own directory: `template` is a
    // validated relative path, but `dir.join` plus a `..` would still
    // escape — and `..` already refuses above, so this is the backstop,
    // not the only defense.
    if !lexically_inside(dir, &template_path) {
        return Err(format!(
            "target `{}`: template `{}` escapes its directory",
            raw.name, raw.template
        ));
    }
    let template_bytes = super::apply::read_regular(&template_path).map_err(|error| {
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
    Ok(TargetDef {
        name: raw.name,
        output: raw.output,
        reload: raw.reload,
        toml_bytes,
        template_bytes,
    })
}

/// The output stays under the config home: relative, normalized and
/// `..`-free by grammar ([`check_target_path`]), and no symlink on the
/// way out. A symlinked directory on the path (say `kitty` pointing at
/// `/etc`) refuses instead of writing through it.
pub fn check_output_inside_config(config_home: &Path, output: &str) -> Result<(), String> {
    if let Some(error) = check_target_path("output", output) {
        return Err(error.to_string());
    }
    let canonical_home = std::fs::canonicalize(config_home)
        .map_err(|error| format!("cannot resolve {}: {error}", config_home.display()))?;
    let full = canonical_home.join(output);
    // Every existing prefix must be a real directory, never a symlink:
    // a link on the path would redirect the render outside the home.
    let mut prefix = canonical_home.clone();
    for component in Path::new(output).components() {
        let Component::Normal(part) = component else {
            return Err("output must be a plain relative path".to_owned());
        };
        prefix.push(part);
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "{} is a symlink: refusing to render through it",
                    prefix.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("cannot inspect {}: {error}", prefix.display()));
            }
        }
    }
    // Where the parent exists, resolve it and confirm it stays inside:
    // lexical safety plus symlink safety, both checked, neither trusted
    // alone.
    let mut parent = full.clone();
    parent.pop();
    if parent.exists() {
        let canonical_parent = std::fs::canonicalize(&parent)
            .map_err(|error| format!("cannot resolve {}: {error}", parent.display()))?;
        if !canonical_parent.starts_with(&canonical_home) {
            return Err(format!(
                "{} escapes the config home through a symlink",
                full.display()
            ));
        }
    }
    Ok(())
}

/// Whether `joined` stays lexically inside `dir` (`.` and `..`
/// resolved, no filesystem access): the backstop behind the grammar.
fn lexically_inside(dir: &Path, joined: &Path) -> bool {
    let mut stack: Vec<&std::ffi::OsStr> = Vec::new();
    for component in dir.components() {
        match component {
            Component::Normal(part) => stack.push(part),
            Component::ParentDir => {
                stack.pop();
            }
            Component::CurDir => {}
            other => stack.push(other.as_os_str()),
        }
    }
    let depth = stack.len();
    for component in joined.components() {
        match component {
            Component::Normal(part) => stack.push(part),
            Component::ParentDir => {
                stack.pop();
            }
            Component::CurDir => {}
            other => stack.push(other.as_os_str()),
        }
    }
    stack.len() > depth
}

/// Renders every target against the resolved look: unknown tokens refuse
/// here, so a typo rolls the swap back before anything is recorded. The
/// result is `(output, bytes)` pairs, sorted by output.
pub fn render_all(defs: &[TargetDef], merged: &ResolvedLook) -> Result<Vec<TargetFile>, String> {
    let tokens = target_tokens(merged);
    let mut files = Vec::with_capacity(defs.len());
    for def in defs {
        let text = std::str::from_utf8(&def.template_bytes)
            .map_err(|_| format!("target `{}`: template is not valid UTF-8", def.name))?;
        let rendered = render_template(text, &tokens)
            .map_err(|error| format!("target `{}`: cannot render: {error}", def.name))?;
        files.push(TargetFile {
            output: def.output.clone(),
            bytes: rendered.into_bytes(),
        });
    }
    files.sort_by(|a, b| a.output.cmp(&b.output));
    Ok(files)
}

/// What one target reload did: the user command ran after a successful
/// swap, best-effort. A missing app is a note, never an error, and a
/// failing reload never undoes the swap — the swap already repointed and
/// reloaded its peers by the time these run.
pub struct ReloadOutcome {
    /// Targets whose reload exited zero (`target <name>`).
    pub reloaded: Vec<String>,
    /// Everything else: missing binaries, timeouts, non-zero exits.
    pub notes: Vec<String>,
}

/// Runs every declared reload, in target-name order, after a successful
/// swap. Each argv runs directly (never through a shell: no word
/// splitting, no globbing, no `$VAR` expansion), with the declared wait
/// (default 30 s). A child outliving its wait is killed and reported.
pub fn run_reloads(defs: &[TargetDef]) -> ReloadOutcome {
    let mut outcome = ReloadOutcome {
        reloaded: Vec::new(),
        notes: Vec::new(),
    };
    for def in defs {
        let Some(reload) = &def.reload else {
            continue;
        };
        let timeout = Duration::from_secs(reload_timeout(reload));
        match run_one(&reload.argv, timeout) {
            Ok(note) => {
                outcome.reloaded.push(format!("target {}", def.name));
                if let Some(note) = note {
                    outcome.notes.push(format!("target `{}`: {note}", def.name));
                }
            }
            Err(note) => outcome.notes.push(format!("target `{}`: {note}", def.name)),
        }
    }
    outcome
}

/// Runs one argv directly: `argv[0]` is a bare name on `PATH` or an
/// absolute path (relative paths with a `/` never parse, so only those
/// two shapes arrive). `Ok(None)` is a clean zero exit; `Ok(Some)` is a
/// zero exit worth naming; `Err` is everything else, as a note. Every
/// run goes through the shared bounded runner, so a reload that forks
/// and holds the pipes open is killed with its whole process group at
/// the deadline, and a reload flooding stdout is killed at the byte cap
/// with its truncation noted.
fn run_one(argv: &[String], timeout: Duration) -> Result<Option<String>, String> {
    use super::bounded::{BoundedError, MAX_OUTPUT_BYTES, run_bounded};
    let program = Path::new(&argv[0]);
    let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
    let output = match run_bounded(program, &args, timeout) {
        Ok(output) => output,
        Err(BoundedError::Spawn(error)) => {
            return Err(if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "reload `{}` not run: the app is not installed ({error}): left alone",
                    argv[0]
                )
            } else {
                format!("reload `{}` not run: {error}", argv[0])
            });
        }
        Err(BoundedError::Timeout { secs, .. }) => {
            return Err(format!(
                "reload `{}` timed out after {secs}s and was killed: the swap stands",
                argv[0]
            ));
        }
        Err(BoundedError::OutputCap { .. }) => {
            return Err(format!(
                "reload `{}` wrote past {MAX_OUTPUT_BYTES} bytes: output truncated and killed: the swap stands",
                argv[0]
            ));
        }
        Err(BoundedError::Wait(detail)) => {
            return Err(format!(
                "reload `{}` could not be waited on: {detail}",
                argv[0]
            ));
        }
    };
    if output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim().to_owned();
        if stderr.is_empty() {
            return Ok(None);
        }
        return Ok(Some(format!(
            "reload wrote to stderr: {}",
            truncate(&stderr, 256)
        )));
    }
    let mut detail = format!("reload exited with {}", output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim().to_owned();
    if !stderr.is_empty() {
        detail.push_str(": ");
        detail.push_str(&truncate(&stderr, 256));
    }
    Err(format!("{detail}: the swap stands"))
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated]", &text[..end])
}

/// Resolves the config home the way the tool does: an explicit dir wins,
/// else `$XDG_CONFIG_HOME`, else `~/.config`. Empty counts as unset.
/// `None` only when no home resolves at all — and then there are no
/// user targets to discover.
fn resolve_home() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME");
    scoot_look::catalog::config_home(xdg.as_deref(), home.as_deref())
}
