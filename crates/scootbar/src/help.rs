//! Single-sourced bar help: tables for the daemon flags, the `msg`
//! commands, the exit codes and the environment; the `--help --json`
//! document renders from them, and the prose pages (`cli::usage`,
//! `cli::msg_help`, [`crate::cli::daemon_help`]) are checked against them,
//! so neither form can drift from the other.
//!
//! The typo guesser is not here but in [`scoot_ipc::suggest`], the one copy
//! every binary shares; likewise the docs URL is that crate's
//! [`scoot_ipc::DOCS_URL`], so the domain moves in one edit. A cold path
//! (one process per `--help`): the small allocations cost nothing at
//! runtime.

use serde_json::{Map, Value};

use scoot_ipc::DOCS_URL;

use crate::modules::REGISTRY;

/// Version of the `--help --json` document below. Bumped whenever a field is
/// added, renamed or removed, so a script can refuse what it does not know
/// rather than misread it.
pub const SCHEMA_VERSION: u32 = 1;

/// One `daemon` flag: its spelling, what follows it, its default, and what
/// it does. Mirrors `cli::FLAGS` plus the prose -- the drift tests pin both
/// directions, so a flag added to the parser without a row here fails.
pub struct FlagDoc {
    pub flag: &'static str,
    pub takes: &'static str,
    pub default: &'static str,
    pub description: &'static str,
}

/// Every `daemon` flag, in the order the help lists them.
pub const FLAG_DOCS: &[FlagDoc] = &[
    FlagDoc {
        flag: "--outputs",
        takes: "LIST",
        default: "all",
        description: "all, or comma-separated connector names: only those outputs get a bar",
    },
    FlagDoc {
        flag: "--edge",
        takes: "EDGE",
        default: "top",
        description: "top or bottom: which edge the bar sits along",
    },
    FlagDoc {
        flag: "--layer",
        takes: "LAYER",
        default: "top",
        description: "bottom, top or overlay: where the bar stacks",
    },
    FlagDoc {
        flag: "--exclusive",
        takes: "BOOL",
        default: "true",
        description: "true reserves the bar's height, so windows sit beside it; false floats over them",
    },
    FlagDoc {
        flag: "--height",
        takes: "N",
        default: "28",
        description: "the bar's height in logical pixels, 1 to 1024",
    },
    FlagDoc {
        flag: "--margin",
        takes: "M",
        default: "0",
        description: "space between the bar and the output's edges, CSS-shaped, each 0 to 1024",
    },
    FlagDoc {
        flag: "--background",
        takes: "COLOR",
        default: "'#1e1e2e'",
        description: "the bar's color, '#rrggbb' (quote it: the shell reads '#' as a comment)",
    },
    FlagDoc {
        flag: "--foreground",
        takes: "COLOR",
        default: "'#cdd6f4'",
        description: "the text's color, '#rrggbb'",
    },
    FlagDoc {
        flag: "--font",
        takes: "PATH",
        default: "first of DejaVu Sans, Noto Sans found",
        description: "a .ttf or .otf file; with none found, the bar refuses to start",
    },
    FlagDoc {
        flag: "--font-size",
        takes: "N",
        default: "14",
        description: "the text's size (the em) in logical pixels, 1 to 256",
    },
    FlagDoc {
        flag: "--left",
        takes: "IDS",
        default: "empty (the clock in the center, in a build with the clock)",
        description: "the modules along the left, comma-separated, in order",
    },
    FlagDoc {
        flag: "--center",
        takes: "IDS",
        default: "empty (the clock in the center, in a build with the clock)",
        description: "the modules along the center, comma-separated, in order",
    },
    FlagDoc {
        flag: "--right",
        takes: "IDS",
        default: "empty",
        description: "the modules along the right, comma-separated, in order",
    },
    FlagDoc {
        flag: "--padding",
        takes: "N",
        default: "8",
        description: "logical pixels either side of each module, 0 to 1024",
    },
    FlagDoc {
        flag: "--spacing",
        takes: "N",
        default: "0",
        description: "logical pixels between neighbouring modules, 0 to 1024",
    },
    FlagDoc {
        flag: "--config",
        takes: "PATH",
        default: "$XDG_CONFIG_HOME/scoot/bar.toml",
        description: "read another config file instead of the default one",
    },
    FlagDoc {
        flag: "--check",
        takes: "(none)",
        default: "absent",
        description: "validate instead of running: exits 0 printing `ok`, or 1 with the error",
    },
    #[cfg(feature = "clock")]
    FlagDoc {
        flag: "--clock-format",
        takes: "FMT",
        default: "'%-I:%M %P'",
        description: "the clock, as strftime; with %S or %T it ticks every second",
    },
];

/// One `msg` command: its name, its shape, and what it does.
pub struct MsgDoc {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
}

/// What `msg set` takes, for this build: with the `push` feature it names
/// the `push` module, without it no module takes a value, so the wording
/// names none (every `set` is refused `takes no set value` there).
#[cfg(feature = "push")]
const SET_DESCRIPTION: &str = "a JSON value for the `push` module ID";
#[cfg(not(feature = "push"))]
const SET_DESCRIPTION: &str =
    "a JSON value for a module ID (this build has no module that takes one)";

/// Every `msg` command, in the order the help lists them.
pub const MSG_COMMANDS: &[MsgDoc] = &[
    MsgDoc {
        name: "query",
        usage: "query [ID]",
        description: "each placed module's state as JSON (or only module ID's)",
    },
    MsgDoc {
        name: "layout",
        usage: "layout",
        description: "each output's bar and each module's rectangle, in logical pixels",
    },
    MsgDoc {
        name: "invoke",
        usage: "invoke ID ACTION [NUMBER] [--output NAME]",
        description: "run module ID's ACTION as a click would",
    },
    MsgDoc {
        name: "subscribe",
        usage: "subscribe [module] [output]",
        description: "stay connected and print one JSON line per event",
    },
    MsgDoc {
        name: "reload",
        usage: "reload",
        description: "re-read the config file and live-apply it",
    },
    MsgDoc {
        name: "hide",
        usage: "hide",
        description: "take the bars away, releasing their space to windows",
    },
    MsgDoc {
        name: "show",
        usage: "show",
        description: "make the bars again",
    },
    MsgDoc {
        name: "toggle",
        usage: "toggle",
        description: "hide if shown, show if hidden",
    },
    MsgDoc {
        name: "version",
        usage: "version",
        description: "the daemon's version and protocol, as JSON",
    },
    MsgDoc {
        name: "kill",
        usage: "kill",
        description: "stop the daemon, once its reply is sent",
    },
    MsgDoc {
        name: "set",
        usage: "set ID JSON",
        description: SET_DESCRIPTION,
    },
];

/// Exit codes the binary uses: 0 for success (help included), 1 when the
/// run failed, 2 when the invocation itself was wrong.
pub const EXIT_CODES: &[(i32, &str)] = &[
    (
        0,
        "success: the reply is on stdout (help and --version count)",
    ),
    (
        1,
        "the run failed: a bad file, no daemon, a refused value, the compositor going away",
    ),
    (
        2,
        "usage error: an unknown command, flag or value (the error names it)",
    ),
];

/// Environment the binary reads.
pub const ENVIRONMENT: &[(&str, &str)] = &[
    ("WAYLAND_DISPLAY", "the compositor to show the bar on"),
    (
        "XDG_RUNTIME_DIR",
        "where the control socket lives (scootbar-DISPLAY.sock)",
    ),
    (
        "XDG_CONFIG_HOME",
        "where bar.toml lives (~/.config/scoot/bar.toml by default)",
    ),
];

/// The `--help --json` document as a value. The modules list comes from
/// [`REGISTRY`] -- the same registry the parser and the prose read -- so a
/// build without a module documents only what it has, in both forms.
pub fn json_value() -> Value {
    let mut root = Map::new();
    root.insert("schema_version".into(), Value::from(SCHEMA_VERSION));
    root.insert("binary".into(), Value::from("scootbar"));
    root.insert("about".into(), Value::from("status bar for Wayland"));
    root.insert(
        "commands".into(),
        Value::from(vec![
            command_entry(
                "daemon",
                "scootbar daemon [OPTIONS]",
                "run the bar on this Wayland display's outputs",
            ),
            command_entry(
                "msg",
                "scootbar msg COMMAND",
                "ask the running daemon: query, reload, hide, show, toggle, version, kill, set",
            ),
        ]),
    );
    root.insert(
        "daemon_flags".into(),
        Value::from(
            FLAG_DOCS
                .iter()
                .map(|flag| {
                    let mut entry = Map::new();
                    entry.insert("flag".into(), Value::from(flag.flag));
                    entry.insert("takes".into(), Value::from(flag.takes));
                    entry.insert("default".into(), Value::from(flag.default));
                    entry.insert("description".into(), Value::from(flag.description));
                    Value::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    root.insert(
        "msg_commands".into(),
        Value::from(
            MSG_COMMANDS
                .iter()
                .map(|command| {
                    let mut entry = Map::new();
                    entry.insert("name".into(), Value::from(command.name));
                    entry.insert("usage".into(), Value::from(command.usage));
                    entry.insert("description".into(), Value::from(command.description));
                    Value::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    root.insert(
        "modules".into(),
        Value::from(
            REGISTRY
                .iter()
                .map(|spec| {
                    let mut entry = Map::new();
                    entry.insert("id".into(), Value::from(spec.id));
                    entry.insert(
                        "actions".into(),
                        Value::from(
                            spec.actions
                                .iter()
                                .map(|action| {
                                    let mut invoke = Map::new();
                                    invoke.insert("name".into(), Value::from(action.name));
                                    invoke.insert(
                                        "arg".into(),
                                        Value::from(match action.arg {
                                            crate::modules::ArgKind::None => "none",
                                            crate::modules::ArgKind::Required => "number",
                                        }),
                                    );
                                    Value::Object(invoke)
                                })
                                .collect::<Vec<_>>(),
                        ),
                    );
                    Value::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    root.insert(
        "exit_codes".into(),
        Value::from(
            EXIT_CODES
                .iter()
                .map(|(code, meaning)| {
                    let mut entry = Map::new();
                    entry.insert("code".into(), Value::from(*code));
                    entry.insert("meaning".into(), Value::from(*meaning));
                    Value::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    root.insert(
        "environment".into(),
        Value::from(
            ENVIRONMENT
                .iter()
                .map(|(name, description)| {
                    let mut entry = Map::new();
                    entry.insert("name".into(), Value::from(*name));
                    entry.insert("description".into(), Value::from(*description));
                    Value::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    root.insert(
        "docs".into(),
        Value::from(format!("{DOCS_URL}/scootbar/cli.md")),
    );
    Value::Object(root)
}

fn command_entry(name: &str, usage: &str, description: &str) -> Value {
    let mut entry = Map::new();
    entry.insert("name".into(), Value::from(name));
    entry.insert("usage".into(), Value::from(usage));
    entry.insert("description".into(), Value::from(description));
    Value::Object(entry)
}

/// The `--help --json` document, pretty-printed.
pub fn json() -> String {
    serde_json::to_string_pretty(&json_value()).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_table_matches_the_parser_both_ways() {
        // The drift pin: a flag the parser takes without a row here fails,
        // and a row for no flag fails.
        let table: Vec<&str> = FLAG_DOCS.iter().map(|flag| flag.flag).collect();
        for flag in crate::cli::FLAGS {
            assert!(table.contains(flag), "{flag} parses but has no help row");
        }
        // `--check` is the one switch the parser answers before `FLAGS`
        // (it takes no value), so it has a row without being in `FLAGS`.
        assert!(table.contains(&"--check"), "--check lost its help row");
        for flag in &table {
            if *flag == "--check" {
                continue;
            }
            assert!(
                crate::cli::FLAGS.contains(flag),
                "{flag} has a help row but does not parse"
            );
        }
    }

    #[test]
    fn every_flag_and_msg_command_is_in_the_prose_and_in_the_json() {
        let prose = format!(
            "{}{}{}",
            crate::cli::usage(),
            crate::cli::daemon_help(),
            crate::cli::msg_help()
        );
        let document = json_value();
        let flags: Vec<&str> = document["daemon_flags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["flag"].as_str().unwrap())
            .collect();
        for flag in FLAG_DOCS {
            assert!(
                prose.contains(flag.flag),
                "`{}` is tabulated but not in the prose",
                flag.flag
            );
            assert!(
                flags.contains(&flag.flag),
                "`{}` is tabulated but not in the JSON",
                flag.flag
            );
        }
        let commands: Vec<&str> = document["msg_commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        for command in MSG_COMMANDS {
            assert!(
                prose.contains(command.name),
                "`{}` is tabulated but not in the prose",
                command.name
            );
            assert!(
                commands.contains(&command.name),
                "`{}` is tabulated but not in the JSON",
                command.name
            );
        }
    }

    #[test]
    fn the_json_modules_are_the_registry() {
        // A build without a module documents only what it has, in both
        // forms -- this pins the JSON half (the prose half is the
        // `modules_section`/`*_help!` machinery).
        let document = json_value();
        let modules = document["modules"].as_array().unwrap();
        assert_eq!(modules.len(), REGISTRY.len());
        for (entry, spec) in modules.iter().zip(REGISTRY.iter()) {
            assert_eq!(entry["id"].as_str().unwrap(), spec.id);
            assert_eq!(
                entry["actions"].as_array().unwrap().len(),
                spec.actions.len()
            );
        }
    }

    #[test]
    fn the_json_parses_versioned_and_plain() {
        let document: Value = serde_json::from_str(&json()).unwrap();
        assert_eq!(document["schema_version"], Value::from(SCHEMA_VERSION));
        assert_eq!(document["binary"], Value::from("scootbar"));
    }

    #[test]
    fn the_set_row_names_no_push_module_without_the_feature() {
        // The JSON half of the build gate: the `set` row names the `push`
        // module exactly when this build has one (the prose half is
        // `the_help_matches_the_build` in `cli::tests`).
        let document = json_value();
        let commands = document["msg_commands"].as_array().unwrap();
        let set = commands
            .iter()
            .find(|entry| entry["name"] == "set")
            .expect("the `set` command has a row in every build");
        assert_eq!(
            set["description"].as_str().unwrap().contains("push"),
            cfg!(feature = "push"),
            "`set` says: {}",
            set["description"].as_str().unwrap()
        );
    }

    #[test]
    fn the_json_docs_come_from_the_one_constant() {
        // The domain lives once, in `scoot-ipc`: the document renders from
        // it, so a move is one edit and this fails until every literal
        // follows.
        let document = json_value();
        assert_eq!(
            document["docs"].as_str().unwrap(),
            &format!("{}/scootbar/cli.md", scoot_ipc::DOCS_URL),
        );
    }
}
