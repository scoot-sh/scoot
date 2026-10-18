//! Tests for the format: every rule in the design's Format paragraph,
//! against the spec's own examples plus hostile input.

use super::*;

// A minimal valid file all fixtures build on.
const MINIMAL: &str = "[Desktop Entry]\nType=Application\nName=Foo\nExec=foo\n";

#[test]
fn minimal_file_parses() {
    let file = parse(MINIMAL.as_bytes()).unwrap();
    assert_eq!(file.groups.len(), 1);
    assert_eq!(file.groups[0].name, "Desktop Entry");
    assert_eq!(file.groups[0].get("Type"), Some("Application"));
    assert_eq!(file.groups[0].get("Name"), Some("Foo"));
    assert!(file.notes.is_empty());
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let file = parse(b"# a comment\n\n[Desktop Entry]\n# another\nName=Foo\n").unwrap();
    assert_eq!(file.groups[0].get("Name"), Some("Foo"));
    assert!(file.notes.is_empty());
}

#[test]
fn missing_desktop_entry_group_skips_the_file() {
    assert_eq!(
        parse(b"[Desktop Action Foo]\nName=Foo\n"),
        Err(Error::NoDesktopEntry)
    );
    assert_eq!(parse(b""), Err(Error::NoDesktopEntry));
    assert_eq!(parse(b"# only a comment\n"), Err(Error::NoDesktopEntry));
}

#[test]
fn desktop_entry_must_come_first() {
    // A `[Desktop Action]` group first means no leading `[Desktop
    // Entry]`: the file is skipped.
    assert_eq!(
        parse(b"[Desktop Action Bar]\nName=Bar\n[Desktop Entry]\nName=Foo\n"),
        Err(Error::NoDesktopEntry)
    );
}

#[test]
fn key_splits_at_the_first_equals_with_spaces_trimmed() {
    let file = parse(b"[Desktop Entry]\n  Name  =  Foo Bar  \nExec=a=b\n").unwrap();
    assert_eq!(file.groups[0].get("Name"), Some("Foo Bar"));
    assert_eq!(file.groups[0].get("Exec"), Some("a=b"));
}

#[test]
fn line_without_equals_is_ignored_with_a_note() {
    let file = parse(b"[Desktop Entry]\nName\nName=Foo\n").unwrap();
    assert_eq!(file.groups[0].get("Name"), Some("Foo"));
    assert_eq!(file.notes.len(), 1);
}

#[test]
fn key_outside_any_group_is_ignored() {
    let file = parse(b"Name=Foo\n[Desktop Entry]\nName=Bar\n").unwrap();
    assert_eq!(file.groups[0].get("Name"), Some("Bar"));
    assert_eq!(file.notes.len(), 1);
}

#[test]
fn duplicate_keys_first_wins_with_a_note() {
    let file = parse(b"[Desktop Entry]\nName=First\nName=Second\n").unwrap();
    assert_eq!(file.groups[0].get("Name"), Some("First"));
    assert_eq!(file.notes.len(), 1);
    assert!(file.notes[0].contains("duplicate"), "{}", file.notes[0]);
}

#[test]
fn repeated_groups_merge_with_a_note() {
    let file = parse(b"[Desktop Entry]\nName=Foo\n[Desktop Entry]\nComment=Hi\n").unwrap();
    assert_eq!(file.groups.len(), 1);
    assert_eq!(file.groups[0].get("Name"), Some("Foo"));
    assert_eq!(file.groups[0].get("Comment"), Some("Hi"));
    assert!(file.notes.iter().any(|note| note.contains("repeated")));
}

#[test]
fn bad_key_names_are_ignored() {
    for bad in [
        "",
        "Na me",
        "Name!",
        "Name[de",
        "Name[]",
        "Name[a=b]",
        "=value",
    ] {
        let text = format!("[Desktop Entry]\n{bad}=x\nName=Foo\n");
        let file = parse(text.as_bytes()).unwrap();
        assert_eq!(file.groups[0].get("Name"), Some("Foo"), "for `{bad}`");
        assert!(
            file.groups[0].keys.iter().all(|(key, _)| key == "Name"),
            "for `{bad}`"
        );
    }
}

#[test]
fn locale_suffixed_keys_keep_their_suffix() {
    let file = parse("[Desktop Entry]\nName[de_DE]=Föö\nName=Foo\n".as_bytes()).unwrap();
    assert_eq!(file.groups[0].get("Name[de_DE]"), Some("Föö"));
    assert_eq!(file.groups[0].get("Name"), Some("Foo"));
}

#[test]
fn malformed_utf8_skips_the_file() {
    assert_eq!(
        parse(b"[Desktop Entry]\nName=\xff\xfe\n"),
        Err(Error::NotUtf8)
    );
}

#[test]
fn overlong_line_skips_the_file() {
    let mut text = String::from("[Desktop Entry]\nName=");
    text.push_str(&"x".repeat(MAX_LINE + 1));
    text.push('\n');
    assert_eq!(parse(text.as_bytes()), Err(Error::LineTooLong(2)));
}

#[test]
fn line_at_exactly_max_line_parses() {
    let mut text = String::from("[Desktop Entry]\nName=");
    text.push_str(&"x".repeat(MAX_LINE - 5));
    text.push('\n');
    // "Name=" is 5 bytes; the line is exactly MAX_LINE long.
    assert!(parse(text.as_bytes()).is_ok());
}

#[test]
fn too_many_groups_skip_the_file() {
    let mut text = String::from("[Desktop Entry]\nName=Foo\n");
    for index in 0..MAX_GROUPS {
        text.push_str(&format!("[Group{index}]\nKey=Value\n"));
    }
    assert_eq!(parse(text.as_bytes()), Err(Error::TooManyGroups));
}

#[test]
fn long_strings_are_cut_at_a_char_boundary() {
    let mut text = String::from("[Desktop Entry]\nName=");
    // Multi-byte characters straddling the 1 KiB cut.
    text.push_str(&"é".repeat(600));
    text.push('\n');
    let file = parse(text.as_bytes()).unwrap();
    let name = file.groups[0].get("Name").unwrap();
    assert!(name.len() <= MAX_STRING, "{}", name.len());
    assert!(
        name.len().is_multiple_of(2),
        "cut mid-character: {}",
        name.len()
    );
}

#[test]
fn escapes_apply_in_order() {
    assert_eq!(unescape(r"a\sb\nc\td\re\\f").unwrap(), "a b\nc\td\re\\f");
}

#[test]
fn unknown_escapes_refuse_the_key() {
    assert_eq!(unescape(r"a\qb"), Err('q'));
    assert_eq!(unescape(r"trailing\"), Err('\\'));
}

#[test]
fn lists_split_on_unescaped_semicolons() {
    assert_eq!(split_list("a;b;c;").unwrap(), vec!["a", "b", "c"]);
    assert_eq!(split_list("a;b;c").unwrap(), vec!["a", "b", "c"]);
    assert_eq!(split_list("").unwrap(), Vec::<String>::new());
}

#[test]
fn lists_keep_inner_empties_and_escape_items() {
    assert_eq!(split_list("a;;b;").unwrap(), vec!["a", "", "b"]);
    assert_eq!(split_list(r"a\;b;c").unwrap(), vec!["a;b", "c"]);
    assert_eq!(split_list(r"a\sb").unwrap(), vec!["a b"]);
}

#[test]
fn lists_refuse_bad_escapes_per_item() {
    assert!(split_list(r"a;b\qc").is_err());
}

#[test]
fn booleans_are_exact() {
    assert_eq!(parse_bool("true"), Some(true));
    assert_eq!(parse_bool("false"), Some(false));
    for bad in ["True", "TRUE", "1", "yes", " true", "true "] {
        assert_eq!(parse_bool(bad), None, "{bad}");
    }
}

#[test]
fn control_characters_are_dropped_from_display() {
    assert_eq!(clean_display("a\x07b\x1bc\u{7f}d"), "abcd");
    assert_eq!(clean_display("plain"), "plain");
}

#[test]
fn control_characters_in_headers_are_not_groups() {
    // A `[` line holding controls is an ignored line, not a group:
    // controls can never ride a group name into an action id.
    let file = parse(b"[\x07]\n[Desktop Entry]\nName=Foo\n").unwrap();
    assert_eq!(file.groups.len(), 1);
    assert_eq!(file.groups[0].name, "Desktop Entry");
    // A suffixed key holding controls is a bad key name.
    let file = parse("[Desktop Entry]\nName[de\x07]=x\nName=Foo\n".as_bytes()).unwrap();
    assert!(file.groups[0].keys.iter().all(|(key, _)| key == "Name"));
}

#[test]
fn crlf_files_parse() {
    let file = parse(b"[Desktop Entry]\r\nName=Foo\r\n").unwrap();
    assert_eq!(file.groups[0].get("Name"), Some("Foo"));
}

#[test]
fn group_header_needs_both_brackets() {
    // `[Desktop Entry` without `]` is a key-less line, ignored; the
    // file then has no group and is skipped.
    assert_eq!(
        parse(b"[Desktop Entry\nName=Foo\n"),
        Err(Error::NoDesktopEntry)
    );
    // An empty header is not a group either.
    assert_eq!(
        parse(b"[]\n[Desktop Entry]\nName=Foo\n")
            .unwrap()
            .groups
            .len(),
        1
    );
}
