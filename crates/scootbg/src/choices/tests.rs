use std::sync::Arc;

use super::Choices;
use crate::color::Color;
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::wallpaper::{Image, Wallpaper};

fn c(text: &str) -> Option<Wallpaper> {
    Some(Wallpaper::Color(Color::parse(text).unwrap()))
}

fn image(serial: u64) -> Option<Wallpaper> {
    Some(Wallpaper::Image(Arc::new(Image {
        path: format!("/{serial}.png"),
        look: Look {
            mode: Mode::Fill,
            fill: Color { r: 0, g: 0, b: 0 },
            filter: Filter::Lanczos3,
        },
        serial,
        fetch: None,
    })))
}

/// Records in generation order, as the daemon does for colors.
struct Seq(Choices, u64);

impl Seq {
    fn new() -> Self {
        Self(Choices::default(), 0)
    }

    fn set(&mut self, output: Option<&str>, choice: Option<Wallpaper>) {
        self.1 += 1;
        assert!(self.0.set(output, choice, self.1));
    }

    fn get(&self, name: Option<&str>) -> Option<Wallpaper> {
        self.0.for_output(name, None).cloned()
    }
}

#[test]
fn nothing_is_chosen_at_first() {
    let choices = Choices::default();
    assert_eq!(choices.for_output(Some("DP-1"), None), None);
    assert_eq!(choices.for_output(None, None), None);
}

#[test]
fn the_all_outputs_choice_covers_every_name_and_unnamed_outputs() {
    let mut choices = Seq::new();
    choices.set(None, c("#c03020"));
    assert_eq!(choices.get(Some("DP-1")), c("#c03020"));
    assert_eq!(choices.get(Some("HEADLESS-9")), c("#c03020"));
    assert_eq!(choices.get(None), c("#c03020"));
}

#[test]
fn a_named_choice_overrides_the_all_outputs_one_for_that_name_only() {
    let mut choices = Seq::new();
    choices.set(None, c("#c03020"));
    choices.set(Some("DP-2"), c("#101014"));
    assert_eq!(choices.get(Some("DP-2")), c("#101014"));
    assert_eq!(choices.get(Some("DP-1")), c("#c03020"));
    // Cleared by name: nothing there, the rest keep the color.
    choices.set(Some("DP-2"), None);
    assert_eq!(choices.get(Some("DP-2")), None);
    assert_eq!(choices.get(Some("DP-1")), c("#c03020"));
    assert_eq!(choices.0.named_len(), 1, "replaced in place, not appended");
}

#[test]
fn an_all_outputs_choice_replaces_every_named_one() {
    let mut choices = Seq::new();
    choices.set(Some("DP-1"), c("#111111"));
    choices.set(Some("DP-2"), None);
    choices.set(None, c("#222222"));
    assert_eq!(choices.0.named_len(), 0);
    assert_eq!(choices.get(Some("DP-1")), c("#222222"));
    assert_eq!(choices.get(Some("DP-2")), c("#222222"));
    // And clearing every output clears them all.
    choices.set(Some("DP-1"), c("#333333"));
    choices.set(None, None);
    assert_eq!(choices.get(Some("DP-1")), None);
}

#[test]
fn names_match_exactly() {
    let mut choices = Seq::new();
    choices.set(Some("DP-1"), c("#111111"));
    assert_eq!(choices.get(Some("dp-1")), None);
    assert_eq!(choices.get(Some("DP-10")), None);
    assert_eq!(choices.get(Some("DP-")), None);
}

/// An image decoded after newer requests landed: it changes only what
/// nothing newer chose, and the newest request wins either way.
#[test]
fn an_older_request_landing_late_never_overrides_a_newer_one() {
    // Every output: image 1 requested, then a color for DP-2 (2), then the
    // image lands. Every output shows it, except DP-2, which keeps its
    // newer color.
    let mut choices = Choices::default();
    assert!(choices.set(Some("DP-2"), c("#101014"), 2));
    assert!(!choices.supersedes(None, 1));
    assert!(choices.set(None, image(1), 1));
    assert_eq!(choices.for_output(Some("DP-1"), None).cloned(), image(1));
    assert_eq!(
        choices.for_output(Some("DP-2"), None).cloned(),
        c("#101014")
    );

    // A newer every-output choice supersedes any older request entirely.
    let mut choices = Choices::default();
    assert!(choices.set(None, c("#222222"), 5));
    assert!(choices.supersedes(None, 4));
    assert!(choices.supersedes(Some("DP-1"), 4));
    assert!(!choices.set(None, image(4), 4));
    assert!(!choices.set(Some("DP-1"), image(3), 3));
    assert_eq!(
        choices.for_output(Some("DP-1"), None).cloned(),
        c("#222222")
    );
    // But not a newer one.
    assert!(!choices.supersedes(Some("DP-1"), 6));
    assert!(choices.set(Some("DP-1"), image(6), 6));
    assert_eq!(choices.for_output(Some("DP-1"), None).cloned(), image(6));

    // A newer choice by name supersedes an older one for that name only.
    let mut choices = Choices::default();
    assert!(choices.set(Some("DP-1"), c("#333333"), 8));
    assert!(choices.supersedes(Some("DP-1"), 7));
    assert!(!choices.supersedes(Some("DP-2"), 7));
    assert!(!choices.set(Some("DP-1"), image(7), 7));
    assert!(choices.set(Some("DP-2"), image(7), 7));
    assert_eq!(
        choices.for_output(Some("DP-1"), None).cloned(),
        c("#333333")
    );
    assert_eq!(choices.for_output(Some("DP-2"), None).cloned(), image(7));
    assert_eq!(choices.named_len(), 2);
}

/// What the state file writes: the every-output choice once one was made
/// (a clear is one), and the named ones oldest first.
#[test]
fn every_and_named_list_what_was_chosen() {
    use crate::color::Color;
    use crate::wallpaper::Wallpaper;
    let red = Some(Wallpaper::Color(Color { r: 255, g: 0, b: 0 }));
    let mut choices = Choices::default();
    assert!(choices.every().is_none(), "nothing chosen yet");
    assert_eq!(choices.named().count(), 0);
    choices.set(Some("B"), red.clone(), 1);
    choices.set(Some("A"), None, 2);
    assert!(choices.every().is_none());
    let names: Vec<_> = choices
        .named()
        .map(|(n, c, made)| (n, c.is_some(), made))
        .collect();
    assert_eq!(names, [("B", true, 1), ("A", false, 2)]);
    choices.forget("B");
    choices.forget("nobody");
    assert_eq!(choices.named().map(|(n, ..)| n).collect::<Vec<_>>(), ["A"]);
    choices.set(None, None, 3);
    assert_eq!(choices.every(), Some(&None), "a clear of every output");
    assert_eq!(choices.named().count(), 0);
}

#[test]
fn exact_does_not_fall_back_and_fill_keeps_the_generation() {
    let mut choices = Choices::default();
    assert_eq!(choices.exact(None), None);
    assert!(!choices.fill(None, None));
    choices.set(None, c("#010101"), 5);
    choices.set(Some("DP-1"), None, 5);
    assert_eq!(
        choices.exact(Some("DP-2")),
        None,
        "no fallback to every output"
    );
    assert_eq!(choices.exact(Some("DP-1")), Some(&None));
    assert!(choices.fill(Some("DP-1"), c("#020202")));
    assert!(!choices.fill(Some("DP-2"), None));
    // Filled at generation 5: a request of 5 is still superseded only by
    // something newer, and a newer one replaces it.
    assert!(!choices.supersedes(Some("DP-1"), 5));
    assert!(choices.set(Some("DP-1"), None, 6));
    assert!(choices.fill(None, None));
    assert_eq!(choices.exact(None), Some(&None));
    assert_eq!(choices.named_len(), 1, "filling every output drops nothing");
}

/// Records workspace mappings in generation order, as the daemon does.
fn workspace(
    choices: &mut Choices,
    generation: &mut u64,
    output: Option<&str>,
    name: &str,
    choice: Option<Wallpaper>,
) {
    *generation += 1;
    assert!(choices.set_workspace(
        output,
        name,
        choice,
        crate::transition::Spec::none(),
        *generation
    ));
}

fn shown(choices: &Choices, output: &str, active: Option<&str>) -> Option<Wallpaper> {
    choices.for_output(Some(output), active).cloned()
}

#[test]
fn a_workspace_mapping_shows_only_while_its_workspace_is_active() {
    let mut choices = Choices::default();
    let mut generation = 0;
    generation += 1;
    choices.set(None, c("#c03020"), generation);
    workspace(&mut choices, &mut generation, None, "2", c("#101014"));
    // Inactive: the base shows.
    assert_eq!(shown(&choices, "DP-1", None), c("#c03020"));
    assert_eq!(shown(&choices, "DP-1", Some("2")), c("#101014"));
    // Another workspace, or none: back to base.
    assert_eq!(shown(&choices, "DP-1", Some("1")), c("#c03020"));
    assert_eq!(shown(&choices, "DP-1", None), c("#c03020"));
    // Another output never left base.
    assert_eq!(shown(&choices, "HDMI-A-1", Some("2")), c("#101014"));
    assert_eq!(shown(&choices, "HDMI-A-1", None), c("#c03020"));
}

#[test]
fn the_newest_of_base_and_workspace_wins() {
    let mut choices = Choices::default();
    let mut generation = 0;
    // Workspace first, base after: the base covers it while newest.
    workspace(&mut choices, &mut generation, None, "2", c("#101014"));
    generation += 1;
    choices.set(None, c("#c03020"), generation);
    assert_eq!(shown(&choices, "DP-1", Some("2")), c("#c03020"));
    // And a still newer workspace mapping wins again.
    workspace(&mut choices, &mut generation, None, "2", c("#202020"));
    assert_eq!(shown(&choices, "DP-1", Some("2")), c("#202020"));
}

#[test]
fn an_output_mapping_beats_a_global_one_when_newer_and_vice_versa() {
    let mut choices = Choices::default();
    let mut generation = 0;
    generation += 1;
    choices.set(None, c("#c03020"), generation);
    workspace(&mut choices, &mut generation, None, "2", c("#101014"));
    workspace(
        &mut choices,
        &mut generation,
        Some("DP-1"),
        "2",
        c("#202020"),
    );
    assert_eq!(shown(&choices, "DP-1", Some("2")), c("#202020"));
    assert_eq!(shown(&choices, "HDMI-A-1", Some("2")), c("#101014"));
}

#[test]
fn an_older_workspace_request_landing_late_changes_nothing() {
    let mut choices = Choices::default();
    assert!(choices.set_workspace(None, "2", c("#101014"), crate::transition::Spec::none(), 9));
    // Older: refused.
    assert!(!choices.set_workspace(None, "2", c("#202020"), crate::transition::Spec::none(), 4));
    assert_eq!(shown(&choices, "DP-1", Some("2")), c("#101014"));
}

#[test]
fn a_cleared_mapping_is_not_brought_back_by_an_older_image() {
    let mut choices = Choices::default();
    choices.set(None, c("#c03020"), 1);
    assert!(choices.set_workspace(None, "2", c("#101014"), crate::transition::Spec::none(), 9));
    // Cleared at 11: the mapping is gone, and an image request of 7
    // landing after cannot bring it back.
    assert!(choices.set_workspace(None, "2", None, crate::transition::Spec::none(), 11));
    assert!(!choices.set_workspace(None, "2", c("#303030"), crate::transition::Spec::none(), 7));
    assert_eq!(shown(&choices, "DP-1", Some("2")), c("#c03020"));
    assert!(
        !choices.has_workspace_mappings(),
        "cleared ones do not count"
    );
}

#[test]
fn more_than_max_workspaces_is_refused() {
    let mut choices = Choices::default();
    for index in 0..super::MAX_WORKSPACES {
        assert!(
            choices.set_workspace(
                None,
                &format!("ws-{index}"),
                c("#101014"),
                crate::transition::Spec::none(),
                index as u64 + 1
            ),
            "key {index} fits"
        );
    }
    assert!(
        !choices.set_workspace(
            None,
            "one-too-many",
            c("#101014"),
            crate::transition::Spec::none(),
            1000
        ),
        "a new key past the bound is refused"
    );
    // Updating a key held already still works, and cleared keys do not
    // count: clearing one frees a slot.
    assert!(choices.set_workspace(
        None,
        "ws-0",
        c("#101014"),
        crate::transition::Spec::none(),
        1001
    ));
    assert!(choices.set_workspace(None, "ws-1", None, crate::transition::Spec::none(), 1002));
    assert!(choices.set_workspace(
        None,
        "one-more",
        c("#101014"),
        crate::transition::Spec::none(),
        1003
    ));
    assert!(choices.has_workspace_mappings());
}

#[test]
fn the_switch_transition_is_the_winning_mapping() {
    use crate::transition::{Kind, Spec};
    let mut choices = Choices::default();
    let mut generation = 0;
    generation += 1;
    choices.set(None, c("#c03020"), generation);
    generation += 1;
    assert!(choices.set_workspace(
        None,
        "2",
        c("#101014"),
        Spec {
            kind: Kind::Fade,
            ..Spec::none()
        },
        generation
    ));
    assert_eq!(
        choices.transition_for(Some("DP-1"), None),
        Spec::none(),
        "inactive: at once, like the base"
    );
    let spec = choices.transition_for(Some("DP-1"), Some("2"));
    assert_eq!(spec.kind, Kind::Fade);
    assert_eq!(shown(&choices, "DP-1", Some("2")), c("#101014"));
}

/// Adopting another profile clears every workspace mapping, and the buffer
/// accounting with it: what `adopt` (`daemon::config`) does before it drops
/// the stashes. The stash drop itself is `Canvas::drop_all_stash`
/// (`daemon::canvas`); this pins the choices half so a later change cannot
/// leave mappings behind while claiming to have adopted.
#[test]
fn clearing_workspaces_returns_buffer_accounting_to_base() {
    let mut choices = Choices::default();
    choices.set(None, c("#c03020"), 1);
    assert!(choices.set_workspace(None, "2", image(2), crate::transition::Spec::none(), 2));
    assert!(choices.set_workspace(None, "3", image(3), crate::transition::Spec::none(), 3));
    assert!(choices.has_workspace_mappings());
    assert_eq!(choices.workspace_images_for(None).len(), 2);
    choices.clear_workspaces();
    assert!(!choices.has_workspace_mappings());
    assert_eq!(
        choices.workspace_images_for(None).len(),
        0,
        "no mapping: no buffer to keep"
    );
    assert_eq!(
        choices.workspace_images_for(Some("DP-1")).len(),
        0,
        "no mapping on any output either"
    );
}

/// A slideshow step and a workspace mapping share one timeline: the newer
/// wins, whichever it is. A step is a base `set` at a newer generation
/// (`daemon::rotation` records and chooses exactly like a restored image);
/// a mapping set after a step wins until the next step, which covers it
/// again. `clear` (base) stops the show and wins outright; `clear
/// --workspace` only takes one mapping off.
#[test]
fn slideshow_steps_and_workspace_mappings_share_one_timeline() {
    let mut choices = Choices::default();
    // Base first, then a workspace mapping: the mapping shows there.
    choices.set(None, c("#c03020"), 1);
    assert!(choices.set_workspace(None, "2", image(2), crate::transition::Spec::none(), 2));
    assert_eq!(shown(&choices, "DP-1", Some("2")), image(2));
    assert_eq!(shown(&choices, "DP-1", Some("1")), c("#c03020"));
    // A slideshow step (a base image at a newer generation) covers the
    // older mapping until a still newer mapping.
    choices.set(None, image(3), 3);
    assert_eq!(
        shown(&choices, "DP-1", Some("2")),
        image(3),
        "the step is newer than the mapping"
    );
    // A mapping set after the step wins until the next step.
    assert!(choices.set_workspace(None, "2", image(4), crate::transition::Spec::none(), 4));
    assert_eq!(shown(&choices, "DP-1", Some("2")), image(4));
    // The next step covers it again.
    choices.set(None, image(5), 5);
    assert_eq!(
        shown(&choices, "DP-1", Some("2")),
        image(5),
        "the next step is newest again"
    );
    // A base `clear` wins outright (and stops the show in the daemon); a
    // workspace clear only takes one mapping off.
    choices.set(None, None, 6);
    assert_eq!(shown(&choices, "DP-1", Some("2")), None);
}
