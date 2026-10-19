//! The command line's tests. Env-free: every case is pure argument
//! parsing, so no `serial_test`-style exclusion is needed.

use super::*;

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

#[test]
fn list_alone_parses() {
    let args = parse_args(&argv(&["scootlaunch", "--list"])).unwrap();
    assert_eq!(
        args,
        Args {
            list: true,
            all: false,
            lenient: false,
            json: false,
            help: false,
            version: false,
            query: None,
            run_id: None,
            action: None,
            font: None,
            config: None,
            check: false,
            dmenu: false,
            run: false,
            prompt: None,
            prompt_only: None,
            password: false,
            no_run_if_empty: false,
            only_match: false,
            index: false,
            placeholder: None,
            layer: None,
            colors: ColorOverrides::default(),
        }
    );
}

#[test]
fn query_takes_the_next_argument_as_text() {
    let args = parse_args(&argv(&["scootlaunch", "--query", "fire"])).unwrap();
    assert_eq!(args.query, Some("fire".to_string()));
    assert!(!args.list);
}

#[test]
fn query_takes_every_modifier() {
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--query",
        "fire",
        "--all",
        "--lenient",
        "--json",
    ]))
    .unwrap();
    assert_eq!(args.query, Some("fire".to_string()));
    assert!(args.all && args.lenient && args.json);
}

#[test]
fn query_text_may_look_like_a_flag() {
    // TEXT is the next argument, whatever it holds.
    let args = parse_args(&argv(&["scootlaunch", "--query", "--json"])).unwrap();
    assert_eq!(args.query, Some("--json".to_string()));
    assert!(!args.json);
}

#[test]
fn query_without_text_is_a_loud_error() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--query"])),
        Err(Error::NeedsValue("--query".to_string()))
    );
}

#[test]
fn list_and_query_conflict() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--list", "--query", "fire"])),
        Err(Error::ConflictingCommands)
    );
}

#[test]
fn list_with_every_modifier_parses() {
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--list",
        "--all",
        "--lenient",
        "--json",
    ]))
    .unwrap();
    assert!(args.list && args.all && args.lenient && args.json);
}

#[test]
fn bare_invocation_is_the_panel() {
    let args = parse_args(&argv(&["scootlaunch"])).unwrap();
    assert!(!args.list && args.query.is_none() && !args.check && !args.help);
}

#[test]
fn panel_flags_parse() {
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--font",
        "/tmp/a.ttf",
        "--config",
        "/tmp/launch.toml",
        "--lenient",
    ]))
    .unwrap();
    assert_eq!(args.font.as_deref(), Some("/tmp/a.ttf"));
    assert_eq!(args.config.as_deref(), Some("/tmp/launch.toml"));
    assert!(args.lenient);
}

#[test]
fn check_takes_an_optional_config() {
    let args = parse_args(&argv(&["scootlaunch", "--check"])).unwrap();
    assert!(args.check && args.config.is_none());
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--check",
        "--config",
        "/tmp/x.toml",
    ]))
    .unwrap();
    assert!(args.check);
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--check", "--list"])),
        Err(Error::ConflictingCommands)
    );
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--list", "--font", "/tmp/a.ttf"])),
        Err(Error::NeedsCommand("--font".to_string()))
    );
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--font"])),
        Err(Error::NeedsValue("--font".to_string()))
    );
}

#[test]
fn unknown_flags_name_themselves_and_the_supported_list() {
    let error = parse_args(&argv(&["scootlaunch", "--frobnicate"])).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("--frobnicate"), "{text}");
    assert!(text.contains("--list"), "{text}");
    assert!(text.contains("--dmenu"), "{text}");
    assert!(text.contains("--json"), "{text}");
    assert_eq!(
        error,
        Error::UnknownFlag("--frobnicate".to_string()),
        "loud, never silently ignored"
    );
}

#[test]
fn single_dash_unknown_is_unknown() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "-x"])),
        Err(Error::UnknownFlag("-x".to_string()))
    );
}

#[test]
fn flag_with_value_is_not_a_flag() {
    // `--json=true` must not parse as `--json`: a misparsed flag that
    // takes a value would eat the next argument.
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--list", "--json=true"])),
        Err(Error::UnknownFlag("--json=true".to_string()))
    );
}

#[test]
fn positional_arguments_are_refused() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--list", "firefox"])),
        Err(Error::UnexpectedArg("firefox".to_string()))
    );
}

#[test]
fn modifiers_need_a_command() {
    // `--lenient` shapes the panel's own index too, so it stands alone;
    // `--all` and `--json` modify the headless commands only.
    assert!(parse_args(&argv(&["scootlaunch", "--lenient"])).is_ok());
    for flag in ["--all", "--json"] {
        assert_eq!(
            parse_args(&argv(&["scootlaunch", flag])),
            Err(Error::NeedsCommand(flag.to_string())),
            "{flag} without --list or --query"
        );
    }
    // ... but either command satisfies them.
    assert!(parse_args(&argv(&["scootlaunch", "--query", "x", "--json"])).is_ok());
}

#[test]
fn help_and_version_win_over_usage_errors() {
    assert!(
        parse_args(&argv(&["scootlaunch", "--json", "--help"]))
            .unwrap()
            .help
    );
    assert!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--version"]))
            .unwrap()
            .version
    );
}

#[test]
fn short_help_and_version_parse() {
    assert!(parse_args(&argv(&["scootlaunch", "-h"])).unwrap().help);
    assert!(parse_args(&argv(&["scootlaunch", "-V"])).unwrap().version);
}

#[test]
fn repeated_flags_are_idempotent() {
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--list",
        "--list",
        "--json",
        "--json",
    ]))
    .unwrap();
    assert!(args.list && args.json && !args.all);
}

#[test]
fn dmenu_alone_is_the_dmenu_mode() {
    let args = parse_args(&argv(&["scootlaunch", "--dmenu"])).unwrap();
    assert!(args.dmenu && !args.run && !args.list);
    assert!(args.query.is_none() && !args.check);
}

#[test]
fn run_and_its_alias_open_run_mode() {
    assert!(parse_args(&argv(&["scootlaunch", "--run"])).unwrap().run);
    assert!(
        parse_args(&argv(&["scootlaunch", "--list-executables-in-path"]))
            .unwrap()
            .run
    );
}

#[test]
fn dmenu_flags_take_attached_or_next_values() {
    // Every repo call site's shape, attached.
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--dmenu",
        "--prompt=wifi: ",
        "--no-run-if-empty",
        "--only-match",
    ]))
    .unwrap();
    assert_eq!(args.prompt.as_deref(), Some("wifi: "));
    assert!(args.no_run_if_empty && args.only_match);
    // The space form reads the same.
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--dmenu",
        "--prompt",
        "clipboard: ",
        "--placeholder",
        "pick one",
        "--layer",
        "overlay",
    ]))
    .unwrap();
    assert_eq!(args.prompt.as_deref(), Some("clipboard: "));
    assert_eq!(args.placeholder.as_deref(), Some("pick one"));
    assert_eq!(args.layer.as_deref(), Some("overlay"));
    // A value holding `=` rides through whole.
    let args = parse_args(&argv(&["scootlaunch", "--dmenu", "--prompt=a=b"])).unwrap();
    assert_eq!(args.prompt.as_deref(), Some("a=b"));
}

#[test]
fn password_is_a_bare_flag() {
    assert!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--password"]))
            .unwrap()
            .password
    );
    // A value-taking shape is not the flag: `--password=x` cannot
    // silently eat the next argument, and `=x` names itself loudly.
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--password=x"])),
        Err(Error::UnknownFlag("--password=x".to_string()))
    );
}

#[test]
fn colors_parse_as_rrggbbaa() {
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--dmenu",
        "--background-color=1e1e2eff",
        "--text-color=cdd6f4ff",
        "--border-color=f9e2afff",
        "--selection-color=cba6f7ff",
        "--selection-text-color=11111bff",
        "--match-color=f38ba8ff",
        "--prompt-color=a6adc8ff",
    ]))
    .unwrap();
    assert!(args.colors.background.is_some());
    assert!(args.colors.text.is_some());
    assert!(args.colors.prompt.is_some());
    // Only the background may be translucent.
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--text-color=cdd6f480"])),
        Err(Error::InvalidValue {
            flag: "--text-color".to_string(),
            value: "cdd6f480".to_string(),
            expected: "an opaque rrggbbaa (...ff; only --background-color may be translucent)",
        })
    );
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--match-color=xyz"])),
        Err(Error::InvalidValue {
            flag: "--match-color".to_string(),
            value: "xyz".to_string(),
            expected: "rrggbbaa (eight hex digits)",
        })
    );
}

#[test]
fn layer_takes_top_or_overlay() {
    assert!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--layer=top"]))
            .unwrap()
            .layer
            .is_some()
    );
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--layer=bottom"])),
        Err(Error::InvalidValue {
            flag: "--layer".to_string(),
            value: "bottom".to_string(),
            expected: "top or overlay",
        })
    );
}

#[test]
fn commands_conflict() {
    for extra in [
        "--list",
        "--check",
        "--dmenu",
        "--run",
        "--list-executables-in-path",
    ] {
        assert_eq!(
            parse_args(&argv(&["scootlaunch", "--query", "x", extra])),
            Err(Error::ConflictingCommands),
            "{extra} with --query"
        );
    }
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--run"])),
        Err(Error::ConflictingCommands)
    );
    assert_eq!(
        parse_args(&argv(&[
            "scootlaunch",
            "--dmenu",
            "--prompt=a",
            "--prompt-only=b"
        ])),
        Err(Error::ConflictingCommands),
        "two prompts are two commands"
    );
}

#[test]
fn dmenu_flags_need_dmenu() {
    for flag in [
        "--prompt",
        "--prompt-only",
        "--password",
        "--no-run-if-empty",
        "--only-match",
        "--index",
        "--placeholder",
    ] {
        let tail: Vec<&str> = if matches!(flag, "--prompt" | "--prompt-only" | "--placeholder") {
            vec![flag, "x"]
        } else {
            vec![flag]
        };
        let mut words = vec!["scootlaunch"];
        words.extend(tail);
        assert_eq!(
            parse_args(&argv(&words)),
            Err(Error::NeedsCommand(flag.to_string())),
            "{flag} without --dmenu"
        );
    }
    // ... but --dmenu satisfies every one of them.
    assert!(
        parse_args(&argv(&[
            "scootlaunch",
            "--dmenu",
            "--prompt",
            "x",
            "--password",
            "--no-run-if-empty",
            "--only-match",
            "--index",
            "--placeholder",
            "y"
        ]))
        .is_ok()
    );
}

#[test]
fn colors_and_layer_need_a_panel() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--list", "--text-color=cdd6f4ff"])),
        Err(Error::NeedsCommand("--background-color".to_string()))
    );
    // The bare panel, --run and --dmenu all take them (the launcher
    // script passes them to the bare panel too).
    assert!(
        parse_args(&argv(&[
            "scootlaunch",
            "--layer=overlay",
            "--background-color=1e1e2eff"
        ]))
        .is_ok()
    );
    assert!(
        parse_args(&argv(&[
            "scootlaunch",
            "--run",
            "--selection-color=cba6f7ff"
        ]))
        .is_ok()
    );
}

#[test]
fn value_flags_without_a_value_are_loud() {
    for flag in [
        "--prompt",
        "--prompt-only",
        "--placeholder",
        "--layer",
        "--background-color",
        "--font",
        "--config",
    ] {
        assert_eq!(
            parse_args(&argv(&["scootlaunch", "--dmenu", flag])),
            Err(Error::NeedsValue(flag.to_string())),
            "{flag} with nothing after it"
        );
    }
}

#[test]
fn bool_flags_with_equals_are_not_the_flag() {
    // A misparsed value-taking flag must never eat the next argument.
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--dmenu", "--only-match=true"])),
        Err(Error::UnknownFlag("--only-match=true".to_string()))
    );
}

#[test]
fn run_id_takes_a_value_attached_or_next() {
    let args = parse_args(&argv(&["scootlaunch", "--run-id", "firefox.desktop"])).unwrap();
    assert_eq!(args.run_id.as_deref(), Some("firefox.desktop"));
    assert!(args.action.is_none());
    let args = parse_args(&argv(&["scootlaunch", "--run-id=firefox.desktop"])).unwrap();
    assert_eq!(args.run_id.as_deref(), Some("firefox.desktop"));
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--run-id",
        "firefox.desktop",
        "--action",
        "new-window",
    ]))
    .unwrap();
    assert_eq!(args.action.as_deref(), Some("new-window"));
    let args = parse_args(&argv(&[
        "scootlaunch",
        "--run-id=firefox.desktop",
        "--action=new-window",
    ]))
    .unwrap();
    assert_eq!(args.run_id.as_deref(), Some("firefox.desktop"));
    assert_eq!(args.action.as_deref(), Some("new-window"));
    // `--lenient` shapes the headless index too.
    assert!(parse_args(&argv(&["scootlaunch", "--run-id", "x", "--lenient"])).is_ok());
}

#[test]
fn run_id_without_a_value_is_loud() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--run-id"])),
        Err(Error::NeedsValue("--run-id".to_string()))
    );
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--run-id", "x", "--action"])),
        Err(Error::NeedsValue("--action".to_string()))
    );
}

#[test]
fn action_needs_run_id() {
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--action", "new-window"])),
        Err(Error::NeedsCommand("--action".to_string()))
    );
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--list", "--action", "x"])),
        Err(Error::NeedsCommand("--action".to_string()))
    );
}

#[test]
fn run_id_conflicts_with_other_commands() {
    for extra in [
        "--list",
        "--check",
        "--dmenu",
        "--run",
        "--list-executables-in-path",
    ] {
        assert_eq!(
            parse_args(&argv(&["scootlaunch", "--run-id", "x", extra])),
            Err(Error::ConflictingCommands),
            "{extra} with --run-id"
        );
    }
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--query", "x", "--run-id", "y"])),
        Err(Error::ConflictingCommands)
    );
    // `--all` and `--json` modify `--list`/`--query` only: with
    // `--run-id` they name themselves loudly (the output is always one
    // JSON object, hidden entries included).
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--run-id", "x", "--json"])),
        Err(Error::NeedsCommand("--json".to_string()))
    );
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--run-id", "x", "--all"])),
        Err(Error::NeedsCommand("--all".to_string()))
    );
    // Panel options shape no headless command.
    assert_eq!(
        parse_args(&argv(&["scootlaunch", "--run-id", "x", "--font", "a"])),
        Err(Error::NeedsCommand("--font".to_string()))
    );
}
