//! The toolkit step: dconf keys plus the owned toolkit files, rendered
//! once at apply, with no resident daemon.
//!
//! GTK's live knobs are dconf keys (portal-gtk for GTK 4 and libadwaita,
//! GSettings for GTK 3); the look's colors reach new GTK apps through the
//! `gtk.css` `@import`, and new Qt apps through qt6ct's `color_scheme_path`
//! (both through the state link, so a swap rewrites no file). Running GTK
//! apps keep their `gtk.css` palette and running Qt apps their palette
//! until they restart; the reply says so.
//!
//! The rules, both layers of "never overwrite a value someone else set"
//! (design D11):
//!
//! - Skipped entirely with `apply --no-toolkit`, or persistently after
//!   `scoot look toolkit off` (kept in `look.json` as `toolkit = false`).
//! - Before writing, the tool reads the live value. Unset, or equal to what
//!   the tool last wrote (kept in `look.json` per key), means the tool owns
//!   the key: write the look's value and record it.
//! - Anything else means someone else changed it (a settings app, a hand
//!   `gsettings set`): leave it and name it in the reply.
//! - The one exception is the first write with no record yet: the tool
//!   adopts the key, whatever the live value is. On a non-Nix box there is
//!   no Nix key-name list, so the first `apply` overwrites a value the user
//!   chose in a settings app — the opt-outs above ship with it, and the
//!   docs say so where the first apply is described.
//! - Files follow the same rule by content: absent or carrying the
//!   `Managed by scoot look` marker means the tool owns the path; anything
//!   else (a hand-written `gtk.css`, a home-manager link into the store)
//!   is left alone and named.
//!
//! A box without `dconf` gets a note, not a refusal. Every step here is
//! best-effort and reported; a toolkit failure never refuses a swap.

use std::path::Path;

use scoot_look::state::{Record, read_capped_follow, write_atomic};
use scoot_look::{
    ResolvedLook, font_desc, gtk_theme_name, render_fontconfig, render_gtk_ini, render_qt6ct_conf,
};

/// The dconf key the tool owned first (kept for the tests and the reply).
pub const COLOR_SCHEME_KEY: &str = "/org/gnome/desktop/interface/color-scheme";

/// Every dconf key the step owns, as (short record name, full path).
/// All six live under `org.gnome.desktop.interface`: the dark-mode signal
/// GTK 4 and libadwaita follow through portal-gtk, GTK 3's theme switch,
/// and the icon, cursor and font knobs GTK 3 reads directly.
pub const TOOL_KEYS: &[(&str, &str)] = &[
    ("color-scheme", "/org/gnome/desktop/interface/color-scheme"),
    ("gtk-theme", "/org/gnome/desktop/interface/gtk-theme"),
    ("icon-theme", "/org/gnome/desktop/interface/icon-theme"),
    ("cursor-theme", "/org/gnome/desktop/interface/cursor-theme"),
    ("cursor-size", "/org/gnome/desktop/interface/cursor-size"),
    ("font-name", "/org/gnome/desktop/interface/font-name"),
];

/// The value for a dark look, as `dconf write` takes it.
pub const PREFER_DARK: &str = "'prefer-dark'";
/// The value for a light look, as `dconf write` takes it.
pub const PREFER_LIGHT: &str = "'prefer-light'";

/// Whether the record says the toolkit step runs. Absent means on:
/// only an explicit `toolkit off` skips every dconf write and file.
pub fn toolkit_on(record: &Record) -> bool {
    record.toolkit.unwrap_or(true)
}

/// The `color-scheme` value for a polarity: dark looks read dark.
pub fn want_for_polarity(dark: bool) -> &'static str {
    if dark { PREFER_DARK } else { PREFER_LIGHT }
}

/// Wraps a toolkit string in dconf's single quotes. Every value this
/// wraps passes the identifier grammar upstream (no quotes possible), but
/// a hand-built look skips that gate, so a quote here refuses rather than
/// injecting a second GVariant.
fn quote_dconf_str(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 128 || value.contains('\0') || value.contains('\'') {
        return Err(format!("toolkit value is not a plain string: `{value}`"));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(format!("toolkit value is not a plain string: `{value}`"));
    }
    Ok(format!("'{value}'"))
}

/// The wanted dconf values for a look, as (short name, value): the
/// dark-mode signal and theme from the polarity, icons, cursor and font
/// from their tokens.
pub fn wanted_dconf(look: &ResolvedLook) -> Result<Vec<(String, String)>, String> {
    let dark = look.polarity == scoot_look::schema::Polarity::Dark;
    Ok(vec![
        (
            "color-scheme".to_owned(),
            want_for_polarity(dark).to_owned(),
        ),
        (
            "gtk-theme".to_owned(),
            quote_dconf_str(gtk_theme_name(dark))?,
        ),
        ("icon-theme".to_owned(), quote_dconf_str(&look.icons_theme)?),
        (
            "cursor-theme".to_owned(),
            quote_dconf_str(&look.cursor.theme)?,
        ),
        ("cursor-size".to_owned(), look.cursor.size.to_string()),
        (
            "font-name".to_owned(),
            quote_dconf_str(&font_desc(&look.fonts.sans, look.fonts.app_size))?,
        ),
    ])
}

/// One dconf read and write, so tests script the settings backend.
pub trait Dconf {
    /// The live value: `Ok(None)` when unset, `Ok(Some)` when set,
    /// `Err` when unreadable. Unreadable is never "unset": callers leave
    /// the key alone rather than recording an original they never saw.
    fn read(&self, key: &str) -> Result<Option<String>, String>;
    /// Writes `value`, exactly as given.
    fn write(&self, key: &str, value: &str) -> Result<(), String>;
    /// Resets `key` to unset (what it was before the tool's first write).
    fn reset(&self, key: &str) -> Result<(), String>;
    /// Whether the backend exists at all. A missing `dconf` is a note.
    fn available(&self) -> bool;
}

/// The real backend: the `dconf` CLI on `PATH`.
pub struct CliDconf {
    program: Option<std::path::PathBuf>,
}

impl CliDconf {
    pub fn new() -> Self {
        Self {
            program: find_on_path("dconf"),
        }
    }
}

impl Default for CliDconf {
    fn default() -> Self {
        Self::new()
    }
}

impl Dconf for CliDconf {
    fn available(&self) -> bool {
        self.program.is_some()
    }

    fn read(&self, key: &str) -> Result<Option<String>, String> {
        let program = self
            .program
            .as_ref()
            .ok_or_else(|| "no `dconf`".to_owned())?;
        let out = super::apply::run_with_timeout(program, &["read", key])
            .map_err(|error| format!("cannot read {key}: {error}"))?;
        if !out.status.success() {
            return Err(format!("`dconf read {key}` failed"));
        }
        let text =
            String::from_utf8(out.stdout).map_err(|_| format!("`dconf read {key}` not UTF-8"))?;
        let trimmed = text.trim().to_owned();
        // `dconf read` prints nothing for an unset key.
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed))
        }
    }

    fn write(&self, key: &str, value: &str) -> Result<(), String> {
        let program = self
            .program
            .as_ref()
            .ok_or_else(|| "no `dconf` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(program, &["write", key, value])?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("`dconf write {key}` failed"))
        }
    }

    fn reset(&self, key: &str) -> Result<(), String> {
        let program = self
            .program
            .as_ref()
            .ok_or_else(|| "no `dconf` on PATH".to_owned())?;
        let out = super::apply::run_with_timeout(program, &["reset", key])?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("`dconf reset {key}` failed"))
        }
    }
}

/// Finds `name` on `PATH`: each directory joined with the name, first
/// regular file wins. Symlinks resolve at exec time, which is what `PATH`
/// lookup means everywhere else.
pub fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| {
                std::fs::symlink_metadata(candidate)
                    .map(|meta| meta.is_file() || meta.is_symlink())
                    .unwrap_or(false)
            })
    })
}

/// Whether `state_dir` is a private look state (a headless or
/// `--private-look` session's): its leaf is `scoot-look-<display>`, or it
/// is the `$SCOOT_LOOK_STATE` dir. Toolkit writes never touch the user's
/// real dconf or configs from one unless explicitly asked (an explicit
/// `--state` pointing at the user state).
pub fn is_private_state(state_dir: &Path) -> bool {
    use std::path::Path as P;
    let leaf = state_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if leaf.starts_with(scoot_look::state::PRIVATE_PREFIX) {
        return true;
    }
    if let Some(session) = std::env::var_os(scoot_look::state::STATE_ENV)
        && !session.is_empty()
        && P::new(&session) == state_dir
    {
        return true;
    }
    false
}

/// Applies the toolkit dconf step for a swap: writes the look's keys under
/// the D11 rules, recording what was written in `record`. Returns the
/// notes for the reply. Never fails the swap: a missing backend or a failed
/// write is a note.
/// The original live value is saved once per key (before the first write)
/// into `record.dconf_orig` (`""` means it was unset), so
/// `toolkit off` can restore it. A private session never writes: its swaps
/// must not move the user's real desktop.
pub fn apply_toolkit(
    state_dir: &Path,
    record: &mut Record,
    look: &ResolvedLook,
    dconf: &dyn Dconf,
) -> Vec<String> {
    let mut notes = Vec::new();
    if !toolkit_on(record) {
        notes.push(
            "toolkit writes off (`scoot look toolkit off`): toolkit keys left alone".to_owned(),
        );
        return notes;
    }
    if is_private_state(state_dir) {
        notes.push(
            "private look state (headless or --private-look): toolkit keys left alone".to_owned(),
        );
        return notes;
    }
    if !dconf.available() {
        notes.push("no `dconf` on PATH: toolkit keys left alone".to_owned());
        return notes;
    }
    let wanted = match wanted_dconf(look) {
        Ok(wanted) => wanted,
        Err(error) => {
            notes.push(format!("toolkit keys not written: {error}"));
            return notes;
        }
    };
    for (name, want) in &wanted {
        let Some(path) = TOOL_KEYS
            .iter()
            .find(|(short, _)| short == name)
            .map(|(_, path)| *path)
        else {
            continue;
        };
        apply_one_key(record, name, path, want, dconf, &mut notes);
    }
    notes
}

/// One key under the D11 rules: read the live value, write when owned or
/// adopted, name whoever else owns it.
fn apply_one_key(
    record: &mut Record,
    name: &str,
    path: &str,
    want: &str,
    dconf: &dyn Dconf,
    notes: &mut Vec<String>,
) {
    // The live value first: an unreadable key stops here, before any
    // record or write. It must never map to "was unset".
    let live = match dconf.read(path) {
        Ok(live) => live,
        Err(error) => {
            notes.push(format!("{name} unreadable ({error}): left alone"));
            return;
        }
    };
    // Save the original once, before the first write, so `toolkit off`
    // can restore it. Only a readable live value is ever saved, and only
    // when the tool holds no record for the key at all.
    if !record.dconf_wrote.contains_key(name) && !record.dconf_orig.contains_key(name) {
        record
            .dconf_orig
            .insert(name.to_owned(), live.clone().unwrap_or_default());
    }
    match live {
        // Unset, or what the tool last wrote: the tool owns the key.
        None => write_and_record(record, name, path, want, dconf, notes),
        Some(live) if record.dconf_wrote.get(name).is_some_and(|w| *w == live) => {
            write_and_record(record, name, path, want, dconf, notes)
        }
        // No record yet: adopt the key (the documented first-apply
        // exception), whatever the live value is. The original is saved
        // above, so `toolkit off` restores it.
        Some(_) if !record.dconf_wrote.contains_key(name) => {
            notes.push(format!(
                "{name} had no scoot record: adopting it (use `apply --no-toolkit` to opt out)"
            ));
            write_and_record(record, name, path, want, dconf, notes);
        }
        Some(live) => {
            notes.push(format!(
                "{name} is {live} (set outside `scoot look`): left alone"
            ));
        }
    }
}

/// Writes `want` and records it; a failed write is a note naming the key.
fn write_and_record(
    record: &mut Record,
    name: &str,
    path: &str,
    want: &str,
    dconf: &dyn Dconf,
    notes: &mut Vec<String>,
) {
    match dconf.write(path, want) {
        Ok(()) => {
            record.dconf_wrote.insert(name.to_owned(), want.to_owned());
            notes.push(format!("{name} set to {want}"));
        }
        Err(error) => {
            notes.push(format!("{name} not written: {error}"));
        }
    }
}

/// `toolkit off`: stop owning every key. Restores each live value from
/// before the tool's first write (saved per key; empty means it was
/// unset, restored with `reset`) — but only when the tool wrote what
/// is still live. A key the tool never wrote, one someone else changed
/// since, or one that cannot be read is left alone with a note. A private
/// state never touches the real keys at all. Either way the record clears,
/// so after `off` then `on` the next apply adopts fresh instead of
/// reporting "set outside `scoot look`" over a stale write. Returns the
/// notes for the reply; a failed restore is a note, never a refusal.
pub fn toolkit_off(state_dir: &Path, record: &mut Record, dconf: &dyn Dconf) -> Vec<String> {
    let mut notes = Vec::new();
    if is_private_state(state_dir) {
        notes.push(
            "private look state (headless or --private-look): toolkit keys left alone".to_owned(),
        );
    } else if !dconf.available() {
        notes.push("no `dconf` on PATH: toolkit keys left alone".to_owned());
    } else {
        // `BTreeMap` iterates sorted, so the notes read the same way
        // every run.
        for name in record.dconf_wrote.keys().cloned().collect::<Vec<_>>() {
            let Some(path) = TOOL_KEYS
                .iter()
                .find(|(short, _)| short == &name)
                .map(|(_, path)| *path)
            else {
                continue;
            };
            let written = record.dconf_wrote.get(&name).cloned().unwrap_or_default();
            match dconf.read(path) {
                Ok(Some(live)) if live == written => {
                    let restored = match record.dconf_orig.get(&name).map(String::as_str) {
                        Some("") => dconf.reset(path),
                        Some(orig) => dconf.write(path, orig),
                        // A record that wrote before the original was saved:
                        // the restore target is unknown, so leave it.
                        None => {
                            notes.push(format!("{name} has no saved original: left alone"));
                            continue;
                        }
                    };
                    if let Err(error) = restored {
                        notes.push(format!("{name} not restored: {error}"));
                    }
                }
                Ok(Some(live)) => {
                    notes.push(format!(
                        "{name} is {live} (set outside `scoot look`): left alone"
                    ));
                }
                Ok(None) => {
                    notes.push(format!("{name} already unset: nothing to restore"));
                }
                Err(error) => {
                    notes.push(format!("{name} unreadable ({error}): left alone"));
                }
            }
        }
        if record.dconf_wrote.is_empty() {
            notes.push("no toolkit keys written by `scoot look`: left alone".to_owned());
        }
    }
    record.dconf_wrote.clear();
    record.dconf_orig.clear();
    notes
}

/// The marker a tool-owned file carries near its top: absent or
/// marked means the tool owns the path; anything else is someone else's
/// file and is left alone. Line 1 for CSS, INI and qt6ct; behind the XML
/// declaration for fontconfig (see [`is_managed`]).
pub const MANAGED_MARK: &str = "Managed by scoot look";

/// One toolkit file, relative to the config home.
pub struct OwnedFile {
    /// `gtk-3.0/gtk.css`, `qt6ct/qt6ct.conf`, and so on: one `/`, no
    /// `..`, never absolute.
    pub rel: &'static str,
    /// The full wanted contents, marker first.
    pub contents: String,
}

/// The wanted toolkit files for a look: the two `gtk.css` `@import`s of
/// the link's `gtk/gtk.css` (new GTK apps take the new palette), the two
/// `settings.ini`s, the qt6ct config through the link's scheme, and the
/// fontconfig default. `link` is the state link's own path, so the
/// indirection survives every swap with no rewrite.
pub fn wanted_files(look: &ResolvedLook, link: &Path) -> Result<Vec<OwnedFile>, String> {
    let link_text = link
        .to_str()
        .ok_or_else(|| "the state link path is not UTF-8: toolkit files left alone".to_owned())?;
    if link_text.contains('\0')
        || link_text.contains('"')
        || link_text.chars().any(|c| c.is_control())
    {
        return Err("the state link path is not a plain path: toolkit files left alone".to_owned());
    }
    let css = format!(
        "/* {MANAGED_MARK}. Do not edit: re-rendered on every apply. New apps read the active look through the state link. */\n@import url(\"file://{link_text}/gtk/gtk.css\");\n"
    );
    let ini = render_gtk_ini(look);
    let scheme_path = format!("{link_text}/qt/scoot-look.conf");
    let qt_conf = render_qt6ct_conf(look, &scheme_path)
        .map_err(|error| format!("cannot render qt6ct.conf: {error}"))?;
    // The marker rides on its own `#` line (what the profile's generated
    // file already opens with, and what qt6ct's parser tolerates in the
    // scheme beside it), so ownership survives a re-read.
    let qt_conf = format!("# {MANAGED_MARK}. Do not edit: re-rendered on every apply.\n{qt_conf}");
    let fonts = render_fontconfig(look);
    // The marker rides behind the XML declaration (which must stay
    // first), where the ownership read still finds it.
    let fonts = fonts.replacen(
        "?>\n",
        &format!("?>\n<!-- {MANAGED_MARK}. Do not edit: re-rendered on every apply. -->\n"),
        1,
    );
    Ok(vec![
        OwnedFile {
            rel: "gtk-3.0/gtk.css",
            contents: css.clone(),
        },
        OwnedFile {
            rel: "gtk-4.0/gtk.css",
            contents: css,
        },
        OwnedFile {
            rel: "gtk-3.0/settings.ini",
            contents: format!("# {MANAGED_MARK}. Do not edit: re-rendered on every apply.\n{ini}"),
        },
        OwnedFile {
            rel: "gtk-4.0/settings.ini",
            contents: format!("# {MANAGED_MARK}. Do not edit: re-rendered on every apply.\n{ini}"),
        },
        OwnedFile {
            rel: "qt6ct/qt6ct.conf",
            contents: qt_conf,
        },
        OwnedFile {
            rel: "fontconfig/conf.d/10-scoot-look.conf",
            contents: fonts,
        },
    ])
}

/// Applies the toolkit files for a swap: writes each owned file under
/// `config_home`, leaving foreign files alone. Best-effort notes, never a
/// refusal. `config_home` is `None` when no home resolves at all.
pub fn apply_toolkit_files(
    state_dir: &Path,
    record: &Record,
    look: &ResolvedLook,
    config_home: Option<&Path>,
    link: &Path,
) -> Vec<String> {
    let mut notes = Vec::new();
    if !toolkit_on(record) {
        notes.push(
            "toolkit writes off (`scoot look toolkit off`): toolkit files left alone".to_owned(),
        );
        return notes;
    }
    if is_private_state(state_dir) {
        notes.push(
            "private look state (headless or --private-look): toolkit files left alone".to_owned(),
        );
        return notes;
    }
    let Some(config_home) = config_home else {
        notes.push("no home directory: toolkit files left alone".to_owned());
        return notes;
    };
    let files = match wanted_files(look, link) {
        Ok(files) => files,
        Err(error) => {
            notes.push(error);
            return notes;
        }
    };
    for file in &files {
        notes.extend(apply_one_file(config_home, file));
    }
    notes
}

/// Whether `text` is a tool-owned file: the marker sits in the first
/// lines (line 1 for CSS, INI and qt6ct; line 4 for fontconfig, behind
/// the XML declaration and doctype, which must stay first).
fn is_managed(text: &str) -> bool {
    text.lines().take(5).any(|line| line.contains(MANAGED_MARK))
}

/// One file under the own-it-if-absent-or-marked rule.
fn apply_one_file(config_home: &Path, file: &OwnedFile) -> Vec<String> {
    let mut notes = Vec::new();
    let path = config_home.join(file.rel);
    let live = match read_capped_follow(&path) {
        Ok(text) => Some(text),
        Err(error) if std::io::ErrorKind::NotFound == error.kind() => None,
        Err(error) => {
            notes.push(format!("{} unreadable ({error}): left alone", file.rel));
            return notes;
        }
    };
    match live {
        None => {
            if let Err(error) = write_owned(config_home, file) {
                notes.push(format!("cannot write {}: {error}: left alone", file.rel));
            } else {
                notes.push(format!("wrote {}", file.rel));
            }
        }
        Some(live) if live == file.contents => {}
        Some(live) if is_managed(&live) => {
            if let Err(error) = write_owned(config_home, file) {
                notes.push(format!("cannot write {}: {error}: left alone", file.rel));
            } else {
                notes.push(format!("updated {}", file.rel));
            }
        }
        Some(_) => {
            notes.push(format!(
                "{} is managed outside `scoot look`: left alone",
                file.rel
            ));
        }
    }
    notes
}

/// Writes one owned file atomically (temp file plus `rename` in the same
/// directory, so a reader never sees a half-written config). The parent
/// directory is created first.
fn write_owned(config_home: &Path, file: &OwnedFile) -> Result<(), String> {
    let (dir, name) = file
        .rel
        .rsplit_once('/')
        .ok_or_else(|| "toolkit file has no parent".to_owned())?;
    let dir = config_home.join(dir);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    write_atomic(&dir, name, file.contents.as_bytes())
}

#[cfg(test)]
mod tests;
