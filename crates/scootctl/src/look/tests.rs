//! `scoot look` parsing tests: verbs, flags anywhere, and refusals.

use std::path::PathBuf;

use scoot_look::Origin;

use super::{Command, parse_look};
use crate::cli::Error;

fn parse(args: &[&str]) -> Result<super::Look, Error> {
    parse_look(args.iter().map(|arg| (*arg).to_owned()))
}

fn command(args: &[&str]) -> Result<Command, Error> {
    parse(args).map(|look| look.command)
}

#[test]
fn verbs_parse_bare() {
    assert_eq!(command(&["list"]), Ok(Command::List));
    assert_eq!(command(&["current"]), Ok(Command::Current));
    assert_eq!(
        command(&["show", "moonrise"]),
        Ok(Command::Show {
            name: "moonrise".to_owned()
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--no-toolkit"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: true,
            from_compositor: false,
            for_secs: None,
            keep: false,
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--from-compositor"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            from_compositor: true,
            for_secs: None,
            keep: false,
        })
    );
    assert_eq!(command(&["toolkit"]), Ok(Command::Toolkit { action: None }));
    assert_eq!(
        command(&["toolkit", "off"]),
        Ok(Command::Toolkit {
            action: Some(false)
        })
    );
    assert_eq!(
        command(&["toolkit", "on"]),
        Ok(Command::Toolkit { action: Some(true) })
    );
}

#[test]
fn validate_and_build_take_a_target_and_an_origin() {
    assert_eq!(
        command(&["validate", "looks/moonrise"]),
        Ok(Command::Validate {
            target: "looks/moonrise".to_owned(),
            origin: Origin::User,
        })
    );
    assert_eq!(
        command(&["validate", "moonrise", "--origin", "shipped"]),
        Ok(Command::Validate {
            target: "moonrise".to_owned(),
            origin: Origin::Shipped,
        })
    );
    assert_eq!(
        command(&["build", "looks/moonrise", "--out", "/tmp/out"]),
        Ok(Command::Build {
            target: "looks/moonrise".to_owned(),
            out: PathBuf::from("/tmp/out"),
            origin: Origin::User,
        })
    );
    assert_eq!(
        command(&["build", "moonrise", "--out=/tmp/out"]),
        Ok(Command::Build {
            target: "moonrise".to_owned(),
            out: PathBuf::from("/tmp/out"),
            origin: Origin::User,
        })
    );
    assert_eq!(
        parse(&["build", "moonrise"]),
        Err(Error::Missing("a directory after --out"))
    );
}

#[test]
fn json_state_and_socket_ride_anywhere() {
    let look =
        parse(&["--json", "list", "--state", "/s", "--socket=/sock"]).expect("flags anywhere");
    assert!(look.json);
    assert_eq!(look.state, Some(PathBuf::from("/s")));
    assert_eq!(look.socket, Some(PathBuf::from("/sock")));
    let look = parse(&["apply", "moonrise", "--json"]).expect("a trailing flag");
    assert!(look.json);
    assert_eq!(
        look.command,
        Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            from_compositor: false,
            for_secs: None,
            keep: false,
        }
    );
}

#[test]
fn cycle_verbs_parse_bare() {
    use super::CycleStep;
    assert_eq!(
        command(&["next"]),
        Ok(Command::Cycle {
            step: CycleStep::Next
        })
    );
    assert_eq!(
        command(&["prev"]),
        Ok(Command::Cycle {
            step: CycleStep::Prev
        })
    );
    assert_eq!(
        command(&["random"]),
        Ok(Command::Cycle {
            step: CycleStep::Random
        })
    );
    assert_eq!(
        command(&["next", "extra"]),
        Err(Error::Unknown("extra".to_owned()))
    );
}

#[test]
fn apply_for_and_keep_parse() {
    assert_eq!(
        command(&["apply", "moonrise", "--for", "20"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            from_compositor: false,
            for_secs: Some(20),
            keep: false,
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--for=20"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            from_compositor: false,
            for_secs: Some(20),
            keep: false,
        })
    );
    assert_eq!(
        command(&["apply", "--keep"]),
        Ok(Command::Apply {
            name: None,
            no_toolkit: false,
            from_compositor: false,
            for_secs: None,
            keep: true,
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--keep"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            from_compositor: false,
            for_secs: None,
            keep: true,
        })
    );
    // A preview that is kept needs no timer.
    assert!(command(&["apply", "moonrise", "--for", "20", "--keep"]).is_err());
    // Zero seconds would revert before the swap lands.
    assert_eq!(
        command(&["apply", "moonrise", "--for", "0"]),
        Err(Error::Invalid {
            what: "--for",
            value: "0".to_owned(),
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--for", "soon"]),
        Err(Error::Invalid {
            what: "--for",
            value: "soon".to_owned(),
        })
    );
    assert_eq!(
        command(&["apply", "--for", "20"]),
        Err(Error::Missing("a look name"))
    );
}

#[test]
fn unknown_verbs_teach_and_bad_names_refuse() {
    assert_eq!(
        command(&["lis"]),
        Err(Error::Hinted {
            kind: "look command",
            what: "lis".to_owned(),
            suggestion: "list".to_owned(),
            topic: "scoot look --help",
        })
    );
    assert_eq!(
        command(&["show", "../evil"]),
        Err(Error::Invalid {
            what: "look name",
            value: "../evil".to_owned(),
        })
    );
    assert_eq!(
        command(&["apply", "LOUD"]),
        Err(Error::Invalid {
            what: "look name",
            value: "LOUD".to_owned(),
        })
    );
    assert_eq!(command(&["show"]), Err(Error::Missing("a look name")));
    assert_eq!(command(&["apply"]), Err(Error::Missing("a look name")));
    assert_eq!(command(&[]), Err(Error::Missing("a look command")));
    assert_eq!(
        command(&["toolkit", "maybe"]),
        Err(Error::Invalid {
            what: "toolkit action",
            value: "maybe".to_owned(),
        })
    );
    assert_eq!(
        command(&["validate", "moonrise", "--origin", "nix"]),
        Err(Error::Invalid {
            what: "--origin",
            value: "nix".to_owned(),
        })
    );
    // Slashless non-names refuse at parse, like `show` and `apply`.
    assert_eq!(
        command(&["validate", "LOUD"]),
        Err(Error::Invalid {
            what: "look name",
            value: "LOUD".to_owned(),
        })
    );
    assert_eq!(
        command(&["build", "../evil", "--out", "/tmp/out"]),
        Ok(Command::Build {
            target: "../evil".to_owned(),
            out: PathBuf::from("/tmp/out"),
            origin: Origin::User,
        })
    );
    assert_eq!(
        command(&["list", "extra"]),
        Err(Error::Unknown("extra".to_owned()))
    );
}

#[test]
fn the_usage_names_every_verb() {
    let text = super::usage();
    for verb in super::COMMANDS {
        assert!(text.contains(verb), "--help hides `{verb}`");
    }
    for flag in [
        "--json",
        "--state",
        "--socket",
        "--no-toolkit",
        "--for",
        "--keep",
        "--out",
        "--origin",
    ] {
        assert!(text.contains(flag), "--help hides `{flag}`");
    }
}
