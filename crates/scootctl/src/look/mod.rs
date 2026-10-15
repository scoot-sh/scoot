//! `scoot look`: list, inspect and atomically swap looks.
//!
//! A subcommand of the existing CLI, not a new user-facing binary: `scoot`
//! is already the client on every platform. Every command takes `--json`
//! (machine-readable on stdout, notes on stderr); exit codes follow
//! `scoot msg` (0 applied, 1 the run failed, 2 the invocation was wrong).
//!
//! Offline commands (`list`, `show`, `validate`, `build`, `current`) work
//! with no Nix, no home-manager and no compositor running. `apply` moves
//! only the link when nothing runs, and drives each app's control-socket
//! reload, scoped to the look, when they do.

pub mod apply;
pub mod catalog;
pub mod reload;
pub mod toolkit;

use std::path::{Path, PathBuf};

use scoot_look::Origin;

use crate::cli::Error;
use crate::output;

use apply::{
    ApplyOptions, BuildTarget, apply_with, build_to_out, resolve_state_dir, validate_dir,
    validate_entry,
};
use catalog::{Catalog, discover, origin_name};
use reload::{CompositorReload, IpcCompositor, bar_targets};
use toolkit::{CliDconf, toolkit_on};

/// One `scoot look` invocation: the command, the state/socket selection,
/// and whether the reply renders as JSON.
#[derive(Debug, PartialEq)]
pub struct Look {
    pub command: Command,
    pub state: Option<PathBuf>,
    pub socket: Option<PathBuf>,
    pub json: bool,
}

/// The `scoot look` commands.
#[derive(Debug, PartialEq)]
pub enum Command {
    /// The catalog, in search order, with shadowed names marked.
    List,
    /// The active look, the link, and the shadowed keys.
    Current,
    /// One look's tokens and contrast.
    Show { name: String },
    /// Validate a directory or a catalog name without touching anything.
    Validate { target: String, origin: Origin },
    /// Render a directory or a catalog name into `--out` without applying.
    Build {
        target: String,
        out: PathBuf,
        origin: Origin,
    },
    /// Render, record, repoint, reload in order, roll back on refusal.
    Apply { name: String, no_toolkit: bool },
    /// Show or persist the toolkit writes (`on` or `off`).
    Toolkit { action: Option<bool> },
}

/// Every look command, for the JSON document and `did you mean` guesses.
pub const COMMANDS: &[&str] = &[
    "list", "current", "show", "validate", "build", "apply", "toolkit",
];

/// The usage line, shared by `scoot look --help` and the top-level help.
pub const LOOK_USAGE: &str = "\
    look list [--json]
    look current [--json]
    look show NAME [--json]
    look validate DIR|NAME [--origin shipped|declared|user|installed] [--json]
    look build DIR|NAME --out DIR [--origin shipped|declared|user|installed] [--json]
    look apply NAME [--no-toolkit] [--json]
    look toolkit [on|off] [--json]

    --state DIR takes the look state (default: $SCOOT_LOOK_STATE, then
    $XDG_STATE_HOME/scoot, then ~/.local/state/scoot); --socket PATH takes
    the compositor socket (default: $SCOOT_SOCKET, then
    $XDG_RUNTIME_DIR/scoot.sock)";

/// Parses the arguments after `look`.
pub fn parse_look<I: IntoIterator<Item = String>>(args: I) -> Result<Look, Error> {
    let mut verb: Option<String> = None;
    let mut state = None;
    let mut socket = None;
    let mut json = false;
    let mut positionals: Vec<String> = Vec::new();
    // Global flags ride before the verb or after it (`--json` anywhere;
    // `--state`/`--socket` with `=` or a following word). Anything else
    // past the verb is the subcommand's own (`--no-toolkit`, `--origin`,
    // `--out`), parsed by it, never here.
    let mut words = args.into_iter().peekable();
    while let Some(word) = words.next() {
        if word == "--json" {
            json = true;
            continue;
        }
        if let Some(dir) = word.strip_prefix("--state=") {
            state = Some(PathBuf::from(dir));
            continue;
        }
        if let Some(path) = word.strip_prefix("--socket=") {
            socket = Some(PathBuf::from(path));
            continue;
        }
        if word == "--state" {
            match words.next() {
                Some(dir) => state = Some(PathBuf::from(dir)),
                None => return Err(Error::Missing("a path after --state")),
            }
            continue;
        }
        if word == "--socket" {
            match words.next() {
                Some(path) => socket = Some(PathBuf::from(path)),
                None => return Err(Error::Missing("a path after --socket")),
            }
            continue;
        }
        if verb.is_none() {
            verb = Some(word);
        } else {
            positionals.push(word);
        }
    }
    let verb = verb.ok_or(Error::Missing("a look command"))?;
    let command = match verb.as_str() {
        "list" => {
            refuse_trailing(&positionals)?;
            Command::List
        }
        "current" => {
            refuse_trailing(&positionals)?;
            Command::Current
        }
        "show" => match positionals.as_slice() {
            [name] => {
                check_name(name)?;
                Command::Show { name: name.clone() }
            }
            [] => return Err(Error::Missing("a look name")),
            [_, extra, ..] => {
                return Err(hinted(
                    "flag",
                    extra.clone(),
                    &["--json", "--state", "--socket"],
                    "scoot look show --help",
                ));
            }
        },
        "validate" => {
            let (target, origin) = target_and_origin(&positionals)?;
            Command::Validate { target, origin }
        }
        "build" => {
            let (target, origin, out) = build_args(&positionals)?;
            Command::Build {
                target,
                out,
                origin,
            }
        }
        "apply" => {
            let mut name: Option<String> = None;
            let mut no_toolkit = false;
            for word in positionals {
                if word == "--no-toolkit" {
                    no_toolkit = true;
                } else if name.is_none() {
                    name = Some(word);
                } else {
                    return Err(Error::Unknown(word));
                }
            }
            let name = name.ok_or(Error::Missing("a look name"))?;
            check_name(&name)?;
            Command::Apply { name, no_toolkit }
        }
        "toolkit" => {
            let action = match positionals.as_slice() {
                [] => None,
                [only] if only == "on" => Some(true),
                [only] if only == "off" => Some(false),
                [only] => {
                    return Err(Error::Invalid {
                        what: "toolkit action",
                        value: only.clone(),
                    });
                }
                _ => return Err(Error::Unknown(positionals[1].clone())),
            };
            Command::Toolkit { action }
        }
        other => {
            return Err(hinted(
                "look command",
                other.to_owned(),
                &[
                    "list", "current", "show", "validate", "build", "apply", "toolkit",
                ],
                "scoot look --help",
            ));
        }
    };
    Ok(Look {
        command,
        state,
        socket,
        json,
    })
}

/// The full help text for `scoot look --help`.
pub fn usage() -> String {
    format!(
        "\
scoot look -- list, inspect and atomically swap looks

USAGE:
{LOOK_USAGE}

COMMANDS:
    list        the catalog in search order (user, installed, home-manager,
                system, shipped), with shadowed names marked
    current     the active look, the link, and every shadowed key
    show        one look's tokens, wallpaper, fonts and contrast
    validate    resolve and check a look without touching anything
    build       render a look into --out without applying it
    apply       render, validate, record, repoint, then reload each app's
                look layer in order (the compositor, then every bar),
                rolling back on a refusal; --no-toolkit skips the
                color-scheme write for this apply
    toolkit     show whether the color-scheme write is on, or persist
                `toolkit on|off` in look.json

A look is presentation only (colors, fonts, opacity, shape, cursor, the
wallpaper): a swap never applies pending edits to your own files, and a
refusal keeps the running look. On a home-manager machine before
look-nix-catalog, set `programs.scoot.desktop.look = null`,
`programs.scoot.stylix.enable = false` and
`programs.scootbar.stylix.enable = false`, or the Nix-written values in
your own files shadow every look (`current` lists them).

EXIT CODES:
    0  success: the reply is on stdout (help counts)
    1  the run failed: an unknown look, a refused value, a refused reload
    2  usage error: an unknown command, flag or value (the error names it)
"
    )
}

fn refuse_trailing(positionals: &[String]) -> Result<(), Error> {
    if let Some(extra) = positionals.first() {
        return Err(hinted(
            "flag",
            extra.clone(),
            &["--json", "--state", "--socket"],
            "scoot look --help",
        ));
    }
    Ok(())
}

fn check_name(name: &str) -> Result<(), Error> {
    if scoot_look::grammar::is_name(name) {
        Ok(())
    } else {
        Err(Error::Invalid {
            what: "look name",
            value: name.to_owned(),
        })
    }
}

fn target_and_origin(positionals: &[String]) -> Result<(String, Origin), Error> {
    let mut target: Option<String> = None;
    let mut origin = Origin::User;
    let mut words = positionals.iter().peekable();
    while let Some(word) = words.next() {
        if word == "--origin" {
            let value = words.next().ok_or(Error::Missing(
                "shipped|declared|user|installed after --origin",
            ))?;
            origin = match value.as_str() {
                "shipped" => Origin::Shipped,
                "declared" => Origin::Declared,
                "user" => Origin::User,
                "installed" => Origin::Installed,
                _ => {
                    return Err(Error::Invalid {
                        what: "--origin",
                        value: value.clone(),
                    });
                }
            };
        } else if target.is_none() {
            target = Some(word.clone());
        } else {
            return Err(hinted(
                "flag",
                word.clone(),
                &["--origin", "--json", "--state", "--socket"],
                "scoot look --help",
            ));
        }
    }
    let target = target.ok_or(Error::Missing("a directory or a look name"))?;
    Ok((target, origin))
}

fn build_args(positionals: &[String]) -> Result<(String, Origin, PathBuf), Error> {
    let mut target: Option<String> = None;
    let mut origin = Origin::User;
    let mut out: Option<PathBuf> = None;
    let mut words = positionals.iter().peekable();
    while let Some(word) = words.next() {
        if word == "--origin" {
            let value = words.next().ok_or(Error::Missing(
                "shipped|declared|user|installed after --origin",
            ))?;
            origin = match value.as_str() {
                "shipped" => Origin::Shipped,
                "declared" => Origin::Declared,
                "user" => Origin::User,
                "installed" => Origin::Installed,
                _ => {
                    return Err(Error::Invalid {
                        what: "--origin",
                        value: value.clone(),
                    });
                }
            };
        } else if word == "--out" {
            let value = words
                .next()
                .ok_or(Error::Missing("a directory after --out"))?;
            if out.is_some() {
                return Err(Error::Unknown(word.clone()));
            }
            out = Some(PathBuf::from(value));
        } else if target.is_none() {
            target = Some(word.clone());
        } else {
            return Err(hinted(
                "flag",
                word.clone(),
                &["--out", "--origin", "--json", "--state", "--socket"],
                "scoot look build --help",
            ));
        }
    }
    let target = target.ok_or(Error::Missing("a directory or a look name"))?;
    let out = out.ok_or(Error::Missing("a directory after --out"))?;
    Ok((target, origin, out))
}

fn hinted(kind: &'static str, what: String, candidates: &[&str], topic: &'static str) -> Error {
    match scoot_ipc::suggest(&what, candidates.iter().copied()) {
        Some(suggestion) => Error::Hinted {
            kind,
            what,
            suggestion: suggestion.to_owned(),
            topic,
        },
        None => Error::Unknown(what),
    }
}

/// Runs one `scoot look` invocation: prints the reply and returns whether
/// the run succeeded. Notes and warnings go to stderr in human mode, into
/// the document in `--json`; a failure is `Err` for exit 1.
pub fn run(look: &Look) -> Result<(), String> {
    match &look.command {
        Command::List => run_list(look),
        Command::Current => run_current(look),
        Command::Show { name } => run_show(look, name),
        Command::Validate { target, origin } => run_validate(look, target, *origin),
        Command::Build {
            target,
            out,
            origin,
        } => run_build(look, target, out, *origin),
        Command::Apply { name, no_toolkit } => run_apply(look, name, *no_toolkit),
        Command::Toolkit { action } => run_toolkit(look, *action),
    }
}

fn warn_all(notes: &[String]) {
    for note in notes {
        output::warn(format_args!("note: {note}"));
    }
}

fn run_list(look: &Look) -> Result<(), String> {
    let catalog = discover();
    if look.json {
        let looks: Vec<serde_json::Value> = catalog
            .entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "name": entry.name,
                    "title": entry.raw.title.clone().unwrap_or_default(),
                    "polarity": entry.raw.polarity.map(|p| p.as_str()).unwrap_or("dark"),
                    "origin": origin_name(entry.origin),
                    "source": entry.dir,
                })
            })
            .collect();
        let shadowed: Vec<serde_json::Value> = catalog
            .shadowed
            .iter()
            .map(|(name, dir)| serde_json::json!({"name": name, "source": dir}))
            .collect();
        output::print_line(
            &serde_json::to_string_pretty(&serde_json::json!({
                "looks": looks,
                "shadowed": shadowed,
                "notes": catalog.notes,
            }))
            .unwrap_or_default(),
        )
        .map_err(|error| error.to_string())?;
    } else {
        if catalog.entries.is_empty() {
            output::print_line("no looks found").map_err(|error| error.to_string())?;
        }
        for entry in &catalog.entries {
            let title = entry.raw.title.clone().unwrap_or_default();
            let polarity = entry.raw.polarity.map(|p| p.as_str()).unwrap_or("dark");
            output::print_line(&format!(
                "{}  {} ({}, {})",
                entry.name,
                title,
                origin_name(entry.origin),
                polarity
            ))
            .map_err(|error| error.to_string())?;
        }
        for (name, dir) in &catalog.shadowed {
            output::print_line(&format!(
                "{}  shadowed by an earlier catalog entry ({})",
                name,
                dir.display()
            ))
            .map_err(|error| error.to_string())?;
        }
        warn_all(&catalog.notes);
    }
    Ok(())
}

fn run_current(look: &Look) -> Result<(), String> {
    let state_dir = resolve_state_dir(look.state.as_deref())?;
    let record = match scoot_look::state::read_record(&state_dir) {
        scoot_look::state::RecordRead::Present(record) => Some(record),
        scoot_look::state::RecordRead::Missing => None,
        scoot_look::state::RecordRead::Corrupt(reason) => {
            return Err(format!("the recorded choice is corrupt: {reason}"));
        }
    };
    let link = state_dir.join(scoot_look::state::LINK_NAME);
    let link_target = std::fs::read_link(&link)
        .ok()
        .map(|target| target.display().to_string());
    let consistent = match (&record, &link_target) {
        (Some(record), Some(target)) => target.as_str() == record.rendered,
        (None, None) => true,
        _ => false,
    };
    // Shadowed keys, read from the files against the active render.
    let mut shadowed = Vec::new();
    let mut notes = Vec::new();
    if let Some(record) = &record {
        let target = PathBuf::from(&record.rendered);
        let (keys, mut file_notes) = apply::shadowed_from_files(&target);
        shadowed = keys;
        notes.append(&mut file_notes);
        if !consistent {
            notes.push(format!(
                "the link points at {} but look.json records {}: the next apply repairs it",
                link_target.as_deref().unwrap_or("(nothing)"),
                record.rendered
            ));
        }
    } else if link_target.is_some() {
        notes.push(
            "the link points somewhere with no recorded choice: the next apply repairs it"
                .to_owned(),
        );
    }
    shadowed.sort();
    shadowed.dedup();
    if look.json {
        output::print_line(
            &serde_json::to_string_pretty(&serde_json::json!({
                "active": record.as_ref().map(|record| serde_json::json!({
                    "name": record.name,
                    "rendered": record.rendered,
                    "generation": record.generation,
                    "chosen_by": record.chosen_by,
                    "toolkit": record.toolkit.unwrap_or(true),
                })),
                "link": link_target,
                "consistent": consistent,
                "shadowed": shadowed,
                "notes": notes,
            }))
            .unwrap_or_default(),
        )
        .map_err(|error| error.to_string())?;
    } else {
        match &record {
            Some(record) => {
                output::print_line(&format!(
                    "{} (generation {})",
                    record.name, record.generation
                ))
                .map_err(|error| error.to_string())?;
                output::print_line(&format!("rendered: {}", record.rendered))
                    .map_err(|error| error.to_string())?;
            }
            None => {
                output::print_line("no look applied yet").map_err(|error| error.to_string())?;
            }
        }
        if shadowed.is_empty() {
            output::print_line("shadowed keys: none").map_err(|error| error.to_string())?;
        } else {
            output::print_line(&format!("shadowed keys: {}", shadowed.join(", ")))
                .map_err(|error| error.to_string())?;
        }
        warn_all(&notes);
    }
    Ok(())
}

fn run_show(look: &Look, name: &str) -> Result<(), String> {
    let catalog = discover();
    let entry = catalog
        .find(name)
        .ok_or_else(|| unknown_look_here(name, &catalog))?;
    let merged = scoot_look::resolve(&catalog.sources(), name)
        .map_err(|error| format!("cannot resolve `{name}`: {error}"))?;
    let report = scoot_look::check_contrast(&merged, entry.origin);
    let contrast: Vec<serde_json::Value> = report
        .pairs
        .iter()
        .map(|pair| {
            serde_json::json!({
                "name": pair.name,
                "ratio": pair.ratio,
                "threshold": pair.threshold,
                "refuses": pair.refuses,
            })
        })
        .collect();
    let warnings: Vec<String> = report
        .warnings()
        .iter()
        .map(|warning| {
            format!(
                "{} is {} (needs {})",
                warning.name, warning.ratio, warning.threshold
            )
        })
        .collect();
    let refusals: Vec<String> = report
        .refusals()
        .iter()
        .map(|refusal| {
            format!(
                "{} is {} (needs {})",
                refusal.name, refusal.ratio, refusal.threshold
            )
        })
        .collect();
    if look.json {
        let palette = &merged.palette;
        output::print_line(
            &serde_json::to_string_pretty(&serde_json::json!({
                "name": merged.name,
                "title": merged.title,
                "description": merged.description,
                "polarity": merged.polarity.as_str(),
                "origin": origin_name(entry.origin),
                "source": entry.dir,
                "license": merged.license,
                "credits": merged.credits,
                "palette": {
                    "background": palette.background, "foreground": palette.foreground,
                    "dim": palette.dim, "accent": palette.accent, "urgent": palette.urgent,
                    "surface": palette.surface, "hover": palette.hover, "focus": palette.focus,
                    "border": palette.border, "accent-fg": palette.accent_fg,
                    "selection": palette.selection, "selection-text": palette.selection_text,
                    "match": palette.r#match,
                },
                "terminal": merged.terminal,
                "fonts": {
                    "ui": merged.fonts.ui, "sans": merged.fonts.sans, "mono": merged.fonts.mono,
                    "size": merged.fonts.size, "popup-size": merged.fonts.popup_size,
                    "app-size": merged.fonts.app_size, "terminal-size": merged.fonts.terminal_size,
                },
                "opacity": {"bar": merged.opacity.bar, "popups": merged.opacity.popups, "terminal": merged.opacity.terminal},
                "shape": {"radius": merged.shape.radius, "ring-width": merged.shape.ring_width, "ring-inactive-width": merged.shape.ring_inactive_width},
                "cursor": {"theme": merged.cursor.theme, "size": merged.cursor.size, "accent": merged.cursor.accent},
                "wallpaper": merged.wallpaper.as_ref().map(|wallpaper| serde_json::json!({
                    "image": wallpaper.image, "color": wallpaper.color,
                    "mode": wallpaper.mode.map(|mode| mode.as_str()),
                    "fill": wallpaper.fill, "transition": wallpaper.transition.map(|transition| transition.as_str()),
                })),
                "icons": merged.icons_theme,
                "night": merged.night_temperature,
                "extends": entry.raw.extends,
                "contrast": contrast,
                "warnings": warnings,
                "refusals": refusals,
            }))
            .unwrap_or_default(),
        )
        .map_err(|error| error.to_string())?;
    } else {
        let print = |line: &str| output::print_line(line).map_err(|error| error.to_string());
        print(&format!("{} — {}", merged.name, merged.title))?;
        if !merged.description.is_empty() {
            print(&merged.description)?;
        }
        print(&format!(
            "polarity {}, origin {}, source {}",
            merged.polarity.as_str(),
            origin_name(entry.origin),
            entry.dir.display()
        ))?;
        let palette = &merged.palette;
        print(&format!(
            "palette: background {} foreground {} dim {} accent {} urgent {}",
            palette.background, palette.foreground, palette.dim, palette.accent, palette.urgent
        ))?;
        print(&format!(
            "         surface {} hover {} focus {} border {} accent-fg {}",
            palette.surface, palette.hover, palette.focus, palette.border, palette.accent_fg
        ))?;
        print(&format!(
            "         selection {} selection-text {} match {}",
            palette.selection, palette.selection_text, palette.r#match
        ))?;
        print(&format!(
            "fonts: {} / {} / {} ({}pt, popup {}pt, app {}pt, terminal {}pt)",
            merged.fonts.ui,
            merged.fonts.sans,
            merged.fonts.mono,
            merged.fonts.size,
            merged.fonts.popup_size,
            merged.fonts.app_size,
            merged.fonts.terminal_size
        ))?;
        print(&format!(
            "shape: radius {} ring {} inactive {}; opacity: bar {} popups {} terminal {}",
            merged.shape.radius,
            merged.shape.ring_width,
            merged.shape.ring_inactive_width,
            merged.opacity.bar,
            merged.opacity.popups,
            merged.opacity.terminal
        ))?;
        print(&format!(
            "cursor: {} {} accent {}; icons {}; night {:?}",
            merged.cursor.theme,
            merged.cursor.size,
            merged.cursor.accent,
            merged.icons_theme,
            merged.night_temperature
        ))?;
        match &merged.wallpaper {
            Some(wallpaper) => print(&format!(
                "wallpaper: {}",
                wallpaper
                    .image
                    .as_deref()
                    .or(wallpaper.color.as_deref())
                    .unwrap_or("(none)")
            ))?,
            None => print("wallpaper: none")?,
        }
        for warning in &warnings {
            print(&format!("contrast warning: {warning}"))?;
        }
        for refusal in &refusals {
            print(&format!("contrast refusal: {refusal}"))?;
        }
        warn_all(&catalog.notes);
    }
    Ok(())
}

fn unknown_look_here(name: &str, catalog: &Catalog) -> String {
    let mut names: Vec<&str> = catalog
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    names.sort();
    if names.is_empty() {
        return format!("unknown look `{name}`: the catalog is empty");
    }
    format!("unknown look `{name}` (available: {})", names.join(", "))
}

fn run_validate(look: &Look, target: &str, origin: Origin) -> Result<(), String> {
    let catalog = discover();
    let checks = apply::ProdChecks::new();
    let path = PathBuf::from(target);
    let validation = if path.is_dir() {
        validate_dir(&path, origin, &catalog, &checks)?
    } else {
        check_name(target).map_err(|error| error.to_string())?;
        validate_entry(&catalog, target, &checks)?
    };
    let valid = validation.errors.is_empty();
    if look.json {
        output::print_line(
            &serde_json::to_string_pretty(&serde_json::json!({
                "target": target,
                "valid": valid,
                "errors": validation.errors,
                "warnings": validation.warnings,
                "notes": validation.notes,
            }))
            .unwrap_or_default(),
        )
        .map_err(|error| error.to_string())?;
    } else {
        if valid {
            output::print_line(&format!("valid: {target}")).map_err(|error| error.to_string())?;
        } else {
            for error in &validation.errors {
                output::warn(format_args!("error: {error}"));
            }
        }
        for warning in &validation.warnings {
            output::print_line(&format!("warning: {target}: {warning}"))
                .map_err(|error| error.to_string())?;
        }
        warn_all(&validation.notes);
    }
    if valid {
        Ok(())
    } else {
        Err(format!("invalid: {target}"))
    }
}

fn run_build(look: &Look, target: &str, out: &Path, origin: Origin) -> Result<(), String> {
    let catalog = discover();
    let checks = apply::ProdChecks::new();
    let path = PathBuf::from(target);
    let build_target = if path.is_dir() {
        BuildTarget::Dir { path, origin }
    } else {
        check_name(target).map_err(|error| error.to_string())?;
        BuildTarget::Name(target.to_owned())
    };
    let report = build_to_out(&build_target, out, &catalog, &checks)?;
    if look.json {
        output::print_line(
            &serde_json::to_string_pretty(&serde_json::json!({
                "source": target,
                "name": report.name,
                "out": report.out,
                "warnings": report.warnings,
                "notes": report.notes,
            }))
            .unwrap_or_default(),
        )
        .map_err(|error| error.to_string())?;
    } else {
        output::print_line(&format!("built {target} -> {}", report.out))
            .map_err(|error| error.to_string())?;
        for warning in &report.warnings {
            output::print_line(&format!("warning: {warning}"))
                .map_err(|error| error.to_string())?;
        }
        warn_all(&report.notes);
    }
    Ok(())
}

fn run_apply(look: &Look, name: &str, no_toolkit: bool) -> Result<(), String> {
    let catalog = discover();
    let cache = apply::render_cache_dir()?;
    let state_dir = resolve_state_dir(look.state.as_deref())?;
    let options = ApplyOptions {
        lock_wait: apply::LOCK_WAIT,
        no_toolkit,
    };
    let mut compositor: Box<dyn CompositorReload> = match &look.socket {
        Some(path) => Box::new(IpcCompositor::explicit(path.clone())),
        None => Box::new(IpcCompositor::default_socket()),
    };
    let (bars, notes) = bar_targets();
    let bar_refs: Vec<&dyn reload::BarReload> = bars
        .iter()
        .map(|bar| bar as &dyn reload::BarReload)
        .collect();
    let checks = apply::ProdChecks::new();
    let dconf = CliDconf::new();
    match apply_with(
        name,
        &options,
        &catalog,
        &cache,
        &state_dir,
        None,
        compositor.as_mut(),
        &bar_refs,
        &checks,
        &dconf,
    ) {
        Ok(report) => {
            let mut all_notes = notes;
            all_notes.extend(report.notes.iter().cloned());
            if look.json {
                output::print_line(
                    &serde_json::to_string_pretty(&serde_json::json!({
                        "applied": report.name,
                        "rendered": report.rendered,
                        "generation": report.generation,
                        "reloaded": report.reloaded,
                        "skipped": report.skipped,
                        "shadowed": report.shadowed,
                        "invalid": report.invalid,
                        "notes": all_notes,
                    }))
                    .unwrap_or_default(),
                )
                .map_err(|error| error.to_string())?;
            } else {
                output::print_line(&format!("applied: {}", report.name))
                    .map_err(|error| error.to_string())?;
                if !report.reloaded.is_empty() {
                    output::print_line(&format!("reloaded: {}", report.reloaded.join(", ")))
                        .map_err(|error| error.to_string())?;
                }
                if report.shadowed.is_empty() {
                    output::print_line("shadowed keys: none").map_err(|error| error.to_string())?;
                } else {
                    output::print_line(&format!("shadowed keys: {}", report.shadowed.join(", ")))
                        .map_err(|error| error.to_string())?;
                }
                if !report.invalid.is_empty() {
                    output::print_line(&format!(
                        "invalid user keys: {}",
                        report.invalid.join(", ")
                    ))
                    .map_err(|error| error.to_string())?;
                }
                // Skips and notes go to stderr in human mode: stdout stays
                // the reply.
                for skipped in &report.skipped {
                    output::warn(format_args!("skipped: {skipped}"));
                }
                for note in &all_notes {
                    output::warn(format_args!("note: {note}"));
                }
            }
            Ok(())
        }
        Err(failure) => Err(failure.to_string()),
    }
}

fn run_toolkit(look: &Look, action: Option<bool>) -> Result<(), String> {
    let state_dir = resolve_state_dir(look.state.as_deref())?;
    let mut record = match scoot_look::state::read_record(&state_dir) {
        scoot_look::state::RecordRead::Present(record) => record,
        scoot_look::state::RecordRead::Missing => {
            return Err("no look applied yet: toolkit state lives in look.json (`apply --no-toolkit` skips the first write)".to_owned());
        }
        scoot_look::state::RecordRead::Corrupt(reason) => {
            return Err(format!("the recorded choice is corrupt: {reason}"));
        }
    };
    if let Some(on) = action {
        use scoot_look::state::acquire_lock;
        let _lock = acquire_lock(&state_dir, apply::LOCK_WAIT)
            .map_err(|error| format!("cannot lock the state dir: {error}"))?;
        record.toolkit = Some(on);
        scoot_look::state::write_record(&state_dir, &record)
            .map_err(|error| format!("cannot record the toolkit state: {error}"))?;
    }
    let on = toolkit_on(&record);
    if look.json {
        output::print_line(
            &serde_json::to_string_pretty(&serde_json::json!({
                "toolkit": on,
                "color_scheme": record.color_scheme,
            }))
            .unwrap_or_default(),
        )
        .map_err(|error| error.to_string())?;
    } else {
        output::print_line(if on {
            "toolkit writes: on"
        } else {
            "toolkit writes: off"
        })
        .map_err(|error| error.to_string())?;
        match &record.color_scheme {
            Some(scheme) => output::print_line(&format!("last color-scheme written: {scheme}"))
                .map_err(|error| error.to_string())?,
            None => output::print_line("no color-scheme written yet")
                .map_err(|error| error.to_string())?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
