use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use super::{Command, Error, Topic, USAGE, parse, version_string};
use crate::color::{Color, ColorError};
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
        ("set", Topic::Set),
        ("clear", Topic::Clear),
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
    for listed in [
        "daemon",
        "set",
        "clear",
        "query",
        "version",
        "kill",
        "--version",
        "--help",
    ] {
        assert!(
            USAGE.split_whitespace().any(|word| word == listed),
            "{listed} missing"
        );
    }
    for later in ["apply-config", "--mode", "--fill", "--profile"] {
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
    for unknown in ["apply-config", "--daemon", "", "Query", "Set"] {
        assert_eq!(args(&[unknown]), Err(Error::Unknown(unknown.to_owned())));
    }
    assert_eq!(
        args(&["help", "apply-config"]),
        Err(Error::Unknown("apply-config".to_owned()))
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

fn set(color: &str, output: Option<&str>) -> Result<Command, Error> {
    Ok(Command::Client(Request::Set {
        color: Color::parse(color).unwrap(),
        output: output.map(|o| o.to_owned().into()),
    }))
}

fn clear(output: Option<&str>) -> Result<Command, Error> {
    Ok(Command::Client(Request::Clear {
        output: output.map(|o| o.to_owned().into()),
    }))
}

#[test]
fn set_takes_a_color_and_an_optional_output_in_any_order() {
    assert_eq!(args(&["set", "#c03020"]), set("#c03020", None));
    assert_eq!(args(&["set", "#C03020"]), set("#c03020", None));
    assert_eq!(
        args(&["set", "#c03020", "--output", "DP-1"]),
        set("#c03020", Some("DP-1"))
    );
    assert_eq!(
        args(&["set", "--output", "DP-1", "#c03020"]),
        set("#c03020", Some("DP-1"))
    );
    assert_eq!(
        args(&["set", "--output=DP-1", "#c03020"]),
        set("#c03020", Some("DP-1"))
    );
    // A name may look like anything, even a flag or a color.
    assert_eq!(
        args(&["set", "#c03020", "--output", "--help"]),
        set("#c03020", Some("--help"))
    );
    assert_eq!(
        args(&["set", "#c03020", "--output", "#000000"]),
        set("#c03020", Some("#000000"))
    );
}

#[test]
fn clear_takes_an_optional_output() {
    assert_eq!(args(&["clear"]), clear(None));
    assert_eq!(
        args(&["clear", "--output", "HDMI-A-1"]),
        clear(Some("HDMI-A-1"))
    );
    assert_eq!(args(&["clear", "--output="]), clear(Some("")));
}

#[test]
fn a_path_is_refused_until_images_exist() {
    for path in [
        "~/Pictures/hills.jpg",
        "./#draft.png",
        "/tmp/x.png",
        "c03020",
        "red",
    ] {
        let error = args(&["set", path]).unwrap_err();
        assert_eq!(
            error,
            Error::Color {
                argument: path.to_owned(),
                error: ColorError::NotAColor
            }
        );
        assert!(error.to_string().contains("later version"), "{error}");
    }
}

#[test]
fn a_malformed_color_is_a_usage_error() {
    for bad in ["#fff", "#c03020ff", "#c0302g", "#c03020 ", " #c03020", "#"] {
        let expected = if bad.starts_with('#') {
            ColorError::Malformed
        } else {
            ColorError::NotAColor
        };
        assert_eq!(
            args(&["set", bad]),
            Err(Error::Color {
                argument: bad.to_owned(),
                error: expected
            }),
            "{bad:?}"
        );
    }
}

#[test]
fn set_and_clear_refuse_what_they_do_not_take() {
    assert_eq!(args(&["set"]), Err(Error::MissingColor));
    assert_eq!(args(&["set", "--output", "DP-1"]), Err(Error::MissingColor));
    assert_eq!(
        args(&["set", "#c03020", "--output"]),
        Err(Error::MissingValue {
            command: "set",
            flag: "--output"
        })
    );
    assert_eq!(
        args(&["clear", "--output", "A", "--output", "B"]),
        Err(Error::Repeated {
            command: "clear",
            flag: "--output"
        })
    );
    for (list, extra) in [
        (&["set", "#c03020", "#101014"][..], "#101014"),
        (&["set", "#c03020", "--mode", "fit"], "--mode"),
        (&["set", "-x"], "-x"),
        (&["set", "#c03020", "--help"], "--help"),
        (&["clear", "#c03020"], "#c03020"),
        (&["clear", "--help", "x"], "x"),
    ] {
        match args(list) {
            Err(Error::Unexpected { argument, .. }) => assert_eq!(argument, extra, "{list:?}"),
            other => panic!("{list:?}: {other:?}"),
        }
    }
    let bad = OsString::from_vec(b"DP-\xff".to_vec());
    assert!(matches!(
        parse([OsString::from("clear"), OsString::from("--output"), bad]),
        Err(Error::Unexpected { .. })
    ));
}
