//! A one-shot timer as a poll-loop fd: made when a destructive row is
//! armed, read and dropped when it fires, so a module with nothing armed
//! owns no timer and an idle bar is woken by none. (The bluetooth and media
//! modules' own; copied, not shared, so no module's timing depends on
//! another's.)

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::time::Duration;

use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

pub(super) struct OneShot(OwnedFd);

impl OneShot {
    /// A timer that fires once, `after` from now. `None` when the fd cannot
    /// be made or armed (out of descriptors): the caller does without, and
    /// the armed row is disarmed the slower way (on the next row or popup
    /// event), never the wrong one.
    pub(super) fn after(after: Duration) -> Option<Self> {
        let fd = timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        )
        .ok()?;
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                // A zero value would disarm it: at least a nanosecond.
                tv_sec: after.as_secs() as i64,
                tv_nsec: i64::from(after.subsec_nanos()).max(1),
            },
        };
        timerfd_settime(&fd, TimerfdTimerFlags::empty(), &spec).ok()?;
        Some(Self(fd))
    }

    /// Reads the expiry (the fd is readable until it is read).
    pub(super) fn drain(&self) {
        let mut expirations = [0u8; 8];
        let _ = rustix::io::read(&self.0, &mut expirations);
    }
}

impl AsFd for OneShot {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
