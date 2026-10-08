//! Single-sourced wallpaper help: a table of the commands, the exit codes
//! and the environment; the `--help --json` document renders from it, and
//! the prose pages (`cli::usage` and friends) are checked against it, so
//! neither form can drift from the other.
//!
//! The typo guesser is not here but in [`scoot_ipc::suggest`], the one copy
//! every binary shares; likewise the docs URL is that crate's
//! [`scoot_ipc::DOCS_URL`], so the domain moves in one edit. A cold path (one
//! process per `--help`): the small allocations cost nothing at runtime.

use serde_json::{Map, Value};

use scoot_ipc::DOCS_URL;

/// Version of the `--help --json` document below. Bumped whenever a field is
/// added, renamed or removed, so a script can refuse what it does not know
/// rather than misread it.
pub const SCHEMA_VERSION: u32 = 2;

/// One command: its name, its shape, and what it does. Mirrors `cli`'s
/// command dispatch -- the drift tests pin both directions, so a command
/// added to the parser without a row here fails.
pub struct CommandDoc {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
}

/// Every command, in the order the help lists them.
pub const COMMANDS: &[CommandDoc] = &[
    CommandDoc {
        name: "daemon",
        usage: "daemon [--profile NAME] [--no-restore]",
        description: "run the wallpaper daemon for this Wayland display",
    },
    CommandDoc {
        name: "set",
        usage: "set COLOR|PATH|URL|DIR [--output NAME] [--workspace NAME] [--mode MODE] [--fill COLOR] [--filter FILTER] [--no-animate] [--sha256 HEX] [--every DURATION] [--shuffle] [--transition KIND] [--duration-ms MS] [--easing EASING] [--angle DEGREES] [--position X,Y]",
        description: "show a color, an image or a rotating directory on every output, or on one",
    },
    CommandDoc {
        name: "clear",
        usage: "clear [--output NAME] [--workspace NAME]",
        description: "back to the compositor's own background",
    },
    CommandDoc {
        name: "query",
        usage: "query",
        description: "print what each output shows, as JSON",
    },
    CommandDoc {
        name: "version",
        usage: "version",
        description: "print the running daemon's version and protocol, as JSON",
    },
    CommandDoc {
        name: "kill",
        usage: "kill",
        description: "stop the running daemon",
    },
    CommandDoc {
        name: "apply-config",
        usage: "apply-config [--profile NAME] JSON",
        description: "apply scoot's [wallpaper] section (what scoot runs)",
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
        "the run failed: no daemon, a refused value, drawing failed",
    ),
    (
        2,
        "usage error: an unknown command, flag or value (the error names it)",
    ),
];

/// Environment the binary reads.
pub const ENVIRONMENT: &[(&str, &str)] = &[
    ("WAYLAND_DISPLAY", "the compositor to show the wallpaper on"),
    (
        "XDG_RUNTIME_DIR",
        "where the control socket lives (scootbg-NAME.sock)",
    ),
    (
        "XDG_STATE_HOME",
        "where profiles are saved (~/.local/state/scootbg)",
    ),
];

/// The `--help --json` document as a value.
pub fn json_value() -> Value {
    let mut root = Map::new();
    root.insert("schema_version".into(), Value::from(SCHEMA_VERSION));
    root.insert("binary".into(), Value::from("scootbg"));
    root.insert("about".into(), Value::from("wallpaper daemon for Wayland"));
    root.insert(
        "commands".into(),
        Value::from(
            COMMANDS
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
        "set_values".into(),
        Value::from(vec![
            values_entry("mode", &["fill", "fit", "stretch", "center", "tile"]),
            values_entry(
                "filter",
                &["lanczos3", "catmull-rom", "bilinear", "nearest"],
            ),
            values_entry("transition", &["none", "fade", "wipe", "grow"]),
            values_entry(
                "easing",
                &["linear", "ease-in", "ease-out", "ease-in-out", "smooth"],
            ),
        ]),
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
        Value::from(format!("{DOCS_URL}/scootbg/cli.md")),
    );
    Value::Object(root)
}

fn values_entry(flag: &str, values: &[&str]) -> Value {
    let mut entry = Map::new();
    entry.insert("flag".into(), Value::from(flag));
    entry.insert(
        "values".into(),
        Value::from(
            values
                .iter()
                .map(|value| Value::from(*value))
                .collect::<Vec<_>>(),
        ),
    );
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
    fn every_command_is_in_the_prose_and_in_the_json() {
        let prose = format!(
            "{}{}{}{}{}{}{}{}",
            crate::cli::usage(),
            crate::cli::daemon_help(),
            crate::cli::SET_HELP,
            crate::cli::CLEAR_HELP,
            crate::cli::QUERY_HELP,
            crate::cli::VERSION_HELP,
            crate::cli::KILL_HELP,
            crate::cli::APPLY_CONFIG_HELP,
        );
        let document = json_value();
        let names: Vec<&str> = document["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        for command in COMMANDS {
            assert!(
                prose.contains(command.name),
                "`{}` is tabulated but not in the prose",
                command.name
            );
            assert!(
                names.contains(&command.name),
                "`{}` is tabulated but not in the JSON",
                command.name
            );
        }
    }

    #[test]
    fn every_flag_in_a_usage_block_is_in_its_json_row() {
        // Each command's own help page opens with a USAGE block; every
        // `--flag` it names must be in that command's tabulated usage, so
        // the JSON cannot fall behind a flag the parser gained (`--sha256`
        // was the one that did).
        let pages = [
            ("daemon", crate::cli::daemon_help()),
            ("set", crate::cli::SET_HELP.to_owned()),
            ("clear", crate::cli::CLEAR_HELP.to_owned()),
            ("query", crate::cli::QUERY_HELP.to_owned()),
            ("version", crate::cli::VERSION_HELP.to_owned()),
            ("kill", crate::cli::KILL_HELP.to_owned()),
            ("apply-config", crate::cli::APPLY_CONFIG_HELP.to_owned()),
        ];
        for (name, page) in pages {
            let row = COMMANDS
                .iter()
                .find(|command| command.name == name)
                .unwrap_or_else(|| panic!("`{name}` has a page but no row"));
            let usage = page
                .split("USAGE:")
                .nth(1)
                .and_then(|rest| rest.split("\n\n").next())
                .unwrap_or("");
            for word in usage.split(|c: char| c.is_whitespace() || "[]|=".contains(c)) {
                let flag = word.trim_matches(|c: char| c == ',' || c == '\'');
                if flag.starts_with("--") && flag.len() > 2 {
                    assert!(
                        row.usage.contains(flag),
                        "`{name}`'s help page names `{flag}` but its JSON usage does not: {}",
                        row.usage
                    );
                }
            }
        }
    }

    #[test]
    fn the_set_values_are_what_the_parser_takes() {
        // The JSON's enums cannot drift from the parsers: every value the
        // document names must parse, in both spellings the flag takes.
        let document = json_value();
        let sets = document["set_values"].as_array().unwrap();
        let modes = &sets[0]["values"];
        for mode in modes.as_array().unwrap() {
            let name = mode.as_str().unwrap();
            assert!(crate::image::Mode::from_name(name).is_some(), "{name}");
        }
        let filters = &sets[1]["values"];
        for filter in filters.as_array().unwrap() {
            let name = filter.as_str().unwrap();
            assert!(crate::image::Filter::from_name(name).is_some(), "{name}");
        }
        let transitions = &sets[2]["values"];
        for transition in transitions.as_array().unwrap() {
            let name = transition.as_str().unwrap();
            assert!(crate::transition::Kind::from_name(name).is_some(), "{name}");
        }
        let easings = &sets[3]["values"];
        for easing in easings.as_array().unwrap() {
            let name = easing.as_str().unwrap();
            assert!(
                crate::transition::Easing::from_name(name).is_some(),
                "{name}"
            );
        }
    }

    #[test]
    fn the_json_parses_versioned() {
        let document: Value = serde_json::from_str(&json()).unwrap();
        assert_eq!(document["schema_version"], Value::from(SCHEMA_VERSION));
        assert_eq!(document["binary"], Value::from("scootbg"));
        assert_eq!(
            document["commands"].as_array().unwrap().len(),
            COMMANDS.len()
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
            &format!("{}/scootbg/cli.md", scoot_ipc::DOCS_URL),
        );
    }

    #[test]
    fn the_main_pages_name_the_live_agent_index() {
        // The docs site is live: both pages carrying a docs URL end on the
        // real index, rendered from the one constant, never a hardcoded
        // domain.
        let index = format!("{}/llms.txt", scoot_ipc::DOCS_URL);
        assert!(
            crate::cli::usage().contains(&index),
            "the main page lost the live agent index"
        );
        assert!(
            crate::cli::daemon_help().contains(&index),
            "the daemon page lost the live agent index"
        );
    }
}
