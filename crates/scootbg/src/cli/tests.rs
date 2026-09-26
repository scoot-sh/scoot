use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use super::{Command, Error, Topic, USAGE, parse, version_string};
use crate::protocol::Request;

fn args(list: &[&str]) -> Result<Command, Error> {
    parse(list.iter().map(OsString::from))
}

#[test]
fn each_command_parses() {
    assert_eq!(args(&["daemon"]), Ok(Command::Daemon));
    assert_eq!(args(&["query"]), Ok(Command::Client(Request::Query)));
    assert_eq!(args(&["kill"]), Ok(Command::Client(Request::Kill)));
    assert_eq!(args(&["version"]), Ok(Command::Client(Request::Version)));
    assert_eq!(args(&["--version"]), Ok(Command::Version));
    assert_eq!(args(&["-V"]), Ok(Command::Version));
}

#[test]
fn help_is_there_for_everything() {
    for help in [&["--help"][..], &["-h"], &["help"]] {
        assert_eq!(args(help), Ok(Command::Help(Topic::Main)));
    }
    for (name, topic) in [
        ("daemon", Topic::Daemon),
        ("query", Topic::Query),
        ("version", Topic::Version),
        ("kill", Topic::Kill),
    ] {
        assert_eq!(args(&[name, "--help"]), Ok(Command::Help(topic)));
        assert_eq!(args(&[name, "-h"]), Ok(Command::Help(topic)));
        assert_eq!(args(&["help", name]), Ok(Command::Help(topic)));
        assert!(topic.text().starts_with(&format!("scootbg {name} -- ")));
    }
}

#[test]
fn the_main_help_lists_only_what_works() {
    for listed in ["daemon", "query", "version", "kill", "--version", "--help"] {
        assert!(USAGE.contains(listed), "{listed} missing");
    }
    for later in ["set", "clear", "apply-config"] {
        assert!(
            !USAGE.split_whitespace().any(|word| word == later),
            "{later} is advertised before it exists"
        );
    }
}

#[test]
fn nothing_is_an_error() {
    assert_eq!(args(&[]), Err(Error::Missing));
}

#[test]
fn unknown_commands_are_errors() {
    for unknown in ["set", "clear", "apply-config", "--daemon", "", "Query"] {
        assert_eq!(args(&[unknown]), Err(Error::Unknown(unknown.to_owned())));
    }
    assert_eq!(
        args(&["help", "set"]),
        Err(Error::Unknown("set".to_owned()))
    );
}

#[test]
fn extra_arguments_are_errors() {
    assert_eq!(
        args(&["query", "DP-1"]),
        Err(Error::Unexpected {
            command: "query",
            argument: "DP-1".to_owned()
        })
    );
    assert_eq!(
        args(&["daemon", "--help", "x"]),
        Err(Error::Unexpected {
            command: "daemon",
            argument: "x".to_owned()
        })
    );
    assert_eq!(
        args(&["--version", "--help"]),
        Err(Error::Unexpected {
            command: "--version",
            argument: "--help".to_owned()
        })
    );
    assert!(matches!(
        args(&["help", "kill", "extra"]),
        Err(Error::Unexpected {
            command: "help",
            ..
        })
    ));
    let message = args(&["kill", "now"]).unwrap_err().to_string();
    assert!(message.contains("scootbg kill --help"), "{message}");
}

#[test]
fn a_non_utf8_argument_is_an_error_not_a_panic() {
    let bad = OsString::from_vec(b"qu\xffery".to_vec());
    assert!(matches!(parse([bad.clone()]), Err(Error::Unknown(_))));
    assert!(matches!(
        parse([OsString::from("query"), bad.clone()]),
        Err(Error::Unexpected { .. })
    ));
    assert!(matches!(
        parse([OsString::from("help"), bad]),
        Err(Error::Unknown(_))
    ));
}

#[test]
fn the_version_line_names_the_protocol() {
    let line = version_string();
    assert!(line.starts_with("scootbg "));
    assert!(line.contains(env!("CARGO_PKG_VERSION")));
    assert!(line.ends_with("(protocol 1)"));
}
