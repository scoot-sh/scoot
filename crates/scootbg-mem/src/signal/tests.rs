//! Only what needs no signal to be sent: the set, and a drain of an empty
//! pipe. Blocking signals in the test harness's process would outlive the
//! test, so the delivery path is covered end to end instead, by the
//! daemon's own integration tests (SIGTERM and SIGINT to a live daemon;
//! `crates/scootbg/tests/`).

use rustix::runtime::Signal;

use super::{TERMINATION, termination_set};

#[test]
fn the_set_is_term_int_and_hup_only() {
    assert_eq!(TERMINATION, [Signal::TERM, Signal::INT, Signal::HUP]);
    let set = termination_set();
    for signal in TERMINATION {
        assert!(set.contains(signal));
    }
    for other in [Signal::KILL, Signal::PIPE, Signal::CHILD, Signal::USR1] {
        assert!(!set.contains(other));
    }
}

#[test]
fn every_termination_signal_fits_the_pipe_byte() {
    for signal in TERMINATION {
        let raw = signal.as_raw();
        assert!((1..=255).contains(&raw));
        assert_ne!(raw as u8, super::WAIT_FAILED);
    }
}
