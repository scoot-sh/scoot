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
        self.0.for_output(name).cloned()
    }
}

#[test]
fn nothing_is_chosen_at_first() {
    let choices = Choices::default();
    assert_eq!(choices.for_output(Some("DP-1")), None);
    assert_eq!(choices.for_output(None), None);
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
    assert_eq!(choices.for_output(Some("DP-1")).cloned(), image(1));
    assert_eq!(choices.for_output(Some("DP-2")).cloned(), c("#101014"));

    // A newer every-output choice supersedes any older request entirely.
    let mut choices = Choices::default();
    assert!(choices.set(None, c("#222222"), 5));
    assert!(choices.supersedes(None, 4));
    assert!(choices.supersedes(Some("DP-1"), 4));
    assert!(!choices.set(None, image(4), 4));
    assert!(!choices.set(Some("DP-1"), image(3), 3));
    assert_eq!(choices.for_output(Some("DP-1")).cloned(), c("#222222"));
    // But not a newer one.
    assert!(!choices.supersedes(Some("DP-1"), 6));
    assert!(choices.set(Some("DP-1"), image(6), 6));
    assert_eq!(choices.for_output(Some("DP-1")).cloned(), image(6));

    // A newer choice by name supersedes an older one for that name only.
    let mut choices = Choices::default();
    assert!(choices.set(Some("DP-1"), c("#333333"), 8));
    assert!(choices.supersedes(Some("DP-1"), 7));
    assert!(!choices.supersedes(Some("DP-2"), 7));
    assert!(!choices.set(Some("DP-1"), image(7), 7));
    assert!(choices.set(Some("DP-2"), image(7), 7));
    assert_eq!(choices.for_output(Some("DP-1")).cloned(), c("#333333"));
    assert_eq!(choices.for_output(Some("DP-2")).cloned(), image(7));
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
    let names: Vec<_> = choices.named().map(|(n, c)| (n, c.is_some())).collect();
    assert_eq!(names, [("B", true), ("A", false)]);
    choices.set(None, None, 3);
    assert_eq!(choices.every(), Some(&None), "a clear of every output");
    assert_eq!(choices.named().count(), 0);
}
