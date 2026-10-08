use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use super::{ApplyOptions, Command, DaemonOptions, Error, Topic, parse, usage, version_string};
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
            usage().split_whitespace().any(|word| word == listed),
            "{listed} missing"
        );
    }
    for later in ["--serve", "--mode", "--fill", "--profile"] {
        assert!(
            !usage().split_whitespace().any(|word| word == later),
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
    for unknown in ["apply", "", "--serve"] {
        assert_eq!(args(&[unknown]), Err(Error::Unknown(unknown.to_owned())));
    }
    // Close enough to guess: the nearest command, named with where to read.
    for (typo, command) in [("--daemon", "daemon"), ("Query", "query"), ("Set", "set")] {
        assert_eq!(
            args(&[typo]),
            Err(Error::Hint {
                command: "scootbg",
                what: typo.to_owned(),
                suggestion: command.to_owned(),
                topic: "scootbg --help",
            })
        );
    }
    assert_eq!(
        args(&["help", "apply"]),
        Err(Error::Unknown("apply".to_owned()))
    );
}

#[test]
fn help_json_routes_and_typos_teach() {
    assert_eq!(args(&["--help", "--json"]), Ok(Command::Help(Topic::Json)));
    assert_eq!(args(&["help", "--json"]), Ok(Command::Help(Topic::Json)));
    assert_eq!(
        args(&["set", "--help", "--json"]),
        Ok(Command::Help(Topic::Json))
    );
    assert_eq!(
        args(&["daemon", "--help", "--json"]),
        Ok(Command::Help(Topic::Json))
    );
    assert_eq!(args(&["help", "help"]), Ok(Command::Help(Topic::Main)));
    assert_eq!(args(&["set", "help"]), Ok(Command::Help(Topic::Set)));
    assert_eq!(args(&["query", "help"]), Ok(Command::Help(Topic::Query)));
    assert_eq!(args(&["daemon", "help"]), Ok(Command::Help(Topic::Daemon)));
    assert!(Topic::Json.text().contains("\"schema_version\""));
    // A typo'd flag names the flag it meant and where to read.
    let error = args(&["daemon", "--profiel", "x"]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "unexpected `--profiel` for `daemon` \
         (did you mean `--profile`? see `scootbg daemon --help`)"
    );
    let error = args(&["set", "/a.png", "--mod", "fill"]).unwrap_err();
    assert!(error.to_string().contains("`--mode`"), "{error}");
}

#[test]
fn help_pages_stay_plain_short_and_ordered() {
    // The contract: plain text (no color escapes), wrapped under 100
    // columns, the same section order in every page.
    let main = Topic::Main.text();
    let mut cursor = 0;
    for section in [
        "USAGE:",
        "COMMANDS:",
        "EXAMPLES:",
        "EXIT CODES:",
        "ENVIRONMENT:",
        "SEE ALSO:",
    ] {
        let found = main[cursor..]
            .find(section)
            .unwrap_or_else(|| panic!("`{section}` missing or out of order"));
        cursor += found + section.len();
    }
    for topic in [
        Topic::Main,
        Topic::Daemon,
        Topic::Set,
        Topic::Clear,
        Topic::Query,
        Topic::Version,
        Topic::Kill,
        Topic::ApplyConfig,
    ] {
        let page = topic.text();
        assert!(
            !page.contains('\x1b'),
            "{topic:?} must not carry color escapes"
        );
        for line in page.as_ref().lines() {
            assert!(
                line.chars().count() < 100,
                "{topic:?}: line over 99 columns: `{line}`"
            );
        }
        assert!(page.contains("SEE ALSO:"), "{topic:?} lost its see-also");
    }
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
        transition: crate::transition::Spec::none(),
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
            animate: true,
        }),
        output: output.map(|o| o.to_owned().into()),
        transition: crate::transition::Spec::none(),
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
            animate: true,
        }),
        output: output.map(|o| o.to_owned().into()),
        transition: crate::transition::Spec::none(),
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
fn set_and_clear_take_a_workspace() {
    // A color for a workspace, in any order.
    assert_eq!(
        args(&["set", "#c03020", "--workspace", "2"]),
        Ok(Command::Client(Request::SetWorkspace {
            show: Show::Color(Color::parse("#c03020").unwrap()),
            output: None,
            workspace: "2".into(),
            transition: crate::transition::Spec::none(),
        }))
    );
    assert_eq!(
        args(&["set", "--workspace=2", "--output", "DP-1", "#c03020"]),
        Ok(Command::Client(Request::SetWorkspace {
            show: Show::Color(Color::parse("#c03020").unwrap()),
            output: Some("DP-1".into()),
            workspace: "2".into(),
            transition: crate::transition::Spec::none(),
        }))
    );
    // An image for a workspace.
    assert_eq!(
        args(&["set", "/p/a.png", "--workspace", "web"]),
        Ok(Command::Client(Request::SetWorkspace {
            show: Show::Image(ImageRequest {
                source: Source::Path("/p/a.png".into()),
                mode: Mode::Fill,
                fill: Color::parse("#000000").unwrap(),
                filter: Filter::Lanczos3,
            }),
            output: None,
            workspace: "web".into(),
            transition: crate::transition::Spec::none(),
        }))
    );
    // Clearing a workspace mapping.
    assert_eq!(
        args(&["clear", "--workspace", "2"]),
        Ok(Command::Client(Request::ClearWorkspace {
            output: None,
            workspace: "2".into(),
        }))
    );
    assert_eq!(
        args(&["clear", "--output", "DP-1", "--workspace", "2"]),
        Ok(Command::Client(Request::ClearWorkspace {
            output: Some("DP-1".into()),
            workspace: "2".into(),
        }))
    );
    // Without --workspace, the same lines are the old requests.
    assert_eq!(args(&["set", "#c03020"]), set("#c03020", None));
    assert_eq!(args(&["clear"]), clear(None));
}

#[test]
fn a_bad_workspace_is_a_usage_error() {
    assert!(matches!(
        args(&["set", "#c03020", "--workspace", ""]),
        Err(Error::BadWorkspace { .. })
    ));
    assert!(matches!(
        args(&["clear", "--workspace", ""]),
        Err(Error::BadWorkspace { .. })
    ));
    let long = "w".repeat(crate::choices::MAX_WORKSPACE_NAME + 1);
    assert!(matches!(
        args(&["set", "#c03020", "--workspace", &long]),
        Err(Error::BadWorkspace { .. })
    ));
    // Twice is repeated, like every other flag.
    assert!(matches!(
        args(&["set", "#c03020", "--workspace", "1", "--workspace", "2"]),
        Err(Error::Repeated { .. })
    ));
    // Missing its value.
    assert!(matches!(
        args(&["set", "#c03020", "--workspace"]),
        Err(Error::MissingValue { .. })
    ));
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
    for extra in ["--output", "restore", "-p"] {
        assert_eq!(
            args(&["daemon", extra]),
            Err(Error::Unexpected {
                command: "daemon",
                argument: extra.to_owned()
            })
        );
    }
    // Close enough to guess: `--profiles=x` meant `--profile`.
    assert_eq!(
        args(&["daemon", "--profiles=x"]),
        Err(Error::Hint {
            command: "daemon",
            what: "--profiles=x".into(),
            suggestion: "--profile".into(),
            topic: "scootbg daemon --help",
        })
    );
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
        section: Box::new(Section::parse(json.as_bytes()).unwrap()),
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

#[test]
fn set_takes_a_transition() {
    use crate::transition::{Easing, Kind, Spec};
    let with = |transition: Spec| {
        Ok(Command::Client(Request::Set {
            show: Show::Color(Color::parse("#c03020").unwrap()),
            output: None,
            transition,
        }))
    };
    // Defaults: none, so the change lands at once.
    assert_eq!(args(&["set", "#c03020"]), with(Spec::none()));
    assert_eq!(
        args(&["set", "#c03020", "--transition", "none"]),
        with(Spec::none())
    );
    // An explicit `none` ignores the rest, as on the wire.
    assert_eq!(
        args(&[
            "set",
            "#c03020",
            "--transition",
            "none",
            "--duration-ms",
            "5"
        ]),
        with(Spec::none())
    );
    assert_eq!(
        args(&[
            "set",
            "#c03020",
            "--transition",
            "fade",
            "--duration-ms",
            "800",
            "--easing",
            "linear",
        ]),
        with(Spec {
            kind: Kind::Fade,
            duration_ms: 800,
            easing: Easing::Linear,
            angle_deg: 0.0,
            pos: (0.5, 0.5),
        })
    );
    assert_eq!(
        args(&[
            "set",
            "#c03020",
            "--transition=wipe",
            "--angle=90",
            "--duration-ms=0",
        ]),
        with(Spec {
            kind: Kind::Wipe,
            duration_ms: 0,
            easing: Easing::EaseOut,
            angle_deg: 90.0,
            pos: (0.5, 0.5),
        })
    );
    assert_eq!(
        args(&[
            "set",
            "#c03020",
            "--transition",
            "grow",
            "--position",
            "0,0",
            "--easing",
            "smooth",
        ]),
        with(Spec {
            kind: Kind::Grow,
            duration_ms: 500,
            easing: Easing::Smooth,
            angle_deg: 0.0,
            pos: (0.0, 0.0),
        })
    );
    // Angles normalize; flags take `=` too, in any order.
    assert_eq!(
        args(&[
            "set",
            "--angle=-90",
            "--transition",
            "wipe",
            "#c03020",
            "--duration-ms",
            "250"
        ]),
        with(Spec {
            kind: Kind::Wipe,
            duration_ms: 250,
            easing: Easing::EaseOut,
            angle_deg: 270.0,
            pos: (0.5, 0.5),
        })
    );
}

#[test]
fn set_refuses_bad_transitions_plainly() {
    for value in ["dissolve", "FADE", ""] {
        let error = args(&["set", "#c03020", "--transition", value]).unwrap_err();
        assert!(
            matches!(error, Error::BadValue { .. }),
            "--transition {value}: {error:?}"
        );
        assert!(error.to_string().contains("--transition"), "{error}");
        assert!(error.to_string().contains("scootbg set --help"), "{error}");
    }
    for (flag, value) in [
        ("--duration-ms", "-1"),
        ("--duration-ms", "60001"),
        ("--duration-ms", "half"),
        ("--easing", "bounce"),
        ("--angle", "NaN"),
        ("--angle", "ninety"),
        ("--position", "0.5"),
        ("--position", "2,0.5"),
        ("--position", "left,top"),
    ] {
        let error = args(&["set", "#c03020", "--transition", "fade", flag, value]).unwrap_err();
        assert!(
            matches!(error, Error::BadValue { .. }),
            "{flag} {value}: {error:?}"
        );
        assert!(error.to_string().contains(flag), "{error}");
        assert!(error.to_string().contains("scootbg set --help"), "{error}");
    }
    // A transition flag without `--transition` teaches rather than being
    // ignored.
    for flag in ["--duration-ms", "--easing", "--angle", "--position"] {
        let value = if flag == "--position" { "0,0" } else { "1" };
        assert_eq!(
            args(&["set", "#c03020", flag, value]),
            Err(Error::TransitionOnly(flag))
        );
    }
    assert_eq!(
        args(&["set", "#c03020", "--transition"]).unwrap_err(),
        Error::MissingValue {
            command: "set",
            flag: "--transition"
        }
    );
    assert_eq!(
        args(&[
            "set",
            "#c03020",
            "--transition",
            "fade",
            "--transition",
            "wipe"
        ])
        .unwrap_err(),
        Error::Repeated {
            command: "set",
            flag: "--transition"
        }
    );
    // `clear` takes no transition: the generic refusal names the help that
    // says why.
    match args(&["clear", "--transition", "fade"]) {
        Err(Error::Unexpected { argument, .. }) => assert_eq!(argument, "--transition"),
        other => panic!("{other:?}"),
    }
}

fn slideshow_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sbg-cli-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    dir
}

#[allow(clippy::too_many_arguments)]
fn slideshow(
    dir: &str,
    every_secs: u64,
    shuffle: bool,
    mode: Mode,
    fill: &str,
    filter: Filter,
    output: Option<&str>,
) -> Result<Command, Error> {
    Ok(Command::Client(Request::Set {
        show: Show::Slideshow(crate::protocol::SlideshowRequest {
            dir: dir.to_owned().into(),
            every_secs,
            shuffle,
            mode,
            fill: Color::parse(fill).unwrap(),
            filter,
        }),
        output: output.map(|o| o.to_owned().into()),
        transition: crate::transition::Spec::none(),
    }))
}

#[test]
fn a_directory_with_every_is_a_slideshow() {
    let dir = slideshow_dir("set");
    let path = dir.to_str().unwrap().to_owned();
    assert_eq!(
        args(&["set", &path, "--every", "30m"]),
        slideshow(
            &path,
            1800,
            false,
            Mode::Fill,
            "#000000",
            Filter::Lanczos3,
            None
        )
    );
    assert_eq!(
        args(&[
            "set",
            &path,
            "--every=2h",
            "--shuffle",
            "--mode",
            "fit",
            "--output",
            "DP-1"
        ]),
        slideshow(
            &path,
            7200,
            true,
            Mode::Fit,
            "#000000",
            Filter::Lanczos3,
            Some("DP-1")
        )
    );
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn a_directory_without_every_names_it() {
    let dir = slideshow_dir("noevery");
    let path = dir.to_str().unwrap().to_owned();
    assert_eq!(
        args(&["set", &path]),
        Err(Error::DirectoryNeedsEvery(path.clone()))
    );
    let message = Error::DirectoryNeedsEvery(path).to_string();
    assert!(message.contains("--every 30m"), "{message}");
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn every_without_a_directory_is_refused() {
    let dir = slideshow_dir("everyfile");
    let file = dir.join("a.png");
    std::fs::write(&file, b"fake").unwrap();
    let path = file.to_str().unwrap().to_owned();
    assert_eq!(
        args(&["set", &path, "--every", "30m"]),
        Err(Error::EveryNeedsDirectory(path))
    );
    assert_eq!(
        args(&["set", "#000000", "--every", "30m"]),
        Err(Error::EveryNeedsDirectory("#000000".into()))
    );
    assert_eq!(
        args(&["set", "https://example.com/a.png", "--every", "30m"]),
        Err(Error::EveryNeedsDirectory(
            "https://example.com/a.png".into()
        ))
    );
    // Missing entirely: still not a directory.
    assert_eq!(
        args(&["set", "/no/such/file.png", "--every", "30m"]),
        Err(Error::EveryNeedsDirectory("/no/such/file.png".into()))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_slideshow_with_a_workspace_is_refused() {
    let dir = slideshow_dir("wsslide");
    let path = dir.to_str().unwrap().to_owned();
    assert_eq!(
        args(&["set", &path, "--every", "30m", "--workspace", "2"]),
        Err(Error::SlideshowWithWorkspace(path.clone()))
    );
    let message = Error::SlideshowWithWorkspace(path).to_string();
    assert!(message.contains("--workspace"), "{message}");
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn shuffle_without_every_is_refused() {
    assert_eq!(
        args(&["set", "/a", "--shuffle"]),
        Err(Error::ShuffleNeedsEvery)
    );
    assert_eq!(
        args(&["set", "/a", "--shuffle", "--shuffle"]),
        Err(Error::Repeated {
            command: "set",
            flag: "--shuffle"
        })
    );
    assert!(matches!(
        args(&["set", "/a", "--shuffle=x"]),
        Err(Error::Unexpected { .. })
    ));
    assert!(matches!(
        args(&["clear", "--every", "30m"]),
        Err(Error::Unexpected { .. })
    ));
}

#[test]
fn every_with_a_bad_duration_is_refused() {
    use crate::rotation::EveryError;
    let dir = slideshow_dir("badevery");
    let path = dir.to_str().unwrap().to_owned();
    assert_eq!(
        args(&["set", &path, "--every", "30s"]),
        Err(Error::BadEvery(EveryError::TooShort))
    );
    assert_eq!(
        args(&["set", &path, "--every", "90s"]),
        Err(Error::BadEvery(EveryError::NotAligned))
    );
    assert_eq!(
        args(&["set", &path, "--every", "never"]),
        Err(Error::BadEvery(EveryError::BadFormat("never".into())))
    );
    assert_eq!(
        args(&[
            "set",
            &path,
            "--every",
            "30m",
            "--sha256",
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        ]),
        Err(Error::ShaImageOnly)
    );
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn no_animate_stills_the_image() {
    let Command::Client(Request::Set {
        show: Show::Image(image),
        ..
    }) = args(&["set", "/tmp/a.gif", "--no-animate"]).unwrap()
    else {
        panic!("not an image set");
    };
    assert!(!image.animate, "--no-animate stills");
    // Without it, animation checks stay on.
    let Command::Client(Request::Set {
        show: Show::Image(image),
        ..
    }) = args(&["set", "/tmp/a.gif"]).unwrap()
    else {
        panic!("not an image set");
    };
    assert!(image.animate);
    // On a color it is refused: a color has no frames.
    assert!(matches!(
        args(&["set", "#ffffff", "--no-animate"]),
        Err(Error::ImageOnly(_))
    ));
}
