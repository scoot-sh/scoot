//! The CLI's contract: every flag, the help text, and the exit-code
//! shape. A bare `daemon` owns the name; `--check` never touches a bus.

use super::{Args, Command, Msg, parse_args};
use crate::control::protocol::EventKind;

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
    assert!(matches!(args.command, Command::Daemon));
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
    let error = parse(&["scootnotify", "notify"]).unwrap_err();
    assert!(error.contains("notify"), "{error}");
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

#[test]
fn msg_list_defaults_to_human_and_json_is_a_flag() {
    assert_eq!(
        parse(&["scootnotify", "msg", "list"]).unwrap().command,
        Command::Msg(Msg::List { json: false })
    );
    assert_eq!(
        parse(&["scootnotify", "msg", "list", "--json"]).unwrap().command,
        Command::Msg(Msg::List { json: true })
    );
    assert!(parse(&["scootnotify", "msg", "list", "--all"]).is_err());
}

#[test]
fn msg_dismiss_takes_an_id_or_all_but_not_both_or_neither() {
    assert_eq!(
        parse(&["scootnotify", "msg", "dismiss", "3"]).unwrap().command,
        Command::Msg(Msg::Dismiss { id: Some(3), all: false })
    );
    assert_eq!(
        parse(&["scootnotify", "msg", "dismiss", "--all"]).unwrap().command,
        Command::Msg(Msg::Dismiss { id: None, all: true })
    );
    assert!(parse(&["scootnotify", "msg", "dismiss"]).is_err());
    assert!(parse(&["scootnotify", "msg", "dismiss", "0"]).is_err());
    assert!(parse(&["scootnotify", "msg", "dismiss", "-1"]).is_err());
    assert!(parse(&["scootnotify", "msg", "dismiss", "soon"]).is_err());
    assert!(parse(&["scootnotify", "msg", "dismiss", "3", "--all"]).is_err());
}

#[test]
fn msg_invoke_needs_an_id_and_defaults_its_action() {
    assert_eq!(
        parse(&["scootnotify", "msg", "invoke", "3"]).unwrap().command,
        Command::Msg(Msg::Invoke { id: 3, action: String::from("default") })
    );
    assert_eq!(
        parse(&["scootnotify", "msg", "invoke", "3", "open"]).unwrap().command,
        Command::Msg(Msg::Invoke { id: 3, action: String::from("open") })
    );
    assert!(parse(&["scootnotify", "msg", "invoke"]).is_err());
    assert!(parse(&["scootnotify", "msg", "invoke", "0"]).is_err());
    assert!(parse(&["scootnotify", "msg", "invoke", "3", "a", "b"]).is_err());
}

#[test]
fn msg_reload_takes_an_optional_look_scope() {
    assert_eq!(
        parse(&["scootnotify", "msg", "reload"]).unwrap().command,
        Command::Msg(Msg::Reload { look: false })
    );
    assert_eq!(
        parse(&["scootnotify", "msg", "reload", "--look"]).unwrap().command,
        Command::Msg(Msg::Reload { look: true })
    );
    assert!(parse(&["scootnotify", "msg", "reload", "--config"]).is_err());
}

#[test]
fn msg_subscribe_names_kinds_or_takes_them_all() {
    assert_eq!(
        parse(&["scootnotify", "msg", "subscribe"]).unwrap().command,
        Command::Msg(Msg::Subscribe { events: EventKind::ALL.to_vec() })
    );
    assert_eq!(
        parse(&["scootnotify", "msg", "subscribe", "close"]).unwrap().command,
        Command::Msg(Msg::Subscribe { events: vec![EventKind::Close] })
    );
    let error = parse(&["scootnotify", "msg", "subscribe", "dnd"]).unwrap_err();
    assert!(error.contains("dnd"), "{error}");
}

#[test]
fn msg_wait_filters_are_optional_and_the_timeout_has_a_default() {
    assert_eq!(
        parse(&["scootnotify", "msg", "wait"]).unwrap().command,
        Command::Msg(Msg::Wait { app: None, summary: None, timeout_secs: 30 })
    );
    assert_eq!(
        parse(&[
            "scootnotify", "msg", "wait",
            "--app", "Firefox",
            "--summary-contains", "complete",
            "--timeout", "5",
        ])
        .unwrap()
        .command,
        Command::Msg(Msg::Wait {
            app: Some(String::from("Firefox")),
            summary: Some(String::from("complete")),
            timeout_secs: 5,
        })
    );
    assert!(parse(&["scootnotify", "msg", "wait", "--timeout"]).is_err());
    assert!(parse(&["scootnotify", "msg", "wait", "--timeout", "soon"]).is_err());
    assert!(parse(&["scootnotify", "msg", "wait", "--app"]).is_err());
    assert!(parse(&["scootnotify", "msg", "wait", "--nearby"]).is_err());
}

#[test]
fn msg_version_takes_no_arguments() {
    assert_eq!(
        parse(&["scootnotify", "msg", "version"]).unwrap().command,
        Command::Msg(Msg::Version)
    );
    assert!(parse(&["scootnotify", "msg", "version", "1"]).is_err());
}

#[test]
fn msg_without_a_command_is_a_misuse() {
    let error = parse(&["scootnotify", "msg"]).unwrap_err();
    assert!(error.contains("list"), "{error}");
}

#[test]
fn an_unknown_msg_command_names_itself() {
    let error = parse(&["scootnotify", "msg", "history"]).unwrap_err();
    assert!(error.contains("history"), "{error}");
}
