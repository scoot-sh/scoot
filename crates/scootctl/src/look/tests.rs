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
    assert_eq!(command(&["list"]), Ok(Command::List { names: false }));
    assert_eq!(
        command(&["list", "--names"]),
        Ok(Command::List { names: true })
    );
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
            trust: false,
            no_helix: false,
            no_btop: false,
            from_compositor: false,
            for_secs: None,
            keep: false,
            only_if_generation: None,
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--from-compositor"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            trust: false,
            no_helix: false,
            no_btop: false,
            from_compositor: true,
            for_secs: None,
            keep: false,
            only_if_generation: None,
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
    assert_eq!(command(&["pick"]), Ok(Command::Pick));
    assert_eq!(
        command(&["pick", "extra"]),
        Err(Error::Unknown("extra".to_owned()))
    );
    assert_eq!(command(&["sync"]), Ok(Command::Sync { no_toolkit: false }));
    assert_eq!(
        command(&["sync", "--no-toolkit"]),
        Ok(Command::Sync { no_toolkit: true })
    );
}

#[test]
fn sync_takes_no_name() {
    assert_eq!(
        command(&["sync", "moonrise"]),
        Err(Error::Unknown("moonrise".to_owned()))
    );
}

#[test]
fn validate_and_build_take_a_target_and_an_origin() {
    assert_eq!(
        command(&["validate", "looks/moonrise"]),
        Ok(Command::Validate {
            target: "looks/moonrise".to_owned(),
            origin: Origin::User,
            trust: false,
        })
    );
    assert_eq!(
        command(&["validate", "moonrise", "--origin", "shipped"]),
        Ok(Command::Validate {
            target: "moonrise".to_owned(),
            origin: Origin::Shipped,
            trust: false,
        })
    );
    assert_eq!(
        command(&["build", "looks/moonrise", "--out", "/tmp/out"]),
        Ok(Command::Build {
            target: "looks/moonrise".to_owned(),
            out: PathBuf::from("/tmp/out"),
            origin: Origin::User,
            trust: false,
        })
    );
    assert_eq!(
        command(&["build", "moonrise", "--out=/tmp/out"]),
        Ok(Command::Build {
            target: "moonrise".to_owned(),
            out: PathBuf::from("/tmp/out"),
            origin: Origin::User,
            trust: false,
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
            trust: false,
            no_helix: false,
            no_btop: false,
            from_compositor: false,
            for_secs: None,
            keep: false,
            only_if_generation: None,
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
            trust: false,
            no_helix: false,
            no_btop: false,
            from_compositor: false,
            for_secs: Some(20),
            keep: false,
            only_if_generation: None,
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--for=20"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            trust: false,
            no_helix: false,
            no_btop: false,
            from_compositor: false,
            for_secs: Some(20),
            keep: false,
            only_if_generation: None,
        })
    );
    assert_eq!(
        command(&["apply", "--keep"]),
        Ok(Command::Apply {
            name: None,
            no_toolkit: false,
            trust: false,
            no_helix: false,
            no_btop: false,
            from_compositor: false,
            for_secs: None,
            keep: true,
            only_if_generation: None,
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--keep"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            trust: false,
            no_helix: false,
            no_btop: false,
            from_compositor: false,
            for_secs: None,
            keep: true,
            only_if_generation: None,
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
fn apply_only_if_generation_parses() {
    // The compositor's timed-preview revert passes the generation its
    // preview wrote; a newer choice refuses as superseded, never undone.
    assert_eq!(
        command(&["apply", "moonrise", "--only-if-generation", "7"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            trust: false,
            no_helix: false,
            no_btop: false,
            from_compositor: false,
            for_secs: None,
            keep: false,
            only_if_generation: Some(7),
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--only-if-generation=7"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            trust: false,
            no_helix: false,
            no_btop: false,
            from_compositor: false,
            for_secs: None,
            keep: false,
            only_if_generation: Some(7),
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--only-if-generation"]),
        Err(Error::Missing("a generation after --only-if-generation"))
    );
    assert_eq!(
        command(&["apply", "moonrise", "--only-if-generation", "soon"]),
        Err(Error::Invalid {
            what: "--only-if-generation",
            value: "soon".to_owned(),
        })
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
            trust: false,
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
        "--names",
    ] {
        assert!(text.contains(flag), "--help hides `{flag}`");
    }
}

#[test]
fn apply_trust_and_editor_opt_outs_parse() {
    assert_eq!(
        command(&["apply", "moonrise", "--trust"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            trust: true,
            no_helix: false,
            no_btop: false,
            from_compositor: false,
            for_secs: None,
            keep: false,
            only_if_generation: None,
        })
    );
    assert_eq!(
        command(&["apply", "moonrise", "--no-helix", "--no-btop"]),
        Ok(Command::Apply {
            name: Some("moonrise".to_owned()),
            no_toolkit: false,
            trust: false,
            no_helix: true,
            no_btop: true,
            from_compositor: false,
            for_secs: None,
            keep: false,
            only_if_generation: None,
        })
    );
    assert_eq!(
        command(&["validate", "looks/moonrise", "--trust"]),
        Ok(Command::Validate {
            target: "looks/moonrise".to_owned(),
            origin: Origin::User,
            trust: true,
        })
    );
    assert_eq!(
        command(&[
            "build",
            "looks/moonrise",
            "--out",
            "/tmp/out",
            "--origin",
            "installed",
            "--trust"
        ]),
        Ok(Command::Build {
            target: "looks/moonrise".to_owned(),
            out: PathBuf::from("/tmp/out"),
            origin: Origin::Installed,
            trust: true,
        })
    );
}

#[test]
fn new_takes_a_name_and_a_base() {
    assert_eq!(
        command(&["new", "my-look", "--from", "moonrise"]),
        Ok(Command::New {
            name: "my-look".to_owned(),
            base: "moonrise".to_owned(),
        })
    );
    assert_eq!(
        command(&["new", "my-look", "--from=moonrise"]),
        Ok(Command::New {
            name: "my-look".to_owned(),
            base: "moonrise".to_owned(),
        })
    );
    assert_eq!(
        command(&["new", "my-look"]),
        Err(Error::Missing("a base look after --from"))
    );
    assert_eq!(
        command(&["new", "--from", "moonrise"]),
        Err(Error::Missing("a look name"))
    );
    assert_eq!(
        command(&["new", "LOUD", "--from", "moonrise"]),
        Err(Error::Invalid {
            what: "look name",
            value: "LOUD".to_owned(),
        })
    );
    assert_eq!(
        command(&["new", "my-look", "--from", "LOUD"]),
        Err(Error::Invalid {
            what: "look name",
            value: "LOUD".to_owned(),
        })
    );
}

#[test]
fn watch_takes_a_directory() {
    assert_eq!(
        command(&["watch", "user-looks/mine"]),
        Ok(Command::Watch {
            dir: PathBuf::from("user-looks/mine"),
            no_toolkit: false,
            origin: Origin::User,
        })
    );
    assert_eq!(
        command(&[
            "watch",
            "user-looks/mine",
            "--no-toolkit",
            "--origin",
            "shipped"
        ]),
        Ok(Command::Watch {
            dir: PathBuf::from("user-looks/mine"),
            no_toolkit: true,
            origin: Origin::Shipped,
        })
    );
    assert_eq!(command(&["watch"]), Err(Error::Missing("a look directory")));
    assert_eq!(
        command(&["watch", "user-looks/mine", "--origin", "nix"]),
        Err(Error::Invalid {
            what: "--origin",
            value: "nix".to_owned(),
        })
    );
}

#[test]
fn preview_shows_or_captures() {
    assert_eq!(
        command(&["preview"]),
        Ok(Command::Preview {
            target: None,
            capture: false,
            out: None,
            width: 1600,
            height: 1000,
            timeout_secs: 60,
            origin: Origin::User,
        })
    );
    assert_eq!(
        command(&[
            "preview",
            "moonrise",
            "--capture",
            "--width",
            "800",
            "--height=600",
            "--timeout",
            "30"
        ]),
        Ok(Command::Preview {
            target: Some("moonrise".to_owned()),
            capture: true,
            out: None,
            width: 800,
            height: 600,
            timeout_secs: 30,
            origin: Origin::User,
        })
    );
    assert_eq!(
        command(&["preview", "./mine", "--capture", "--out", "/tmp/shot.png"]),
        Ok(Command::Preview {
            target: Some("./mine".to_owned()),
            capture: true,
            out: Some(PathBuf::from("/tmp/shot.png")),
            width: 1600,
            height: 1000,
            timeout_secs: 60,
            origin: Origin::User,
        })
    );
    assert_eq!(
        command(&["preview", "LOUD"]),
        Err(Error::Invalid {
            what: "look name",
            value: "LOUD".to_owned(),
        })
    );
    assert_eq!(
        command(&["preview", "moonrise", "--width", "0"]),
        Err(Error::Invalid {
            what: "--width",
            value: "0".to_owned(),
        })
    );
    assert_eq!(
        command(&["preview", "moonrise", "--timeout", "0"]),
        Err(Error::Invalid {
            what: "--timeout",
            value: "0".to_owned(),
        })
    );
}
