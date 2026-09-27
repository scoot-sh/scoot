//! Reading `[wallpaper]` out of the config file, resolving its paths, and
//! encoding it as the JSON `scootbg apply-config` takes.
//!
//! # Read leniently, so a mistake here costs only the wallpaper
//!
//! Every other table in the file is read by a derived `Deserialize` with
//! `deny_unknown_fields`, so an unknown key or a wrong type anywhere makes
//! startup discard the *whole* file (see `config.rs`'s module doc). Adding
//! `[wallpaper]` the same way would let a typo in a wallpaper path's key cost
//! a user their binds, and nothing in this section is worth that. So
//! [`WallpaperConfig`] has a hand-written `Deserialize` that records every
//! unknown key and every value of the wrong type as a problem and carries
//! on, and the caller turns any problem into [`WallpaperSetting::Invalid`]
//! (startup: an error in the log and no wallpaper run; reload: the field
//! refused by name, the rest applied).
//!
//! # Nesting: the one way it fails, deliberately
//!
//! What it skips (an unknown key's value, a value of the wrong type) is
//! drained by [`Drain`], which builds nothing and descends at most
//! [`MAX_DRAIN_DEPTH`] levels. Past that it returns a deserialize error, so
//! the whole file fails the way malformed TOML does (startup: logged, full
//! defaults; reload: an error, the running config kept). It must not
//! recurse further: `toml`'s own limits allow a tree 6,561 levels deep
//! (80-level nesting times 80-segment keys, see
//! `docs/backlog/resolved/config-recursion-depth-resolved.md`), and
//! draining that with serde's `IgnoredAny`, as this first did, overflowed a
//! debug build's 8 MiB main thread at startup and on a reload of a running
//! session (review of PR #297, B1). Nothing a wallpaper value needs comes
//! near the bound: a legitimate section is three levels deep. A passthrough
//! `toml::Table` here would be worse still (see `config.rs`'s
//! `parse_or_defaults`).
//!
//! # What scoot checks, and what it leaves to scootbg
//!
//! scoot checks what it owns: the keys (`image`, `color`, `mode`, `fill`,
//! `filter`, `output` and `command`; each output table the first five), that
//! each value is a string (the output list a table of tables), and the
//! paths it resolves. It does not check the values' meaning (a color's
//! syntax, a mode's name): scootbg validates the section strictly and
//! refuses it with exit status 2, which the reaper logs. One parser for
//! those rules, not two that can disagree.
//!
//! # Only the keys the user wrote
//!
//! scootbg fingerprints the section as written: an explicit default
//! (`mode = "fill"`) is a different section from none. So every key is an
//! `Option`, and the JSON carries exactly the ones present, `output`
//! included when it was written as an empty table. `command` is scoot's own
//! key (where to find the binary) and never goes into the JSON.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde::de::{
    self, Deserialize, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor,
};

/// The program run when the section names no `command`: `scootbg`, looked up
/// on `PATH`.
pub const DEFAULT_COMMAND: &str = "scootbg";

/// How many levels of arrays and tables a skipped value may nest before the
/// file is refused as a parse error (see the module doc). A legitimate
/// `[wallpaper]` is three levels deep; a value of the wrong type rarely one.
pub const MAX_DRAIN_DEPTH: usize = 16;

/// The key `toml` puts a date or time under when it hands one to
/// `deserialize_any`: a one-entry map. An implementation detail of the
/// crate, never shown to the user.
const TOML_DATETIME: &str = "$__toml_private_datetime";

/// The most JSON scootbg takes for a section (`section::MAX_SECTION` in
/// scootbg: its request-line bound less 1 KiB). scootbg checks this itself;
/// scoot checks it first so an oversized section is a message naming the
/// section rather than a refusal from the other process, and so the
/// argument can never approach the kernel's per-argument limit
/// (`MAX_ARG_STRLEN`, 128 KiB), where the spawn itself would fail.
pub const MAX_JSON: usize = 64_512;

/// The five keys a wallpaper table has, at the top level and per output.
/// Each is exactly as written (images before resolution), `None` when absent.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Table {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

/// `[wallpaper]` as the file has it. Never fails to deserialize: see the
/// module doc. `problems` lists what was wrong, in file order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WallpaperConfig {
    pub command: Option<String>,
    pub top: Table,
    /// `[wallpaper.output."NAME"]` tables, by name. `Some` whenever `output`
    /// was written, empty or not: an explicit empty table is part of the
    /// section as written. A `BTreeMap` so the JSON is the same bytes on
    /// every run (scootbg does not need that, it re-sorts; the reload diff
    /// and the log do).
    pub outputs: Option<BTreeMap<String, Table>>,
    pub problems: Vec<String>,
}

/// What a config file says about the wallpaper, resolved and ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WallpaperSetting {
    /// No `[wallpaper]` table: scoot runs nothing at startup (and sends
    /// `{}` on a reload that removed one).
    Absent,
    /// A usable section.
    Section(Section),
    /// A section with a problem, named in the string: startup logs it and
    /// runs nothing; a reload refuses the field and keeps the running one.
    Invalid(String),
}

/// A resolved section: the program to run and the JSON to hand it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// `command`, resolved: a bare name is looked up on `PATH` by the spawn;
    /// anything with a `/` is an absolute path by now.
    pub command: OsString,
    /// The section's wallpaper values as one JSON object, images resolved
    /// to absolute paths, `command` left out.
    pub json: String,
}

impl WallpaperSetting {
    /// Resolves what the file had (`None`: no table) against `config_path`,
    /// the file it came from, and `home`, the `HOME` to expand `~` with.
    pub fn resolve(
        config: Option<WallpaperConfig>,
        config_path: &Path,
        home: Option<&OsStr>,
    ) -> Self {
        let Some(config) = config else {
            return Self::Absent;
        };
        match config.into_section(config_path, home) {
            Ok(section) => Self::Section(section),
            Err(problem) => Self::Invalid(problem),
        }
    }
}

impl WallpaperConfig {
    fn into_section(self, config_path: &Path, home: Option<&OsStr>) -> Result<Section, String> {
        if !self.problems.is_empty() {
            return Err(self.problems.join("; "));
        }
        let paths = Paths::new(config_path, home);
        let command = match self.command {
            None => OsString::from(DEFAULT_COMMAND),
            Some(command) => paths.command(&command)?,
        };
        let top = paths.table(self.top, "wallpaper")?;
        let output = match self.outputs {
            None => None,
            Some(outputs) => {
                let mut resolved = BTreeMap::new();
                for (name, table) in outputs {
                    let at = format!("wallpaper.output.{}", quoted(&name));
                    let table = paths.table(table, &at)?;
                    resolved.insert(name, table);
                }
                Some(resolved)
            }
        };
        let json = serde_json::to_string(&Json {
            image: top.image.as_deref(),
            color: top.color.as_deref(),
            mode: top.mode.as_deref(),
            fill: top.fill.as_deref(),
            filter: top.filter.as_deref(),
            output: output.as_ref(),
        })
        .map_err(|error| format!("the [wallpaper] section cannot be encoded as JSON: {error}"))?;
        if json.len() > MAX_JSON {
            return Err(format!(
                "the [wallpaper] section is {} bytes as JSON; scootbg takes at most {MAX_JSON}",
                json.len()
            ));
        }
        Ok(Section { command, json })
    }
}

/// The JSON object scootbg reads: the top-level table's keys beside
/// `output`. Borrowed, so encoding copies each string once, into the JSON.
#[derive(Serialize)]
struct Json<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fill: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<&'a BTreeMap<String, Table>>,
}

/// A TOML key as the user would write it in a dotted path: bare when TOML
/// allows, else quoted (JSON's escaping is TOML's for every character a
/// connector name can hold).
fn quoted(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if bare {
        name.to_owned()
    } else {
        serde_json::to_string(name).unwrap_or_else(|_| format!("\"{name}\""))
    }
}

/// Resolving the section's paths: `~` and `~/...` against `HOME`, a relative
/// path against the config file's directory.
struct Paths<'a> {
    /// The directory the config file is in, made absolute against the
    /// working directory (never through symlinks: a home-manager config is
    /// a link into the Nix store, and a relative image means the directory
    /// the user sees, not the store's). `None` when that cannot be done (an
    /// unreadable working directory), which only matters to a relative path.
    dir: Option<PathBuf>,
    home: Option<&'a OsStr>,
}

impl<'a> Paths<'a> {
    fn new(config_path: &Path, home: Option<&'a OsStr>) -> Self {
        let dir = std::path::absolute(config_path)
            .ok()
            .and_then(|path| path.parent().map(Path::to_owned));
        Self {
            dir,
            home: home.filter(|home| !home.is_empty()),
        }
    }

    fn table(&self, mut table: Table, at: &str) -> Result<Table, String> {
        if let Some(image) = table.image.take() {
            table.image = Some(self.image(&image, at)?);
        }
        Ok(table)
    }

    fn image(&self, image: &str, at: &str) -> Result<String, String> {
        if image.is_empty() {
            return Err(format!("`{at}.image` is empty"));
        }
        let path = self.resolve(image, &format!("{at}.image"))?;
        path.into_os_string()
            .into_string()
            .map_err(|path| format!("`{at}.image` resolves to {path:?}, which is not UTF-8"))
    }

    /// `command`: a bare name stays a name (the spawn searches `PATH`); a
    /// path is resolved like an image.
    fn command(&self, command: &str) -> Result<OsString, String> {
        if command.is_empty() {
            return Err("`wallpaper.command` is empty".to_owned());
        }
        if command.contains('\0') {
            return Err("`wallpaper.command` contains a NUL byte".to_owned());
        }
        if command != "~" && !command.contains('/') {
            return Ok(OsString::from(command));
        }
        Ok(self.resolve(command, "wallpaper.command")?.into_os_string())
    }

    fn resolve(&self, path: &str, key: &str) -> Result<PathBuf, String> {
        let joined = if path == "~" || path.starts_with("~/") {
            let Some(home) = self.home else {
                return Err(format!(
                    "`{key}` starts with `~` but HOME is not set, so it cannot be expanded"
                ));
            };
            let rest = path
                .strip_prefix('~')
                .unwrap_or(path)
                .trim_start_matches('/');
            Path::new(home).join(rest)
        } else if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            let Some(dir) = &self.dir else {
                return Err(format!(
                    "`{key}` is a relative path, and the config file's directory cannot be \
                     made absolute to resolve it against"
                ));
            };
            dir.join(path)
        };
        // Drops `.` components (`./hills.jpg` reads `/dir/hills.jpg`, not
        // `/dir/./hills.jpg`) and repeated slashes. `..` is kept: resolving
        // it lexically would be wrong across a symlinked directory.
        Ok(joined
            .components()
            .filter(|component| !matches!(component, Component::CurDir))
            .collect())
    }
}

// -- The lenient `Deserialize` ------------------------------------------------

impl<'de> Deserialize<'de> for WallpaperConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(SectionVisitor)
    }
}

struct SectionVisitor;

impl<'de> Visitor<'de> for SectionVisitor {
    type Value = WallpaperConfig;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the [wallpaper] table")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut config = WallpaperConfig::default();
        while let Some(key) = map.next_key::<String>()? {
            if key == TOML_DATETIME {
                map.next_value_seed(Drain::TOP)?;
                return Ok(not_a_table("a date/time"));
            }
            match key.as_str() {
                "command" => {
                    let value = map.next_value::<Text>()?;
                    config.command = value.into_string("wallpaper.command", &mut config.problems);
                }
                "output" => {
                    let outputs = map.next_value::<Outputs>()?;
                    config.problems.extend(outputs.problems);
                    config.outputs = outputs.tables;
                }
                _ => {
                    if !read_table_key(
                        &mut map,
                        &key,
                        "wallpaper",
                        &mut config.top,
                        &mut config.problems,
                    )? {
                        map.next_value_seed(Drain::TOP)?;
                        config.problems.push(format!(
                            "unknown key `wallpaper.{}` (expected image, color, mode, fill, \
                             filter, output or command)",
                            quoted(&key)
                        ));
                    }
                }
            }
        }
        Ok(config)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        Drain::TOP.visit_seq(seq)?;
        Ok(not_a_table("an array"))
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(not_a_table("a boolean"))
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(not_a_table("a number"))
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(not_a_table("a number"))
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(not_a_table("a number"))
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(not_a_table("a string"))
    }
}

fn not_a_table(what: &str) -> WallpaperConfig {
    WallpaperConfig {
        problems: vec![format!(
            "`wallpaper` must be a table ([wallpaper]), not {what}"
        )],
        ..WallpaperConfig::default()
    }
}

/// Reads `key` into `table` if it is one of the five table keys, returning
/// whether it was. `at` is the table's dotted path, for messages.
fn read_table_key<'de, A: MapAccess<'de>>(
    map: &mut A,
    key: &str,
    at: &str,
    table: &mut Table,
    problems: &mut Vec<String>,
) -> Result<bool, A::Error> {
    let slot = match key {
        "image" => &mut table.image,
        "color" => &mut table.color,
        "mode" => &mut table.mode,
        "fill" => &mut table.fill,
        "filter" => &mut table.filter,
        _ => return Ok(false),
    };
    let value = map.next_value::<Text>()?;
    *slot = value.into_string(&format!("{at}.{key}"), problems);
    Ok(true)
}

/// A value that should be a string: the string, or what it was instead.
enum Text {
    String(String),
    Other(&'static str),
}

impl Text {
    fn into_string(self, key: &str, problems: &mut Vec<String>) -> Option<String> {
        match self {
            Self::String(string) => Some(string),
            Self::Other(what) => {
                problems.push(format!("`{key}` must be a string, not {what}"));
                None
            }
        }
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(TextVisitor)
    }
}

struct TextVisitor;

impl<'de> Visitor<'de> for TextVisitor {
    type Value = Text;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string")
    }

    fn visit_str<E>(self, value: &str) -> Result<Text, E> {
        Ok(Text::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Text, E> {
        Ok(Text::String(value))
    }

    fn visit_bool<E>(self, _: bool) -> Result<Text, E> {
        Ok(Text::Other("a boolean"))
    }

    fn visit_i64<E>(self, _: i64) -> Result<Text, E> {
        Ok(Text::Other("a number"))
    }

    fn visit_u64<E>(self, _: u64) -> Result<Text, E> {
        Ok(Text::Other("a number"))
    }

    fn visit_f64<E>(self, _: f64) -> Result<Text, E> {
        Ok(Text::Other("a number"))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Text, A::Error> {
        Drain::TOP.visit_seq(seq)?;
        Ok(Text::Other("an array"))
    }

    // A TOML date or time arrives as a one-entry map too.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Text, A::Error> {
        let Some(key) = map.next_key::<String>()? else {
            return Ok(Text::Other("a table"));
        };
        let datetime = key == TOML_DATETIME;
        // This map is the skipped value, so its entries are one level in.
        map.next_value_seed(Drain::TOP.deeper::<A::Error>()?)?;
        Drain::TOP.visit_map(map)?;
        Ok(Text::Other(if datetime {
            "a date/time"
        } else {
            "a table"
        }))
    }
}

/// `output`: a table of per-output tables, with its own problems.
struct Outputs {
    tables: Option<BTreeMap<String, Table>>,
    problems: Vec<String>,
}

impl<'de> Deserialize<'de> for Outputs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(OutputsVisitor)
    }
}

struct OutputsVisitor;

impl OutputsVisitor {
    fn wrong(what: &str) -> Outputs {
        Outputs {
            tables: None,
            problems: vec![format!(
                "`wallpaper.output` must be a table of tables ([wallpaper.output.\"NAME\"]), \
                 not {what}"
            )],
        }
    }
}

impl<'de> Visitor<'de> for OutputsVisitor {
    type Value = Outputs;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a table of per-output tables")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Outputs, A::Error> {
        let mut tables = BTreeMap::new();
        let mut problems = Vec::new();
        while let Some(name) = map.next_key::<String>()? {
            if name == TOML_DATETIME {
                map.next_value_seed(Drain::TOP)?;
                return Ok(Self::wrong("a date/time"));
            }
            let at = format!("wallpaper.output.{}", quoted(&name));
            let table = map.next_value_seed(OutputSeed {
                at: &at,
                problems: &mut problems,
            })?;
            if let Some(table) = table {
                // TOML itself refuses a key given twice, so this never
                // replaces an entry.
                tables.insert(name, table);
            }
        }
        Ok(Outputs {
            tables: Some(tables),
            problems,
        })
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Outputs, A::Error> {
        Drain::TOP.visit_seq(seq)?;
        Ok(Self::wrong("an array"))
    }

    fn visit_bool<E>(self, _: bool) -> Result<Outputs, E> {
        Ok(Self::wrong("a boolean"))
    }

    fn visit_i64<E>(self, _: i64) -> Result<Outputs, E> {
        Ok(Self::wrong("a number"))
    }

    fn visit_u64<E>(self, _: u64) -> Result<Outputs, E> {
        Ok(Self::wrong("a number"))
    }

    fn visit_f64<E>(self, _: f64) -> Result<Outputs, E> {
        Ok(Self::wrong("a number"))
    }

    fn visit_str<E>(self, _: &str) -> Result<Outputs, E> {
        Ok(Self::wrong("a string"))
    }
}

/// One `[wallpaper.output."NAME"]` table: `None` (with a problem) when it is
/// not a table at all.
struct OutputSeed<'p> {
    at: &'p str,
    problems: &'p mut Vec<String>,
}

impl<'de> de::DeserializeSeed<'de> for OutputSeed<'_> {
    type Value = Option<Table>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl OutputSeed<'_> {
    fn wrong(self, what: &str) -> Option<Table> {
        self.problems
            .push(format!("`{}` must be a table, not {what}", self.at));
        None
    }
}

impl<'de> Visitor<'de> for OutputSeed<'_> {
    type Value = Option<Table>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a per-output wallpaper table")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut table = Table::default();
        while let Some(key) = map.next_key::<String>()? {
            if key == TOML_DATETIME {
                map.next_value_seed(Drain::TOP)?;
                return Ok(self.wrong("a date/time"));
            }
            if !read_table_key(&mut map, &key, self.at, &mut table, self.problems)? {
                map.next_value_seed(Drain::TOP)?;
                self.problems.push(format!(
                    "unknown key `{}.{}` (an output's table takes image, color, mode, fill \
                     and filter)",
                    self.at,
                    quoted(&key)
                ));
            }
        }
        Ok(Some(table))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        Drain::TOP.visit_seq(seq)?;
        Ok(self.wrong("an array"))
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(self.wrong("a boolean"))
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(self.wrong("a number"))
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(self.wrong("a number"))
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(self.wrong("a number"))
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(self.wrong("a string"))
    }
}

/// Skips a value this section has no use for, building nothing, and
/// refuses one nested more than [`MAX_DRAIN_DEPTH`] levels deep with a
/// deserialize error instead of recursing (see the module doc). The `usize`
/// is how many arrays and tables enclose the value being drained, counted
/// from the value the section skips.
#[derive(Clone, Copy)]
struct Drain(usize);

impl Drain {
    /// A skipped value itself.
    const TOP: Self = Self(0);

    /// The drain for a value one level inside this one, or the error that
    /// ends the parse.
    fn deeper<E: de::Error>(self) -> Result<Self, E> {
        if self.0 >= MAX_DRAIN_DEPTH {
            return Err(E::custom(format!(
                "a value in [wallpaper] nests more than {MAX_DRAIN_DEPTH} levels of arrays \
                 and tables deep"
            )));
        }
        Ok(Self(self.0 + 1))
    }
}

impl<'de> DeserializeSeed<'de> for Drain {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Drain {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E>(self, _: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E>(self, _: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_i128<E>(self, _: i128) -> Result<(), E> {
        Ok(())
    }

    fn visit_u128<E>(self, _: u128) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E>(self, _: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E>(self, _: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_bytes<E>(self, _: &[u8]) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self.deeper::<D::Error>()?)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let inner = self.deeper::<A::Error>()?;
        while seq.next_element_seed(inner)?.is_some() {}
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let inner = self.deeper::<A::Error>()?;
        // Keys are strings in TOML: nothing to descend into.
        while map.next_key::<IgnoredAny>()?.is_some() {
            map.next_value_seed(inner)?;
        }
        Ok(())
    }
}
