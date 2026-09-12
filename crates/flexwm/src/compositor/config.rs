//! Loading `[layout]`/`[binds]` from a TOML config file into the types the
//! rest of the compositor already uses: [`flexwm_core::Config`] and
//! [`Keybindings`].
//!
//! # Failure semantics (read this before changing any of them)
//!
//! - An explicit `--config PATH` that doesn't exist or can't be read is a
//!   hard startup error (see [`load`]) -- the caller pointed at this file on
//!   purpose, so silently ignoring it would be worse than failing loud.
//! - Every other failure -- no file at the default path, malformed TOML, an
//!   unknown field (`deny_unknown_fields`), a bad individual bind -- logs a
//!   `tracing::error!`/`tracing::warn!` and falls back to defaults. It never
//!   fails startup.
//!
//! That second rule is deliberate and easy to "fix" into a hard failure
//! without understanding the cost of doing so: on `--tty`, the real
//! deployment target, flexwm *is* the session -- there is no other window
//! manager to fall back to and no easy remote access the way this project's
//! dev VM has over SSH. A compositor that refuses to start over a config
//! typo is a hard lockout on real hardware. Starting with defaults and
//! saying what's wrong in the log is strictly better than that, even though
//! it means a typo can go unnoticed until someone reads the log.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use flexwm_core::Config;
use serde::Deserialize;
use smithay::input::keyboard::Keysym;

use super::input::keysym_named;
use super::keybindings::{Bound, Keybindings, Modifiers};

/// `[layout]`. Mirrors `flexwm_core::Config` field-for-field, each optional
/// so a partial table (e.g. just `gap`) leaves the rest at their defaults
/// rather than requiring every field to be repeated.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct LayoutConfig {
    gap: Option<i32>,
    column_widths: Option<Vec<f64>>,
    default_column_width: Option<usize>,
}

impl LayoutConfig {
    fn into_config(self) -> Config {
        let defaults = Config::default();
        Config {
            gap: self.gap.unwrap_or(defaults.gap),
            column_widths: self.column_widths.unwrap_or(defaults.column_widths),
            default_column_width: self
                .default_column_width
                .unwrap_or(defaults.default_column_width),
        }
        // Range/sanity clamping (negative gap, an out-of-range default
        // index, ...) is `World::new`'s job via `Config::validated`, not
        // this module's -- no need to duplicate it here.
    }
}

/// The whole file. `binds`' values are parsed lazily, one at a time (see
/// [`apply_binds`]), so one bad bind can't take the rest down with it.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    layout: Option<LayoutConfig>,
    #[serde(default)]
    binds: HashMap<String, String>,
}

/// What loading a config file (or using defaults) produces: always a
/// complete, usable pair, never a partial state -- see the module doc.
#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
    pub keybindings: Keybindings,
}

impl LoadedConfig {
    fn defaults() -> Self {
        Self {
            config: Config::default(),
            keybindings: Keybindings::default(),
        }
    }

    fn from_file(file: FileConfig) -> Self {
        let config = file.layout.unwrap_or_default().into_config();
        let mut keybindings = Keybindings::default();
        apply_binds(&mut keybindings, file.binds);
        Self {
            config,
            keybindings,
        }
    }
}

/// An explicit `--config PATH` that doesn't exist or can't be read. The one
/// config-related failure that stops startup -- see the module doc.
#[derive(Debug)]
pub struct ConfigFileError {
    path: PathBuf,
    source: io::Error,
}

impl fmt::Display for ConfigFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "could not read config file `{}`: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for ConfigFileError {}

/// Resolves the config path and loads it. `explicit` is `--config PATH`,
/// when given. See the module doc for the three-way fallback behavior this
/// implements.
pub fn load(explicit: Option<&Path>) -> Result<LoadedConfig, ConfigFileError> {
    match explicit {
        Some(path) => load_from(path, true),
        None => {
            match default_path(
                std::env::var_os("XDG_CONFIG_HOME"),
                std::env::var_os("HOME"),
            ) {
                Some(path) => load_from(&path, false),
                // Neither XDG_CONFIG_HOME nor HOME is set: nowhere sensible
                // to look, same as "the default file doesn't exist".
                None => Ok(LoadedConfig::defaults()),
            }
        }
    }
}

/// `$XDG_CONFIG_HOME/flexwm/config.toml`, else `~/.config/flexwm/config.toml`.
/// A pure function of the two env vars it needs, like
/// `flexwm_ipc::socket::resolve`, so this is testable without touching real
/// environment state.
fn default_path(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let non_empty = |v: &OsString| !v.is_empty();
    xdg_config_home
        .filter(non_empty)
        .map(|dir| PathBuf::from(dir).join("flexwm/config.toml"))
        .or_else(|| {
            home.filter(non_empty)
                .map(|dir| PathBuf::from(dir).join(".config/flexwm/config.toml"))
        })
}

/// Loads `path`. `explicit` says whether this came from `--config`, which is
/// what decides whether a missing/unreadable file is a hard error or silent
/// defaults -- see the module doc.
fn load_from(path: &Path, explicit: bool) -> Result<LoadedConfig, ConfigFileError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if explicit => {
            return Err(ConfigFileError {
                path: path.to_owned(),
                source,
            });
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            // No config yet, or a fresh install: not a mistake, nothing to
            // warn about.
            return Ok(LoadedConfig::defaults());
        }
        Err(source) => {
            // The default path exists in some sense but couldn't be read
            // (permissions, a broken symlink, ...) -- same "never block
            // startup" rule as a parse failure below.
            tracing::error!(
                path = %path.display(), %source,
                "could not read the default config file; using defaults"
            );
            return Ok(LoadedConfig::defaults());
        }
    };
    Ok(parse_or_defaults(&text, path))
}

/// Parses `text`; a parse failure (malformed TOML, or a `deny_unknown_fields`
/// rejection) logs why and falls back to full defaults instead of failing
/// startup -- see the module doc.
fn parse_or_defaults(text: &str, path: &Path) -> LoadedConfig {
    match toml::from_str::<FileConfig>(text) {
        Ok(file) => LoadedConfig::from_file(file),
        Err(error) => {
            tracing::error!(
                path = %path.display(), %error,
                "could not parse config file; using defaults"
            );
            LoadedConfig::defaults()
        }
    }
}

/// Parses every `[binds]` entry and applies the ones that check out, on top
/// of whatever `keybindings` already holds (the defaults).
///
/// Two isolation rules, both in service of the module doc's "a config
/// mistake degrades gracefully" principle:
///
/// - A bind that fails to parse (bad combo syntax, an unknown key name, a
///   bad action string, trailing text after the action) is skipped with a
///   `tracing::warn!` naming just that bind; every other bind in the file
///   still loads.
/// - Two *different* combo strings that resolve to the same (modifiers,
///   key) once parsed -- aliases (`super`/`logo`/`meta`/`cmd`), modifier
///   order (`super+shift+h` vs `shift+super+h`), or case (`Super+H` vs
///   `super+h`, see `parse_combo`) -- can't be given a well-defined winner:
///   `binds` is a `HashMap`, and its iteration order has no relationship to
///   the order the keys were written in the TOML source, so "the last one
///   in the file wins" isn't something this code can honor truthfully.
///   Rather than pick a winner that would silently vary from run to run,
///   every bind in the colliding group is skipped, with one warning naming
///   all of them; whatever was bound to that combo before this file was
///   loaded (a default, or nothing) is left alone.
fn apply_binds(keybindings: &mut Keybindings, binds: HashMap<String, String>) {
    let mut parsed: Vec<(String, Modifiers, Keysym, Bound)> = Vec::new();
    for (raw, value) in &binds {
        match parse_bind(raw, value) {
            Ok((mods, keysym, bound)) => parsed.push((raw.clone(), mods, keysym, bound)),
            Err(reason) => {
                tracing::warn!(
                    bind = %raw, value = %value, %reason,
                    "skipping an invalid config-file bind"
                );
            }
        }
    }

    // Group by the resolved combo (keyed on the keysym's raw code rather
    // than the `Keysym` itself, which doesn't implement `Hash`) so
    // collisions are found regardless of `HashMap`'s iteration order.
    let mut groups: HashMap<(Modifiers, u32), Vec<(String, Bound)>> = HashMap::new();
    for (raw, mods, keysym, bound) in parsed {
        groups
            .entry((mods, keysym.raw()))
            .or_default()
            .push((raw, bound));
    }

    for ((mods, keysym_raw), mut group) in groups {
        if group.len() > 1 {
            let mut names: Vec<&str> = group.iter().map(|(raw, _)| raw.as_str()).collect();
            names.sort_unstable();
            tracing::warn!(
                binds = ?names,
                "these config-file binds all resolve to the same key combination; \
                 none of them will be applied -- remove all but one"
            );
            continue;
        }
        let (_, bound) = group.pop().expect("group.len() == 1");
        let keysym = Keysym::from(keysym_raw);
        keybindings.insert(mods, keysym, bound);
    }
}

/// Parses one `[binds]` entry: `key` is the TOML key (`"super+h"`), `value`
/// is an action string in exactly the grammar `flexwm msg action ...` uses
/// (`"focus-column left"`, `"close"`, `"spawn" "foot"`, ...) -- see
/// `cli::action`, reused here rather than duplicated.
fn parse_bind(key: &str, value: &str) -> Result<(Modifiers, Keysym, Bound), String> {
    let (mods, keysym) = parse_combo(key)?;
    let mut tokens = value.split_whitespace().map(str::to_owned);
    let action = crate::cli::action(&mut tokens).map_err(|error| error.to_string())?;
    if tokens.next().is_some() {
        return Err(format!("trailing text after the action in `{value}`"));
    }
    Ok((mods, keysym, Bound::Action(action.into())))
}

/// Parses a combo string (`"super+shift+t"`) into this table's `Modifiers`
/// plus a `Keysym`. Reuses `flexwm_ipc::KeyCombo`'s modifier/key-name
/// splitting -- already parsed the same way for `flexwm msg key ...` and
/// well-tested there -- rather than writing a second parser for the same
/// `mod+mod+key` syntax.
///
/// The key name is lowercased before resolving it. xkb gives single Latin
/// letters two *different* valid keysyms depending on case (`h` and `H` are
/// both real, distinct keysyms), but `keybindings.rs`'s whole table -- like
/// niri and sway -- binds by the unshifted (level-0) keysym only, tracking
/// Shift as an ordinary modifier (see that module's doc). Without
/// lowercasing first, `"Super+H"` would resolve to `Keysym::H`: a
/// syntactically valid bind that can never fire, since no real keypress
/// ever reports that keysym under this project's matching scheme -- exactly
/// the silently-wrong-config class this loader exists to avoid. Multi-
/// character names (`Return`, `F1`) are unaffected: lowercasing them just
/// takes `keysym_named`'s existing case-insensitive fallback path, the same
/// one already used for e.g. `"return"`.
fn parse_combo(s: &str) -> Result<(Modifiers, Keysym), String> {
    let combo: flexwm_ipc::KeyCombo =
        s.parse().map_err(|error: flexwm_ipc::ParseKeyComboError| {
            format!("invalid key combination `{s}`: {error}")
        })?;
    let mut mods = Modifiers::default();
    for modifier in &combo.modifiers {
        match modifier {
            flexwm_ipc::Modifier::Super => mods.super_ = true,
            flexwm_ipc::Modifier::Shift => mods.shift = true,
            flexwm_ipc::Modifier::Ctrl => mods.ctrl = true,
            flexwm_ipc::Modifier::Alt => mods.alt = true,
        }
    }
    let keysym = keysym_named(&combo.key.to_ascii_lowercase())
        .ok_or_else(|| format!("unknown key `{}`", combo.key))?;
    Ok((mods, keysym))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flexwm_core::{Action, Horizontal};

    use super::*;

    fn write_temp(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("config.toml");
        let mut file = fs::File::create(&path).expect("create temp config file");
        file.write_all(contents.as_bytes())
            .expect("write temp config");
        (dir, path)
    }

    #[test]
    fn a_full_valid_file_round_trips() {
        let toml = r#"
            [layout]
            gap = 20
            column_widths = [0.25, 0.5, 0.75]
            default_column_width = 2

            [binds]
            "super+n" = "focus-column right"
            "super+shift+q" = "spawn foot -e htop"
        "#;
        let file: FileConfig = toml::from_str(toml).expect("valid toml");
        assert_eq!(
            file.layout,
            Some(LayoutConfig {
                gap: Some(20),
                column_widths: Some(vec![0.25, 0.5, 0.75]),
                default_column_width: Some(2),
            })
        );
        assert_eq!(file.binds.len(), 2);

        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.config.gap, 20);
        assert_eq!(loaded.config.column_widths, vec![0.25, 0.5, 0.75]);
        assert_eq!(loaded.config.default_column_width, 2);
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("n").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::FocusColumn(Horizontal::Right)))
        );
    }

    #[test]
    fn a_partial_layout_table_only_overrides_what_it_names() {
        let toml = "[layout]\ngap = 3\n";
        let file: FileConfig = toml::from_str(toml).unwrap();
        let loaded = LoadedConfig::from_file(file);
        assert_eq!(loaded.config.gap, 3);
        assert_eq!(loaded.config.column_widths, Config::default().column_widths);
        assert_eq!(
            loaded.config.default_column_width,
            Config::default().default_column_width
        );
    }

    #[test]
    fn deny_unknown_fields_rejects_a_typo() {
        let toml = "[layout]\ngaps = 5\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    #[test]
    fn deny_unknown_fields_rejects_a_typo_at_the_top_level() {
        let toml = "[layuot]\ngap = 5\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    #[test]
    fn a_missing_file_at_the_default_path_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.toml");
        let loaded = load_from(&missing, false).expect("a missing default path is not an error");
        assert_eq!(loaded.config, Config::default());
    }

    #[test]
    fn an_explicit_missing_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.toml");
        let error = load_from(&missing, true).expect_err("an explicit --config must be readable");
        assert!(error.to_string().contains("does-not-exist.toml"));
    }

    #[test]
    fn a_user_bind_overrides_the_matching_default() {
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "super+h" = "close"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid config");
        assert_eq!(
            loaded.keybindings.match_key(
                Keysym::h,
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::CloseFocused))
        );
    }

    #[test]
    fn a_malformed_bind_is_skipped_and_the_rest_of_the_file_still_loads() {
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "super+notakey" = "close"
            "super+n" = "focus-column right"
        "#,
        );
        let loaded = load_from(&path, true).expect("the file still parses as toml");
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("n").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::FocusColumn(Horizontal::Right)))
        );
    }

    #[test]
    fn a_bad_action_string_is_skipped_without_touching_other_binds() {
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "super+n" = "not-a-real-action"
            "super+m" = "close"
        "#,
        );
        let loaded = load_from(&path, true).expect("the file still parses as toml");
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("m").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::CloseFocused))
        );
    }

    #[test]
    fn trailing_text_after_the_action_is_rejected() {
        assert_eq!(
            parse_bind("super+n", "focus-column left extra-garbage"),
            Err("trailing text after the action in `focus-column left extra-garbage`".into())
        );
    }

    #[test]
    fn malformed_toml_falls_back_to_full_defaults_not_an_error() {
        let (_dir, path) = write_temp("this is not valid toml [[[");
        let loaded = load_from(&path, true).expect("a parse failure must never fail startup");
        assert_eq!(loaded.config, Config::default());
    }

    #[test]
    fn an_unknown_field_falls_back_to_full_defaults_not_an_error() {
        let (_dir, path) = write_temp("[layout]\ngaps = 5\n");
        let loaded = load_from(&path, true).expect("a bad field must never fail startup");
        assert_eq!(loaded.config, Config::default());
    }

    #[test]
    fn toml_itself_rejects_a_literal_duplicate_key() {
        // TOML forbids defining the same key twice in a table -- this is a
        // syntax error the `toml` crate's parser raises, before `FileConfig`
        // (or its `HashMap`) ever sees it. There is no "duplicate key" case
        // for this module's code to handle at the Rust level.
        let toml = "[binds]\n\"super+h\" = \"close\"\n\"super+h\" = \"quit\"\n";
        assert!(toml::from_str::<FileConfig>(toml).is_err());
    }

    #[test]
    fn two_combo_strings_that_normalize_the_same_are_both_skipped() {
        // "Super+H" and "super+h" are two *different* TOML keys (so this
        // parses fine as TOML), but both resolve to the same (mods, keysym)
        // once parsed -- exactly the collision `apply_binds` refuses to
        // arbitrate. Neither is applied; the default for Super+h survives.
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "Super+H" = "close"
            "super+h" = "quit"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid toml");
        assert_eq!(
            loaded.keybindings.match_key(
                Keysym::h,
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            Some(Bound::Action(Action::FocusColumn(Horizontal::Left))),
            "the collision must be skipped entirely, leaving the default in place"
        );
    }

    #[test]
    fn a_multi_character_key_name_is_also_case_insensitive() {
        // Unlike a single letter, "return" isn't itself a distinct valid
        // keysym -- exact-case lookup fails and falls back to
        // case-insensitive, landing on the same `Keysym::Return` either way
        // -- so lowercasing it first (see `parse_combo`'s doc) is a no-op
        // here. This is what exercises that fallback path, as opposed to
        // the single-letter test above exercising the exact-match path.
        assert_eq!(
            parse_combo("super+Return").unwrap(),
            parse_combo("super+return").unwrap()
        );
    }

    #[test]
    fn modifier_aliases_collide_like_case_does() {
        let (_dir, path) = write_temp(
            r#"
            [binds]
            "super+n" = "close"
            "logo+n" = "quit"
        "#,
        );
        let loaded = load_from(&path, true).expect("valid toml");
        assert_eq!(
            loaded.keybindings.match_key(
                keysym_named("n").unwrap(),
                Modifiers {
                    super_: true,
                    ..Modifiers::default()
                }
            ),
            None,
            "no default exists for Super+n, and the collision must not apply either alias"
        );
    }

    #[test]
    fn default_path_prefers_xdg_config_home_over_home() {
        assert_eq!(
            default_path(Some("/xdg".into()), Some("/home/u".into())),
            Some(PathBuf::from("/xdg/flexwm/config.toml"))
        );
    }

    #[test]
    fn default_path_falls_back_to_home() {
        assert_eq!(
            default_path(None, Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.config/flexwm/config.toml"))
        );
    }

    #[test]
    fn default_path_is_none_with_neither_var_set() {
        assert_eq!(default_path(None, None), None);
    }
}
