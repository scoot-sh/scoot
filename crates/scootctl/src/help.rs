//! Single-sourced client help: one table per surface, text and JSON rendered
//! from it.
//!
//! [`REQUESTS`] and [`ACTIONS`] are the one copy of the request/action
//! reference: the prose blocks in [`crate::cli`] (`REQUESTS_HELP`,
//! `ACTIONS_HELP`), the `EXAMPLES` / `EXIT CODES` / `ENVIRONMENT` sections of
//! [`usage`], and the [`json`] document are all rendered or checked against
//! these tables, so neither form can drift from the other. The tests below
//! pin both directions: every row's syntax appears in the prose, and every
//! row's name appears in the JSON.
//!
//! A cold path (one process per `--help`): the small allocations in `usage`
//! and `json` cost nothing at runtime.

use serde_json::{Map, Value};

/// Version of the `--help --json` document below. Bumped whenever a field is
/// added, renamed or removed, so a script can refuse what it does not know
/// rather than misread it.
pub const SCHEMA_VERSION: u32 = 1;

/// Where the human reference lives. There is no published docs site yet (see
/// `docs/backlog/packaging/docs-site.md`): until it exists, help points at
/// the reference pages in the repo, which the site will later publish with
/// `/llms.txt`.
pub const DOCS_URL: &str = "https://github.com/scoot-sh/scoot/tree/main/docs";

/// One request verb: what it takes, what it does, one real invocation, and
/// the shape of its reply.
pub struct RequestDoc {
    pub verb: &'static str,
    pub syntax: &'static str,
    pub description: &'static str,
    pub example: &'static str,
    pub reply: &'static str,
}

/// Every request verb both clients take, in the order the prose lists them.
pub const REQUESTS: &[RequestDoc] = &[
    RequestDoc {
        verb: "version",
        syntax: "version",
        description: "the running compositor's version and IPC protocol",
        example: "scootctl version",
        reply: "`{\"type\":\"version\",...}`",
    },
    RequestDoc {
        verb: "outputs",
        syntax: "outputs",
        description: "every output's name, rectangle, scale and power state",
        example: "scootctl outputs",
        reply: "`{\"type\":\"outputs\",\"outputs\":[...]}`",
    },
    RequestDoc {
        verb: "windows",
        syntax: "windows",
        description: "every window: id, app id, title, output, workspace, focus",
        example: "scootctl windows",
        reply: "`{\"type\":\"windows\",\"windows\":[{\"id\":7,...}]}`",
    },
    RequestDoc {
        verb: "reload",
        syntax: "reload",
        description: "re-read the config file and re-apply what can be re-applied live",
        example: "scootctl reload",
        reply: "`{\"type\":\"reloaded\",...}`",
    },
    RequestDoc {
        verb: "keyboard",
        syntax: "keyboard",
        description: "the active keyboard layout's name and index",
        example: "scootctl keyboard",
        reply: "`{\"type\":\"keyboard\",\"index\":0,\"name\":\"English (US)\"}`",
    },
    RequestDoc {
        verb: "output-power",
        syntax: "output-power ID|all on|off",
        description: "switch an output's panel off or on",
        example: "scootctl output-power all off",
        reply: "`{\"type\":\"ok\",...}`",
    },
    RequestDoc {
        verb: "action",
        syntax: "action ACTION [ARGUMENT...]",
        description: "run a layout action (see `help actions`)",
        example: "scootctl action focus-column left",
        reply: "`{\"type\":\"ok\",...}`",
    },
    RequestDoc {
        verb: "screenshot",
        syntax: "screenshot [--output ID] [--out FILE] [--no-cursor]",
        description: "capture the screen as PNG (stdout, or FILE with --out)",
        example: "scootctl screenshot --out /tmp/shot.png",
        reply: "prints `800x600, 12345 bytes -> /tmp/shot.png`; without `--out`, raw PNG on stdout",
    },
    RequestDoc {
        verb: "pointer",
        syntax: "pointer move X Y | pointer click X Y [left|right|middle]",
        description: "move, click, press/release, or scroll (also: pointer button, pointer scroll)",
        example: "scootctl pointer click 800 500",
        reply: "`{\"type\":\"ok\",...}`",
    },
    RequestDoc {
        verb: "key",
        syntax: "key COMBO",
        description: "press one key combination (e.g. Return, ctrl+shift+t)",
        example: "scootctl key super+Return",
        reply: "`{\"type\":\"ok\",...}`",
    },
    RequestDoc {
        verb: "type",
        syntax: "type TEXT",
        description: "type text on the active keyboard layout",
        example: "scootctl type \"hello\"",
        reply: "`{\"type\":\"ok\",...}`",
    },
    RequestDoc {
        verb: "wait-idle",
        syntax: "wait-idle [--quiet-ms N] [--timeout-ms N]",
        description: "block until nothing has redrawn for --quiet-ms (default 200)",
        example: "scootctl wait-idle --quiet-ms 200",
        reply: "`{\"type\":\"idle\",\"waited_ms\":213}`",
    },
    RequestDoc {
        verb: "subscribe",
        syntax: "subscribe [EVENT...]",
        description: "stream events (output, keyboard, workspace; default: output) until killed",
        example: "scootctl subscribe workspace",
        reply: "one compact JSON object per line, e.g. `{\"type\":\"subscribed\",...}` then events",
    },
];

/// One layout action: its name, its argument shape, and what it does.
pub struct ActionDoc {
    pub name: &'static str,
    pub args: &'static str,
    pub description: &'static str,
}

/// Every action verb, in the order the prose lists them.
pub const ACTIONS: &[ActionDoc] = &[
    ActionDoc {
        name: "focus-column",
        args: "left|right",
        description: "focus the neighbouring column",
    },
    ActionDoc {
        name: "move-column",
        args: "left|right",
        description: "move the focused column sideways",
    },
    ActionDoc {
        name: "consume-or-expel",
        args: "left|right",
        description: "consume into or expel from the neighbouring column",
    },
    ActionDoc {
        name: "focus-window",
        args: "up|down",
        description: "focus the window above or below",
    },
    ActionDoc {
        name: "move-window",
        args: "up|down",
        description: "move the focused window up or down",
    },
    ActionDoc {
        name: "focus-workspace",
        args: "up|down",
        description: "switch to the workspace above or below",
    },
    ActionDoc {
        name: "move-window-to-workspace",
        args: "up|down",
        description: "send the focused window to the workspace above or below",
    },
    ActionDoc {
        name: "focus-window-id",
        args: "ID",
        description: "focus a window by id (see `windows`)",
    },
    ActionDoc {
        name: "focus-workspace-index",
        args: "N [--output ID]",
        description: "switch to workspace N (0-based) on the focused or named output",
    },
    ActionDoc {
        name: "move-window-to-workspace-index",
        args: "N",
        description: "send the focused window to workspace N (0-based)",
    },
    ActionDoc {
        name: "focus-output",
        args: "ID",
        description: "focus an output by id (see `outputs`)",
    },
    ActionDoc {
        name: "move-window-to-output",
        args: "ID",
        description: "send the focused window to an output by id",
    },
    ActionDoc {
        name: "focus-output-index",
        args: "N",
        description: "focus the Nth output (0-based, creation order)",
    },
    ActionDoc {
        name: "move-window-to-output-index",
        args: "N",
        description: "send the focused window to the Nth output",
    },
    ActionDoc {
        name: "focus-output-left",
        args: "(none)",
        description: "focus the output to the left",
    },
    ActionDoc {
        name: "focus-output-right",
        args: "(none)",
        description: "focus the output to the right",
    },
    ActionDoc {
        name: "move-window-to-output-left",
        args: "(none)",
        description: "send the focused window to the output on the left",
    },
    ActionDoc {
        name: "move-window-to-output-right",
        args: "(none)",
        description: "send the focused window to the output on the right",
    },
    ActionDoc {
        name: "cycle-column-width",
        args: "(none)",
        description: "step the focused column through its widths",
    },
    ActionDoc {
        name: "set-column-width",
        args: "N",
        description: "set the focused column's width preset by index",
    },
    ActionDoc {
        name: "toggle-fullscreen",
        args: "(none)",
        description: "toggle fullscreen on the focused window",
    },
    ActionDoc {
        name: "set-fullscreen",
        args: "ID on|off",
        description: "set fullscreen on a window by id",
    },
    ActionDoc {
        name: "toggle-maximize",
        args: "(none)",
        description: "toggle maximize on the focused window",
    },
    ActionDoc {
        name: "set-maximized",
        args: "ID on|off",
        description: "set maximize on a window by id",
    },
    ActionDoc {
        name: "toggle-floating",
        args: "(none)",
        description: "toggle floating on the focused window",
    },
    ActionDoc {
        name: "set-floating",
        args: "ID on|off",
        description: "set floating on a window by id",
    },
    ActionDoc {
        name: "toggle-floating-focus",
        args: "(none)",
        description: "toggle focus between floating and tiled windows",
    },
    ActionDoc {
        name: "move-floating",
        args: "ID X Y",
        description: "move a floating window to logical coordinates",
    },
    ActionDoc {
        name: "resize-floating",
        args: "ID WIDTH HEIGHT",
        description: "resize a floating window",
    },
    ActionDoc {
        name: "close",
        args: "(none)",
        description: "close the focused window",
    },
    ActionDoc {
        name: "spawn",
        args: "COMMAND...",
        description: "run a program (the rest of the line is the command)",
    },
    ActionDoc {
        name: "quit",
        args: "(none)",
        description: "quit the compositor",
    },
];

/// Exit codes both clients use: 0 for success (help included), 1 when the
/// request ran and failed, 2 when the invocation itself was wrong.
pub const EXIT_CODES: &[(i32, &str)] = &[
    (
        0,
        "success: the reply is on stdout (help and --version count)",
    ),
    (
        1,
        "the request failed: no daemon, a refused request, a failed write",
    ),
    (
        2,
        "usage error: an unknown verb, flag or value (the error names it)",
    ),
];

/// Environment both clients read.
pub const ENVIRONMENT: &[(&str, &str)] = &[
    (
        "SCOOT_SOCKET",
        "the IPC socket's path; the default is $XDG_RUNTIME_DIR/scoot.sock",
    ),
    (
        "XDG_RUNTIME_DIR",
        "where the default socket lives; missing is a one-line startup error",
    ),
];

/// The topics `help [TOPIC]` takes, with what each prints.
pub const TOPICS: &[(&str, &str)] = &[
    (
        "requests",
        "every request verb with its syntax and one example",
    ),
    ("actions", "every layout action with its argument shape"),
    ("exit-codes", "what each exit code means"),
    ("environment", "the environment variables the client reads"),
];

/// A `help [TOPIC]` page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    Requests,
    Actions,
    ExitCodes,
    Environment,
}

impl Topic {
    /// Parses a topic name; `None` is "no such topic".
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "requests" => Some(Self::Requests),
            "actions" => Some(Self::Actions),
            "exit-codes" => Some(Self::ExitCodes),
            "environment" => Some(Self::Environment),
            _ => None,
        }
    }

    /// Every topic name, for `did you mean` and the JSON.
    pub fn names() -> impl Iterator<Item = &'static str> {
        TOPICS.iter().map(|(name, _)| *name)
    }
}

/// The closest candidate to `input`, if it is close enough to be a typo
/// rather than a guess. Plain Levenshtein over chars; the bar is about a
/// quarter of the longer word (at least 1, at most 3), so `windwos` finds
/// `windows` while `--bogus` finds nothing to guess. An exact match is never
/// a suggestion.
pub fn suggest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;
    for candidate in candidates {
        if candidate == input {
            continue;
        }
        let distance = levenshtein(input, candidate);
        if best.is_none_or(|(_, d)| distance < d) {
            best = Some((candidate, distance));
        }
    }
    let (candidate, distance) = best?;
    let longest = input.chars().count().max(candidate.chars().count());
    let allowance = (longest / 4 + 1).min(3).max(1);
    (distance <= allowance && distance < longest).then_some(candidate)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitution = prev[j] + usize::from(ca != cb);
            current[j + 1] = (prev[j + 1] + 1).min(current[j] + 1).min(substitution);
        }
        std::mem::swap(&mut prev, &mut current);
    }
    prev[b.len()]
}

/// The full `--help` text: the prose grammar blocks plus the sections
/// rendered from the tables below. `binary` is `scootctl` or `scoot msg`,
/// so the examples name the binary being read; `version` says whether that
/// front-end answers `--version` (`scootctl` does, `scoot msg` does not --
/// there it would be a request verb, and is refused as one).
pub fn usage(binary: &str, requests_help: &str, actions_help: &str, version: bool) -> String {
    let mut text = String::new();
    text.push_str(&format!(
        "{binary} -- remote-control client for the scoot Wayland compositor\n\
         \n\
         USAGE:\n\
         \x20   {binary} REQUEST\n"
    ));
    if version {
        text.push_str(&format!("\x20   {binary} --version\n"));
    }
    text.push_str(&format!(
        "\x20   {binary} --help [--json]\n\
         \x20   {binary} help [TOPIC|VERB|--json]\n\
         \n\
         REQUESTS:\n\
         \x20   {requests_help}\n\
         ACTIONS:\n\
         \x20   {actions_help}\n"
    ));
    text.push_str("EXAMPLES:\n");
    for request in REQUESTS.iter().take(6) {
        let example = request.example.replace("scootctl", binary);
        text.push_str(&format!(
            "    {example}\n        {reply}\n",
            reply = request.reply
        ));
    }
    text.push_str("    -- and `help <verb>` (e.g. `help screenshot`) prints one verb's row\n");
    text.push_str("\nEXIT CODES:\n");
    for (code, meaning) in EXIT_CODES {
        text.push_str(&format!("    {code}  {meaning}\n"));
    }
    text.push_str("\nENVIRONMENT:\n");
    for (name, description) in ENVIRONMENT {
        text.push_str(&format!("    {name}  {description}\n"));
    }
    text.push_str(&format!(
        "\nSEE ALSO:\n\
         \x20   `help requests`, `help actions`, `help exit-codes`, `help environment`\n\
         \x20   docs: {DOCS_URL}/ipc.md\n\
         \x20   (published with /llms.txt once the docs site lands)\n"
    ));
    text
}

/// One topic's page: the rows it names, rendered plain. `binary` names the
/// front-end, so the examples read `scoot msg ...` under the alias.
pub fn topic_text(topic: Topic, binary: &str) -> String {
    let mut text = String::new();
    match topic {
        Topic::Requests => {
            text.push_str("REQUESTS:\n");
            for request in REQUESTS {
                text.push_str(&format!(
                    "    {}\n        {}\n        e.g. `{}`\n",
                    request.syntax,
                    request.description,
                    request.example.replace("scootctl", binary)
                ));
            }
        }
        Topic::Actions => {
            text.push_str("ACTIONS:\n");
            for action in ACTIONS {
                text.push_str(&format!(
                    "    {} {}\n        {}\n",
                    action.name, action.args, action.description
                ));
            }
        }
        Topic::ExitCodes => {
            text.push_str("EXIT CODES:\n");
            for (code, meaning) in EXIT_CODES {
                text.push_str(&format!("    {code}  {meaning}\n"));
            }
        }
        Topic::Environment => {
            text.push_str("ENVIRONMENT:\n");
            for (name, description) in ENVIRONMENT {
                text.push_str(&format!("    {name}  {description}\n"));
            }
        }
    }
    text
}

/// One verb's row, for `help <verb>`: its syntax, description, example and
/// reply shape. `None` when no verb is named that.
/// One verb's row, for `help <verb>`: its syntax, description, one example
/// and its reply shape. `binary` names the front-end, so the example reads
/// `scoot msg ...` under the alias. `None` when no verb is named that.
pub fn verb_text(verb: &str, binary: &str) -> Option<String> {
    REQUESTS
        .iter()
        .find(|request| request.verb == verb)
        .map(|request| {
            format!(
                "{}:\n    {}\n    {} -- e.g. `{}`\n    Reply: {}\n",
                request.verb,
                request.syntax,
                request.description,
                request.example.replace("scootctl", binary),
                request.reply
            )
        })
}

/// The `--help --json` document as a value, so other binaries (notably
/// `scoot msg`) can embed the client surface in their own document.
/// `binary` names the front-end being described.
pub fn json_value(binary: &str, version: bool) -> Value {
    let mut root = Map::new();
    root.insert("schema_version".into(), Value::from(SCHEMA_VERSION));
    root.insert("binary".into(), Value::from(binary));
    root.insert(
        "about".into(),
        Value::from("remote-control client for the scoot Wayland compositor"),
    );
    root.insert(
        "usage".into(),
        Value::from({
            let mut lines = vec![Value::from(format!("{binary} REQUEST"))];
            if version {
                lines.push(Value::from(format!("{binary} --version")));
            }
            lines.push(Value::from(format!("{binary} --help [--json]")));
            lines.push(Value::from(format!("{binary} help [TOPIC|VERB|--json]")));
            lines
        }),
    );
    root.insert(
        "requests".into(),
        Value::from(
            REQUESTS
                .iter()
                .map(|request| {
                    let mut entry = Map::new();
                    entry.insert("name".into(), Value::from(request.verb));
                    entry.insert("syntax".into(), Value::from(request.syntax));
                    entry.insert("description".into(), Value::from(request.description));
                    entry.insert("example".into(), Value::from(request.example));
                    entry.insert("reply".into(), Value::from(request.reply));
                    Value::Object(entry)
                })
                .collect::<Vec<_>>(),
        ),
    );
    root.insert(
        "actions".into(),
        Value::from(
            ACTIONS
                .iter()
                .map(|action| {
                    let mut entry = Map::new();
                    entry.insert("name".into(), Value::from(action.name));
                    entry.insert("args".into(), Value::from(action.args));
                    entry.insert("description".into(), Value::from(action.description));
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
        "topics".into(),
        Value::from(
            TOPICS
                .iter()
                .map(|(name, _)| Value::from(*name))
                .collect::<Vec<_>>(),
        ),
    );
    root.insert("docs".into(), Value::from(format!("{DOCS_URL}/ipc.md")));
    Value::Object(root)
}

/// The `--help --json` document, pretty-printed.
pub fn json(binary: &str, version: bool) -> String {
    serde_json::to_string_pretty(&json_value(binary, version)).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_request_verb_is_in_the_prose_and_in_the_json() {
        // The drift pin, both directions: a verb added to one form without
        // the other fails here.
        let prose = format!("{}{}", crate::cli::REQUESTS_HELP, crate::cli::ACTIONS_HELP);
        let document = json_value("scootctl", true);
        let names: Vec<&str> = document["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        for request in REQUESTS {
            assert!(
                prose.contains(request.syntax),
                "`{}` is in the table but not the prose",
                request.verb
            );
            assert!(
                names.contains(&request.verb),
                "`{}` is in the table but not the JSON",
                request.verb
            );
        }
        let actions: Vec<&str> = document["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        for action in ACTIONS {
            assert!(
                prose.contains(action.name),
                "`{}` is in the table but not the prose",
                action.name
            );
            assert!(
                actions.contains(&action.name),
                "`{}` is in the table but not the JSON",
                action.name
            );
        }
    }

    #[test]
    fn the_full_text_covers_every_row_and_section_in_order() {
        let text = usage(
            "scootctl",
            crate::cli::REQUESTS_HELP,
            crate::cli::ACTIONS_HELP,
            true,
        );
        for request in REQUESTS {
            assert!(text.contains(request.syntax), "`{}` missing", request.verb);
        }
        for action in ACTIONS {
            assert!(text.contains(action.name), "`{}` missing", action.name);
        }
        let sections = [
            "USAGE:",
            "REQUESTS:",
            "ACTIONS:",
            "EXAMPLES:",
            "EXIT CODES:",
        ];
        let mut cursor = 0;
        for section in sections {
            let found = text[cursor..].find(section).unwrap_or_else(|| {
                panic!("`{section}` missing or out of order");
            });
            cursor += found + section.len();
        }
        for tail in ["ENVIRONMENT:", "SEE ALSO:"] {
            assert!(
                text[cursor..].contains(tail),
                "`{tail}` missing or out of order"
            );
        }
    }

    #[test]
    fn help_text_stays_plain_short_and_machine_shaped() {
        // The contract: plain text (no color escapes), wrapped under 100
        // columns, and the JSON parses with the versioned schema.
        let text = usage(
            "scootctl",
            crate::cli::REQUESTS_HELP,
            crate::cli::ACTIONS_HELP,
            true,
        );
        assert!(!text.contains('\x1b'), "help must not carry color escapes");
        for line in text.lines() {
            assert!(line.chars().count() < 100, "line over 99 columns: `{line}`");
        }
        let document: Value = serde_json::from_str(&json("scootctl", true)).unwrap();
        assert_eq!(document["schema_version"], Value::from(SCHEMA_VERSION));
        assert_eq!(
            document["requests"].as_array().unwrap().len(),
            REQUESTS.len()
        );
        assert_eq!(document["actions"].as_array().unwrap().len(), ACTIONS.len());
    }

    #[test]
    fn every_topic_parses_and_has_a_page() {
        for (name, _) in TOPICS {
            let topic = Topic::parse(name).unwrap_or_else(|| panic!("`{name}` does not parse"));
            assert!(
                !topic_text(topic, "scootctl").is_empty(),
                "`{name}` has no page"
            );
        }
        assert_eq!(Topic::parse("frobnicate"), None);
    }

    #[test]
    fn every_verb_has_its_own_row() {
        for request in REQUESTS {
            let row = verb_text(request.verb, "scootctl")
                .unwrap_or_else(|| panic!("`{}` has no row", request.verb));
            assert!(row.contains(request.syntax));
            assert!(row.contains(request.example));
        }
        assert_eq!(verb_text("frobnicate", "scootctl"), None);
    }

    #[test]
    fn typos_find_their_verb_and_garbage_finds_nothing() {
        let verbs: Vec<&str> = REQUESTS.iter().map(|request| request.verb).collect();
        for (typo, verb) in [
            ("subscrib", "subscribe"),
            ("windwos", "windows"),
            ("ouptuts", "outputs"),
            ("screeshot", "screenshot"),
            ("actoin", "action"),
            ("keybaord", "keyboard"),
        ] {
            assert_eq!(suggest(typo, verbs.clone()), Some(verb), "{typo}");
        }
        for garbage in ["xyzzy", "", "q"] {
            assert_eq!(suggest(garbage, verbs.clone()), None, "{garbage}");
        }
    }

    #[test]
    fn action_typos_find_their_action() {
        let names: Vec<&str> = ACTIONS.iter().map(|action| action.name).collect();
        assert_eq!(suggest("focus-colum", names.clone()), Some("focus-column"));
        assert_eq!(
            suggest("togle-fullscreen", names.clone()),
            Some("toggle-fullscreen")
        );
        assert_eq!(suggest("xyzzy", names), None);
    }
}
