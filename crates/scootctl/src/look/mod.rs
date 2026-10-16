//! `scoot look`: list, inspect and atomically swap looks.
//!
//! A subcommand of the existing CLI, not a new user-facing binary: `scoot`
//! is already the client on every platform. Every command takes `--json`
//! (machine-readable on stdout, notes on stderr); exit codes follow
//! `scoot msg` (0 applied, 1 the run failed, 2 the invocation was wrong).
//!
//! Offline commands (`list`, `show`, `validate`, `build`, `current`) work
//! with no Nix, no home-manager and no compositor running. `apply` with
//! nothing running records the choice and moves the link, and the next
//! session starts on it; with apps running it drives each app's
//! control-socket reload, scoped to the look, and rolls back on refusal.
//!
//! A crash between the repoint and the reloads leaves the apps on the old
//! look with the link on the new one (the design's section 4.5 order: the
//! apps read the link, so it moves before they reload). Re-running
//! `apply`, or any app's next reload, heals it: each scoped reload reads
//! the link fresh.

pub mod apply;
pub mod build_dir;
pub mod catalog;
pub mod cycle;
pub mod foot;
pub mod leaves;
pub mod reload;
pub mod toolkit;

use std::path::{Path, PathBuf};

use scoot_look::Origin;

use crate::cli::Error;
use crate::output;

use leaves::CliLeaves;

use apply::{
    ApplyOptions, BuildTarget, apply_with, build_to_out, resolve_state_dir, validate_dir,
    validate_entry,
};
use catalog::{Catalog, discover, origin_name};
use reload::{CompositorReload, IpcCompositor, bar_targets};
use scoot_look::state::Record;
use toolkit::{CliDconf, Dconf, toolkit_off, toolkit_on};

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
    Apply {
        name: Option<String>,
        no_toolkit: bool,
        from_compositor: bool,
        for_secs: Option<u64>,
        keep: bool,
        only_if_generation: Option<u64>,
    },
    /// The next, previous or a random catalog look, in search order.
    Cycle { step: CycleStep },
    /// The detached `--for` reverter (hidden: spawned by `apply --for`,
    /// never typed). Sleeps out the delay, then reverts only while the
    /// generation still matches.
    PreviewRevert { from: u64, after: u64 },
    /// Show or persist the toolkit writes (`on` or `off`).
    Toolkit { action: Option<bool> },
}

/// Which catalog step a `Cycle` command takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CycleStep {
    Next,
    Prev,
    Random,
}

/// Every look command, for the JSON document and `did you mean` guesses.
/// `_preview-revert` stays out: it is spawned, never typed.
pub const COMMANDS: &[&str] = &[
    "list", "current", "show", "validate", "build", "apply", "next", "prev", "random", "toolkit",
];

/// The usage line, shared by `scoot look --help` and the top-level help.
pub const LOOK_USAGE: &str = "\
    look list [--json]
    look current [--json]
    look show NAME [--json]
    look validate DIR|NAME [--origin shipped|declared|user|installed] [--json]
    look build DIR|NAME --out DIR [--origin shipped|declared|user|installed] [--json]
    look apply [NAME] [--for SECONDS] [--keep] [--no-toolkit] [--from-compositor] [--only-if-generation N] [--json]
    look next|prev|random [--json]
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
            // Empty counts as unset, like the environment rule.
            state = (!dir.is_empty()).then(|| PathBuf::from(dir));
            continue;
        }
        if let Some(path) = word.strip_prefix("--socket=") {
            // Empty counts as unset, like the environment rule.
            socket = (!path.is_empty()).then(|| PathBuf::from(path));
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
        "apply" => parse_apply(&positionals)?,
        "next" => {
            refuse_trailing(&positionals)?;
            Command::Cycle {
                step: CycleStep::Next,
            }
        }
        "prev" => {
            refuse_trailing(&positionals)?;
            Command::Cycle {
                step: CycleStep::Prev,
            }
        }
        "random" => {
            refuse_trailing(&positionals)?;
            Command::Cycle {
                step: CycleStep::Random,
            }
        }
        "_preview-revert" => parse_preview_revert(&positionals)?,
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
                    "list", "current", "show", "validate", "build", "apply", "next", "prev",
                    "random", "toolkit",
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
                signal the third-party daemons (mako, fcitx5, the OSD and
                night-light units, best-effort), rolling back on a
                refusal; --no-toolkit skips the
                toolkit step (dconf keys and toolkit files, including
                the color-scheme write) for this apply (the first apply
                with no record adopts each key whatever it holds);
                --from-compositor marks the choice as compositor-driven
                (what the compositor's own IPC child passes; `current`
                shows it);
                --for SECONDS previews the look and reverts after the
                delay (a detached child, so closing the terminal never
                strands it); --keep with a name applies for good and
                cancels any pending preview, bare keeps the current look;
                --only-if-generation N applies only while the recorded
                generation still equals N (the compositor's timed-preview
                revert passes the generation its preview wrote, so the
                revert stands down instead of undoing a newer choice)
    next        the next catalog look after the active one, wrapping
    prev        the previous catalog look before the active one, wrapping
    random      a random catalog look other than the active one
    toolkit     show whether the toolkit step is on, or persist
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
    // A slashless target that is not a valid name can never validate (a
    // directory by that name fails the name-matches-directory check), so
    // it refuses at parse (exit 2), like `show` and `apply` do. A target
    // with a slash, `.` or `..` is directory-shaped and defers to the run.
    if !target.contains('/') && target != "." && target != ".." {
        check_name(&target)?;
    }
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
        } else if let Some(value) = word.strip_prefix("--out=") {
            if value.is_empty() {
                return Err(Error::Missing("a directory after --out"));
            }
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
    // Slashless non-names refuse at parse, as in `validate`.
    if !target.contains('/') && target != "." && target != ".." {
        check_name(&target)?;
    }
    Ok((target, origin, out))
}

fn parse_apply(positionals: &[String]) -> Result<Command, Error> {
    let mut name: Option<String> = None;
    let mut no_toolkit = false;
    let mut from_compositor = false;
    let mut for_secs: Option<u64> = None;
    let mut keep = false;
    let mut only_if_generation: Option<u64> = None;
    let mut words = positionals.iter().peekable();
    while let Some(word) = words.next() {
        if word == "--no-toolkit" {
            no_toolkit = true;
        } else if word == "--from-compositor" {
            from_compositor = true;
        } else if word == "--keep" {
            keep = true;
        } else if word == "--only-if-generation" {
            let value = words
                .next()
                .ok_or(Error::Missing("a generation after --only-if-generation"))?;
            only_if_generation = Some(value.parse::<u64>().map_err(|_| Error::Invalid {
                what: "--only-if-generation",
                value: value.clone(),
            })?);
        } else if let Some(value) = word.strip_prefix("--only-if-generation=") {
            if value.is_empty() {
                return Err(Error::Missing("a generation after --only-if-generation"));
            }
            only_if_generation = Some(value.parse::<u64>().map_err(|_| Error::Invalid {
                what: "--only-if-generation",
                value: value.to_owned(),
            })?);
        } else if word == "--for" {
            let value = words.next().ok_or(Error::Missing("seconds after --for"))?;
            for_secs = Some(parse_seconds(value)?);
        } else if let Some(value) = word.strip_prefix("--for=") {
            if value.is_empty() {
                return Err(Error::Missing("seconds after --for"));
            }
            for_secs = Some(parse_seconds(value)?);
        } else if name.is_none() {
            name = Some(word.clone());
        } else {
            return Err(Error::Unknown(word.clone()));
        }
    }
    if for_secs.is_some() && keep {
        return Err(Error::Invalid {
            what: "--for with --keep",
            value: "a preview that is kept needs no timer".to_owned(),
        });
    }
    let Some(name) = name else {
        if keep {
            return Ok(Command::Apply {
                name: None,
                no_toolkit,
                from_compositor,
                for_secs: None,
                keep: true,
                only_if_generation,
            });
        }
        return Err(Error::Missing("a look name"));
    };
    check_name(&name)?;
    Ok(Command::Apply {
        name: Some(name),
        no_toolkit,
        from_compositor,
        for_secs,
        keep,
        only_if_generation,
    })
}

/// Seconds for `apply --for`: a positive integer. Zero would revert before
/// the swap's own reloads land; the reverter sleeps this long detached, so
/// there is no upper bound worth enforcing beyond the integer itself.
fn parse_seconds(value: &str) -> Result<u64, Error> {
    match value.parse::<u64>() {
        Ok(secs) if secs >= 1 => Ok(secs),
        _ => Err(Error::Invalid {
            what: "--for",
            value: value.to_owned(),
        }),
    }
}

/// The detached reverter's argv (hidden: spawned by `apply --for`, never
/// typed): `_preview-revert --from GENERATION --after SECONDS`.
fn parse_preview_revert(positionals: &[String]) -> Result<Command, Error> {
    let mut from: Option<u64> = None;
    let mut after: Option<u64> = None;
    let mut words = positionals.iter().peekable();
    while let Some(word) = words.next() {
        if word == "--from" {
            let value = words
                .next()
                .ok_or(Error::Missing("a generation after --from"))?;
            from = Some(value.parse::<u64>().map_err(|_| Error::Invalid {
                what: "--from",
                value: value.clone(),
            })?);
        } else if word == "--after" {
            let value = words
                .next()
                .ok_or(Error::Missing("seconds after --after"))?;
            after = Some(value.parse::<u64>().map_err(|_| Error::Invalid {
                what: "--after",
                value: value.clone(),
            })?);
        } else {
            return Err(Error::Unknown(word.clone()));
        }
    }
    match (from, after) {
        (Some(from), Some(after)) => Ok(Command::PreviewRevert { from, after }),
        _ => Err(Error::Missing(
            "a generation after --from and seconds after --after",
        )),
    }
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
        Command::Apply {
            name,
            no_toolkit,
            from_compositor,
            for_secs,
            keep,
            only_if_generation,
        } => run_apply(
            look,
            name.as_deref(),
            *no_toolkit,
            *from_compositor,
            *for_secs,
            *keep,
            *only_if_generation,
        ),
        Command::Cycle { step } => run_cycle(look, *step),
        Command::PreviewRevert { from, after } => run_preview_revert(look, *from, *after),
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
    // A bare name is always a catalog look; a path needs a slash (use
    // `./name` for a directory in the cwd), so a cwd directory named like
    // a look never shadows the catalog.
    let is_path = target.contains('/') || target == "." || target == "..";
    let path = PathBuf::from(target);
    let validation = if is_path {
        if path.is_dir() {
            validate_dir(&path, origin, &catalog, &checks)?
        } else {
            return Err(format!(
                "`{target}` is not a directory: validate takes a directory or a catalog name (`./{target}` for a cwd directory)"
            ));
        }
    } else {
        let mut validation = validate_entry(&catalog, target, &checks)?;
        if origin != Origin::User {
            validation.notes.push(
                "`--origin` ignored for catalog names: the origin comes from the directory"
                    .to_owned(),
            );
        }
        validation
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
    // Bare names are catalog looks; paths need a slash (`./name` for cwd).
    let is_path = target.contains('/') || target == "." || target == "..";
    let path = PathBuf::from(target);
    let build_target = if is_path {
        if path.is_dir() {
            BuildTarget::Dir { path, origin }
        } else {
            return Err(format!(
                "`{target}` is not a directory: build takes a directory or a catalog name"
            ));
        }
    } else {
        if origin != Origin::User {
            output::warn(format_args!(
                "note: `--origin` ignored for catalog names: the origin comes from the directory"
            ));
        }
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

fn run_apply(
    look: &Look,
    name: Option<&str>,
    no_toolkit: bool,
    from_compositor: bool,
    for_secs: Option<u64>,
    keep: bool,
    only_if_generation: Option<u64>,
) -> Result<(), String> {
    let Some(name) = name else {
        return run_keep(look);
    };
    // Word the `--keep` note honestly: peek at the preview file before
    // the swap moves the generation out from under it.
    let had_preview = resolve_state_dir(look.state.as_deref())
        .map(|dir| matches!(cycle::read_preview(&dir), cycle::PreviewRead::Present(_)))
        .unwrap_or(false);
    let options = ApplyOptions {
        lock_wait: apply::LOCK_WAIT,
        no_toolkit,
        from_compositor,
        preview: for_secs.is_some(),
        only_if_generation,
    };
    let armed = apply_and_print(look, name, &options, for_secs)?;
    if keep && had_preview {
        output::warn(format_args!(
            "note: kept {name}: the pending preview is cancelled"
        ));
    }
    if let (Some(preview), Some(secs)) = (armed, for_secs) {
        arm_preview_timer(look, preview.from_generation, secs)?;
    }
    Ok(())
}

/// Runs one cycling step: resolves the neighbor in catalog order and
/// applies it through the same transaction a named apply uses. The target
/// is picked before the swap takes its lock, so two quick presses can land
/// on the same look.
fn run_cycle(look: &Look, step: CycleStep) -> Result<(), String> {
    let catalog = discover();
    let state_dir = resolve_state_dir(look.state.as_deref())?;
    let current = match scoot_look::state::read_record(&state_dir) {
        scoot_look::state::RecordRead::Present(record) => Some(record.name),
        scoot_look::state::RecordRead::Missing => None,
        scoot_look::state::RecordRead::Corrupt(reason) => {
            return Err(format!("the recorded choice is corrupt: {reason}"));
        }
    };
    let target = match step {
        CycleStep::Next => cycle::cycle_target(&catalog, current.as_deref(), cycle::Step::Next),
        CycleStep::Prev => cycle::cycle_target(&catalog, current.as_deref(), cycle::Step::Prev),
        CycleStep::Random => cycle::random_target(&catalog, current.as_deref()),
    };
    let Some(target) = target else {
        return Err("the catalog is empty".to_owned());
    };
    let options = ApplyOptions {
        lock_wait: apply::LOCK_WAIT,
        no_toolkit: false,
        from_compositor: false,
        preview: false,
        only_if_generation: None,
    };
    apply_and_print(look, &target, &options, None).map(|_| ())
}

/// Keeps a timed preview: forgets the pending revert, staying on the
/// current look.
fn run_keep(look: &Look) -> Result<(), String> {
    let state_dir = resolve_state_dir(look.state.as_deref())?;
    match cycle::keep_preview(&state_dir, apply::LOCK_WAIT) {
        Ok(name) => {
            if look.json {
                output::print_line(
                    &serde_json::to_string_pretty(&serde_json::json!({"kept": name}))
                        .unwrap_or_default(),
                )
                .map_err(|error| error.to_string())?;
            } else {
                output::print_line(&format!("kept: {name}")).map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        Err(message) => Err(message),
    }
}

/// The detached reverter (hidden `_preview-revert`): sleeps out the delay,
/// then reverts only while the generation still matches. Never holds a
/// lock across the sleep: the previewing apply released its own long ago.
/// A swap running when the delay ends waits the reverter out (bounded
/// retries, guards re-checked each round), never drops the revert.
fn run_preview_revert(look: &Look, from: u64, after: u64) -> Result<(), String> {
    if after > 0 {
        std::thread::sleep(std::time::Duration::from_secs(after));
    }
    let catalog = discover();
    let cache = apply::render_cache_dir()?;
    let state_dir = resolve_state_dir(look.state.as_deref())?;
    let mut compositor: Box<dyn CompositorReload> = match &look.socket {
        Some(path) => Box::new(IpcCompositor::explicit(path.clone())),
        None => Box::new(IpcCompositor::default_socket()),
    };
    let (bars, _) = bar_targets();
    let bar_refs: Vec<&dyn reload::BarReload> = bars
        .iter()
        .map(|bar| bar as &dyn reload::BarReload)
        .collect();
    let checks = apply::ProdChecks::new();
    let dconf = CliDconf::new();
    let leaves = CliLeaves::new();
    match cycle::revert_with_retry(
        from,
        &catalog,
        &cache,
        &state_dir,
        None,
        compositor.as_mut(),
        &bar_refs,
        &checks,
        &dconf,
        &leaves,
        apply::LOCK_WAIT,
        cycle::REVERT_RETRIES,
        cycle::REVERT_RETRY_DELAY,
    ) {
        Ok(cycle::RevertOutcome::Reverted { name }) => {
            let line = match &name {
                Some(name) => format!("reverted to {name}"),
                None => "reverted: no look active".to_owned(),
            };
            if look.json {
                output::print_line(
                    &serde_json::to_string_pretty(&serde_json::json!({
                        "reverted_to": name,
                        "from_generation": from,
                    }))
                    .unwrap_or_default(),
                )
                .map_err(|error| error.to_string())?;
            } else {
                output::print_line(&line).map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        Ok(cycle::RevertOutcome::Superseded) | Ok(cycle::RevertOutcome::NothingPending) => {
            if look.json {
                output::print_line(
                    &serde_json::to_string_pretty(&serde_json::json!({
                        "reverted_to": serde_json::Value::Null,
                        "from_generation": from,
                        "superseded": true,
                    }))
                    .unwrap_or_default(),
                )
                .map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        Err(message) => Err(message),
    }
}

/// Spawns the detached reverter after a `--for` apply armed its preview.
/// The child outlives this CLI (its own session, stdio to null). The
/// generation comes from the completed swap itself, not a re-read, so a
/// concurrent newer apply between the swap and the spawn cannot retarget
/// the timer: the reverter simply finds its generation moved and stands
/// down. When the spawn itself fails the preview would stick past its
/// delay, so the file is forgotten loudly and the look stays applied,
/// plainly said.
fn arm_preview_timer(look: &Look, from_generation: u64, secs: u64) -> Result<(), String> {
    let state_dir = resolve_state_dir(look.state.as_deref())?;
    let state_dir = apply::absolutize(&state_dir).map_err(|error| error.to_string())?;
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let (_, args) = cycle::reverter_argv(
        &exe,
        &state_dir,
        look.socket.as_deref(),
        from_generation,
        secs,
    );
    match cycle::spawn_reverter(&exe, &args) {
        Ok(()) => {
            output::warn(format_args!(
                "note: preview: reverts in {secs}s (`scoot look apply --keep` keeps it)"
            ));
            Ok(())
        }
        Err(error) => {
            cycle::clear_preview(&state_dir);
            Err(format!(
                "applied, but {error}: the preview timer never started, so the look stays until changed"
            ))
        }
    }
}

fn apply_and_print(
    look: &Look,
    name: &str,
    options: &ApplyOptions,
    for_secs: Option<u64>,
) -> Result<Option<cycle::Preview>, String> {
    let catalog = discover();
    let cache = apply::render_cache_dir()?;
    let state_dir = resolve_state_dir(look.state.as_deref())?;
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
    let leaves = CliLeaves::new();
    match apply_with(
        name,
        options,
        &catalog,
        &cache,
        &state_dir,
        None,
        compositor.as_mut(),
        &bar_refs,
        &checks,
        &dconf,
        &leaves,
    ) {
        Ok(report) => {
            let mut all_notes = notes;
            all_notes.extend(report.notes.iter().cloned());
            let preview_json = report.preview.as_ref().map(|preview| {
                serde_json::json!({
                    "revert_to": preview.revert_to,
                    "from_generation": preview.from_generation,
                })
            });
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
                        "preview": preview_json,
                        "notes": all_notes,
                    }))
                    .unwrap_or_default(),
                )
                .map_err(|error| error.to_string())?;
            } else {
                output::print_line(&format!("applied: {}", report.name))
                    .map_err(|error| error.to_string())?;
                if let (Some(preview), Some(secs)) = (&report.preview, for_secs) {
                    let back = preview.revert_to.as_deref().unwrap_or("(no look)");
                    output::print_line(&format!(
                        "preview: reverts to {back} in {secs}s (`scoot look apply --keep` keeps it)"
                    ))
                    .map_err(|error| error.to_string())?;
                }
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
            Ok(report.preview.clone())
        }
        Err(failure) => {
            // `--json` still prints a document on stdout (like `validate`
            // does): the failure shape is `{"target", "applied": false,
            // "error", "rolled_back_to", "rollback_errors"}. The text error
            // still goes to stderr and exits 1.
            if look.json {
                output::print_line(
                    &serde_json::to_string_pretty(&serde_json::json!({
                        "target": name,
                        "applied": false,
                        "error": failure.message,
                        "rolled_back_to": failure.rolled_back_to,
                        "rollback_errors": failure.rollback_errors,
                    }))
                    .unwrap_or_default(),
                )
                .map_err(|error| error.to_string())?;
            }
            Err(failure.to_string())
        }
    }
}

/// `toolkit off`: stop owning `color-scheme`. Restores the live value
/// from before the tool's first write (see [`toolkit_off`]). Either way
/// the record clears, so after `off` then `on` the next apply adopts
/// fresh instead of reporting "set outside `scoot look`" over a stale
/// write.
fn run_toolkit_off(state_dir: &Path, record: &mut Record, dconf: &dyn Dconf) {
    for note in toolkit_off(state_dir, record, dconf) {
        output::warn(format_args!("note: {note}"));
    }
}

fn run_toolkit(look: &Look, action: Option<bool>) -> Result<(), String> {
    let state_dir = resolve_state_dir(look.state.as_deref())?;
    // A mutation holds the lock across its read and its write: a
    // concurrent apply between the two would otherwise be overwritten
    // with a stale record (a generation rewind plus a lost
    // `color-scheme` ownership). A status read takes no lock.
    use scoot_look::state::acquire_lock;
    let _lock = match action {
        Some(_) => Some(
            acquire_lock(&state_dir, apply::LOCK_WAIT)
                .map_err(|error| format!("cannot lock the state dir: {error}"))?,
        ),
        None => None,
    };
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
        record.toolkit = Some(on);
        // Turning off stops owning the key: restore the live value from
        // before the tool's first write, but only when the tool wrote
        // what is still live. Anything else (never wrote, someone else
        // changed it, unreadable, a private state) leaves the key alone.
        // Either way the record clears, so a later `on` adopts fresh
        // instead of crying "set outside" over a stale write.
        if !on {
            let dconf = CliDconf::new();
            run_toolkit_off(&state_dir, &mut record, &dconf);
        }
        scoot_look::state::write_record(&state_dir, &record)
            .map_err(|error| format!("cannot record the toolkit state: {error}"))?;
    }
    let on = toolkit_on(&record);
    let keys = record.dconf_wrote.clone();
    if look.json {
        output::print_line(
            &serde_json::to_string_pretty(&serde_json::json!({
                "toolkit": on,
                "color_scheme": keys.get("color-scheme"),
                "keys": keys,
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
        if keys.is_empty() {
            output::print_line("no toolkit keys written yet").map_err(|error| error.to_string())?;
        } else {
            for (key, value) in &keys {
                output::print_line(&format!("last {key} written: {value}"))
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
