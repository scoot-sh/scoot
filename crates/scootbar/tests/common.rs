//! The app binary under test, and the shared harness: `scootui`'s
//! `test-support` holds the sessions and screenshot readers (moved there
//! by `scootui-extract-wayland` so the shell apps' tests share them); this
//! only says which binary is under test here. `CARGO_BIN_EXE_scootbar`
//! exists only when a test binary builds (never in a library), which is
//! why it lives in this shim and not in the harness.

use std::path::PathBuf;

pub use scootui::test_support::*;

/// This test run's `scootbar`: each test passes it to the harness when it
/// builds a session or a bare command.
pub fn scootbar_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_scootbar"))
}
