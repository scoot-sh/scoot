//! The command line's tests. Env-free: every case is pure argument
//! parsing, so no `serial_test`-style exclusion is needed.

use super::*;

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

#[test]
fn list_alone_parses() {
    let args = parse_args(&argv(&["scootlaunch", "--list"])).unwrap();
    assert_eq!(
        args,
        Args {
            list: true,
            all: false,
            lenient: false,
            json: false,
            help: false,
            version: false,
            query: None,
        }
    );
}

#[test]
fn query_takes_the_next_argument_as_text() {
    let args = parse_args(&argv(&["scootlaunch", "--query", "fire"])).unwrap();
    assert_eq!(args.query, Some("fire".to_string()));
    assert!(!args.list);
}

#[test]
fn query_takes_every_modifier() {
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--query",
        "fire",
        "--all",
        "--lenient",
        "--json",
    ]))
    .unwrap();
    assert_eq!(args.query, Some("fire".to_string()));
    assert!(args.all && args.lenient && args.json);
}

#[test]
fn query_text_may_look_like_a_flag() {
    // TEXT is the next argument, whatever it holds.
    let args = parse_args(&argv(&["scootlaunch", "--query", "--json"])).unwrap();
    assert_eq!(args.query, Some("--json".to_string()));
    assert!(!args.json);
}

#[test]
fn query_without_text_is_a_loud_error() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--query"])),
        Err(Error::NeedsValue("--query".to_string()))
    );
}

#[test]
fn list_and_query_conflict() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--list", "--query", "fire"])),
        Err(Error::ConflictingCommands)
    );
}

#[test]
fn list_with_every_modifier_parses() {
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--list",
        "--all",
        "--lenient",
        "--json",
    ]))
    .unwrap();
    assert!(args.list && args.all && args.lenient && args.json);
}

#[test]
fn bare_invocation_is_no_command() {
    assert_eq!(parse_args(&argv(&["scootlaunch"])), Err(Error::NoCommand));
}

#[test]
fn unknown_flags_name_themselves_and_the_supported_list() {
    let error = parse_args(&argv(&["scootlaunch", "--dmenu"])).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("--dmenu"), "{text}");
    assert!(text.contains("--list"), "{text}");
    assert!(text.contains("--json"), "{text}");
    // A dmenu-mode flag today is unknown: this ticket builds no dmenu.
    assert_eq!(
        error,
        Error::UnknownFlag("--dmenu".to_string()),
        "loud, never silently ignored"
    );
}

#[test]
fn single_dash_unknown_is_unknown() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "-x"])),
        Err(Error::UnknownFlag("-x".to_string()))
    );
}

#[test]
fn flag_with_value_is_not_a_flag() {
    // `--json=true` must not parse as `--json`: a misparsed flag that
    // takes a value would eat the next argument.
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--list", "--json=true"])),
        Err(Error::UnknownFlag("--json=true".to_string()))
    );
}

#[test]
fn positional_arguments_are_refused() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--list", "firefox"])),
        Err(Error::UnexpectedArg("firefox".to_string()))
    );
}

#[test]
fn modifiers_need_a_command() {
    for flag in ["--all", "--lenient", "--json"] {
        assert_eq!(
            parse_args(&argv(&["scootlaunch", flag])),
            Err(Error::NeedsCommand(flag.to_string())),
            "{flag} without --list or --query"
        );
    }
    // ... but either command satisfies them.
    assert!(parse_args(&argv(&["scootlaunch", "--query", "x", "--json"])).is_ok());
}

#[test]
fn help_and_version_win_over_usage_errors() {
    assert!(
        parse_args(&argv(&["scootlaunch", "--json", "--help"]))
            .unwrap()
            .help
    );
    assert!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--version"]))
            .unwrap()
            .version
    );
}

#[test]
fn short_help_and_version_parse() {
    assert!(parse_args(&argv(&["scootlaunch", "-h"])).unwrap().help);
    assert!(parse_args(&argv(&["scootlaunch", "-V"])).unwrap().version);
}

#[test]
fn repeated_flags_are_idempotent() {
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--list",
        "--list",
        "--json",
        "--json",
    ]))
    .unwrap();
    assert!(args.list && args.json && !args.all);
}
