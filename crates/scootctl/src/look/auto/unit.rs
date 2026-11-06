//! The pure half of `scoot look auto`: the schedule grammar, which look
//! is due at a wall-clock minute, and the two unit files (a timer and the
//! oneshot service it starts) with a strict reader for the service line
//! this module wrote. No I/O here, so golden files and hostile inputs pin
//! it without a systemd.

use std::path::{Path, PathBuf};

use scoot_look::grammar::is_name;

use crate::cli::Error;

/// The service the timer starts, and the timer itself.
pub const SERVICE: &str = "scoot-look-auto.service";
pub const TIMER: &str = "scoot-look-auto.timer";

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

/// The inputs of one generated pair: the schedule, the binary the service
/// runs, and the state dir when the user passed `--state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub schedule: Schedule,
    pub exe: PathBuf,
    pub state: Option<PathBuf>,
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
        let s = &self.schedule;
        line.push_str(&format!(
            " {FIRE_VERB} --light {} --dark {} --schedule {}/{}",
            s.light, s.dark, s.light_from, s.dark_from
        ));
        Ok(line)
    }

    /// The oneshot service. It runs the fire verb, which decides the side
    /// from the wall clock when it runs: two catch-up runs after a long
    /// suspend therefore agree instead of racing.
    pub fn service(&self) -> Result<String, String> {
        Ok(format!(
            "# {MARKER}. Do not edit: `scoot look auto --off` removes it, `scoot look auto` rewrites it.\n\
             [Unit]\n\
             Description=scoot look: switch to the look the schedule says is due\n\
             \n\
             [Service]\n\
             Type=oneshot\n\
             ExecStart={}\n",
            self.exec_start()?
        ))
    }

    /// The timer: one `OnCalendar=` per boundary, in the system's local
    /// time zone, and `Persistent=true` so a boundary that passed while
    /// the user manager was down (shutdown, logged out) fires on the next
    /// start.
    pub fn timer(&self) -> String {
        let s = &self.schedule;
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
    let [
        "",
        FIRE_VERB,
        "--light",
        light,
        "--dark",
        dark,
        "--schedule",
        times,
    ] = words.as_slice()
    else {
        return Err(bad());
    };
    let schedule = Schedule::parse(light, dark, times).map_err(|_| bad())?;
    Ok(Plan {
        schedule,
        exe,
        state,
    })
}
