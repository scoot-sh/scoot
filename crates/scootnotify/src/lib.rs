//! `scootnotify`: the notification daemon's library — the D-Bus server
//! and the model, headless in this ticket.
//!
//! The binary (`main.rs`) is the thin CLI over this: `daemon`,
//! `daemon --check`, `--help`, `--version`. The fuzz targets
//! (`fuzz/`, their own workspace) call the `fuzz_check` functions here.
//!
//! No `unsafe` here. Nothing here links beyond what `std` links.

#![forbid(unsafe_code)]
// The parsers' refusal idiom is `Err(())`: a misshapen body, a refused
// frame, a skipped value. It carries no information by design (a refusal
// names its reason where a reason exists, and the wire errors carry
// their own text), so a custom error type would add variants without
// readers — the same rule `scoot-dbus` documents.
// Re-typing every reader would be a redesign, not this ticket.
#![allow(clippy::result_unit_err)]

// Tests pinning allocations count heap allocations through this
// (scootbg_mem::count), forwarding everything else to the system
// allocator. Test-only, Linux-only like the crate it comes from: the
// shipped library uses the default. Without this the pins read a
// vacuous 0 (the allocator they count through would be nobody's).
#[cfg(all(test, target_os = "linux"))]
#[global_allocator]
static COUNTING_ALLOC: scootbg_mem::CountingAlloc = scootbg_mem::CountingAlloc;

pub mod bus;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod look;
pub mod markup;
pub mod model;
pub mod popup;
pub mod view;

/// Replays a committed fuzz corpus (`fuzz/corpus/<kind>` plus
/// `fuzz/regressions/<kind>`) through `check` on stable: every seed
/// must survive without a panic. `expect_ok` asserts every seed parses
/// (for corpora of valid inputs); corpora with refusal seeds pass
/// `false` and let the check's own assertions do the pinning.
#[cfg(test)]
pub(crate) fn replay_corpus(kind: &str, expect_ok: bool, check: fn(&[u8]) -> bool) {
    use std::path::PathBuf;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz");
    let mut count = 0;
    for dir in ["corpus", "regressions"] {
        let dir = root.join(dir).join(kind);
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|error| {
            panic!("the {kind} corpus is missing ({}): {error}", dir.display())
        });
        for entry in entries {
            let entry = entry.expect("a corpus entry reads");
            if !entry.file_type().expect("a corpus type reads").is_file() {
                continue;
            }
            // Tracking placeholders (`.gitkeep`), not seeds.
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let bytes = std::fs::read(entry.path()).expect("a seed reads");
            let ok = check(&bytes);
            if expect_ok {
                assert!(ok, "{} must parse", entry.path().display());
            }
            count += 1;
        }
    }
    assert!(count > 0, "the {kind} corpus is empty");
}
