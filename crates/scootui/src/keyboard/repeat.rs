//! Key repeat: one `timerfd`, armed only while a repeatable key is held.
//!
//! The compositor hands the rate and delay over in `repeat_info`; the
//! client owns the ticking. Exactly one timerfd exists and only while a
//! repeatable key is held: arming creates it, every cancel path drops it,
//! so an idle seat holds no fd and takes no wakeups. At most one key
//! repeats at a time (the latest press wins, like the compositor's own
//! bind repeat): a second press while one is held moves the repeat to the
//! new key, and releasing a key that is not the repeat key changes
//! nothing.
//!
//! A rate of 0 means no repeat at all: presses never arm, and (per the
//! protocol) the compositor may then send `repeated`-state keys itself,
//! which the seat layer turns into ticks without this timer.
//!
//! Cancel paths, each load-bearing: the repeat key's release, keyboard
//! focus loss, a keymap change mid-hold, and a repeat-info change to rate
//! 0 (a change to nonzero values re-arms from now).

use std::os::unix::io::{BorrowedFd, OwnedFd};

use rustix::fd::AsFd;
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};

/// One-shot repeat state for a seat.
#[derive(Debug, Default)]
pub struct Repeater {
    timer: Option<OwnedFd>,
    key: Option<u32>,
    rate: i32,
    delay: i32,
}

impl Repeater {
    /// Idle: no timer, no key, and whatever the compositor last said.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a `repeat_info` event. Negative values are illegal on the
    /// wire; a sender that emits them is misbehaving, so they are ignored
    /// and the previous pair stands (last-good, like a refused look
    /// reload). A change while a key is held re-arms from now, so a new
    /// rate never waits for the next press and rate 0 stops this hold.
    pub fn set_repeat_info(&mut self, rate: i32, delay: i32) {
        if rate < 0 || delay < 0 {
            return;
        }
        self.rate = rate;
        self.delay = delay;
        if let Some(key) = self.key {
            if self.rate > 0 {
                self.arm(key);
            } else {
                // Rate 0 ends the live hold: nothing may tick, and the
                // compositor may take over with `repeated` states.
                self.disarm();
            }
        }
    }

    /// A press: arm when the key repeats and the rate allows, replacing
    /// any in-flight repeat (latest wins). A press that cannot repeat
    /// leaves a live repeat alone: holding `a` and tapping Shift must not
    /// stop the `a`s.
    pub fn press(&mut self, evdev: u32, repeatable: bool) {
        if repeatable && self.rate > 0 {
            self.arm(evdev);
        }
    }

    /// A release: only the repeat key's own release disarms; any other
    /// key's release is irrelevant to the ticking.
    pub fn release(&mut self, evdev: u32) {
        if self.key == Some(evdev) {
            self.disarm();
        }
    }

    /// Keyboard focus left: the compositor will not send the release for
    /// a key held across the loss, so drop the timer rather than repeat
    /// into the next focus.
    pub fn focus_lost(&mut self) {
        self.disarm();
    }

    /// The keymap changed: keycodes may mean different keys now, so the
    /// held repeat is over.
    pub fn keymap_changed(&mut self) {
        self.disarm();
    }

    /// Whether the timer exists (a repeatable key is held at a nonzero
    /// rate). The idle invariant the acceptance test pins: false with no
    /// key held.
    pub fn armed(&self) -> bool {
        self.timer.is_some()
    }

    /// The last `repeat_info` rate: 0 means the compositor may drive
    /// repetition itself (see the module docs).
    pub fn rate(&self) -> i32 {
        self.rate
    }

    /// The key the armed timer belongs to, if any.
    pub fn held(&self) -> Option<u32> {
        self.key
    }

    /// The fd to poll for expirations, when armed. Readable means at
    /// least one tick is due; the app calls [`Repeater::take_expirations`]
    /// and emits that many repeats.
    pub fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
        self.timer.as_ref().map(AsFd::as_fd)
    }

    /// The expirations since the last call (0 when nothing is due or a
    /// disarm raced the poll). Reading the timerfd both reports and
    /// clears: each expiration becomes exactly one repeat downstream. The
    /// fd is nonblocking, so this never stalls the daemon loop.
    pub fn take_expirations(&self) -> u64 {
        let Some(timer) = self.timer.as_ref() else {
            return 0;
        };
        let mut buf = [0u8; 8];
        match rustix::io::read(timer, &mut buf) {
            Ok(8) => u64::from_ne_bytes(buf),
            _ => 0,
        }
    }

    fn arm(&mut self, evdev: u32) {
        let first = millis(self.delay.max(0));
        // The rate divides evenly on every compositor that sends one
        // (scoot's 25/s is 40 ms); integer division keeps the step exact
        // rather than accumulating a float remainder.
        let step = millis(1000 / self.rate.max(1));
        let spec = Itimerspec {
            it_interval: step,
            it_value: first,
        };
        match timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        ) {
            Ok(timer) => {
                if timerfd_settime(&timer, TimerfdTimerFlags::empty(), &spec).is_ok() {
                    self.timer = Some(timer);
                    self.key = Some(evdev);
                } else {
                    // Without a live timer the repeat never fires: claim
                    // nothing, so the state never says armed when it is
                    // not (the compositor does the same on its own arm
                    // failure).
                    self.disarm();
                }
            }
            Err(_) => self.disarm(),
        }
    }

    fn disarm(&mut self) {
        // Drop the fd: disarmed means no timer at all, not a zeroed one.
        self.timer = None;
        self.key = None;
    }
}

/// A millisecond count as a `Timespec`, saturating rather than wrapping
/// on absurd inputs (a hostile `repeat_info` never becomes a negative
/// timer).
fn millis(ms: i32) -> Timespec {
    let ms = ms.max(0) as u64;
    Timespec {
        tv_sec: (ms / 1000) as _,
        tv_nsec: ((ms % 1000) * 1_000_000) as _,
    }
}
