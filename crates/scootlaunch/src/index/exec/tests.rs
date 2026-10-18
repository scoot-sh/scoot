//! Tests for Exec: every rule in the design's Exec section,
//! including the must-reject cases, in both modes.

use super::*;

fn expansion<'a>(icon: Option<&'a str>) -> Expansion<'a> {
    Expansion {
        icon,
        name: "Föö Bar",
        desktop_path: "/home/user/.local/share/applications/foo.desktop",
    }
}

fn strict(exec: &str) -> Result<Vec<String>, Error> {
    expand(exec, &expansion(Some("foo")), false, false)
}

#[test]
fn plain_command_splits_on_spaces() {
    assert_eq!(strict("foo bar baz").unwrap(), vec!["foo", "bar", "baz"]);
    assert_eq!(strict("  foo   bar  ").unwrap(), vec!["foo", "bar"]);
    assert_eq!(strict("foo\tbar").unwrap(), vec!["foo", "bar"]);
}

#[test]
fn double_quotes_group_and_unescape_four() {
    assert_eq!(
        strict(r#"foo "bar baz" qux"#).unwrap(),
        vec!["foo", "bar baz", "qux"]
    );
    // `\" \$ \\ \`` unescape; anything else refuses in strict mode,
    // and stays literal in lenient mode.
    assert_eq!(
        strict(r#"foo "a\"b""#).unwrap(),
        vec!["foo", r#"a"b"#],
        "phase 1 passes the escape through; the quoting rule owns it"
    );
    assert_eq!(strict(r#"foo "a\$b""#).unwrap(), vec!["foo", "a$b"]);
    // `\\` is the general escape for one backslash: phase 1 yields
    // `\b`, which the strict quoting rule refuses and lenient keeps.
    assert_eq!(strict(r#"foo "a\\b""#), Err(Error::QuotedEscape('b')));
    let out = expand(r#"foo "a\\b""#, &expansion(None), false, true).unwrap();
    assert_eq!(out, vec!["foo", r"a\b"]);
    assert_eq!(strict("foo \"a\\`b\"").unwrap(), vec!["foo", "a`b"]);
    assert_eq!(strict(r#"foo "a\qb""#), Err(Error::QuotedEscape('q')));
}

#[test]
fn lenient_leaves_unknown_quoted_escapes_alone() {
    let out = expand(r#"foo "a\qb""#, &expansion(None), false, true).unwrap();
    assert_eq!(out, vec!["foo", r"a\qb"]);
}

#[test]
fn quoting_applies_after_escapes() {
    // `\\s` is the general escape for a backslash plus `s`: phase 1
    // yields `\s`, which is not a general escape and reaches phase 2
    // as an unquoted backslash — refused in strict mode, literal `s`
    // in lenient mode. A general `\"` never exists: `\"` in the file
    // rides phase 1 through to the quoting rule.
    assert_eq!(strict(r"foo A\\sB"), Err(Error::UnquotedReserved('\\')));
    let out = expand(r"foo A\\sB", &expansion(None), false, true).unwrap();
    assert_eq!(out, vec!["foo", "AsB"]);
}

#[test]
fn strict_rejects_unquoted_reserved() {
    for reserved in [
        '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`',
    ] {
        let exec = format!("foo a{reserved}b");
        assert_eq!(
            strict(&exec),
            Err(Error::UnquotedReserved(reserved)),
            "for `{reserved}`"
        );
    }
}

#[test]
fn strict_rejects_unclosed_quotes() {
    assert_eq!(strict("foo \"bar"), Err(Error::UnclosedQuote));
    // A trailing backslash rides phase 1 through; phase 2 then ends
    // inside quotes either way.
    assert_eq!(strict("foo \"bar\\"), Err(Error::UnclosedQuote));
}

#[test]
fn executable_may_not_contain_equals() {
    assert_eq!(strict("fo=o bar"), Err(Error::EqualsInExecutable));
    // `=` past argv[0] is fine.
    assert_eq!(strict("foo a=b").unwrap(), vec!["foo", "a=b"]);
}

#[test]
fn empty_exec_is_invalid() {
    assert_eq!(strict(""), Err(Error::Empty));
    assert_eq!(strict("   "), Err(Error::Empty));
}

#[test]
fn file_codes_are_removed() {
    for code in ["%f", "%F", "%u", "%U"] {
        assert_eq!(
            strict(&format!("foo {code}")).unwrap(),
            vec!["foo"],
            "for {code}"
        );
        assert_eq!(
            strict(&format!("foo a{code}b")).unwrap(),
            vec!["foo", "ab"],
            "for {code}"
        );
    }
}

#[test]
fn deprecated_codes_are_removed() {
    for code in ["%d", "%D", "%n", "%N", "%v", "%m"] {
        assert_eq!(
            strict(&format!("foo {code}")).unwrap(),
            vec!["foo"],
            "for {code}"
        );
    }
}

#[test]
fn icon_code_becomes_two_arguments_or_nothing() {
    assert_eq!(strict("foo %i").unwrap(), vec!["foo", "--icon", "foo"]);
    // Empty or missing Icon: the code vanishes with its argument.
    let no_icon = Expansion {
        icon: None,
        ..expansion(None)
    };
    assert_eq!(
        expand("foo %i", &no_icon, false, false).unwrap(),
        vec!["foo"]
    );
    let empty_icon = Expansion {
        icon: Some(""),
        ..expansion(Some(""))
    };
    assert_eq!(
        expand("foo %i", &empty_icon, false, false).unwrap(),
        vec!["foo"]
    );
    // Inside a larger argument it expands in place.
    assert_eq!(strict("foo --icon=%i").unwrap(), vec!["foo", "--icon=foo"]);
}

#[test]
fn name_and_path_codes_expand_as_one_argument() {
    assert_eq!(strict("foo %c").unwrap(), vec!["foo", "Föö Bar"]);
    assert_eq!(
        strict("foo %k").unwrap(),
        vec!["foo", "/home/user/.local/share/applications/foo.desktop"]
    );
    // Expanded once: a Name holding spaces or quotes injects no
    // arguments and is never re-scanned.
    let tricky = Expansion {
        name: "a b %f \"c\"",
        ..expansion(None)
    };
    assert_eq!(
        expand("foo %c", &tricky, false, false).unwrap(),
        vec!["foo", "a b %f \"c\""]
    );
}

#[test]
fn percent_percent_becomes_percent() {
    assert_eq!(strict("foo 100%%").unwrap(), vec!["foo", "100%"]);
}

#[test]
fn unknown_codes_reject() {
    assert_eq!(strict("foo %x"), Err(Error::UnknownFieldCode('x')));
    assert_eq!(strict("foo %s"), Err(Error::UnknownFieldCode('s')));
    assert_eq!(strict("foo %"), Err(Error::TrailingPercent));
}

#[test]
fn quoted_codes_only_percent_percent() {
    assert_eq!(strict("foo \"100%%\"").unwrap(), vec!["foo", "100%"]);
    assert_eq!(strict("foo \"%c\""), Err(Error::QuotedFieldCode('c')));
    assert_eq!(strict("foo \"%f\""), Err(Error::QuotedFieldCode('f')));
    assert_eq!(strict("foo \"%i\""), Err(Error::QuotedFieldCode('i')));
    assert_eq!(strict("foo \"a%\""), Err(Error::TrailingPercent));
}

#[test]
fn quoted_empty_stays_but_removed_codes_vanish() {
    assert_eq!(strict("foo \"\"").unwrap(), vec!["foo", ""]);
    assert_eq!(strict("foo \"%%\"").unwrap(), vec!["foo", "%"]);
}

#[test]
fn unknown_escapes_reach_the_quoting_rule() {
    // `\q` is no general escape, so phase 1 passes it through;
    // unquoted, the strict quoting rule refuses the backslash.
    assert_eq!(strict(r"foo \q"), Err(Error::UnquotedReserved('\\')));
}

#[test]
fn terminal_prefixes_the_default_terminal() {
    assert_eq!(
        expand("foo bar", &expansion(None), true, false).unwrap(),
        vec!["foot", "foo", "bar"]
    );
}

#[test]
fn lenient_accepts_single_quotes() {
    let out = expand("foo 'bar baz'", &expansion(None), false, true).unwrap();
    assert_eq!(out, vec!["foo", "bar baz"]);
    // No escapes and no codes inside single quotes: literal.
    let out = expand("foo 'a\\b %f'", &expansion(None), false, true).unwrap();
    assert_eq!(out, vec!["foo", r"a\b %f"]);
    // Unclosed single quotes still refuse.
    assert_eq!(
        expand("foo 'bar", &expansion(None), false, true),
        Err(Error::UnclosedQuote)
    );
}

#[test]
fn lenient_passes_unquoted_reserved_through() {
    let out = expand("foo a&b|c;d$e", &expansion(None), false, true).unwrap();
    assert_eq!(out, vec!["foo", "a&b|c;d$e"]);
    // A backslash quotes the next character, whatever it is.
    let out = expand("foo a\\ b", &expansion(None), false, true).unwrap();
    assert_eq!(out, vec!["foo", "a b"]);
}

#[test]
fn lenient_keeps_field_codes_strict() {
    // Leniency is quoting only: the must-reject cases reject here too.
    assert_eq!(
        expand("foo %x", &expansion(None), false, true),
        Err(Error::UnknownFieldCode('x'))
    );
    assert_eq!(
        expand("foo \"%c\"", &expansion(None), false, true),
        Err(Error::QuotedFieldCode('c'))
    );
    assert_eq!(
        expand("fo=o", &expansion(None), false, true),
        Err(Error::EqualsInExecutable)
    );
}

#[test]
fn spec_section_7_examples() {
    // The quoting examples the spec works through (§7).
    assert_eq!(
        strict("foo --foo bar").unwrap(),
        vec!["foo", "--foo", "bar"]
    );
    assert_eq!(
        strict("foo \"--foo bar\"").unwrap(),
        vec!["foo", "--foo bar"]
    );
}
