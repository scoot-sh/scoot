//! The tables that define modules by name: what each takes, how a layout
//! places them, and that every refusal names its dotted key.

use super::tests::read;
use crate::action::{Action, Trigger};
use crate::modules::custom::{Kind, MAX_EXEC};
use crate::modules::payload::Format;

fn err(text: &str) -> String {
    read(text).unwrap_err().to_string()
}

const ALL_THREE: &str = r#"
left = ["go", "weather", "status"]

[button.go]
text = "Go"
icon = "x"
margin = 4
on-click = { exec = ["true"] }

[exec.weather]
command = ["sh", "-c", "echo hi"]
format = "json"
placeholder = "..."
on-scroll-up = { scoot = "quit" }

[push.status]
placeholder = "ok"
"#;

#[test]
fn the_three_kinds_are_read_and_placed_by_name() {
    let config = read(ALL_THREE).unwrap();
    assert_eq!(config.layout.left, ["go", "weather", "status"]);
    let kinds: Vec<(&str, &str)> = config
        .modules
        .custom
        .iter()
        .map(|c| (c.id, c.kind.name()))
        .collect();
    // Defined in a fixed order by kind (button, push, exec), each kind's in
    // name order: the placement is the lists', not this.
    assert_eq!(
        kinds,
        [("go", "button"), ("status", "push"), ("weather", "exec")]
    );
    let Kind::Exec(exec) = &config.modules.custom[2].kind else {
        panic!()
    };
    assert_eq!(exec.command, ["sh", "-c", "echo hi"]);
    assert_eq!(exec.format, Format::Json);
    assert_eq!(exec.placeholder, "...");
    let Kind::Button(button) = &config.modules.custom[0].kind else {
        panic!()
    };
    assert_eq!(button.text, "Go");
    assert!(button.icon.is_some());
    // The margin and the bindings are by name.
    assert_eq!(config.layout.margin_of("go"), 4);
    assert!(matches!(
        config.modules.bindings_of("go").get(Trigger::Click),
        Some(Action::Exec(_))
    ));
    assert!(matches!(
        config.modules.bindings_of("weather").get(Trigger::ScrollUp),
        Some(Action::Scoot(_))
    ));
    assert!(config.modules.bindings_of("status").is_empty());
}

#[test]
fn a_name_is_the_same_str_on_every_read() {
    let a = read(ALL_THREE).unwrap();
    let b = read(ALL_THREE).unwrap();
    assert_eq!(a, b, "equal files read equal (a reload compares)");
    assert!(std::ptr::eq(a.layout.left[0], b.layout.left[0]));
}

#[test]
fn a_table_no_list_places_is_defined_but_not_started() {
    let config = read("center = []\n[push.idle]\n").unwrap();
    assert_eq!(config.modules.custom.len(), 1);
    let started = crate::modules::start(
        &config.layout,
        &config.modules,
        &mut Vec::new(),
        &mut |_, _| {},
    );
    assert!(started.is_empty());
}

#[test]
fn a_bad_name_is_refused_by_what_it_takes() {
    for bad in ["has space", "-lead", "dot.ted", ""] {
        let error = err(&format!("[push.\"{bad}\"]\n"));
        assert!(
            error.contains("push.") && error.contains("module name takes"),
            "{bad:?}: {error}"
        );
    }
    let long = "n".repeat(33);
    let error = err(&format!("[push.{long}]\n"));
    assert!(error.contains("module name takes 1 to 32"), "{error}");
}

#[cfg(feature = "clock")]
#[test]
fn a_built_in_name_is_taken() {
    let error = err("[button.clock]\ntext = \"x\"\n");
    assert!(
        error.contains("button.clock") && error.contains("built-in"),
        "{error}"
    );
}

#[test]
fn a_name_is_one_modules_across_the_kinds() {
    let error = err("[button.same]\n[push.same]\n");
    assert!(
        error.contains("push.same") && error.contains("already defined as a button"),
        "{error}"
    );
}

#[test]
fn an_unknown_key_in_a_table_is_a_loud_error() {
    let error = err("[exec.x]\ncommand = [\"true\"]\ninterval = 5\n");
    assert!(error.contains("interval"), "{error}");
    let error = err("[push.x]\ntext = \"no\"\n");
    assert!(error.contains("text"), "{error}");
}

#[test]
fn a_list_naming_an_undefined_module_lists_what_there_is() {
    let error = err("left = [\"nope\"]\n[push.status]\n");
    assert!(
        error.contains("left") && error.contains("no module `nope`") && error.contains("status"),
        "{error}"
    );
}

#[test]
fn an_output_table_may_place_a_defined_module() {
    let config = read("[push.status]\n[output.\"DP-1\"]\ncenter = [\"status\"]\n").unwrap();
    assert!(
        config
            .outputs
            .to_start(&config.layout)
            .placed()
            .any(|(_, id)| id == "status")
    );
}

#[test]
fn exec_needs_a_command_of_the_right_shape() {
    for (table, why) in [
        ("[exec.x]\n", "is required"),
        ("[exec.x]\ncommand = \"sh -c 'echo'\"\n", "takes an array"),
        ("[exec.x]\ncommand = []\n", "takes a command"),
        ("[exec.x]\ncommand = [\"\"]\n", "empty string"),
        ("[exec.x]\ncommand = [1]\n", "strings only"),
        ("[exec.x]\ncommand = [\"a\\u0000b\"]\n", "NUL"),
    ] {
        let error = err(table);
        assert!(
            error.contains("exec.x.command") && error.contains(why),
            "{table}: {error}"
        );
    }
    let many = vec!["\"a\""; 33].join(", ");
    assert!(err(&format!("[exec.x]\ncommand = [{many}]\n")).contains("exec.x.command"));
    let long = "x".repeat(4097);
    assert!(err(&format!("[exec.x]\ncommand = [\"{long}\"]\n")).contains("4096 bytes"));
}

#[test]
fn exec_format_is_text_or_json() {
    let config = read("[exec.x]\ncommand = [\"true\"]\n").unwrap();
    let Kind::Exec(exec) = &config.modules.custom[0].kind else {
        panic!()
    };
    assert_eq!(exec.format, Format::Text);
    let error = err("[exec.x]\ncommand = [\"true\"]\nformat = \"yaml\"\n");
    assert!(
        error.contains("exec.x.format") && error.contains("text or json"),
        "{error}"
    );
}

#[test]
fn a_custom_tables_interaction_keys_are_checked_with_its_own_key() {
    let error = err("[button.b]\non-click = { exec = \"no shell\" }\n");
    assert!(
        error.contains("button.b.on-click") && error.contains("array"),
        "{error}"
    );
    let error = err("[push.p]\non-scroll-up = \"next\"\n");
    assert!(
        error.contains("push.p.on-scroll-up") && error.contains("no actions of its own"),
        "{error}"
    );
}

#[test]
fn a_custom_margin_is_bounded() {
    assert!(read("[button.b]\nmargin = 1024\n").is_ok());
    let error = err("[button.b]\nmargin = 1025\n");
    assert!(error.contains("button.b.margin"), "{error}");
}

#[test]
fn a_button_icon_follows_the_clocks_rules_under_its_own_key() {
    let error = err("[button.b]\nicon = \"ab\"\n");
    assert!(
        error.contains("button.b.icon") && error.contains("one character"),
        "{error}"
    );
    let error = err("[button.b]\nicon = \"a\"\nicon-path = \"M0 0\"\n");
    assert!(
        error.contains("button.b.icon-path") && error.contains("a button shows one icon"),
        "{error}"
    );
    let error = err("[button.b]\nicon-viewbox = \"0 0 1 1\"\n");
    assert!(error.contains("button.b.icon-viewbox"), "{error}");
    #[cfg(not(feature = "icon-image"))]
    {
        let error = err("[button.b]\nicon-image = \"/x.png\"\n");
        assert!(
            error.contains("button.b.icon-image") && error.contains("`icon-image` Cargo feature"),
            "{error}"
        );
    }
}

#[test]
fn push_and_exec_icons_follow_the_clocks_rules_under_their_own_keys() {
    for kind in ["push", "exec"] {
        let table = if kind == "push" {
            "[push.p]\n".to_owned()
        } else {
            "[exec.e]\ncommand = [\"true\"]\n".to_owned()
        };
        // One glyph is read.
        let config = read(&format!("{table}icon = \"x\"\n")).unwrap();
        let (push_icon, exec_icon) = match &config.modules.custom[0].kind {
            Kind::Push(push) => (push.icon.clone(), None),
            Kind::Exec(exec) => (None, exec.icon.clone()),
            _ => panic!(),
        };
        assert!(
            push_icon.or(exec_icon).is_some(),
            "{kind}: the icon is read"
        );
        // Two are refused naming the second, as the clock's.
        let error = err(&format!("{table}icon = \"x\"\nicon-path = \"M0 0\"\n"));
        assert!(
            error.contains(&format!("{kind}.")) && error.contains("shows one icon"),
            "{kind}: {error}"
        );
        // A viewbox without a path is refused under its own key.
        let error = err(&format!("{table}icon-viewbox = \"0 0 1 1\"\n"));
        assert!(error.contains(&format!("{kind}.")), "{kind}: {error}");
        // Two characters are refused under the icon's own key.
        let error = err(&format!("{table}icon = \"ab\"\n"));
        assert!(
            error.contains(&format!("{kind}.")) && error.contains("one character"),
            "{kind}: {error}"
        );
        #[cfg(not(feature = "icon-image"))]
        {
            let error = err(&format!("{table}icon-image = \"/x.png\"\n"));
            assert!(
                error.contains(&format!("{kind}.")) && error.contains("`icon-image` Cargo feature"),
                "{kind}: {error}"
            );
        }
    }
}

#[test]
fn push_and_exec_show_text_defaults_to_shown() {
    let config = read("[push.p]\n[exec.e]\ncommand = [\"true\"]\n").unwrap();
    for custom in &config.modules.custom {
        match &custom.kind {
            Kind::Push(push) => assert!(push.show_text, "push defaults to shown"),
            Kind::Exec(exec) => assert!(exec.show_text, "exec defaults to shown"),
            _ => panic!(),
        }
    }
    let config =
        read("[push.p]\nshow-text = false\n[exec.e]\ncommand = [\"true\"]\nshow-text = false\n")
            .unwrap();
    for custom in &config.modules.custom {
        match &custom.kind {
            Kind::Push(push) => assert!(!push.show_text),
            Kind::Exec(exec) => assert!(!exec.show_text),
            _ => panic!(),
        }
    }
}

#[test]
fn only_so_many_modules_are_defined_and_only_so_many_execs_placed() {
    // 33 tables, one over.
    let mut text = String::new();
    for i in 0..33 {
        text.push_str(&format!("[push.p{i}]\n"));
    }
    let error = err(&text);
    assert!(error.contains("at most 32 modules are defined"), "{error}");
    // MAX_EXEC placed is fine, one more is refused by name.
    let mut tables = String::new();
    let mut names = Vec::new();
    for i in 0..=MAX_EXEC {
        tables.push_str(&format!("[exec.e{i}]\ncommand = [\"true\"]\n"));
        names.push(format!("\"e{i}\""));
    }
    let placed = |n: usize| format!("left = [{}]\n{tables}", names[..n].join(", "));
    assert!(read(&placed(MAX_EXEC)).is_ok());
    let error = err(&placed(MAX_EXEC + 1));
    assert!(error.contains("at most 8 exec modules"), "{error}");
}

#[test]
fn a_defined_exec_that_is_not_placed_does_not_count() {
    let mut tables = String::new();
    for i in 0..20 {
        tables.push_str(&format!("[exec.e{i}]\ncommand = [\"true\"]\n"));
    }
    assert!(read(&format!("center = []\n{tables}")).is_ok());
}
