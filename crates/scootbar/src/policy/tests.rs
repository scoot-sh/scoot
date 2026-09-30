use super::*;
use crate::bar::Edge;

fn named(names: &[&str]) -> Select {
    Select::Named(names.iter().map(|n| (*n).to_owned()).collect())
}

fn over(name: &str) -> Override {
    Override {
        name: name.to_owned(),
        bar: BarOverride::default(),
        modules: None,
    }
}

#[test]
fn all_selects_every_output_named_or_not() {
    let policy = Policy::default();
    assert!(policy.selects(Some("DP-1")));
    assert!(policy.selects(None));
}

#[test]
fn a_list_selects_exactly_its_names() {
    let policy = Policy {
        select: named(&["DP-1", "eDP-1"]),
        overrides: Vec::new(),
    };
    assert!(policy.selects(Some("DP-1")));
    assert!(policy.selects(Some("eDP-1")));
    assert!(!policy.selects(Some("DP-2")));
    // Byte for byte: no case folding, no prefix.
    assert!(!policy.selects(Some("dp-1")));
    assert!(!policy.selects(Some("DP-1 ")));
    assert!(!policy.selects(Some("DP")));
    // Never named: only `all` takes it.
    assert!(!policy.selects(None));
}

#[test]
fn a_hostile_list_is_refused() {
    let check = |select| {
        Policy {
            select,
            overrides: Vec::new(),
        }
        .check()
    };
    assert_eq!(check(named(&[])), Err(PolicyError::NoOutputs));
    assert_eq!(
        check(named(&["DP-1", "DP-1"])),
        Err(PolicyError::Twice("DP-1".into()))
    );
    assert_eq!(
        check(named(&[""])),
        Err(PolicyError::Name(NameError::Empty))
    );
    assert_eq!(
        check(named(&["a\u{1b}[31m"])),
        Err(PolicyError::Name(NameError::Control))
    );
    assert_eq!(
        check(named(&[&"x".repeat(MAX_NAME + 1)])),
        Err(PolicyError::Name(NameError::TooLong))
    );
    assert!(check(named(&[&"x".repeat(MAX_NAME)])).is_ok());
    let many: Vec<String> = (0..=MAX_OUTPUTS).map(|n| format!("O-{n}")).collect();
    assert_eq!(
        check(Select::Named(many.clone())),
        Err(PolicyError::TooManyOutputs)
    );
    assert!(check(Select::Named(many[..MAX_OUTPUTS].to_vec())).is_ok());
    assert!(check(Select::All).is_ok());
}

#[test]
fn an_override_for_an_output_the_list_leaves_out_is_refused() {
    let policy = Policy {
        select: named(&["DP-1"]),
        overrides: vec![over("HDMI-A-1")],
    };
    assert_eq!(
        policy.check(),
        Err(PolicyError::Unselected("HDMI-A-1".into()))
    );
    // Under `all` an override for an output not plugged in is fine.
    let policy = Policy {
        select: Select::All,
        overrides: vec![over("HDMI-A-1")],
    };
    assert_eq!(policy.check(), Ok(()));
}

#[test]
fn an_override_changes_only_what_it_names() {
    let bar = Bar::default();
    let layout = Layout::default();
    let policy = Policy {
        select: Select::All,
        overrides: vec![Override {
            name: "eDP-1".into(),
            bar: BarOverride {
                height: Some(40),
                edge: Some(Edge::Bottom),
                ..Default::default()
            },
            modules: Some(Sections {
                right: vec!["clock"],
                ..Default::default()
            }),
        }],
    };
    let plain = policy.resolve(Some("DP-1"), &bar, &layout);
    assert_eq!(plain.bar, bar);
    assert_eq!(plain.layout, layout);
    assert!(plain.selected);
    let own = policy.resolve(Some("eDP-1"), &bar, &layout);
    assert_eq!(own.bar.height, 40);
    assert_eq!(own.bar.edge, Edge::Bottom);
    assert_eq!(own.bar.layer, bar.layer);
    assert_eq!(own.bar.margin, bar.margin);
    assert_eq!(own.layout.right, ["clock"]);
    // A section not given is empty, and the gaps are the shared ones.
    assert!(own.layout.center.is_empty());
    assert_eq!(own.layout.padding, layout.padding);
    // An unnamed output gets the shared values, never an override.
    assert_eq!(policy.resolve(None, &bar, &layout).bar, bar);
}

#[test]
fn the_start_list_has_every_module_once() {
    let layout = Layout {
        center: vec!["clock"],
        ..Layout::default()
    };
    let policy = Policy {
        select: Select::All,
        overrides: vec![
            Override {
                modules: Some(Sections {
                    left: vec!["clock"],
                    ..Default::default()
                }),
                ..over("A")
            },
            Override {
                modules: Some(Sections {
                    right: vec!["clock"],
                    ..Default::default()
                }),
                ..over("B")
            },
        ],
    };
    let all = policy.to_start(&layout);
    assert_eq!(all.placed().count(), 1);
    assert_eq!(all.center, ["clock"]);
    // Placed only in an override: still started.
    let policy = Policy {
        select: Select::All,
        overrides: vec![Override {
            modules: Some(Sections {
                right: vec!["clock"],
                ..Default::default()
            }),
            ..over("A")
        }],
    };
    let none = Layout {
        center: Vec::new(),
        ..Layout::default()
    };
    assert_eq!(policy.to_start(&none).right, ["clock"]);
}
