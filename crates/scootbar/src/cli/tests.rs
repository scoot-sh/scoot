use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use super::{Command, DAEMON_HELP, Error, ModulesError, Topic, USAGE, parse, version_string};
use crate::bar::{Bar, Edge, MAX_HEIGHT, Margin, MarginError};
use crate::color::{Color, ColorError};
use crate::config::{Config, MAX_FONT_SIZE};
use crate::layout::{Layout, MAX_GAP};

fn run(args: &[&str]) -> Result<Command, Error> {
    parse(args.iter().map(OsString::from))
}

fn config(args: &[&str]) -> Config {
    let mut all = vec!["daemon"];
    all.extend_from_slice(args);
    match run(&all) {
        Ok(Command::Daemon(config)) => *config,
        other => panic!("{args:?}: {other:?}"),
    }
}

fn daemon(args: &[&str]) -> Bar {
    config(args).bar
}

#[test]
fn plain_daemon_is_the_default_bar() {
    assert_eq!(config(&[]), Config::default());
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
    };
    assert_eq!(
        daemon(&["--edge", "bottom", "--height", "40", "--margin", "8,4",]),
        bar
    );
    assert_eq!(
        daemon(&["--margin=8,4", "--height=40", "--edge=bottom"]),
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
    for flag in super::FLAGS {
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
            flag: "--background",
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

#[test]
fn colors_and_text() {
    let config = config(&[
        "--background",
        "#102030",
        "--foreground=#AABBCC",
        "--font",
        "/some/font.ttf",
        "--font-size",
        "20",
    ]);
    assert_eq!(
        config.theme.background,
        Color {
            r: 0x10,
            g: 0x20,
            b: 0x30
        }
    );
    assert_eq!(
        config.theme.foreground,
        Color {
            r: 0xaa,
            g: 0xbb,
            b: 0xcc
        }
    );
    assert_eq!(
        config.font.as_deref(),
        Some(std::path::Path::new("/some/font.ttf"))
    );
    assert_eq!(config.font_size, 20);
    assert!(matches!(
        run(&["daemon", "--foreground", "red"]),
        Err(Error::Color {
            flag: "--foreground",
            ..
        })
    ));
    for bad in ["0", &(MAX_FONT_SIZE + 1).to_string(), "1.5", "+3", ""] {
        assert_eq!(
            run(&["daemon", "--font-size", bad]),
            Err(Error::FontSize(bad.to_owned()))
        );
    }
    assert_eq!(config_font_size_max(), MAX_FONT_SIZE);
}

fn config_font_size_max() -> u32 {
    config(&["--font-size", &MAX_FONT_SIZE.to_string()]).font_size
}

/// A font path is any bytes: a file name need not be UTF-8.
#[test]
fn a_font_path_need_not_be_utf8() {
    let raw = OsString::from_vec(vec![b'/', 0xff]);
    let parsed = parse([
        OsString::from("daemon"),
        OsString::from("--font"),
        raw.clone(),
    ]);
    match parsed {
        Ok(Command::Daemon(config)) => {
            assert_eq!(config.font.unwrap().into_os_string(), raw);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_layout_is_what_is_listed() {
    // The default: the clock in the center.
    let default = config(&[]).layout;
    assert_eq!(default, Layout::default());
    #[cfg(feature = "clock")]
    assert_eq!(default.center, ["clock"]);
    #[cfg(feature = "clock")]
    {
        // Any section given sets the whole layout.
        let right = config(&["--right", "clock"]).layout;
        assert_eq!(right.right, ["clock"]);
        assert!(right.center.is_empty() && right.left.is_empty());
        // Empty is no modules at all.
        assert!(config(&["--center", ""]).layout.is_empty());
        assert_eq!(
            run(&["daemon", "--left", "clock", "--right", "clock"]),
            Err(Error::Modules {
                flag: "--right",
                error: ModulesError::Twice("clock")
            })
        );
        assert_eq!(
            run(&["daemon", "--center", "clock,clock"]),
            Err(Error::Modules {
                flag: "--center",
                error: ModulesError::Twice("clock")
            })
        );
    }
    let unknown = run(&["daemon", "--left", "battery"]);
    assert_eq!(
        unknown,
        Err(Error::Modules {
            flag: "--left",
            error: ModulesError::Unknown("battery".into())
        })
    );
    let message = unknown.unwrap_err().to_string();
    assert!(message.contains("no module `battery`"), "{message}");
    assert!(matches!(
        run(&["daemon", "--left", "clock,"]),
        Err(Error::Modules { .. })
    ));
}

#[test]
fn padding_and_spacing() {
    let layout = config(&["--padding", "0", "--spacing", "12"]).layout;
    assert_eq!((layout.padding, layout.spacing), (0, 12));
    assert_eq!(
        run(&["daemon", "--padding", &(MAX_GAP + 1).to_string()]),
        Err(Error::Gap {
            flag: "--padding",
            value: (MAX_GAP + 1).to_string()
        })
    );
    assert!(matches!(
        run(&["daemon", "--spacing", "-1"]),
        Err(Error::Gap {
            flag: "--spacing",
            ..
        })
    ));
}

#[cfg(feature = "clock")]
#[test]
fn the_clock_format_is_checked_when_read() {
    use crate::modules::clock::format::{Error as FormatError, Format};
    let clock = config(&["--clock-format", "%H:%M"]).modules.clock;
    assert_eq!(clock.format, Format::parse("%H:%M").unwrap());
    assert_eq!(
        run(&["daemon", "--clock-format", "%Q"]),
        Err(Error::ClockFormat {
            value: "%Q".into(),
            error: FormatError::Unknown('Q')
        })
    );
    let message = run(&["daemon", "--clock-format", "a\nb"])
        .unwrap_err()
        .to_string();
    assert!(message.contains("control character"), "{message}");
    assert!(message.contains("\\n"), "the newline is escaped: {message}");
}

/// The help documents what this build has, and nothing it lacks.
#[test]
fn the_help_matches_the_build() {
    let has_clock = crate::modules::find("clock").is_some();
    assert_eq!(DAEMON_HELP.contains("--clock-format"), has_clock);
    assert_eq!(DAEMON_HELP.contains("Modules: clock"), has_clock);
    assert_eq!(USAGE.contains("with a clock"), has_clock);
    for flag in super::FLAGS {
        assert!(DAEMON_HELP.contains(flag), "{flag} is not documented");
    }
    assert!(!DAEMON_HELP.contains("wakes once a minute"));
}
