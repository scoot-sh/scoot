use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use super::{
    Command, DAEMON_HELP, Error, MSG_HELP, ModulesError, Msg, MsgError, Topic, USAGE, parse,
    version_string,
};
use crate::bar::{Bar, Edge, Layer, MAX_HEIGHT, Margin, MarginError};
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
        Ok(Command::Daemon(command)) => *command.config,
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
        ..Bar::default()
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
        Ok(Command::Daemon(command)) => {
            assert_eq!(command.config.font.unwrap().into_os_string(), raw);
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
    let has_workspaces = crate::modules::find("workspaces").is_some();
    assert_eq!(DAEMON_HELP.contains("--clock-format"), has_clock);
    assert_eq!(DAEMON_HELP.contains("Modules: clock"), has_clock);
    assert_eq!(USAGE.contains("a clock"), has_clock);
    assert_eq!(USAGE.contains("workspaces"), has_workspaces);
    assert_eq!(
        DAEMON_HELP.contains("the workspaces module"),
        has_workspaces
    );
    assert_eq!(DAEMON_HELP.contains("ext-workspace-v1"), has_workspaces);
    for flag in super::FLAGS {
        assert!(DAEMON_HELP.contains(flag), "{flag} is not documented");
    }
    assert!(!DAEMON_HELP.contains("wakes once a minute"));
}

#[test]
fn msg_commands_parse() {
    assert_eq!(run(&["msg", "query"]), Ok(Command::Msg(Msg::Query)));
    assert_eq!(run(&["msg", "reload"]), Ok(Command::Msg(Msg::Reload)));
    assert_eq!(run(&["msg", "version"]), Ok(Command::Msg(Msg::Version)));
    assert_eq!(run(&["msg", "kill"]), Ok(Command::Msg(Msg::Kill)));
    assert_eq!(run(&["msg", "--help"]), Ok(Command::Help(Topic::Msg)));
    assert_eq!(run(&["help", "msg"]), Ok(Command::Help(Topic::Msg)));
    assert!(USAGE.contains("msg"));
    assert!(MSG_HELP.contains("reload"));
}

#[test]
#[cfg(any(feature = "clock", feature = "workspaces"))]
fn msg_set_takes_an_id_and_json() {
    #[cfg(feature = "clock")]
    let id = "clock";
    #[cfg(all(not(feature = "clock"), feature = "workspaces"))]
    let id = "workspaces";
    match run(&["msg", "set", id, "{\"on\":true}"]) {
        Ok(Command::Msg(Msg::Set { id: got, value })) => {
            assert_eq!(got, id);
            assert_eq!(value, "{\"on\":true}");
        }
        other => panic!("{other:?}"),
    }
    // Any JSON value goes, even a bare one.
    assert!(matches!(
        run(&["msg", "set", id, "1"]),
        Ok(Command::Msg(Msg::Set { .. }))
    ));
}

#[test]
fn msg_refusals_name_what_is_wrong() {
    assert_eq!(run(&["msg"]), Err(Error::Msg(MsgError::Missing)));
    let unknown = run(&["msg", "halt"]).unwrap_err().to_string();
    assert!(unknown.contains("halt"), "{unknown}");
    assert_eq!(run(&["msg", "set"]), Err(Error::Msg(MsgError::NeedsId)));
    assert_eq!(
        run(&["msg", "set", "clock"]),
        Err(Error::Msg(MsgError::NeedsValue))
    );
    let unknown = run(&["msg", "set", "battery", "{}"])
        .unwrap_err()
        .to_string();
    assert!(unknown.contains("battery"), "{unknown}");
    // A trailing argument is unexpected, naming the command.
    let extra = run(&["msg", "query", "x"]).unwrap_err().to_string();
    assert!(extra.contains("msg"), "{extra}");
}

#[test]
#[cfg(any(feature = "clock", feature = "workspaces"))]
fn msg_set_with_bad_json_names_it() {
    #[cfg(feature = "clock")]
    let id = "clock";
    #[cfg(all(not(feature = "clock"), feature = "workspaces"))]
    let id = "workspaces";
    let bad = run(&["msg", "set", id, "{oops}"]).unwrap_err().to_string();
    assert!(bad.contains("not JSON"), "{bad}");
}

#[test]
fn config_flag_names_the_file_and_changes_nothing_else() {
    match run(&["daemon", "--config", "/tmp/bar.toml"]) {
        Ok(Command::Daemon(command)) => {
            assert_eq!(
                command.file,
                Some(std::path::PathBuf::from("/tmp/bar.toml"))
            );
            assert_eq!(*command.config, Config::default());
        }
        other => panic!("{other:?}"),
    }
    // Either spelling, and twice is still twice.
    assert!(matches!(
        run(&["daemon", "--config=/tmp/bar.toml"]),
        Ok(Command::Daemon(_))
    ));
    assert!(matches!(
        run(&["daemon", "--config", "a", "--config", "b"]),
        Err(Error::Repeated("--config"))
    ));
    assert!(matches!(
        run(&["daemon", "--config"]),
        Err(Error::MissingValue("--config"))
    ));
}

#[test]
#[cfg(any(feature = "clock", feature = "workspaces"))]
fn flags_overlay_the_file_section_by_section() {
    #[cfg(feature = "clock")]
    let id = "clock";
    #[cfg(all(not(feature = "clock"), feature = "workspaces"))]
    let id = "workspaces";
    // One flag replaces its own section; the file's other sections stand.
    #[cfg(all(feature = "clock", feature = "workspaces"))]
    {
        let mut base = Config::default();
        base.layout.left = vec!["workspaces"];
        base.layout.center = vec![];
        let command = match run(&["daemon", "--right", id]) {
            Ok(Command::Daemon(command)) => command,
            other => panic!("{other:?}"),
        };
        command.given.overlay(&mut base).unwrap();
        assert_eq!(base.layout.left, ["workspaces"]);
        assert_eq!(base.layout.right, [id]);
    }
    // One module in the build: an empty section still replaces only
    // itself, leaving the file's placement standing.
    #[cfg(not(all(feature = "clock", feature = "workspaces")))]
    {
        let mut base = Config::default();
        base.layout.left = vec![id];
        base.layout.center = vec![];
        let command = match run(&["daemon", "--center", ""]) {
            Ok(Command::Daemon(command)) => command,
            other => panic!("{other:?}"),
        };
        command.given.overlay(&mut base).unwrap();
        assert_eq!(base.layout.left, [id]);
        assert!(base.layout.center.is_empty());
    }
    // A flag and the file placing the same module twice is refused.
    let mut base = Config {
        layout: Layout {
            left: vec![id],
            center: vec![],
            right: vec![],
            padding: 8,
            spacing: 0,
            separator: 0,
            margins: vec![],
        },
        ..Config::default()
    };
    let command = match run(&["daemon", "--right", id]) {
        Ok(Command::Daemon(command)) => command,
        other => panic!("{other:?}"),
    };
    let error = command.given.overlay(&mut base).unwrap_err().to_string();
    assert!(error.contains("twice"), "{error}");
    // Scalars replace their own value.
    let mut base = Config::default();
    let command = match run(&["daemon", "--height", "40"]) {
        Ok(Command::Daemon(command)) => command,
        other => panic!("{other:?}"),
    };
    command.given.overlay(&mut base).unwrap();
    assert_eq!(base.bar.height, 40);
}

#[test]
fn layer_and_exclusive_flags_take_their_values() {
    let bar = daemon(&["--layer", "overlay", "--exclusive=false"]);
    assert_eq!(bar.layer, Layer::Overlay);
    assert!(!bar.exclusive);
    assert_eq!(daemon(&["--layer=bottom"]).layer, Layer::Bottom);
    assert_eq!(daemon(&[]).layer, Layer::Top);
    assert!(daemon(&["--exclusive", "true"]).exclusive);
    assert_eq!(
        run(&["daemon", "--layer", "background"]),
        Err(Error::Layer("background".into()))
    );
    for bad in ["yes", "1", "True", ""] {
        assert_eq!(
            run(&["daemon", "--exclusive", bad]),
            Err(Error::Exclusive(bad.into()))
        );
    }
    assert_eq!(
        run(&["daemon", "--layer", "top", "--layer", "top"]),
        Err(Error::Repeated("--layer"))
    );
}

#[test]
fn a_layer_flag_overlays_the_files_value() {
    let mut base = Config::default();
    base.bar.layer = Layer::Bottom;
    base.bar.exclusive = false;
    let given = match run(&["daemon", "--layer", "overlay"]) {
        Ok(Command::Daemon(command)) => command.given,
        other => panic!("{other:?}"),
    };
    given.overlay(&mut base).unwrap();
    assert_eq!(base.bar.layer, Layer::Overlay);
    // Not given: the file's value stands.
    assert!(!base.bar.exclusive);
}

#[test]
fn visibility_commands_take_no_arguments() {
    assert_eq!(run(&["msg", "hide"]), Ok(Command::Msg(Msg::Hide)));
    assert_eq!(run(&["msg", "show"]), Ok(Command::Msg(Msg::Show)));
    assert_eq!(run(&["msg", "toggle"]), Ok(Command::Msg(Msg::Toggle)));
    assert!(matches!(
        run(&["msg", "toggle", "now"]),
        Err(Error::Unexpected { .. })
    ));
    for word in ["hide", "show", "toggle"] {
        assert!(super::MSG_HELP.contains(word), "{word}");
    }
}

#[test]
fn outputs_takes_all_or_connector_names() {
    use crate::policy::Select;
    assert_eq!(config(&[]).outputs.select, Select::All);
    assert_eq!(config(&["--outputs", "all"]).outputs.select, Select::All);
    assert_eq!(
        config(&["--outputs=DP-1,eDP-1"]).outputs.select,
        Select::Named(vec!["DP-1".into(), "eDP-1".into()])
    );
    // Only the exact word `all` is all: a connector called `All` is a name.
    assert_eq!(
        config(&["--outputs", "All"]).outputs.select,
        Select::Named(vec!["All".into()])
    );
}

#[test]
fn a_bad_outputs_value_is_refused() {
    for value in [
        "",
        ",",
        "DP-1,",
        ",DP-1",
        "DP-1,,DP-2",
        "DP-1,DP-1",
        "a\u{1b}b",
    ] {
        assert!(
            matches!(
                run(&["daemon", "--outputs", value]),
                Err(Error::Outputs { .. })
            ),
            "{value:?}"
        );
    }
    let many: Vec<String> = (0..=crate::policy::MAX_OUTPUTS)
        .map(|n| format!("O-{n}"))
        .collect();
    assert!(matches!(
        run(&["daemon", "--outputs", &many.join(",")]),
        Err(Error::Outputs { .. })
    ));
    assert_eq!(
        run(&["daemon", "--outputs", "a", "--outputs", "b"]),
        Err(Error::Repeated("--outputs"))
    );
    // Never echoed raw.
    let message = run(&["daemon", "--outputs", "a\u{1b}[31m,a\u{1b}[31m"])
        .unwrap_err()
        .to_string();
    assert!(!message.contains('\u{1b}'), "{message:?}");
}

/// The list over the file's `[output]` tables: a table the list leaves out
/// could never apply, so the pair is refused.
#[test]
fn the_flag_and_the_files_tables_are_checked_together() {
    use crate::policy::{BarOverride, Override, Policy, Select};
    let mut base = Config {
        outputs: Policy {
            select: Select::All,
            overrides: vec![Override {
                name: "DP-2".into(),
                bar: BarOverride::default(),
                modules: None,
            }],
        },
        ..Config::default()
    };
    let overlay = |base: &mut Config, list: &str| match run(&["daemon", "--outputs", list]) {
        Ok(Command::Daemon(command)) => command.given.overlay(base),
        other => panic!("{other:?}"),
    };
    let error = overlay(&mut base, "DP-1").unwrap_err().to_string();
    assert!(
        error.contains("DP-2") && error.contains("--outputs"),
        "{error}"
    );
    assert_eq!(overlay(&mut base, "DP-1,DP-2"), Ok(()));
    assert_eq!(overlay(&mut base, "all"), Ok(()));
    // No flag: the file's own list stands, untouched.
    let mut base = Config::default();
    base.outputs.select = Select::Named(vec!["HDMI-A-1".into()]);
    match run(&["daemon", "--height", "30"]) {
        Ok(Command::Daemon(command)) => command.given.overlay(&mut base).unwrap(),
        other => panic!("{other:?}"),
    }
    assert_eq!(base.outputs.select, Select::Named(vec!["HDMI-A-1".into()]));
}

#[test]
fn check_is_a_switch_that_takes_no_value_and_may_come_once() {
    let check = |args: &[&str]| {
        let mut all = vec!["daemon"];
        all.extend_from_slice(args);
        run(&all)
    };
    match check(&["--check"]) {
        Ok(Command::Daemon(command)) => assert!(command.check),
        other => panic!("{other:?}"),
    }
    match check(&["--height", "40", "--check", "--config", "/x.toml"]) {
        Ok(Command::Daemon(command)) => {
            assert!(command.check);
            assert_eq!(command.file, Some("/x.toml".into()));
            assert_eq!(command.config.bar.height, 40);
        }
        other => panic!("{other:?}"),
    }
    match run(&["daemon"]) {
        Ok(Command::Daemon(command)) => assert!(!command.check),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        check(&["--check", "--check"]),
        Err(Error::Repeated("--check"))
    );
    assert!(matches!(
        check(&["--check=yes"]),
        Err(Error::Unexpected {
            command: "daemon",
            ..
        })
    ));
    // Not a flag of the other commands.
    assert!(run(&["msg", "--check"]).is_err());
}

#[test]
fn the_help_names_check() {
    assert!(USAGE.contains("scootbar daemon --check"));
    assert!(DAEMON_HELP.contains("--check validates"));
}
