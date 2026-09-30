//! A module's interaction keys: what each takes, what is refused and that
//! every refusal names its key.

use super::tests::read;
#[cfg(feature = "workspaces")]
use crate::action::ModuleAction;
#[cfg(feature = "clock")]
use crate::action::ScootAction;
#[cfg(any(feature = "clock", feature = "workspaces"))]
use crate::action::{Action, Trigger};

fn err(text: &str) -> String {
    read(text).unwrap_err().to_string()
}

#[cfg(feature = "clock")]
#[test]
fn an_exec_binding_is_read_as_an_argument_vector() {
    let config = read(
        "[clock]\non-click = { exec = [\"foot\", \"-e\", \"btop\"] }\n\
         on-scroll-up = { exec = [\"x\"] }\n",
    )
    .unwrap();
    let (id, bindings) = &config.modules.bindings[0];
    assert_eq!(*id, "clock");
    assert_eq!(
        bindings.get(Trigger::Click),
        Some(&Action::Exec(vec![
            "foot".into(),
            "-e".into(),
            "btop".into()
        ]))
    );
    assert_eq!(
        bindings.get(Trigger::ScrollUp),
        Some(&Action::Exec(vec!["x".into()]))
    );
    assert_eq!(bindings.get(Trigger::RightClick), None);
}

#[cfg(feature = "clock")]
#[test]
fn a_module_with_no_keys_has_no_bindings_entry() {
    let config = read("[clock]\nformat = \"%H:%M\"\n").unwrap();
    assert!(config.modules.bindings.is_empty());
    assert!(config.modules.bindings_of("clock").is_empty());
}

#[cfg(feature = "clock")]
#[test]
fn an_exec_string_is_refused_because_nothing_runs_a_shell() {
    let error = err("[clock]\non-click = { exec = \"foot -e btop\" }\n");
    assert!(
        error.contains("clock.on-click") && error.contains("array"),
        "{error}"
    );
    assert!(error.contains("sh"), "says how to use a shell: {error}");
}

#[cfg(feature = "clock")]
#[test]
fn a_bad_exec_line_is_refused_naming_its_key() {
    for (bad, why) in [
        ("{ exec = [] }", "command"),
        ("{ exec = [\"\"] }", "empty"),
        ("{ exec = [1] }", "strings"),
        ("{ exec = [\"a\\u0000b\"] }", "NUL"),
        ("{ exec = [\"a\"], scoot = \"quit\" }", "one key"),
        ("{ run = [\"a\"] }", "action kind"),
        ("{}", "one key"),
        ("3", "takes a module action"),
        ("true", "takes a module action"),
        ("[\"a\"]", "takes a module action"),
    ] {
        let error = err(&format!("[clock]\non-right-click = {bad}\n"));
        assert!(
            error.contains("clock.on-right-click") && error.contains(why),
            "{bad}: {error}"
        );
    }
}

#[cfg(feature = "clock")]
#[test]
fn an_exec_line_is_bounded() {
    let many = vec!["\"a\""; crate::action::MAX_EXEC_ARGS].join(", ");
    assert!(read(&format!("[clock]\non-click = {{ exec = [{many}] }}\n")).is_ok());
    let too_many = vec!["\"a\""; crate::action::MAX_EXEC_ARGS + 1].join(", ");
    let error = err(&format!("[clock]\non-click = {{ exec = [{too_many}] }}\n"));
    assert!(error.contains("clock.on-click"), "{error}");
    let long = "x".repeat(crate::action::MAX_EXEC_ARG + 1);
    let error = err(&format!(
        "[clock]\non-click = {{ exec = [\"a\", \"{long}\"] }}\n"
    ));
    assert!(error.contains("bytes"), "{error}");
    let fits = "x".repeat(crate::action::MAX_EXEC_ARG);
    assert!(
        read(&format!(
            "[clock]\non-click = {{ exec = [\"a\", \"{fits}\"] }}\n"
        ))
        .is_ok()
    );
}

#[cfg(feature = "clock")]
#[test]
fn the_clock_has_no_actions_of_its_own() {
    let error = err("[clock]\non-click = \"toggle\"\n");
    assert!(
        error.contains("clock.on-click") && error.contains("no actions"),
        "{error}"
    );
}

#[cfg(feature = "workspaces")]
#[test]
fn a_module_action_is_checked_against_what_the_module_defines() {
    let config = read(
        "[workspaces]\non-scroll-up = \"previous\"\non-scroll-down = \"next\"\n\
         on-middle-click = \"activate 3\"\n",
    )
    .unwrap();
    let bindings = config.modules.bindings_of("workspaces");
    assert_eq!(
        bindings.get(Trigger::ScrollUp),
        Some(&Action::Module(ModuleAction::new("previous", None)))
    );
    assert_eq!(
        bindings.get(Trigger::ScrollDown),
        Some(&Action::Module(ModuleAction::new("next", None)))
    );
    assert_eq!(
        bindings.get(Trigger::MiddleClick),
        Some(&Action::Module(ModuleAction::new("activate", Some(3))))
    );
    // A click with no key set keeps the module's own default.
    assert_eq!(bindings.get(Trigger::Click), None);
}

#[cfg(feature = "workspaces")]
#[test]
fn a_misspelled_or_misused_action_is_refused_naming_the_key_and_the_choices() {
    let error = err("[workspaces]\non-click = \"nxt\"\n");
    assert!(
        error.contains("workspaces.on-click") && error.contains("nxt"),
        "{error}"
    );
    for name in ["activate", "previous", "next"] {
        assert!(error.contains(name), "lists {name}: {error}");
    }
    for (bad, why) in [
        ("activate", "whole number"),
        ("activate x", "whole number"),
        ("activate 1.5", "whole number"),
        ("activate 99999999999", "whole number"),
        ("next 2", "no number"),
        ("activate 1 2", "at most one number"),
        ("", "empty"),
        ("   ", "empty"),
    ] {
        let error = err(&format!("[workspaces]\non-scroll-down = \"{bad}\"\n"));
        assert!(
            error.contains("workspaces.on-scroll-down") && error.contains(why),
            "{bad:?}: {error}"
        );
    }
}

#[cfg(feature = "workspaces")]
#[test]
fn an_unknown_key_is_still_a_loud_error() {
    let error = err("[workspaces]\non-double-click = \"next\"\n");
    assert!(error.contains("on-double-click"), "{error}");
}

#[cfg(feature = "clock")]
#[test]
fn scoot_quit_is_read_and_anything_else_refused() {
    let config = read("[clock]\non-click = { scoot = \"quit\" }\n").unwrap();
    assert_eq!(
        config.modules.bindings_of("clock").get(Trigger::Click),
        Some(&Action::Scoot(ScootAction::Quit))
    );
    for bad in ["\"restart\"", "1", "[\"quit\"]", "\"\"", "\"Quit\""] {
        let error = err(&format!("[clock]\non-click = {{ scoot = {bad} }}\n"));
        assert!(
            error.contains("clock.on-click") && error.contains("quit"),
            "{bad}: {error}"
        );
    }
}

#[cfg(feature = "clock")]
#[test]
fn equal_files_read_equal() {
    // A reload compares configs: bindings must not make equal files differ.
    let text = "[clock]\non-click = { exec = [\"a\", \"b\"] }\n";
    assert_eq!(read(text).unwrap(), read(text).unwrap());
    assert_ne!(
        read(text).unwrap(),
        read("[clock]\non-click = { exec = [\"a\", \"c\"] }\n").unwrap()
    );
}
