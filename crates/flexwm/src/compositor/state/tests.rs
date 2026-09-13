//! Tests for [`socket_error`], the startup message `State::listen` reports
//! instead of panicking when the wayland socket cannot be created.
//!
//! Only the mapping is unit-tested. Actually provoking
//! [`BindError::RuntimeDirNotSet`] needs `$XDG_RUNTIME_DIR` unset for the
//! whole process, which every other test in this binary that builds a
//! [`State`](super::State) depends on -- so the end-to-end behavior (a real
//! release binary, the message on stderr, exit 1, no core dump) is verified on
//! the dev VM and recorded in `ROADMAP.md` instead of faked here.

use std::io;

use super::*;

/// The one case that actually happens, and the one the `.expect()` this
/// replaced turned into a core dump.
#[test]
fn an_unset_runtime_dir_says_which_variable_and_what_to_do() {
    let message = socket_error(BindError::RuntimeDirNotSet);
    assert!(
        message.contains("XDG_RUNTIME_DIR"),
        "the message must name the variable to set: {message}"
    );
    assert!(
        message.contains("writable directory"),
        "the message must say what to set it to: {message}"
    );
}

#[test]
fn an_unwritable_runtime_dir_says_so() {
    let message = socket_error(BindError::PermissionDenied);
    assert!(
        message.contains("not writable"),
        "wrong diagnosis for a permission failure: {message}"
    );
}

#[test]
fn every_socket_taken_says_how_many_were_tried() {
    let message = socket_error(BindError::AlreadyInUse);
    assert!(
        message.contains("wayland-1") && message.contains("wayland-32"),
        "the message must name the range new_auto actually tries: {message}"
    );
}

/// An I/O failure has a real cause attached; losing it would leave an
/// operator with strictly less than `BindError`'s own `Display` gave them.
#[test]
fn an_io_failure_keeps_the_underlying_error() {
    let source = io::Error::from(io::ErrorKind::NotFound);
    let expected = source.to_string();
    let message = socket_error(BindError::Io(source));
    assert!(
        message.contains(&expected),
        "the io error's own text is missing from {message}"
    );
}

/// Whatever the variant, the line `main` prints reads as one sentence
/// starting from what failed -- the same shape as `ipc::init`'s "no socket
/// path: ..." error.
#[test]
fn every_variant_is_prefixed_with_what_failed() {
    for error in [
        BindError::RuntimeDirNotSet,
        BindError::PermissionDenied,
        BindError::AlreadyInUse,
        BindError::Io(io::Error::from(io::ErrorKind::PermissionDenied)),
    ] {
        let message = socket_error(error);
        assert!(
            message.starts_with("no wayland socket: "),
            "unprefixed startup error: {message}"
        );
    }
}
