//! The clock: the local time in a format from `--clock-format`, redrawn on
//! the boundary it shows and at no other time.
//!
//! One `timerfd` ([`timer`]), armed at the next boundary (the next local
//! minute, or second when the format shows seconds), absolute and
//! cancelled by a clock step, so suspend, NTP steps and summer time need no
//! polling: idle, the clock's timer wakes the bar once a minute (and the
//! frame each tick draws brings the compositor's `wl_buffer.release`, a
//! second wakeup). The zone is read
//! as glibc reads it ([`zone`]) with M0's TZif reader ([`tzif`]); the
//! format is a small `strftime` subset ([`format`]).
//!
//! ## Arming, and the check after it
//!
//! Each refresh reads the clock once, shows that instant, and arms the
//! timer at the end of the period shown (the [`Period`] containing it).
//! `CANCEL_ON_SET` reports only a step made while the timer is armed, so a
//! step landing between reading the clock and arming it would go unseen and
//! leave the clock an hour out until the next boundary. So after arming,
//! the clock is read again: if it is no longer inside the period shown, the
//! refresh starts over (a bounded number of times: a clock stepped without
//! end still leaves the loop, showing its latest reading and armed for its
//! next boundary). The same check catches a boundary passing between the
//! reading and the arm, which the spike's loop missed.

pub mod format;
pub mod timer;
pub mod tzif;
pub mod zone;

#[cfg(test)]
mod tests;

use std::fmt::Write;
use std::os::fd::AsFd;

use rustix::event::PollFlags;

use super::{Init, Module, OutputView, Sources, Update, View};
use crate::print::warn;
use format::{Civil, Format};
use timer::{Fired, Timer};
use tzif::Tz;
use zone::{Spec, Stamp};

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "clock";

/// How many times a refresh re-reads a clock that keeps moving under it.
const MAX_ATTEMPTS: u32 = 4;

/// The clock's options.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub format: Format,
}

/// How long one shown value lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Minute,
    Second,
}

impl Period {
    fn seconds(self) -> i64 {
        match self {
            Self::Minute => 60,
            Self::Second => 1,
        }
    }
}

/// The span of time one shown value covers: `[start, next)`, Unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: i64,
    pub next: i64,
}

impl Span {
    /// The period containing `t` in a zone `offset` seconds east of UTC:
    /// local minutes (offsets in whole seconds, as some old ones are, move
    /// the boundary off the UTC minute).
    pub fn containing(t: i64, offset: i32, period: Period) -> Self {
        let length = period.seconds();
        let wall = tzif::clamp(t) + i64::from(offset);
        let start = wall - wall.rem_euclid(length) - i64::from(offset);
        Self {
            start,
            next: start + length,
        }
    }

    pub fn contains(self, t: i64) -> bool {
        self.start <= t && t < self.next
    }
}

pub fn init(settings: &super::Settings) -> Init {
    match Clock::new(settings.clock.clone(), Spec::from_env()) {
        Ok(clock) => Init::Available(Box::new(clock)),
        Err(error) => Init::Unavailable(format!("cannot create its timer: {error}")),
    }
}

pub struct Clock {
    format: Format,
    period: Period,
    spec: Spec,
    /// The zone file as last read; `None` when it could not be `statx`ed.
    stamp: Option<Stamp>,
    tz: Tz,
    timer: Timer,
    /// What is shown, and the text being built (swapped: no allocation
    /// once both have grown to the format's length).
    text: String,
    scratch: String,
    /// Said once each: arming failed (the clock then updates only on the
    /// next event that reaches it), reading the timer failed.
    arm_failed: bool,
    read_failed: bool,
}

impl Clock {
    fn new(settings: Settings, spec: Spec) -> std::io::Result<Self> {
        let timer = Timer::new()?;
        let stamp = spec.path().and_then(zone::stamp);
        let loaded = zone::load(&spec);
        if let Some(problem) = &loaded.problem {
            warn(format_args!("scootbar: clock: {problem}"));
        }
        let period = if settings.format.seconds() {
            Period::Second
        } else {
            Period::Minute
        };
        let mut clock = Self {
            format: settings.format,
            period,
            spec,
            stamp,
            tz: loaded.tz,
            timer,
            text: String::new(),
            scratch: String::new(),
            arm_failed: false,
            read_failed: false,
        };
        let _ = clock.refresh();
        Ok(clock)
    }

    /// Reads the zone file again if it changed since it was read.
    fn check_zone(&mut self) {
        let Some(path) = self.spec.path() else {
            return;
        };
        let Some(stamp) = zone::stamp(path) else {
            // Gone: keep the zone last read (module docs of `zone`).
            return;
        };
        if self.stamp == Some(stamp) {
            return;
        }
        self.stamp = Some(stamp);
        let loaded = zone::load(&self.spec);
        if let Some(problem) = &loaded.problem {
            warn(format_args!("scootbar: clock: {problem}"));
        }
        self.tz = loaded.tz;
    }

    /// Shows the time now and arms the timer for the end of what is shown
    /// (see the module docs). Returns whether the text changed.
    fn refresh(&mut self) -> Update {
        for _ in 0..MAX_ATTEMPTS {
            let now = timer::now();
            let span = self.render(now);
            match self.timer.arm(span.next) {
                Ok(()) => self.arm_failed = false,
                Err(error) => {
                    if !self.arm_failed {
                        warn(format_args!(
                            "scootbar: clock: cannot arm its timer: {error}"
                        ));
                    }
                    self.arm_failed = true;
                    break;
                }
            }
            if span.contains(timer::now()) {
                break;
            }
        }
        if self.scratch == self.text {
            return Update::Unchanged;
        }
        std::mem::swap(&mut self.text, &mut self.scratch);
        Update::Changed
    }

    /// Formats `now` into the scratch text; returns the span it covers.
    fn render(&mut self, now: i64) -> Span {
        let local = self.tz.at(now);
        self.scratch.clear();
        self.format
            .render(&Civil::at(now, local), &mut self.scratch);
        Span::containing(now, local.offset, self.period)
    }
}

impl Module for Clock {
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        sources.add(self.timer.as_fd(), PollFlags::IN);
    }

    fn on_ready(&mut self, _source: usize, _events: PollFlags) -> Update {
        // A tick, a clock step or a spurious wake all end the same way:
        // show the time now and arm for the next boundary, which also
        // clears whatever made the fd readable.
        match self.timer.read() {
            Ok(Fired::Tick | Fired::ClockSet | Fired::Nothing) => {}
            Err(error) => {
                if !self.read_failed {
                    warn(format_args!("scootbar: clock: reading its timer: {error}"));
                }
                self.read_failed = true;
            }
        }
        self.check_zone();
        self.refresh()
    }

    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let _ = view.text_mut().write_str(&self.text);
    }
}
