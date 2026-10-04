//! The power module on its own: the confirm state machine (arm,
//! perform, disarm by another row, by timeout, and by refills after a
//! close), each row's staged action, `rows` hiding, config refusals
//! naming keys, and that a popup refill is deterministic (the no-alloc
//! path formats into the popup's reused buffers). The logind half is
//! `daemon_tests.rs`, against a real `dbus-daemon`.

use std::time::Duration;

use super::{CONFIRM_WINDOW, ROW_NAMES, Settings, row_index, start_on};
use crate::action::{Action, ModuleAction, ScootAction};
use crate::modules::harness::Harness;
use crate::modules::{InvokeError, OutputView, Update};
#[cfg(feature = "popup")]
use crate::popup::{Content, Kind};

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };

/// Settings with every row runnable without a bus: all five commands
/// override their paths, so no test below dials anything real.
fn commanded() -> Settings {
    Settings {
        commands: [
            vec!["lockit".to_owned()],
            vec!["bye".to_owned(), "--now".to_owned()],
            vec!["do-suspend".to_owned()],
            vec!["do-reboot".to_owned()],
            vec!["do-poweroff".to_owned()],
        ],
        ..Settings::default()
    }
}

fn up_with(settings: &Settings) -> Harness {
    Harness::new(start_on(
        "/nonexistent/scootbar-power-test-bus".into(),
        settings,
    ))
}

fn up() -> Harness {
    up_with(&commanded())
}

fn action(name: &'static str) -> ModuleAction {
    ModuleAction::new(name, None)
}

/// The popup's rows as (label, action, closes) triples, in order.
#[cfg(feature = "popup")]
fn rows(content: &Content) -> Vec<(String, &'static str, bool)> {
    content
        .widgets()
        .iter()
        .map(|widget| {
            let (action, closes) = match widget.kind {
                Kind::Button { action, closes, .. } => (action, closes),
                Kind::Text | Kind::Slider { .. } => ("", false),
            };
            (content.label(widget).to_owned(), action, closes)
        })
        .collect()
}

#[cfg(feature = "popup")]
fn popup(harness: &mut Harness) -> (bool, Content) {
    let mut content = Content::default();
    let has = harness.popup(&mut content);
    (has, content)
}

#[test]
fn lock_performs_at_once_with_no_confirm() {
    let mut harness = up();
    assert_eq!(
        harness.invoke(&DP1, &action("lock"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.take_action(),
        Some(Action::Exec(vec!["lockit".to_owned()]))
    );
    // One shot: nothing staged twice.
    assert_eq!(harness.take_action(), None);
    // And nothing armed.
    let value = harness.value_on(None).expect("a value");
    assert!(value.get("armed").is_none());
}

#[test]
#[cfg(feature = "popup")]
fn destructive_rows_arm_first_and_perform_second() {
    let mut harness = up();
    // First invoke arms only that row: nothing staged.
    assert_eq!(
        harness.invoke(&DP1, &action("logout"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(harness.take_action(), None);
    let (_, content) = popup(&mut harness);
    let labels: Vec<String> = rows(&content).iter().map(|row| row.0.clone()).collect();
    assert_eq!(
        labels,
        vec![
            "Lock",
            "Log out? Click again",
            "Suspend",
            "Reboot",
            "Shut down"
        ]
    );
    // The armed row closes on its second click; the rest stay open.
    let row_list = rows(&content);
    assert_eq!(
        row_list.iter().map(|row| row.2).collect::<Vec<_>>(),
        vec![true, true, false, false, false]
    );
    // Second invoke on the same row performs and stages.
    assert_eq!(
        harness.invoke(&DP1, &action("logout"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.take_action(),
        Some(Action::Exec(vec!["bye".to_owned(), "--now".to_owned()]))
    );
    assert_eq!(harness.take_action(), None);
}

#[test]
fn logout_without_its_command_stages_scoot_quit() {
    // No logout-command: the row quits through scoot (the socket's
    // presence is checked in `availability`, which the live run covers
    // with the real session; staging itself needs no environment).
    let mut settings = commanded();
    settings.commands[1] = Vec::new();
    let mut power = super::started(&settings, "/nonexistent/scootbar-power-test-bus".into());
    power.perform(super::Row::Logout).expect("staged");
    assert_eq!(power.staged, Some(Action::Scoot(ScootAction::Quit)));
    // One shot.
    assert_eq!(power.staged.take(), Some(Action::Scoot(ScootAction::Quit)));
    assert_eq!(power.staged, None);
}

#[test]
#[cfg(feature = "popup")]
fn arming_another_row_disarms_the_first() {
    let mut harness = up();
    assert_eq!(
        harness.invoke(&DP1, &action("logout"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.invoke(&DP1, &action("suspend"), 1),
        Ok(Update::Changed)
    );
    // Nothing performed by the switch: both first clicks only arm.
    assert_eq!(harness.take_action(), None);
    let (_, content) = popup(&mut harness);
    let labels: Vec<String> = rows(&content).iter().map(|row| row.0.clone()).collect();
    assert_eq!(
        labels,
        vec![
            "Lock",
            "Log out",
            "Suspend? Click again",
            "Reboot",
            "Shut down"
        ]
    );
}

#[test]
fn a_lock_click_disarms() {
    let mut harness = up();
    assert_eq!(
        harness.invoke(&DP1, &action("logout"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.invoke(&DP1, &action("lock"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.take_action(),
        Some(Action::Exec(vec!["lockit".to_owned()]))
    );
    let value = harness.value_on(None).expect("a value");
    assert!(value.get("armed").is_none());
}

#[test]
fn the_arm_times_out() {
    let mut harness = up();
    assert_eq!(
        harness.invoke(&DP1, &action("reboot"), 1),
        Ok(Update::Changed)
    );
    // The arm timer fires: disarmed, with a change to refill from.
    assert_eq!(
        harness.wait(CONFIRM_WINDOW + Duration::from_secs(2)),
        Some(Update::Changed)
    );
    let value = harness.value_on(None).expect("a value");
    assert!(value.get("armed").is_none());
    // And a click now arms anew rather than performing: still nothing
    // staged after one invoke past the timeout.
    assert_eq!(
        harness.invoke(&DP1, &action("reboot"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(harness.take_action(), None);
}

#[test]
#[cfg(feature = "popup")]
fn an_armed_row_survives_refills() {
    // Arming lengthens the row's label, which resizes the popup, which
    // reopens it (a refill): the arm must survive fills, and only the
    // timer, another row or a perform disarms it.
    let mut harness = up();
    assert_eq!(
        harness.invoke(&DP1, &action("logout"), 1),
        Ok(Update::Changed)
    );
    let (_, first) = popup(&mut harness);
    assert!(
        rows(&first)
            .iter()
            .any(|(label, _, _)| label.contains("Click again"))
    );
    let (_, second) = popup(&mut harness);
    assert!(
        rows(&second)
            .iter()
            .any(|(label, _, _)| label.contains("Click again"))
    );
    // ...and still performs on the second invoke.
    assert_eq!(
        harness.invoke(&DP1, &action("logout"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.take_action(),
        Some(Action::Exec(vec!["bye".to_owned(), "--now".to_owned()]))
    );
}

#[test]
#[cfg(feature = "popup")]
fn rows_hidden_by_config_are_refused_and_unshown() {
    let mut settings = commanded();
    settings.hidden = [false, true, false, true, false];
    let mut harness = up_with(&settings);
    assert_eq!(
        harness.invoke(&DP1, &action("logout"), 1),
        Err(InvokeError::Refused("that row is hidden by power.rows"))
    );
    assert_eq!(
        harness.invoke(&DP1, &action("reboot"), 1),
        Err(InvokeError::Refused("that row is hidden by power.rows"))
    );
    let (has, content) = popup(&mut harness);
    assert!(has);
    let labels: Vec<String> = rows(&content).iter().map(|row| row.0.clone()).collect();
    assert_eq!(labels, vec!["Lock", "Suspend", "Shut down"]);
}

#[test]
#[cfg(feature = "popup")]
fn lock_is_hidden_without_its_command() {
    let mut settings = commanded();
    settings.commands[0] = Vec::new();
    let mut harness = up_with(&settings);
    assert_eq!(
        harness.invoke(&DP1, &action("lock"), 1),
        Err(InvokeError::Refused(
            "no lock command configured (power.lock-command)"
        ))
    );
    let (_, content) = popup(&mut harness);
    let labels: Vec<String> = rows(&content).iter().map(|row| row.0.clone()).collect();
    assert!(
        !labels.iter().any(|label| label.contains("Lock")),
        "{labels:?}"
    );
}

#[test]
fn performing_without_a_bus_is_refused() {
    // The suspend row with no override needs logind: arming is
    // optimistic (unknown shows the row), but performing with no session
    // is refused aloud rather than staged silence.
    let mut settings = commanded();
    settings.commands[2] = Vec::new();
    let mut harness = up_with(&settings);
    assert_eq!(
        harness.invoke(&DP1, &action("suspend"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.invoke(&DP1, &action("suspend"), 1),
        Err(InvokeError::Refused("no system bus"))
    );
    assert_eq!(harness.take_action(), None);
}

#[test]
fn invoke_names_are_exact() {
    let mut harness = up();
    assert_eq!(
        harness.invoke(&DP1, &action("no-such-row"), 1),
        Err(InvokeError::Unknown)
    );
    let numbered = ModuleAction {
        name: "logout".into(),
        arg: Some(1),
    };
    assert_eq!(harness.invoke(&DP1, &numbered, 1), Err(InvokeError::NoArg));
}

#[test]
fn rows_parse_by_name() {
    assert_eq!(ROW_NAMES.len(), 5);
    for (index, name) in ROW_NAMES.iter().enumerate() {
        assert_eq!(row_index(name), Some(index));
    }
    assert_eq!(row_index("quit"), None);
    assert_eq!(row_index(""), None);
    assert_eq!(row_index("Lock"), None);
}

#[test]
fn the_view_shows_only_the_icon() {
    let mut settings = commanded();
    settings.icon = Some(crate::icon::Icon::Glyph('⏻'));
    let harness = up_with(&settings);
    let view = harness.view();
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('⏻'));
    let tooltip = view.tooltip();
    assert!(tooltip.contains("Power"), "{tooltip:?}");
    assert!(tooltip.contains("Log out"), "{tooltip:?}");
}

#[test]
fn without_an_icon_the_module_takes_no_space() {
    let harness = up_with(&commanded());
    let view = harness.view();
    assert!(view.is_empty());
    assert_eq!(view.tooltip(), "");
}

#[test]
fn nothing_is_polled_before_first_use() {
    // Lazy logind: no bus fd and no timer until the popup first opens or
    // an invoke needs it.
    let harness = up();
    assert_eq!(harness.source_count(), 0);
}

#[test]
#[cfg(feature = "popup")]
fn opening_the_popup_connects() {
    // A logind row without its override needs the bus: the popup dials
    // on first open (lazy: nothing before). All overridden, it never
    // would.
    let mut settings = commanded();
    settings.commands[2] = Vec::new();
    let mut harness = up_with(&settings);
    assert_eq!(harness.source_count(), 0);
    let (has, _) = popup(&mut harness);
    assert!(has);
    // The dial waits on the missing socket's directory: one source, and
    // arming adds its timer beside it.
    assert_eq!(harness.source_count(), 1);
    assert_eq!(
        harness.invoke(&DP1, &action("logout"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(harness.source_count(), 2);
}

#[test]
#[cfg(feature = "popup")]
fn a_popup_refill_is_deterministic() {
    // The refill formats into the popup's reused buffers (`format_args!`
    // only): filling twice shows the same rows, allocating nothing the
    // second time holds that the first did not size.
    let mut harness = up();
    assert_eq!(
        harness.invoke(&DP1, &action("suspend"), 1),
        Ok(Update::Changed)
    );
    let (_, first) = popup(&mut harness);
    // (The arm's own refill; a second fill disarms, so arm anew for two
    // armed fills.)
    assert_eq!(
        harness.invoke(&DP1, &action("suspend"), 1),
        Ok(Update::Changed)
    );
    assert_eq!(
        harness.take_action(),
        Some(Action::Exec(vec!["do-suspend".to_owned()]))
    );
    let (_, second) = popup(&mut harness);
    let (_, third) = popup(&mut harness);
    assert_eq!(rows(&second), rows(&third));
    assert!(!rows(&first).is_empty());
}

#[test]
fn query_reports_rows_and_the_arm() {
    let mut harness = up();
    let value = harness.value_on(None).expect("a value");
    assert_eq!(
        value["rows"],
        serde_json::json!(["lock", "logout", "suspend", "reboot", "poweroff"])
    );
    assert!(value.get("armed").is_none());
    assert_eq!(
        harness.invoke(&DP1, &action("poweroff"), 1),
        Ok(Update::Changed)
    );
    let value = harness.value_on(None).expect("a value");
    assert_eq!(value["armed"], serde_json::json!("poweroff"));
}
