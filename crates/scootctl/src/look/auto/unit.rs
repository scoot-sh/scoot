//! The pure half of `scoot look auto`: the schedule grammar, which look
//! is due at a wall-clock minute or when the sun is up, and the two unit
//! files (a trigger, which is a timer for `--schedule` and `--sun` or a
//! path unit for `--follow-portal`, and the oneshot service it starts) with a strict reader for the service line
//! this module wrote. No I/O here, so golden files and hostile inputs pin
//! it without a systemd.

use std::path::{Path, PathBuf};

use scoot_look::grammar::is_name;
use scoot_look::sun::{self, Location};

use crate::cli::Error;

/// The service every trigger starts, and the two triggers: the timer of
/// `--schedule` and the path unit of `--follow-portal`. Exactly one of the
/// two is installed at a time.
pub const SERVICE: &str = "scoot-look-auto.service";
pub const TIMER: &str = "scoot-look-auto.timer";
pub const PATH: &str = "scoot-look-auto.path";

/// A file this module wrote carries this on line 1; absent or marked
/// means `scoot look auto` owns the path, anything else is someone
/// else's file and is left alone.
pub const MARKER: &str = "Managed by scoot look auto";

/// The hidden verb the service runs. It is spawned, never typed.
pub const FIRE_VERB: &str = "_auto-fire";

/// Minutes in a day: the exclusive bound of a [`Clock`].
const DAY: u16 = 24 * 60;

/// A wall-clock minute of the day, `00:00` to `23:59`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock(u16);

impl Clock {
    /// `HH:MM`, exactly: two digits each, hour 00-23, minute 00-59. The
    /// night-light options take the same grammar. `7:5`, `24:00`, `+7:05`
    /// and the empty string are all refused.
    pub fn parse(text: &str) -> Option<Clock> {
        let b = text.as_bytes();
        if b.len() != 5 || b[2] != b':' {
            return None;
        }
        let digit = |c: u8| c.is_ascii_digit().then(|| u16::from(c - b'0'));
        let hour = digit(b[0])? * 10 + digit(b[1])?;
        let minute = digit(b[3])? * 10 + digit(b[4])?;
        (hour < 24 && minute < 60).then_some(Clock(hour * 60 + minute))
    }

    /// The minute of the day, 0 to 1439.
    pub fn minutes(self) -> u16 {
        self.0
    }
}

impl std::fmt::Display for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

/// Light or dark: the two sides of a schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Light,
    Dark,
}

/// The look-name grammar and the two-looks-differ rule, shared by both
/// triggers.
fn check_looks(light: &str, dark: &str) -> Result<(), Error> {
    for name in [light, dark] {
        if !is_name(name) {
            return Err(Error::Invalid {
                what: "look name",
                value: name.to_owned(),
            });
        }
    }
    if light == dark {
        return Err(Error::Invalid {
            what: "--light and --dark (the two looks must differ)",
            value: light.to_owned(),
        });
    }
    Ok(())
}

/// One validated schedule: two looks and the minute each takes over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub light: String,
    pub dark: String,
    pub light_from: Clock,
    pub dark_from: Clock,
}

impl Schedule {
    /// Validates the words. Names follow the look grammar; `--schedule`
    /// is `LIGHT_FROM/DARK_FROM`; equal times and equal looks refuse (a
    /// schedule that never changes anything is a typo, not a request).
    /// Every refusal is a usage error: it needs no catalog and no systemd.
    pub fn parse(light: &str, dark: &str, schedule: &str) -> Result<Schedule, Error> {
        check_looks(light, dark)?;
        let invalid = |what: &'static str, value: &str| Error::Invalid {
            what,
            value: value.to_owned(),
        };
        let (from_light, from_dark) = schedule.split_once('/').ok_or_else(|| {
            invalid(
                "--schedule (want LIGHT_FROM/DARK_FROM, like 07:00/19:00)",
                schedule,
            )
        })?;
        let clock = |text: &str| {
            Clock::parse(text).ok_or_else(|| {
                invalid(
                    "--schedule time (want HH:MM, 00:00 to 23:59, two digits each)",
                    text,
                )
            })
        };
        let light_from = clock(from_light)?;
        let dark_from = clock(from_dark)?;
        if light_from == dark_from {
            return Err(invalid("--schedule (the two times must differ)", schedule));
        }
        Ok(Schedule {
            light: light.to_owned(),
            dark: dark.to_owned(),
            light_from,
            dark_from,
        })
    }

    /// The side due at `now`: light from `light_from` until `dark_from`,
    /// dark from `dark_from` until `light_from`, either window may wrap
    /// midnight. The start minute belongs to the side that starts there.
    pub fn due(&self, now: Clock) -> Side {
        let since_light = (now.0 + DAY - self.light_from.0) % DAY;
        let light_span = (self.dark_from.0 + DAY - self.light_from.0) % DAY;
        if since_light < light_span {
            Side::Light
        } else {
            Side::Dark
        }
    }

    /// The look name for a side.
    pub fn look_for(&self, side: Side) -> &str {
        match side {
            Side::Light => &self.light,
            Side::Dark => &self.dark,
        }
    }
}

/// `--follow-portal`: the two looks, one for each side of the desktop's
/// light/dark setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portal {
    pub light: String,
    pub dark: String,
}

impl Portal {
    /// Validates the names the way [`Schedule::parse`] does.
    pub fn parse(light: &str, dark: &str) -> Result<Portal, Error> {
        check_looks(light, dark)?;
        Ok(Portal {
            light: light.to_owned(),
            dark: dark.to_owned(),
        })
    }

    /// The look name for a side.
    pub fn look_for(&self, side: Side) -> &str {
        match side {
            Side::Light => &self.light,
            Side::Dark => &self.dark,
        }
    }
}

/// `--sun`: the two looks and the place whose sunrise and sunset move
/// between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sun {
    pub light: String,
    pub dark: String,
    pub place: Location,
}

impl Sun {
    /// Validates the names the way [`Schedule::parse`] does, and the
    /// coordinates as [`Location::parse`] reads them. A refusal is a usage
    /// error: it needs no catalog and no systemd.
    pub fn parse(light: &str, dark: &str, lat: &str, lon: &str) -> Result<Sun, Error> {
        check_looks(light, dark)?;
        let invalid = |what: &'static str, value: &str| Error::Invalid {
            what,
            value: value.to_owned(),
        };
        let degrees = |text: &str, limit: f64, what: &'static str| {
            sun::parse_degrees(text)
                .filter(|d| d.abs() <= limit)
                .ok_or_else(|| invalid(what, text))
        };
        let lat = degrees(
            lat,
            90.0,
            "--latitude (want decimal degrees from -90 to 90, like 51.5074)",
        )?;
        let lon = degrees(
            lon,
            180.0,
            "--longitude (want decimal degrees from -180 to 180, like -0.1278)",
        )?;
        // Both are finite and in range by now, so this is always `Some`.
        let place = Location::new(lat, lon)
            .ok_or_else(|| invalid("--latitude and --longitude", "out of range"))?;
        Ok(Sun {
            light: light.to_owned(),
            dark: dark.to_owned(),
            place,
        })
    }

    /// The side due at `unix` seconds UTC: light while the Sun is up.
    pub fn due(&self, unix: i64) -> Side {
        if sun::is_day(self.place, unix) {
            Side::Light
        } else {
            Side::Dark
        }
    }

    /// The look name for a side.
    pub fn look_for(&self, side: Side) -> &str {
        match side {
            Side::Light => &self.light,
            Side::Dark => &self.dark,
        }
    }
}

/// What makes the service run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// A systemd timer on two wall-clock boundaries.
    Schedule(Schedule),
    /// A systemd timer that wakes daily and at login to arm a one-shot
    /// timer for the next sunrise or sunset (see [`super::sun`]).
    Sun(Sun),
    /// A path unit on the dconf database the desktop's setting lives in.
    Portal(Portal),
}

impl Trigger {
    pub fn light(&self) -> &str {
        match self {
            Trigger::Schedule(s) => &s.light,
            Trigger::Sun(s) => &s.light,
            Trigger::Portal(p) => &p.light,
        }
    }

    pub fn dark(&self) -> &str {
        match self {
            Trigger::Schedule(s) => &s.dark,
            Trigger::Sun(s) => &s.dark,
            Trigger::Portal(p) => &p.dark,
        }
    }

    /// The trigger unit this installs, and the other one, which a switch
    /// from the other mode must remove.
    pub fn unit(&self) -> (&'static str, &'static str) {
        match self {
            Trigger::Schedule(_) | Trigger::Sun(_) => (TIMER, PATH),
            Trigger::Portal(_) => (PATH, TIMER),
        }
    }
}

/// The inputs of one generated pair: the trigger, the binary the service
/// runs, and the state dir when the user passed `--state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub trigger: Trigger,
    pub exe: PathBuf,
    pub state: Option<PathBuf>,
    /// The file a path trigger watches. `None` for a schedule, and for a
    /// plan read back from a service file, which does not carry it (the
    /// path unit does: see [`read_watch`]).
    pub watch: Option<PathBuf>,
}

/// Quotes one path for an `ExecStart=` line: double quotes, with `\` and
/// `"` escaped, and the two characters systemd expands inside a command
/// line doubled (`%` specifiers, `$` variables). A path with a control
/// character or one that is not absolute and UTF-8 refuses: the unit
/// would not run what was meant, or would run it from the wrong place.
/// This is the quoting for an argument (the `--state` directory); the
/// executable word has stricter rules, see [`quote_exe`].
pub fn quote_path(path: &Path) -> Result<String, String> {
    let text = path.to_str().ok_or_else(|| {
        format!(
            "`{}` is not UTF-8: a unit file cannot name it",
            path.display()
        )
    })?;
    if !path.is_absolute() {
        return Err(format!(
            "`{text}` is not an absolute path: the unit would run it from systemd's directory"
        ));
    }
    if text.chars().any(char::is_control) {
        return Err(format!(
            "`{}` has a control character: a unit file cannot name it",
            text.escape_debug()
        ));
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

/// Quotes the executable, the first word of `ExecStart=`. systemd reads
/// that word differently from the arguments after it (measured on systemd
/// 261.2 with `systemd-analyze --user verify` and a real start): `"`, `\`
/// and `'` in it are refused ("Executable path contains special
/// characters"), and a `$$` is not collapsed, so the unit fails to start
/// with status 203/EXEC. Space, `%` (as `%%`) and `;` work. The four
/// refused characters refuse here, before anything is written.
pub fn quote_exe(path: &Path) -> Result<String, String> {
    if let Some(text) = path.to_str()
        && let Some(bad) = text.chars().find(|c| matches!(c, '"' | '\'' | '\\' | '$'))
    {
        return Err(format!(
            "systemd cannot run an executable at a path containing `{bad}` ({text}): \
             install or link scoot at a plain path (such as ~/.local/bin/scoot) and run `scoot look auto` from there"
        ));
    }
    quote_path(path)
}

/// The inverse of [`quote_path`] over exactly what it writes; `None` for
/// anything else. Returns the path and the rest of the line after the
/// closing quote.
fn unquote_path(text: &str) -> Option<(PathBuf, &str)> {
    let mut chars = text.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => {
                let rest = &text[at + 2..];
                return Some((PathBuf::from(out), rest));
            }
            '\\' => match chars.next()? {
                (_, e @ ('\\' | '"')) => out.push(e),
                _ => return None,
            },
            '%' => match chars.next()? {
                (_, '%') => out.push('%'),
                _ => return None,
            },
            '$' => match chars.next()? {
                (_, '$') => out.push('$'),
                _ => return None,
            },
            c => out.push(c),
        }
    }
    None
}

impl Plan {
    /// The `ExecStart=` value: the binary, then fixed words. Names and
    /// times passed the grammar, so they are written bare; only the
    /// paths need quoting (the executable by [`quote_exe`], the state
    /// directory by [`quote_path`]). Nothing a look supplies reaches this
    /// line.
    pub fn exec_start(&self) -> Result<String, String> {
        let mut line = quote_exe(&self.exe)?;
        line.push_str(" look");
        if let Some(state) = &self.state {
            line.push_str(" --state ");
            line.push_str(&quote_path(state)?);
        }
        line.push_str(&format!(
            " {FIRE_VERB} --light {} --dark {}",
            self.trigger.light(),
            self.trigger.dark()
        ));
        match &self.trigger {
            Trigger::Schedule(s) => {
                line.push_str(&format!(" --schedule {}/{}", s.light_from, s.dark_from));
            }
            Trigger::Sun(s) => {
                line.push_str(&format!(
                    " --sun --latitude {} --longitude {}",
                    s.place.latitude_text(),
                    s.place.longitude_text()
                ));
            }
            Trigger::Portal(_) => line.push_str(" --follow-portal"),
        }
        Ok(line)
    }

    /// The oneshot service. It runs the fire verb, which decides what to
    /// apply when it runs (a schedule from the wall clock, so two catch-up
    /// runs after a long suspend agree instead of racing; the portal from
    /// the setting as it stands).
    ///
    /// The portal service lifts systemd's start limit (five starts in ten
    /// seconds by default). A path unit starts the service once per
    /// database write, and a settings slider is many writes; a run that
    /// hit the limit would put the path unit into `failed` for good. The
    /// run itself is idempotent, so the limit has nothing to protect, and
    /// the path unit's own trigger limit (200 in 2 s) still bounds a loop.
    pub fn service(&self) -> Result<String, String> {
        let (what, limit) = match self.trigger {
            Trigger::Schedule(_) => ("the schedule says is due", ""),
            Trigger::Sun(_) => ("the sun says is due", ""),
            Trigger::Portal(_) => (
                "the desktop's light/dark setting asks for",
                "StartLimitIntervalSec=0\n",
            ),
        };
        Ok(format!(
            "# {MARKER}. Do not edit: `scoot look auto --off` removes it, `scoot look auto` rewrites it.\n\
             [Unit]\n\
             Description=scoot look: switch to the look {what}\n\
             {limit}\
             \n\
             [Service]\n\
             Type=oneshot\n\
             ExecStart={}\n",
            self.exec_start()?
        ))
    }

    /// The trigger unit's file name and text.
    pub fn trigger_file(&self) -> Result<(&'static str, String), String> {
        match &self.trigger {
            Trigger::Schedule(s) => Ok((TIMER, timer_text(s))),
            Trigger::Sun(_) => Ok((TIMER, sun_timer_text())),
            Trigger::Portal(_) => {
                let watch = self.watch.as_deref().ok_or_else(|| {
                    "internal error: a path trigger with no file to watch".to_owned()
                })?;
                Ok((PATH, path_text(watch)?))
            }
        }
    }
}

/// The timer: one `OnCalendar=` per boundary, in the system's local time
/// zone, and `Persistent=true` so a boundary that passed while the user
/// manager was down (shutdown, logged out) fires on the next start.
pub fn timer_text(s: &Schedule) -> String {
    format!(
        "# {MARKER}. Do not edit: `scoot look auto --off` removes it, `scoot look auto` rewrites it.\n\
         [Unit]\n\
         Description=scoot look: light and dark schedule ({}/{})\n\
         \n\
         [Timer]\n\
         OnCalendar=*-*-* {}:00\n\
         OnCalendar=*-*-* {}:00\n\
         Persistent=true\n\
         AccuracySec=1s\n\
         Unit={SERVICE}\n\
         \n\
         [Install]\n\
         WantedBy=timers.target\n",
        s.light_from, s.dark_from, s.light_from, s.dark_from
    )
}

/// The `--sun` trigger: a timer that cannot name a moving time, so it
/// only wakes the service to arm the real one. The service run arms a
/// one-shot timer for the next sunrise or sunset (see [`super::sun`]) and
/// applies the look due now. This timer wakes it at 00:05 local (a day
/// that began without the previous run, a polar stretch with no boundary
/// to arm), `Persistent=true` so a day missed while the machine was off is
/// made up at the next start, and 30 s after the user manager starts (the
/// one-shot timer lives in `$XDG_RUNTIME_DIR` and is gone after a reboot
/// or a logout).
pub fn sun_timer_text() -> String {
    format!(
        "# {MARKER}. Do not edit: `scoot look auto --off` removes it, `scoot look auto` rewrites it.\n\
         [Unit]\n\
         Description=scoot look: arm the next sunrise or sunset switch\n\
         \n\
         [Timer]\n\
         OnStartupSec=30s\n\
         OnCalendar=*-*-* 00:05:00\n\
         Persistent=true\n\
         AccuracySec=1s\n\
         Unit={SERVICE}\n\
         \n\
         [Install]\n\
         WantedBy=timers.target\n"
    )
}

/// The `PathChanged=` value for `path`: unlike `ExecStart=` it is one
/// verbatim path (no quoting, no `$`), but `%` is a specifier, so it is
/// doubled. Refused when a unit cannot say it: not UTF-8, not absolute, a
/// control character.
fn watch_value(path: &Path) -> Result<String, String> {
    let text = path.to_str().ok_or_else(|| {
        format!(
            "`{}` is not UTF-8: a unit file cannot name it",
            path.display()
        )
    })?;
    if !path.is_absolute() {
        return Err(format!("`{text}` is not an absolute path"));
    }
    if text.chars().any(char::is_control) {
        return Err(format!(
            "`{}` has a control character: a unit file cannot name it",
            text.escape_debug()
        ));
    }
    Ok(text.replace('%', "%%"))
}

/// The path unit: starts the service when the dconf database changes.
/// `PathChanged=` and never `PathExists=` (which would fire for as long as
/// the file exists) or `MakeDirectory=` (which would `mkdir` the database
/// path itself).
pub fn path_text(watch: &Path) -> Result<String, String> {
    Ok(format!(
        "# {MARKER}. Do not edit: `scoot look auto --off` removes it, `scoot look auto` rewrites it.\n\
         [Unit]\n\
         Description=scoot look: follow the desktop's light/dark setting\n\
         \n\
         [Path]\n\
         PathChanged={}\n\
         Unit={SERVICE}\n\
         \n\
         [Install]\n\
         WantedBy=paths.target\n",
        watch_value(watch)?
    ))
}

/// Reads the watched file back out of a path unit this module wrote.
/// `None` for anything that is not exactly one `PathChanged=` line.
pub fn read_watch(text: &str) -> Option<PathBuf> {
    if !is_ours(text) {
        return None;
    }
    let mut lines = text.lines().filter_map(|l| l.strip_prefix("PathChanged="));
    let value = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    Some(PathBuf::from(value.replace("%%", "%")))
}

/// Whether `text` is a file this module wrote: the marker is on line 1.
pub fn is_ours(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with('#') && line.contains(MARKER))
}

/// Reads a service file this module wrote back into its plan. Strict:
/// the `ExecStart=` line must be exactly what [`Plan::exec_start`] writes,
/// and every word must pass the grammar again, so a hand-edited line is
/// reported as unreadable, never half-trusted.
pub fn read_service(text: &str) -> Result<Plan, String> {
    let bad = || "the service file was edited by hand (its ExecStart= is not ours)".to_owned();
    if !is_ours(text) {
        return Err("not a scoot look auto file".to_owned());
    }
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("ExecStart="))
        .ok_or_else(bad)?;
    let (exe, rest) = unquote_path(line).ok_or_else(bad)?;
    let mut rest = rest.strip_prefix(" look").ok_or_else(bad)?;
    let mut state = None;
    if let Some(after) = rest.strip_prefix(" --state ") {
        let (dir, tail) = unquote_path(after).ok_or_else(bad)?;
        state = Some(dir);
        rest = tail;
    }
    let words: Vec<&str> = rest.split(' ').collect();
    let trigger = match words.as_slice() {
        [
            "",
            FIRE_VERB,
            "--light",
            light,
            "--dark",
            dark,
            "--schedule",
            times,
        ] => Trigger::Schedule(Schedule::parse(light, dark, times).map_err(|_| bad())?),
        [
            "",
            FIRE_VERB,
            "--light",
            light,
            "--dark",
            dark,
            "--follow-portal",
        ] => Trigger::Portal(Portal::parse(light, dark).map_err(|_| bad())?),
        [
            "",
            FIRE_VERB,
            "--light",
            light,
            "--dark",
            dark,
            "--sun",
            "--latitude",
            lat,
            "--longitude",
            lon,
        ] => {
            let sun = Sun::parse(light, dark, lat, lon).map_err(|_| bad())?;
            // Only the spelling this module writes: a padded or
            // over-precise number is a hand edit.
            if sun.place.latitude_text() != *lat || sun.place.longitude_text() != *lon {
                return Err(bad());
            }
            Trigger::Sun(sun)
        }
        _ => return Err(bad()),
    };
    Ok(Plan {
        trigger,
        exe,
        state,
        watch: None,
    })
}
