//! The bold face: summaries and `<b>` runs are measured and drawn with it
//! when the look's family has one, and with the old synthetic strike when
//! not. The default (no bold face) bytes are pinned in `tests`.

use std::collections::HashMap;

use ab_glyph::{FontArc, FontVec};
use scootui::paint::Canvas;
use scootui::text::Text;

use super::tests::{make_text, note};
use super::*;
use crate::look::Look;
use crate::model::store::Notification;
use crate::view::layout;

/// The test font plus a bold face of `8` and `x`, 900 units wide.
fn bold_text() -> Text {
    make_text().with_bold(FontArc::new(
        FontVec::try_from_vec(scootui::testfont::build_symbols_advance(['8', 'x'], 900)).unwrap(),
    ))
}

fn stack_of(text: &Text, look: &Look, notes: &[Notification]) -> layout::Stack {
    let live: Vec<&Notification> = notes.iter().collect();
    layout::layout_styled(&live, &Config::default(), text, 14.0, 2_000, look)
}

fn paint_with(text: &mut Text, stack: &layout::Stack, look: &Look) -> Vec<u8> {
    let urgencies = HashMap::from([(1, Urgency::Normal)]);
    let mut pixels = vec![0u8; stack.width as usize * stack.height as usize * 4];
    let mut canvas = Canvas::new(&mut pixels, stack.width, stack.height).unwrap();
    paint(
        &mut canvas,
        text,
        stack,
        &Config::default(),
        look,
        &urgencies,
        None,
        &HashMap::new(),
        None,
    );
    pixels
}

#[test]
fn a_regular_title_weight_makes_a_regular_summary() {
    let notes = [note(1, "88", "")];
    let mut look = Look::default();
    let text = bold_text();
    assert!(stack_of(&text, &look, &notes).cards[0].summary[0].runs[0].bold);
    look.title_weight = 400;
    assert!(!stack_of(&text, &look, &notes).cards[0].summary[0].runs[0].bold);
}

#[test]
fn the_wrap_measures_bold_text_with_the_bold_face() {
    // Thirty `8` fit on one line in regular (8.4 px each at 14 px) and not
    // in bold (12.6 px each): the summary wraps to a second line only
    // because the wrap used the face the line is drawn with.
    let notes = [note(1, &"8".repeat(30), "")];
    let look = Look::default();
    let regular = stack_of(&make_text(), &look, &notes);
    let bold = stack_of(&bold_text(), &look, &notes);
    assert_eq!(regular.cards[0].summary.len(), 1);
    assert_eq!(bold.cards[0].summary.len(), 2);
}

#[test]
fn a_bold_run_draws_in_the_bold_face_and_a_regular_run_does_not() {
    let look = Look::default();
    let plain = [note(1, "", "88")];
    let marked = [note(1, "", "<b>88</b>")];
    for (notes, differs) in [(&plain, false), (&marked, true)] {
        let with_bold = bold_text();
        let without = make_text();
        // The same stack (laid out with one text) painted with each text,
        // so only the face drawn can differ.
        let stack = stack_of(&without, &look, notes);
        let a = paint_with(&mut bold_text(), &stack, &look);
        let b = paint_with(&mut make_text(), &stack, &look);
        assert_eq!(a != b, differs, "bold: {differs}");
        assert!(with_bold.has_bold() && !without.has_bold());
    }
}

#[test]
fn a_heavier_title_adds_a_strike_to_the_bold_face() {
    let notes = [note(1, "88", "")];
    let mut look = Look::default();
    let mut paint_at = |weight: u16| {
        look.title_weight = weight;
        let text = bold_text();
        let stack = stack_of(&text, &look, &notes);
        paint_with(&mut bold_text(), &stack, &look)
    };
    let bold = paint_at(700);
    let heavy = paint_at(900);
    assert_ne!(bold, heavy, "900 strikes the bold face again");
    assert_eq!(bold, paint_at(600), "600 and 700 are both bold");
}
