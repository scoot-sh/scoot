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
            // Field codes expand once: no `%` sequence survives that
            // is not a literal `%%` the input wrote... (untestable in
            // general: `%c` legitimately injects `%f`-looking text.
            // What is testable is the count: at most the input's
            // argument count plus the two `%i`/`--icon` arguments and
            // the terminal prefix.)
            assert!(argv.len() <= text.split([' ', '\t']).count() + 3);
        }
        Some(())
    }
}

fuzz_target!(|data: &[u8]| {
    fuzz::exec(data);
});
