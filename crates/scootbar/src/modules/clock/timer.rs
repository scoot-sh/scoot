//! The clock's timer: one `timerfd` on `CLOCK_REALTIME`, armed at an
//! absolute instant with `TFD_TIMER_CANCEL_ON_SET`.
//!
//! Absolute, so it fires at the wall-clock boundary however long the
//! machine slept; cancel-on-set, so a step of the clock (NTP, `date -s`, a
//! resume from suspend, which the kernel reports as a clock set:
//! `timekeeping_resume` calls `timerfd_resume`) wakes it at once with
//! `ECANCELED` instead of leaving it an hour out. M0 measured both (the
//! record's §2).

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::io::Errno;
use rustix::time::{
    ClockId, Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, clock_gettime,
    timerfd_create, timerfd_settime,
};

/// Why the timer's fd was readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fired {
    /// The deadline passed.
    Tick,
    /// The clock was set (stepped) while armed; the timer is disarmed.
    ClockSet,
    /// Nothing to read: a spurious wake.
    Nothing,
}

#[derive(Debug)]
pub struct Timer {
    fd: OwnedFd,
}

impl Timer {
    pub fn new() -> io::Result<Self> {
        let fd = timerfd_create(
            TimerfdClockId::Realtime,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        )?;
        Ok(Self { fd })
    }

    /// Arms the timer for the instant `at` (Unix seconds, on the second),
    /// replacing any earlier deadline.
    pub fn arm(&self, at: i64) -> io::Result<()> {
        let zero = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let spec = Itimerspec {
            it_interval: zero,
            it_value: Timespec {
                // A deadline of 0 would disarm it; the clock never reads
                // one, and 1 is as good as past.
                tv_sec: at.max(1),
                tv_nsec: 0,
            },
        };
        timerfd_settime(
            &self.fd,
            TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
            &spec,
        )?;
        Ok(())
    }

    /// Reads what woke the fd.
    pub fn read(&self) -> io::Result<Fired> {
        let mut expirations = [0u8; 8];
        match rustix::io::read(&self.fd, &mut expirations) {
            Ok(8) => Ok(Fired::Tick),
            Ok(_) | Err(Errno::AGAIN) => Ok(Fired::Nothing),
            Err(Errno::CANCELED) => Ok(Fired::ClockSet),
            Err(errno) => Err(errno.into()),
        }
    }
}

impl AsFd for Timer {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// The wall clock, in whole Unix seconds.
pub fn now() -> i64 {
    clock_gettime(ClockId::Realtime).tv_sec
}
