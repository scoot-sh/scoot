//! The desktop entry index: data dirs and ids, with precedence and
//! `Hidden` masking; the format and its escapes; localized keys;
//! `OnlyShowIn`/`NotShowIn` and `TryExec`; actions; the Exec quoting
//! and field codes; the cache; symlinks followed with a visited set.
//!
//! Plus `scootlaunch --list [--all] [--lenient] [--json]`, which is
//! all this ticket's binary does. Ranking, the panel, launching and
//! the dmenu mode are later tickets.

pub mod actions;
pub mod cache;
pub mod dirs;
pub mod exec;
pub mod filter;
pub mod locale;
pub mod parse;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// At most this many entries are indexed; past it new files are
/// skipped with a note. Bounds the panel, the JSON and the cache.
pub const MAX_ENTRIES: usize = 4096;
/// At most this many keywords per entry; past it the list is cut with
/// a note. Real entries carry a handful.
pub const MAX_KEYWORDS: usize = 1024;
/// A desktop file past this size is skipped, never truncated into a
/// wrong entry.
pub const MAX_FILE: usize = 64 * 1024;

/// The process environment as the index reads it: explicit fields, so
/// tests construct interesting environments without touching the
/// process-global `std::env` (which nextest isolates per process but
/// `cargo test` shares across threads).
#[derive(Debug, Clone)]
pub struct Env {
    /// `XDG_DATA_HOME`, or `~/.local/share`. Relative values are
    /// ignored (the spec's paths are absolute or defaulted).
    pub xdg_data_home: Option<PathBuf>,
    /// `XDG_DATA_DIRS` in order, empties and relative entries
    /// dropped; unset or empty means [`dirs::DEFAULT_DATA_DIRS`].
    pub xdg_data_dirs: Vec<PathBuf>,
    /// `XDG_CURRENT_DESKTOP`, colon-separated, in order.
    pub xdg_current_desktop: Vec<String>,
    /// `XDG_CACHE_HOME/scootlaunch`, or `~/.cache/scootlaunch`.
    /// `None` (no way to name a cache dir) means no cache, never an
    /// error.
    pub xdg_cache_home: Option<PathBuf>,
    /// `XDG_STATE_HOME`, or `~/.local/state`: the frecency history
    /// lives under it (`scootlaunch/history`). `None` means no
    /// history, never an error.
    pub xdg_state_home: Option<PathBuf>,
    /// The raw locale (`LC_ALL`, then `LC_MESSAGES`, then `LANG`),
    /// kept for the cache key: any change rescans.
    pub locale_raw: String,
    /// The parsed locale, or `None` for empty/`C`/`POSIX` (the
    /// default key is used on its own).
    pub locale: Option<locale::Locale>,
    /// `$PATH` in order, for `TryExec`.
    pub path_dirs: Vec<PathBuf>,
}

impl Env {
    /// Reads the process environment (the binary's entry point; tests
    /// build [`Env`] literally instead).
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let absolute = |value: std::ffi::OsString| {
            let path = PathBuf::from(value);
            path.is_absolute().then_some(path)
        };
        let xdg_data_home = std::env::var_os("XDG_DATA_HOME")
            .and_then(absolute)
            .or_else(|| home.clone().map(|dir| dir.join(".local/share")));
        let xdg_data_dirs = std::env::var_os("XDG_DATA_DIRS")
            .map(|raw| {
                raw.to_string_lossy()
                    .split(':')
                    .filter(|part| !part.is_empty())
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .collect::<Vec<_>>()
            })
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| dirs::DEFAULT_DATA_DIRS.iter().map(PathBuf::from).collect());
        let xdg_current_desktop = std::env::var_os("XDG_CURRENT_DESKTOP")
            .map(|raw| {
                raw.to_string_lossy()
                    .split(':')
                    .filter(|part| !part.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let xdg_cache_home = std::env::var_os("XDG_CACHE_HOME")
            .and_then(absolute)
            .or_else(|| home.clone().map(|dir| dir.join(".cache")));
        let xdg_state_home = std::env::var_os("XDG_STATE_HOME")
            .and_then(absolute)
            .or_else(|| home.clone().map(|dir| dir.join(".local/state")));
        let locale_raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|value| value.to_string_lossy().into_owned())
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        let locale = locale::parse_locale(&locale_raw);
        let path_dirs = std::env::var_os("PATH")
            .map(|raw| std::env::split_paths(&raw).collect())
            .unwrap_or_default();
        Env {
            xdg_data_home,
            xdg_data_dirs,
            xdg_current_desktop,
            xdg_cache_home,
            xdg_state_home,
            locale_raw,
            locale,
            path_dirs,
        }
    }
}

/// One desktop action: its id, translated name, and expanded argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub name: String,
    pub exec: Vec<String>,
}

/// One indexed entry: everything `--list --json` prints and the later
/// tickets rank, draw and launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    pub icon: Option<String>,
    pub exec: Vec<String>,
    pub terminal: bool,
    pub dbus_activatable: bool,
    pub actions: Vec<Action>,
    /// The desktop file, through whatever symlinks led to it.
    pub path: PathBuf,
    /// The `Path=` working directory, if the file names one. The
    /// launch reads it (else `$HOME`); the cache carries it opaquely.
    pub workdir: Option<String>,
    /// Why a hidden entry is hidden. `None` shows.
    pub hidden: Option<filter::HiddenReason>,
}

/// The agent JSON schema (`dev/design/scootlaunch.md`, The agent
/// interface): `id`, `name`, `generic_name`, `comment`, `keywords`,
/// `icon` (name or path), `exec` (the expanded argv), `terminal`,
/// `dbus_activatable`, `actions` (`[{id, name}]`), `path` (the
/// desktop file) and `hidden` (null, or the reason).
#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonEntry {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generic_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub exec: Vec<String>,
    pub terminal: bool,
    pub dbus_activatable: bool,
    pub actions: Vec<JsonAction>,
    pub path: String,
    pub hidden: Option<String>,
}

/// One action in the agent schema.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonAction {
    pub id: String,
    pub name: String,
}

impl JsonEntry {
    pub fn from_entry(entry: &Entry) -> Self {
        JsonEntry {
            id: entry.id.clone(),
            name: entry.name.clone(),
            generic_name: entry.generic_name.clone(),
            comment: entry.comment.clone(),
            keywords: entry.keywords.clone(),
            icon: entry.icon.clone(),
            exec: entry.exec.clone(),
            terminal: entry.terminal,
            dbus_activatable: entry.dbus_activatable,
            actions: entry
                .actions
                .iter()
                .map(|action| JsonAction {
                    id: action.id.clone(),
                    name: action.name.clone(),
                })
                .collect(),
            path: entry.path.to_string_lossy().into_owned(),
            hidden: entry
                .hidden
                .as_ref()
                .map(|reason| reason.as_str().to_string()),
        }
    }
}

/// What `build` may use.
#[derive(Debug, Clone, Copy)]
pub struct BuildOpts {
    /// Read and write `$XDG_CACHE_HOME/scootlaunch/index-v1`.
    pub use_cache: bool,
    /// Exec quoting: accept what GLib accepts, not just the spec.
    pub lenient_exec: bool,
}

/// The built index: entries sorted by id for deterministic output,
/// plus the debug lines the caller prints.
#[derive(Debug, Clone)]
pub struct Index {
    pub entries: Vec<Entry>,
    pub notes: Vec<String>,
}

/// Builds the index: walks every data dir in precedence order, keeps
/// the first file per id (a `Hidden` first file masks the id), drops
/// invalid entries with a note, and caches under the walked
/// identities plus the locale and `XDG_CURRENT_DESKTOP`.
pub fn build(env: &Env, opts: &BuildOpts) -> Index {
    let app_dirs = dirs::resolve_app_dirs(env);
    let mut key_entries = Vec::new();
    let mut found: Vec<(dirs::FoundFile, usize)> = Vec::new();
    let mut notes = Vec::new();
    for (dir_index, app) in app_dirs.iter().enumerate() {
        let (files, mut key, mut dir_notes) = dirs::walk(app);
        key_entries.append(&mut key);
        notes.append(&mut dir_notes);
        for file in files {
            found.push((file, dir_index));
        }
    }
    let key = cache::CacheKey::new(key_entries, &env.locale_raw, &env.xdg_current_desktop);
    if opts.use_cache
        && let Some(entries) = cache::load(env, &key)
    {
        return Index { entries, notes };
    }
    let index = scan(env, opts, &found, &mut notes);
    if opts.use_cache {
        cache::store(env, &key, &index.entries);
    }
    Index {
        entries: index.entries,
        notes,
    }
}

struct Scanned {
    entries: Vec<Entry>,
}

/// Reads, parses and filters every found file: first id wins (a
/// `Hidden=true` first file masks the id across data dirs), unknown
/// types are ignored, invalid entries are dropped with a note.
fn scan(
    env: &Env,
    opts: &BuildOpts,
    found: &[(dirs::FoundFile, usize)],
    notes: &mut Vec<String>,
) -> Scanned {
    let mut entries = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (file, _) in found {
        if entries.len() >= MAX_ENTRIES {
            notes.push(format!(
                "{}: past {MAX_ENTRIES} entries, further files skipped",
                file.path.display()
            ));
            break;
        }
        if !seen.insert(file.id.clone()) {
            notes.push(format!(
                "{}: id `{}` already indexed, the first wins",
                file.path.display(),
                file.id
            ));
            continue;
        }
        let bytes = match read_file(&file.path) {
            Ok(bytes) => bytes,
            Err(note) => {
                notes.push(format!("{}: {note}", file.path.display()));
                continue;
            }
        };
        let parsed = match parse::parse(&bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                notes.push(format!("{}: {error:?}, skipped", file.path.display()));
                continue;
            }
        };
        notes.extend(
            parsed
                .notes
                .iter()
                .map(|note| format!("{}: {note}", file.path.display())),
        );
        let (entry, mut entry_notes) = entry_from_file(env, opts, &file.id, &file.path, &parsed);
        notes.append(&mut entry_notes);
        if let Some(entry) = entry {
            entries.push(entry);
        }
    }
    entries.sort_by(|a: &Entry, b: &Entry| a.id.cmp(&b.id));
    Scanned { entries }
}

/// Builds one entry, or drops it with its reason. Every refusal names
/// the file, so `--list --all --json`'s hidden reasons and these notes
/// together answer "an app is missing".
fn entry_from_file(
    env: &Env,
    opts: &BuildOpts,
    id: &str,
    path: &Path,
    parsed: &parse::ParsedFile,
) -> (Option<Entry>, Vec<String>) {
    let mut notes = Vec::new();
    // `parse` guarantees a leading `[Desktop Entry]`.
    let main = &parsed.groups[0];
    // A `Hidden=true` first file masks the id: the id does not exist.
    if main
        .get("Hidden")
        .and_then(parse::parse_bool)
        .unwrap_or(false)
    {
        notes.push(format!("{}: Hidden=true, masked", path.display()));
        return (None, notes);
    }
    // Only `Type=Application`; unknown types are ignored.
    if main.get("Type") != Some("Application") {
        notes.push(format!("{}: not Type=Application, ignored", path.display()));
        return (None, notes);
    }
    let lookup = |key: &str| main.get(key);
    let Some(name) = locale::get_localized(lookup, "Name", env.locale.as_ref()) else {
        notes.push(format!("{}: no Name, dropped", path.display()));
        return (None, notes);
    };
    let name = parse::clean_display(&name);
    if name.is_empty() {
        notes.push(format!("{}: empty Name, dropped", path.display()));
        return (None, notes);
    }
    let generic_name = locale::get_localized(lookup, "GenericName", env.locale.as_ref())
        .map(|value| parse::clean_display(&value))
        .filter(|value| !value.is_empty());
    let comment = locale::get_localized(lookup, "Comment", env.locale.as_ref())
        .map(|value| parse::clean_display(&value))
        .filter(|value| !value.is_empty());
    let mut keywords = keywords_for(main, env);
    if keywords.len() > MAX_KEYWORDS {
        keywords.truncate(MAX_KEYWORDS);
        notes.push(format!(
            "{}: more than {MAX_KEYWORDS} keywords, cut",
            path.display()
        ));
    }
    let icon = main
        .get("Icon")
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let terminal = main
        .get("Terminal")
        .and_then(parse::parse_bool)
        .unwrap_or(false);
    let dbus_activatable = main
        .get("DBusActivatable")
        .and_then(parse::parse_bool)
        .unwrap_or(false);
    // The lossy path lives in this binding so the expansion can
    // borrow it for the `%k` code.
    let path_string = path.to_string_lossy();
    let expansion = exec::Expansion {
        icon: icon.as_deref(),
        name: &name,
        desktop_path: &path_string,
    };
    let exec_argv = match main.get("Exec") {
        Some(raw) => match exec::expand(raw, &expansion, terminal, opts.lenient_exec) {
            Ok(argv) => argv,
            Err(error) => {
                notes.push(format!("{}: bad Exec ({error}), dropped", path.display()));
                return (None, notes);
            }
        },
        None if dbus_activatable => Vec::new(),
        None => {
            notes.push(format!("{}: no Exec, dropped", path.display()));
            return (None, notes);
        }
    };
    // Actions expand with the same rules as the main Exec; an invalid
    // action Exec drops the action, never the entry.
    let mut action_notes = Vec::new();
    let defs = actions::collect(parsed, env.locale.as_ref(), &mut action_notes, id);
    notes.extend(
        action_notes
            .iter()
            .map(|note| format!("{}: {note}", path.display())),
    );
    let mut actions = Vec::with_capacity(defs.len());
    for def in defs {
        let action_expansion = exec::Expansion {
            icon: icon.as_deref(),
            name: &name,
            desktop_path: &path_string,
        };
        match exec::expand(&def.exec, &action_expansion, terminal, opts.lenient_exec) {
            Ok(argv) => actions.push(Action {
                id: def.id,
                name: def.name,
                exec: argv,
            }),
            Err(error) => notes.push(format!(
                "{}: action `{}` bad Exec ({error}), ignored",
                path.display(),
                def.id
            )),
        }
    }
    let hidden = hidden_for(main, env);
    let workdir = main
        .get("Path")
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    (
        Some(Entry {
            id: id.to_string(),
            name,
            generic_name,
            comment,
            keywords,
            icon,
            exec: exec_argv,
            terminal,
            dbus_activatable,
            actions,
            path: path.to_path_buf(),
            workdir,
            hidden,
        }),
        notes,
    )
}

/// The hidden verdict for a valid entry, in order: `NoDisplay` first
/// (still reachable by `--run-id`, a later ticket), then
/// `OnlyShowIn`/`NotShowIn`, then `TryExec`.
fn hidden_for(main: &parse::Group, env: &Env) -> Option<filter::HiddenReason> {
    if main
        .get("NoDisplay")
        .and_then(parse::parse_bool)
        .unwrap_or(false)
    {
        return Some(filter::HiddenReason::NoDisplay);
    }
    if let Some(reason) = filter::show_in_hidden(main, &env.xdg_current_desktop) {
        return Some(reason);
    }
    filter::try_exec_hidden(main, &env.path_dirs)
}

/// The localized `Keywords` as a list. The localized lookup
/// unescapes, which would fuse `\;` before the split sees it — so the
/// winning raw value is found first (the same order: the spec's table,
/// then the default), and split after.
fn keywords_for(main: &parse::Group, env: &Env) -> Vec<String> {
    let raw = match &env.locale {
        Some(locale) => {
            let mut found = None;
            for candidate in locale::candidates(locale) {
                let suffixed = format!("Keywords[{candidate}]");
                if let Some(raw) = main.get(&suffixed) {
                    found = Some(raw);
                    break;
                }
            }
            found.or_else(|| main.get("Keywords"))
        }
        None => main.get("Keywords"),
    };
    raw.and_then(|value| parse::split_list(value).ok())
        .unwrap_or_default()
}

/// Reads one desktop file: opened `O_NONBLOCK` (a FIFO swapped in
/// between the type check and the open must not block the launcher),
/// re-checked as regular after opening (TOCTOU), at most
/// [`MAX_FILE`] bytes.
fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    use rustix::fs::{Mode, OFlags};
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("cannot open: {error}"))?;
    let file = std::fs::File::from(fd);
    if !file.metadata().map(|meta| meta.is_file()).unwrap_or(false) {
        return Err("not a regular file".to_string());
    }
    let mut bytes = Vec::new();
    use std::io::Read as _;
    if file
        .take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Err("cannot read".to_string());
    }
    if bytes.len() > MAX_FILE {
        return Err(format!("larger than {} KiB", MAX_FILE / 1024));
    }
    Ok(bytes)
}
