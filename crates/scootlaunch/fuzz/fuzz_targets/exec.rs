//! Any bytes as an `Exec` value (skipped unless UTF-8): expansion
//! refuses or accepts without a panic, in both modes, and an accepted
//! argv is non-empty with no `=` in its executable. The checks are
//! `fuzz::exec` below.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../src/index/exec.rs"]
mod exec;

mod fuzz {
    use super::exec;

    pub fn exec(data: &[u8]) -> Option<()> {
        let text = std::str::from_utf8(data).ok()?;
        let expansion = exec::Expansion {
            icon: Some("icon with spaces"),
            name: "na%me \"quoted\"",
            desktop_path: "/x/y.desktop",
        };
        for (terminal, lenient) in [(false, false), (true, false), (false, true), (true, true)] {
            let argv = match exec::expand(text, &expansion, terminal, lenient) {
                Ok(argv) => argv,
                Err(_) => continue,
            };
            // An accepted argv launches: non-empty, and the
            // executable holds no `=`.
            assert!(!argv.is_empty());
            assert!(!argv[usize::from(terminal)].contains('='));
            // Field codes expand once: one input argument yields at
            // most two output arguments (a standalone `%i` becomes
            // `--icon <Icon>`), plus the terminal prefix. A `%c`
            // legitimately injects text that looks like a field code.
            assert!(argv.len() <= 2 * text.split([' ', '\t']).count() + 1);
        }
        Some(())
    }
}

fuzz_target!(|data: &[u8]| {
    fuzz::exec(data);
});
