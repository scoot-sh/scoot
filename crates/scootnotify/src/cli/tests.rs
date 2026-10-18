//! The CLI's contract: every flag, the help text, and the exit-code
//! shape. A bare `daemon` owns the name; `--check` never touches a bus.

use super::{Args, parse_args};

/// Parses `words` (with `words[0]` the binary name) the way `main` feeds
/// `argv`.
fn parse(words: &[&str]) -> Result<Args, String> {
    parse_args(
        &words
            .iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn daemon_with_no_flags_owns_the_name() {
    let args = parse(&["scootnotify", "daemon"]).unwrap();
    assert!(!args.check && args.config.is_none() && !args.help && !args.version);
}

#[test]
fn check_and_config_combine() {
    let args = parse(&["scootnotify", "daemon", "--check", "--config", "/x/y.toml"]).unwrap();
    assert!(args.check);
    assert_eq!(args.config.as_deref(), Some("/x/y.toml"));
}

#[test]
fn a_missing_command_is_a_misuse() {
    assert!(parse(&["scootnotify"]).is_err());
}

#[test]
fn an_unknown_command_names_itself() {
    let error = parse(&["scootnotify", "msg"]).unwrap_err();
    assert!(error.contains("msg"), "{error}");
}

#[test]
fn an_unknown_flag_names_itself() {
    let error = parse(&["scootnotify", "daemon", "--verbose"]).unwrap_err();
    assert!(error.contains("--verbose"), "{error}");
}

#[test]
fn check_without_a_value_is_a_misuse() {
    assert!(parse(&["scootnotify", "daemon", "--config"]).is_err());
}

#[test]
fn a_config_value_may_look_like_a_flag() {
    let args = parse(&["scootnotify", "daemon", "--config", "--check"]).unwrap();
    assert_eq!(args.config.as_deref(), Some("--check"));
}

#[test]
fn help_and_version_win_over_usage_errors() {
    assert!(parse(&["scootnotify", "bogus", "--help"]).unwrap().help);
    assert!(
        parse(&["scootnotify", "daemon", "--bogus", "--version"])
            .unwrap()
            .version
    );
}
