use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use super::{Command, Error, Topic, USAGE, parse, version_string};
use crate::color::{Color, ColorError};
use crate::image::{Filter, Mode};
use crate::protocol::{ImageRequest, Request, Show};

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
        show: Show::Color(Color::parse(color).unwrap()),
        output: output.map(|o| o.to_owned().into()),
    }))
}

fn image(
    path: &str,
    mode: Mode,
    fill: &str,
    filter: Filter,
    output: Option<&str>,
) -> Result<Command, Error> {
    Ok(Command::Client(Request::Set {
        show: Show::Image(ImageRequest {
            path: path.to_owned().into(),
            mode,
            fill: Color::parse(fill).unwrap(),
            filter,
        }),
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
fn a_path_is_an_image_made_absolute() {
    let cwd = std::env::current_dir().unwrap();
    let here = |name: &str| cwd.join(name).into_os_string().into_string().unwrap();
    assert_eq!(
        args(&["set", "/tmp/x.png"]),
        image("/tmp/x.png", Mode::Fill, "#000000", Filter::Lanczos3, None)
    );
    // Relative paths are resolved here: the daemon's directory is not ours.
    assert_eq!(
        args(&["set", "hills.jpg"]),
        image(
            &here("hills.jpg"),
            Mode::Fill,
            "#000000",
            Filter::Lanczos3,
            None
        )
    );
    // A name starting with '#', given as the README says (`.` is dropped;
    // `..` is kept, since resolving it could change the file named).
    assert_eq!(
        args(&["set", "./#draft.png"]),
        image(
            &here("#draft.png"),
            Mode::Fill,
            "#000000",
            Filter::Lanczos3,
            None
        )
    );
    assert_eq!(
        args(&["set", "../up.png"]),
        image(
            &here("../up.png"),
            Mode::Fill,
            "#000000",
            Filter::Lanczos3,
            None
        )
    );
    // Words that are not colors are paths too.
    assert_eq!(
        args(&["set", "red"]),
        image(&here("red"), Mode::Fill, "#000000", Filter::Lanczos3, None)
    );
    assert_eq!(
        args(&[
            "set",
            "/p/a.webp",
            "--mode",
            "fit",
            "--fill",
            "#101014",
            "--filter",
            "nearest",
            "--output",
            "DP-1"
        ]),
        image(
            "/p/a.webp",
            Mode::Fit,
            "#101014",
            Filter::Nearest,
            Some("DP-1")
        )
    );
    assert_eq!(
        args(&[
            "set",
            "--mode=tile",
            "--filter=catmull-rom",
            "--fill=#ABCDEF",
            "/p/a.png"
        ]),
        image("/p/a.png", Mode::Tile, "#abcdef", Filter::CatmullRom, None)
    );
    for mode in Mode::ALL {
        assert_eq!(
            args(&["set", "/a", "--mode", mode.name()]),
            image("/a", mode, "#000000", Filter::Lanczos3, None)
        );
    }
}

#[test]
fn image_flags_are_checked() {
    assert_eq!(
        args(&["set", "/a", "--mode", "cover"]),
        Err(Error::BadValue {
            flag: "--mode",
            value: "cover".into()
        })
    );
    assert_eq!(
        args(&["set", "/a", "--filter", "lanczos"]),
        Err(Error::BadValue {
            flag: "--filter",
            value: "lanczos".into()
        })
    );
    assert_eq!(
        args(&["set", "/a", "--fill", "black"]),
        Err(Error::Color {
            argument: "black".into(),
            error: ColorError::NotAColor
        })
    );
    assert_eq!(
        args(&["set", "/a", "--mode", "fit", "--mode", "fill"]),
        Err(Error::Repeated {
            command: "set",
            flag: "--mode"
        })
    );
    assert_eq!(
        args(&["set", "/a", "--fill"]),
        Err(Error::MissingValue {
            command: "set",
            flag: "--fill"
        })
    );
    // With a color they mean nothing: refused rather than ignored.
    for flag in ["--mode", "--fill", "--filter"] {
        assert_eq!(
            args(&["set", "#000000", flag, "x"]),
            Err(Error::ImageOnly(flag))
        );
    }
    // And `clear` takes none of them.
    assert!(matches!(
        args(&["clear", "--mode", "fit"]),
        Err(Error::Unexpected { .. })
    ));
    let message = Error::BadValue {
        flag: "--mode",
        value: "x".into(),
    }
    .to_string();
    assert!(
        message.contains("fill, fit, stretch, center or tile"),
        "{message}"
    );
}

#[test]
fn a_non_utf8_path_says_why() {
    let bad = OsString::from_vec(b"/pics/\xff.png".to_vec());
    match parse([OsString::from("set"), bad]) {
        Err(error @ Error::NotUtf8(_)) => assert!(error.to_string().contains("UTF-8"), "{error}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_malformed_color_is_a_usage_error() {
    for bad in ["#fff", "#c03020ff", "#c0302g", "#c03020 ", "#"] {
        assert_eq!(
            args(&["set", bad]),
            Err(Error::Color {
                argument: bad.to_owned(),
                error: ColorError::Malformed
            }),
            "{bad:?}"
        );
    }
}

#[test]
fn set_and_clear_refuse_what_they_do_not_take() {
    assert_eq!(args(&["set"]), Err(Error::MissingTarget));
    assert_eq!(
        args(&["set", "--output", "DP-1"]),
        Err(Error::MissingTarget)
    );
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
        (&["set", "/a.png", "/b.png"], "/b.png"),
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
