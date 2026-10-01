use super::*;

#[test]
fn a_name_is_letters_digits_dash_and_underscore() {
    let longest = "n".repeat(MAX_NAME);
    for good in [
        "a",
        "launcher",
        "A1",
        "power-menu",
        "my_mod",
        "0x",
        longest.as_str(),
    ] {
        assert_eq!(check_name(good), Ok(()), "{good}");
    }
}

#[test]
fn a_bad_name_is_refused_by_what_it_takes() {
    let too_long = "n".repeat(MAX_NAME + 1);
    for bad in [
        "",
        "-lead",
        "_lead",
        "has space",
        "dot.ted",
        "slash/",
        "q\"uote",
        "é",
        "tab\t",
        "nul\0",
        too_long.as_str(),
    ] {
        let error = check_name(bad).unwrap_err();
        assert!(error.contains("takes"), "{bad:?}: {error}");
    }
}

#[cfg(feature = "clock")]
#[test]
fn a_built_in_name_is_taken() {
    let error = check_name("clock").unwrap_err();
    assert!(error.contains("built-in"), "{error}");
}

#[test]
fn a_name_is_interned_once_and_reused() {
    let a = intern("interned-once").unwrap();
    let b = intern("interned-once").unwrap();
    assert_eq!(a, "interned-once");
    // The same string, not a copy.
    assert!(std::ptr::eq(a, b));
    let other = intern("interned-other").unwrap();
    assert!(!std::ptr::eq(a, other));
}

#[test]
fn interning_is_capped_and_a_known_name_still_resolves() {
    // A table of this test's own: filling the process-wide one would starve
    // every sibling that interns a name while `cargo test` runs them side by
    // side in one process.
    let table = Mutex::new(Vec::new());
    let first = intern_in(&table, "cap-first").unwrap();
    let mut made = 1;
    for i in 0..MAX_NAMES * 2 {
        if intern_in(&table, &format!("cap-{i}")).is_some() {
            made += 1;
        }
    }
    assert_eq!(made, MAX_NAMES);
    assert_eq!(intern_in(&table, &format!("cap-{}", MAX_NAMES * 2)), None);
    // A name already in the table is found even when it is full.
    assert!(std::ptr::eq(first, intern_in(&table, "cap-first").unwrap()));
}

/// Each exec module polls at most two fds (its pipe or timer, and its
/// pidfd); with the clock's one, all placed together fit what the loop has
/// for the modules.
const _: () = assert!(MAX_EXEC * 2 < super::super::MAX_SOURCES);
