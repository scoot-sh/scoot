//! A one-shot monotonic `timerfd` for the `exec` module: its restart delay
//! and the frame it holds a flood to. Relative, so a clock step or a
//! suspend cannot make it fire early or an hour late.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::time::Duration;

use rustix::io::Errno;
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

#[derive(Debug)]
pub struct Timer {
    fd: OwnedFd,
}

impl Timer {
    pub fn new() -> io::Result<Self> {
        let fd = timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        )?;
        Ok(Self { fd })
    }

    /// Fires once, `after` from now, replacing any earlier deadline. A
    /// zero duration would disarm it, so it is a nanosecond.
    pub fn arm(&self, after: Duration) -> io::Result<()> {
        let after = after.max(Duration::from_nanos(1));
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
                tv_nsec: i64::from(after.subsec_nanos()),
            },
        };
        timerfd_settime(&self.fd, TimerfdTimerFlags::empty(), &spec)?;
        Ok(())
    }

    /// Whether it fired since it was last read (and clears that).
    pub fn fired(&self) -> bool {
        let mut expirations = [0u8; 8];
        match rustix::io::read(&self.fd, &mut expirations) {
            Ok(8) => true,
            Ok(_) | Err(Errno::AGAIN) | Err(Errno::INTR) => false,
            // An fd that cannot be read is no use: said as not fired, and
            // the module's own deadline check covers it.
            Err(_) => false,
        }
    }
}

impl AsFd for Timer {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
