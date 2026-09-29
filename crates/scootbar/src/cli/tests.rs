use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use super::{Command, DAEMON_HELP, Error, Topic, USAGE, parse, version_string};
use crate::bar::{Bar, Edge, MAX_HEIGHT, Margin, MarginError};
use crate::color::{Color, ColorError};

fn run(args: &[&str]) -> Result<Command, Error> {
    parse(args.iter().map(OsString::from))
}

fn daemon(args: &[&str]) -> Bar {
    let mut all = vec!["daemon"];
    all.extend_from_slice(args);
    match run(&all) {
        Ok(Command::Daemon(bar)) => bar,
        other => panic!("{args:?}: {other:?}"),
    }
}

#[test]
fn plain_daemon_is_the_default_bar() {
    assert_eq!(daemon(&[]), Bar::default());
}

#[test]
fn every_flag_takes_its_value_either_way() {
    let bar = Bar {
        edge: Edge::Bottom,
        height: 40,
        margin: Margin {
            top: 8,
            right: 4,
            bottom: 8,
            left: 4,
        },
        background: Color {
            r: 0x10,
            g: 0x20,
            b: 0x30,
        },
    };
    assert_eq!(
        daemon(&[
            "--edge",
            "bottom",
            "--height",
            "40",
            "--margin",
            "8,4",
            "--background",
            "#102030"
        ]),
        bar
    );
    assert_eq!(
        daemon(&[
            "--background=#102030",
            "--margin=8,4",
            "--height=40",
            "--edge=bottom"
        ]),
        bar
    );
}

#[test]
fn help_and_version() {
    assert_eq!(run(&["--help"]), Ok(Command::Help(Topic::Main)));
    assert_eq!(run(&["-h"]), Ok(Command::Help(Topic::Main)));
    assert_eq!(run(&["help"]), Ok(Command::Help(Topic::Main)));
    assert_eq!(run(&["help", "daemon"]), Ok(Command::Help(Topic::Daemon)));
    assert_eq!(run(&["daemon", "--help"]), Ok(Command::Help(Topic::Daemon)));
    assert_eq!(run(&["daemon", "-h"]), Ok(Command::Help(Topic::Daemon)));
    assert_eq!(run(&["--version"]), Ok(Command::Version));
    assert_eq!(run(&["-V"]), Ok(Command::Version));
    assert!(version_string().starts_with("scootbar "));
    assert!(USAGE.contains("scootbar daemon"));
    for flag in ["--edge", "--height", "--margin", "--background"] {
        assert!(DAEMON_HELP.contains(flag), "{flag} is not documented");
    }
}

#[test]
fn usage_errors() {
    assert_eq!(run(&[]), Err(Error::Missing));
    assert_eq!(run(&["bar"]), Err(Error::Unknown("bar".into())));
    assert_eq!(run(&["help", "bar"]), Err(Error::Unknown("bar".into())));
    assert!(matches!(
        run(&["help", "daemon", "x"]),
        Err(Error::Unexpected { .. })
    ));
    assert!(matches!(
        run(&["--version", "x"]),
        Err(Error::Unexpected { .. })
    ));
    assert!(matches!(
        run(&["daemon", "--help", "x"]),
        Err(Error::Unexpected { .. })
    ));
    // `--help` after a flag is not a help request: it is refused, rather
    // than silently dropping what came before.
    assert!(matches!(
        run(&["daemon", "--height", "30", "--help"]),
        Err(Error::Unexpected { .. })
    ));
    assert!(matches!(
        run(&["daemon", "--width", "30"]),
        Err(Error::Unexpected { .. })
    ));
    assert!(matches!(
        run(&["daemon", "stray"]),
        Err(Error::Unexpected { .. })
    ));
    // `=` only splits a flag, never a stray argument.
    assert!(matches!(
        run(&["daemon", "a=b"]),
        Err(Error::Unexpected { .. })
    ));
}

#[test]
fn a_flag_needs_its_value_once() {
    assert_eq!(
        run(&["daemon", "--height"]),
        Err(Error::MissingValue("--height"))
    );
    assert_eq!(
        run(&["daemon", "--edge", "top", "--edge", "bottom"]),
        Err(Error::Repeated("--edge"))
    );
    assert_eq!(
        run(&["daemon", "--height", "30", "--height", "x"]),
        Err(Error::Repeated("--height"))
    );
}

#[test]
fn bad_values_say_what_they_take() {
    assert_eq!(
        run(&["daemon", "--edge", "left"]),
        Err(Error::Edge("left".into()))
    );
    assert_eq!(
        run(&["daemon", "--height", "0"]),
        Err(Error::Height("0".into()))
    );
    assert_eq!(
        run(&["daemon", "--height", &(MAX_HEIGHT + 1).to_string()]),
        Err(Error::Height((MAX_HEIGHT + 1).to_string()))
    );
    assert_eq!(
        run(&["daemon", "--height="]),
        Err(Error::Height(String::new()))
    );
    assert_eq!(
        run(&["daemon", "--margin", "1,2,3,4,5"]),
        Err(Error::Margin {
            value: "1,2,3,4,5".into(),
            error: MarginError::Malformed
        })
    );
    assert_eq!(
        run(&["daemon", "--background", "1e1e2e"]),
        Err(Error::Color {
            value: "1e1e2e".into(),
            error: ColorError
        })
    );
    let message = run(&["daemon", "--height", "x"]).unwrap_err().to_string();
    assert!(message.contains("1 to 1024"), "{message}");
}

#[test]
fn non_utf8_arguments_are_errors_not_panics() {
    let bad = OsString::from_vec(vec![b'd', 0xff]);
    assert!(matches!(parse([bad.clone()]), Err(Error::Unknown(_))));
    assert!(matches!(
        parse([OsString::from("daemon"), bad.clone()]),
        Err(Error::Unexpected { .. })
    ));
    assert!(matches!(
        parse([OsString::from("daemon"), OsString::from("--height"), bad]),
        Err(Error::Height(_))
    ));
}
