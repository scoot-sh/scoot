use std::borrow::Cow;

use super::*;
use crate::modules::{Module, Sources};
use rustix::event::PollFlags;

/// A module that counts what it is asked, with two actions.
#[derive(Default)]
struct Probe {
    invoked: Vec<(Cow<'static, str>, Option<i32>, u32)>,
    change: bool,
}

impl Module for Probe {
    fn sources<'fd>(&'fd self, _: &mut Sources<'_, 'fd>) {}
    fn on_ready(&mut self, _: usize, _: PollFlags) -> Update {
        Update::Unchanged
    }
    fn view(&self, _: &OutputView<'_>, _: &mut crate::modules::View) {}
    fn invoke(
        &mut self,
        _: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        match &*action.name {
            "bump" => {
                self.invoked.push((action.name.clone(), action.arg, steps));
                Ok(if self.change {
                    Update::Changed
                } else {
                    Update::Unchanged
                })
            }
            "refuse" => Err(InvokeError::Refused("not now")),
            _ => Err(InvokeError::Unknown),
        }
    }
}

#[derive(Default)]
struct Counting {
    execs: Vec<Vec<String>>,
    scoots: Vec<ScootAction>,
    popups: u32,
    fail: bool,
}

impl Effects for Counting {
    fn exec(&mut self, argv: &[String]) -> Result<(), String> {
        self.execs.push(argv.to_vec());
        if self.fail {
            Err("no such program".into())
        } else {
            Ok(())
        }
    }
    fn scoot(&mut self, action: ScootAction) -> Result<(), String> {
        self.scoots.push(action);
        Ok(())
    }
    fn popup(&mut self) -> Result<(), String> {
        self.popups += 1;
        Ok(())
    }
}

const OUT: OutputView<'static> = OutputView { name: Some("DP-1") };

#[test]
fn a_module_action_reaches_the_module_with_its_number_and_steps() {
    let mut probe = Probe::default();
    let mut revision = 7;
    let mut effects = Counting::default();
    let action = Action::Module(ModuleAction::new("bump", Some(5)));
    perform(
        &mut probe,
        &mut revision,
        &OUT,
        &action,
        Some(3),
        &mut effects,
    )
    .unwrap();
    assert_eq!(probe.invoked, [("bump".into(), Some(5), 3)]);
    // A click is one step.
    perform(&mut probe, &mut revision, &OUT, &action, None, &mut effects).unwrap();
    assert_eq!(probe.invoked[1].2, 1);
    assert!(effects.execs.is_empty() && effects.scoots.is_empty());
}

#[test]
fn a_changed_view_bumps_the_revision_and_an_unchanged_one_does_not() {
    let mut probe = Probe::default();
    let mut revision = 7;
    let mut effects = Counting::default();
    let action = Action::Module(ModuleAction::new("bump", None));
    perform(&mut probe, &mut revision, &OUT, &action, None, &mut effects).unwrap();
    assert_eq!(revision, 7);
    probe.change = true;
    perform(&mut probe, &mut revision, &OUT, &action, None, &mut effects).unwrap();
    assert_eq!(revision, 8);
    // Wrapping, never a panic.
    let mut revision = u64::MAX;
    perform(&mut probe, &mut revision, &OUT, &action, None, &mut effects).unwrap();
    assert_eq!(revision, 0);
}

#[test]
fn a_refused_module_action_is_a_failure_naming_why() {
    let mut probe = Probe::default();
    let mut revision = 0;
    let mut effects = Counting::default();
    let refused = perform(
        &mut probe,
        &mut revision,
        &OUT,
        &Action::Module(ModuleAction::new("refuse", None)),
        None,
        &mut effects,
    )
    .unwrap_err();
    assert_eq!(refused.to_string(), "not now");
    let unknown = perform(
        &mut probe,
        &mut revision,
        &OUT,
        &Action::Module(ModuleAction::new("nope", None)),
        None,
        &mut effects,
    )
    .unwrap_err();
    assert_eq!(unknown, Failed::Module(InvokeError::Unknown));
    assert_eq!(revision, 0);
}

#[test]
fn exec_goes_to_the_effects_once_per_action_however_many_steps() {
    let mut probe = Probe::default();
    let mut revision = 0;
    let mut effects = Counting::default();
    let action = Action::Exec(vec!["wpctl".into(), "set-mute".into()]);
    perform(&mut probe, &mut revision, &OUT, &action, None, &mut effects).unwrap();
    perform(
        &mut probe,
        &mut revision,
        &OUT,
        &action,
        Some(4),
        &mut effects,
    )
    .unwrap();
    assert_eq!(
        effects.execs,
        [
            vec!["wpctl".to_owned(), "set-mute".to_owned()],
            vec!["wpctl".to_owned(), "set-mute".to_owned()],
        ]
    );
    assert!(probe.invoked.is_empty(), "the module is not asked");
}

#[test]
fn a_failing_command_is_a_failure_not_a_panic() {
    let mut probe = Probe::default();
    let mut revision = 0;
    let mut effects = Counting {
        fail: true,
        ..Counting::default()
    };
    let failed = perform(
        &mut probe,
        &mut revision,
        &OUT,
        &Action::Exec(vec!["nope".into()]),
        None,
        &mut effects,
    )
    .unwrap_err();
    assert_eq!(failed, Failed::Effect("no such program".to_owned()));
}

#[test]
fn scoot_goes_to_the_effects() {
    let mut probe = Probe::default();
    let mut revision = 0;
    let mut effects = Counting::default();
    perform(
        &mut probe,
        &mut revision,
        &OUT,
        &Action::Scoot(ScootAction::Quit),
        None,
        &mut effects,
    )
    .unwrap();
    assert_eq!(effects.scoots, [ScootAction::Quit]);
}

#[test]
fn bindings_are_per_trigger_and_empty_by_default() {
    let mut bindings = Bindings::default();
    assert!(bindings.is_empty());
    for trigger in Trigger::ALL {
        assert_eq!(bindings.get(trigger), None);
    }
    bindings.set(Trigger::ScrollUp, Action::Scoot(ScootAction::Quit));
    assert!(!bindings.is_empty());
    for trigger in Trigger::ALL {
        assert_eq!(
            bindings.get(trigger).is_some(),
            trigger == Trigger::ScrollUp,
            "{trigger:?}"
        );
    }
}

#[test]
fn every_trigger_has_its_own_key() {
    let keys: Vec<_> = Trigger::ALL.iter().map(|t| t.key()).collect();
    assert_eq!(
        keys,
        [
            "on-click",
            "on-right-click",
            "on-middle-click",
            "on-scroll-up",
            "on-scroll-down"
        ]
    );
}

#[test]
fn scoot_actions_parse_by_their_config_name() {
    assert_eq!(ScootAction::parse("quit"), Some(ScootAction::Quit));
    assert_eq!(ScootAction::parse("Quit"), None);
    assert_eq!(ScootAction::parse(""), None);
    assert_eq!(ScootAction::Quit.name(), "quit");
}

#[test]
fn carrying_out_a_module_action_allocates_nothing() {
    // The click and scroll path for a module action: the lookup of the
    // binding, the call, the revision.
    let mut probe = Probe::default();
    probe.invoked.reserve(400);
    let mut revision = 0;
    let mut effects = Counting::default();
    let mut bindings = Bindings::default();
    bindings.set(
        Trigger::ScrollDown,
        Action::Module(ModuleAction::new("bump", Some(2))),
    );
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for _ in 0..100 {
            let action = bindings.get(Trigger::ScrollDown).unwrap();
            // Discarding the result: the probe's push is reserved above.
            let _ = perform(
                &mut probe,
                &mut revision,
                &OUT,
                action,
                Some(3),
                &mut effects,
            );
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!(probe.invoked.len(), 100);
}

#[test]
fn the_popup_action_is_the_bars_to_carry_out_not_the_modules() {
    let mut probe = Probe::default();
    let mut revision = 7;
    let mut effects = Counting::default();
    let action = Action::Module(ModuleAction::new(POPUP, None));
    perform(&mut probe, &mut revision, &OUT, &action, None, &mut effects).unwrap();
    assert_eq!(effects.popups, 1);
    // The module never saw it, and nothing else went out.
    assert!(probe.invoked.is_empty());
    assert_eq!(revision, 7);
    assert!(effects.execs.is_empty() && effects.scoots.is_empty());
}

/// Effects that refuse a popup, as the default does (a build without
/// popups, a surface-less test): the refusal is the failure.
struct NoPopups;

impl Effects for NoPopups {
    fn exec(&mut self, _: &[String]) -> Result<(), String> {
        Ok(())
    }
    fn scoot(&mut self, _: ScootAction) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn effects_with_no_popups_refuse_the_action_by_default() {
    let mut probe = Probe::default();
    let mut revision = 0;
    let refused = perform(
        &mut probe,
        &mut revision,
        &OUT,
        &Action::Module(ModuleAction::new(POPUP, None)),
        None,
        &mut NoPopups,
    )
    .unwrap_err();
    assert_eq!(refused.to_string(), "this build has no popups");
}
