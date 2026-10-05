use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use super::{ApplyOptions, Command, DaemonOptions, Error, Topic, USAGE, parse, version_string};
use crate::color::{Color, ColorError};
use crate::image::{Filter, Mode};
use crate::protocol::{ImageRequest, Request, Show, Source};
use crate::section::Section;
use crate::state::Profile;

fn args(list: &[&str]) -> Result<Command, Error> {
    parse(list.iter().map(OsString::from))
}

#[test]
fn each_command_parses() {
    assert_eq!(
        args(&["daemon"]),
        Ok(Command::Daemon(DaemonOptions::default()))
    );
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
        ("apply-config", Topic::ApplyConfig),
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
        "apply-config",
        "--version",
        "--help",
    ] {
        assert!(
            USAGE.split_whitespace().any(|word| word == listed),
            "{listed} missing"
        );
    }
    for later in ["--serve", "--mode", "--fill", "--profile"] {
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
    for unknown in ["apply", "--daemon", "", "Query", "Set", "--serve"] {
        assert_eq!(args(&[unknown]), Err(Error::Unknown(unknown.to_owned())));
    }
    assert_eq!(
        args(&["help", "apply"]),
        Err(Error::Unknown("apply".to_owned()))
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
            source: Source::Path(path.to_owned().into()),
            mode,
            fill: Color::parse(fill).unwrap(),
            filter,
        }),
        output: output.map(|o| o.to_owned().into()),
    }))
}

fn download(
    url: &str,
    sha256: Option<[u8; 32]>,
    mode: Mode,
    fill: &str,
    filter: Filter,
    output: Option<&str>,
) -> Result<Command, Error> {
    Ok(Command::Client(Request::Set {
        show: Show::Image(ImageRequest {
            source: Source::Url {
                url: url.to_owned().into(),
                sha256,
            },
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
fn a_url_is_a_download_not_a_path() {
    let sha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    let pinned = crate::sha256::digest(b"test");
    // Untouched by the working directory, with the other flags as usual.
    assert_eq!(
        args(&["set", "https://example.com/a.png"]),
        download(
            "https://example.com/a.png",
            None,
            Mode::Fill,
            "#000000",
            Filter::Lanczos3,
            None
        )
    );
    assert_eq!(
        args(&[
            "set",
            "http://127.0.0.1:1/a.png",
            "--mode",
            "fit",
            "--sha256",
            sha,
            "--output",
            "DP-1"
        ]),
        download(
            "http://127.0.0.1:1/a.png",
            Some(pinned),
            Mode::Fit,
            "#000000",
            Filter::Lanczos3,
            Some("DP-1")
        )
    );
    // A hash that is not one, or with a color or a file, is a usage
    // error here, not a daemon round trip.
    assert_eq!(
        args(&["set", "https://example.com/a.png", "--sha256", "zz"]),
        Err(Error::BadSha("zz".into()))
    );
    assert_eq!(
        args(&["set", "#000000", "--sha256", sha]),
        Err(Error::ImageOnly("--sha256"))
    );
    assert_eq!(
        args(&["set", "/a.png", "--sha256", sha]),
        Err(Error::ShaImageOnly)
    );
    assert_eq!(
        args(&["clear", "--sha256", sha]),
        Err(Error::Unexpected {
            command: "clear",
            argument: "--sha256".into(),
        })
    );
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

#[test]
fn daemon_takes_a_profile_and_no_restore() {
    use crate::state::Profile;
    let options = |profile: &str, restore: bool| {
        Ok(Command::Daemon(DaemonOptions {
            profile: Profile::parse(profile).unwrap(),
            restore,
        }))
    };
    assert_eq!(args(&["daemon"]), options("default", true));
    assert_eq!(
        args(&["daemon", "--profile", "scoot"]),
        options("scoot", true)
    );
    assert_eq!(args(&["daemon", "--profile=scoot"]), options("scoot", true));
    assert_eq!(args(&["daemon", "--no-restore"]), options("default", false));
    assert_eq!(
        args(&["daemon", "--no-restore", "--profile", "sway"]),
        options("sway", false)
    );
    assert_eq!(
        args(&["daemon", "--profile=a", "--no-restore"]),
        options("a", false)
    );
    // `--help` first asks for help; later it is just unexpected.
    assert_eq!(
        args(&["daemon", "--help"]),
        Ok(Command::Help(Topic::Daemon))
    );
    assert!(matches!(
        args(&["daemon", "--no-restore", "--help"]),
        Err(Error::Unexpected { .. })
    ));
}

#[test]
fn daemon_flags_are_checked() {
    use crate::state::ProfileError;
    assert_eq!(
        args(&["daemon", "--profile"]),
        Err(Error::MissingValue {
            command: "daemon",
            flag: "--profile"
        })
    );
    for (name, error) in [
        ("", ProfileError::Empty),
        ("a/b", ProfileError::Byte('/')),
        ("..", ProfileError::Dots),
        (".x", ProfileError::Dots),
    ] {
        assert_eq!(
            args(&["daemon", "--profile", name]),
            Err(Error::Profile(error.clone())),
            "{name:?}"
        );
        assert_eq!(
            args(&["daemon", &format!("--profile={name}")]),
            Err(Error::Profile(error)),
            "{name:?}"
        );
    }
    assert_eq!(
        args(&["daemon", "--profile", "a", "--profile", "b"]),
        Err(Error::Repeated {
            command: "daemon",
            flag: "--profile"
        })
    );
    assert_eq!(
        args(&["daemon", "--no-restore", "--no-restore"]),
        Err(Error::Repeated {
            command: "daemon",
            flag: "--no-restore"
        })
    );
    for extra in ["--output", "restore", "--profiles=x", "-p"] {
        assert_eq!(
            args(&["daemon", extra]),
            Err(Error::Unexpected {
                command: "daemon",
                argument: extra.to_owned()
            })
        );
    }
    let message = args(&["daemon", "--profile", "a/b"])
        .unwrap_err()
        .to_string();
    assert!(message.starts_with("`--profile`: "), "{message}");
}

#[test]
fn a_non_utf8_daemon_argument_is_an_error_not_a_panic() {
    let got = parse(vec![
        OsString::from("daemon"),
        OsString::from("--profile"),
        OsString::from_vec(vec![b'a', 0xff]),
    ]);
    assert!(matches!(got, Err(Error::Unexpected { .. })), "{got:?}");
}

fn apply(profile: &str, json: &str, serve: bool) -> Result<Command, Error> {
    Ok(Command::ApplyConfig(ApplyOptions {
        profile: Profile::parse(profile).unwrap(),
        section: Section::parse(json.as_bytes()).unwrap(),
        serve,
    }))
}

#[test]
fn apply_config_takes_a_profile_and_the_section() {
    let json = r##"{"color":"#1e1e2e"}"##;
    assert_eq!(args(&["apply-config", json]), apply("default", json, false));
    assert_eq!(
        args(&["apply-config", "--profile", "scoot", json]),
        apply("scoot", json, false)
    );
    assert_eq!(
        args(&["apply-config", json, "--profile=scoot-nested"]),
        apply("scoot-nested", json, false)
    );
    assert_eq!(
        args(&["apply-config", "--serve", "--profile=scoot", "{}"]),
        apply("scoot", "{}", true)
    );
    // Whitespace around the object is JSON's.
    assert_eq!(
        args(&["apply-config", " {} "]),
        apply("default", "{}", false)
    );
}

#[test]
fn apply_config_arguments_are_checked() {
    assert_eq!(args(&["apply-config"]), Err(Error::MissingSection));
    assert_eq!(
        args(&["apply-config", "--profile", "scoot"]),
        Err(Error::MissingSection)
    );
    assert_eq!(
        args(&["apply-config", "--profile"]),
        Err(Error::MissingValue {
            command: "apply-config",
            flag: "--profile",
        })
    );
    assert!(matches!(
        args(&["apply-config", "--profile", "a/b", "{}"]),
        Err(Error::Profile(_))
    ));
    assert_eq!(
        args(&["apply-config", "--profile=a", "--profile=b", "{}"]),
        Err(Error::Repeated {
            command: "apply-config",
            flag: "--profile",
        })
    );
    assert_eq!(
        args(&["apply-config", "--serve", "--serve", "{}"]),
        Err(Error::Repeated {
            command: "apply-config",
            flag: "--serve",
        })
    );
    assert_eq!(
        args(&["apply-config", "{}", "{}"]),
        Err(Error::Unexpected {
            command: "apply-config",
            argument: "{}".into(),
        })
    );
    assert_eq!(
        args(&["apply-config", "--output", "{}"]),
        Err(Error::Unexpected {
            command: "apply-config",
            argument: "--output".into(),
        })
    );
    for bad in [r#"{"imgae":"/a"}"#, "{", r#"{"image":"a.png"}"#, "[]"] {
        let error = args(&["apply-config", bad]).unwrap_err();
        assert!(matches!(error, Error::Section(_)), "{bad}: {error:?}");
        assert!(error.to_string().contains("apply-config --help"), "{error}");
    }
}

#[test]
fn a_non_utf8_section_is_an_error_not_a_panic() {
    let raw = OsString::from_vec(b"{\"image\":\"/\xff\"}".to_vec());
    let parsed = parse([OsString::from("apply-config"), raw]);
    assert_eq!(parsed, Err(Error::SectionNotUtf8));
}
