//! A `reactive` popup is re-constrained when its parent moves; a
//! non-reactive twin is left alone.
//!
//! The ticket (`docs/backlog/core/popup-reactive-reconstrain.md`): a menu
//! slid inside the output at open time must be re-slid when its column
//! scrolls, answered with a fresh configure pair -- while a non-reactive
//! popup must never be re-configured (the protocol forbids it). Everything
//! here asserts on what the *client* was told, like the rest of this suite.

use scoot_core::{Action, Horizontal};

use super::*;

/// A 60x40 menu opened 10px left of the output's right edge, growing right:
/// 50px of it would be off the output, so `SlideX` pulls it back inside.
/// (The same shape as `window.rs`'s helper, for the middle window here.)
fn menu_near_right_edge(fixture: &Fixture, window: usize) -> Spec {
    let rect = fixture.rect_of(window);
    let right = fixture.output_rect(1).right() - rect.x;
    Spec::menu_at(right - 10, 20, 60, 40)
}

/// Three columns on a 200px output at the default half-width: the strip
/// holds two, so focusing the outer ones scrolls the middle window. Hands
/// back the middle window's index, with the strip showing the right two
/// columns (focus on the last window mapped).
fn three_columns(fixture: &mut Fixture) -> usize {
    fixture.window();
    let middle = fixture.window();
    fixture.window();
    middle
}

/// A reactive `SlideX` menu near the right edge is re-slid when its column
/// scrolls: the second configure arrives with the position re-measured
/// against where the parent is now, not where it was at open time.
#[test]
fn a_reactive_menu_is_re_slid_when_its_column_scrolls() {
    let mut fixture = Fixture::one_output();
    let middle = three_columns(&mut fixture);
    let spec = menu_near_right_edge(&fixture, middle)
        .adjust(Adjust::SlideX)
        .reactive();

    let before = fixture.rect_of(middle);
    let first = fixture.popup(Parent::Window(middle), spec);
    // Slid back inside: its right edge on the output's right edge.
    assert_eq!(first, (CANVAS - before.x - 60, 20, 60, 40));

    // A focus move that scrolls nothing re-configures nothing: the parent
    // did not move, so there is nothing to re-constrain against.
    fixture.state.act(Action::FocusColumn(Horizontal::Left));
    fixture.settle();
    assert_eq!(
        fixture.rect_of(middle),
        before,
        "focusing the already-visible middle column should not move it"
    );
    assert_eq!(
        fixture.popup_count(0),
        1,
        "an unmoved parent should earn no second configure"
    );

    // ...while scrolling its column left-to-right moves the parent right,
    // and the re-constrain answers with the re-slid position.
    fixture.state.act(Action::FocusColumn(Horizontal::Left));
    fixture.settle();
    let after = fixture.rect_of(middle);
    assert_ne!(after, before, "the scroll should have moved the parent");

    let second = fixture.await_popup(0, 1);
    assert_eq!(
        second,
        (CANVAS - after.x - 60, 20, 60, 40),
        "the second configure should re-slide against the parent's new position"
    );
    assert_ne!(second, first, "the re-slid position should differ");
}

/// The same scroll, the same menu, without `set_reactive`: the protocol
/// forbids re-configuring it, so it keeps its open-time geometry -- cut
/// again, and correctly so.
#[test]
fn a_non_reactive_menu_is_not_re_configured_when_its_column_scrolls() {
    let mut fixture = Fixture::one_output();
    let middle = three_columns(&mut fixture);
    let spec = menu_near_right_edge(&fixture, middle).adjust(Adjust::SlideX);

    let before = fixture.rect_of(middle);
    let first = fixture.popup(Parent::Window(middle), spec);
    assert_eq!(
        first,
        (CANVAS - before.x - 60, 20, 60, 40),
        "the twin starts slid inside (sanity)"
    );
    assert_eq!(fixture.popup_count(0), 1);

    fixture.state.act(Action::FocusColumn(Horizontal::Left));
    fixture.settle();
    fixture.state.act(Action::FocusColumn(Horizontal::Left));
    fixture.settle();
    assert_ne!(
        fixture.rect_of(middle),
        before,
        "the scroll should have moved the parent (sanity)"
    );
    assert_eq!(
        fixture.popup_count(0),
        1,
        "a non-reactive popup must never be re-configured"
    );
}

/// Both halves at once, on the same parent and the same scroll: the
/// reactive menu is re-slid, its non-reactive twin is untouched.
#[test]
fn a_reactive_menu_and_its_non_reactive_twin() {
    let mut fixture = Fixture::one_output();
    let middle = three_columns(&mut fixture);
    let slid = menu_near_right_edge(&fixture, middle).adjust(Adjust::SlideX);

    let first = fixture.popup(Parent::Window(middle), slid.reactive());
    let twin = fixture.popup(Parent::Window(middle), slid);
    assert_eq!(first, twin, "twins start from the same geometry");

    fixture.state.act(Action::FocusColumn(Horizontal::Left));
    fixture.settle();
    fixture.state.act(Action::FocusColumn(Horizontal::Left));
    fixture.settle();
    let after = fixture.rect_of(middle);

    let second = fixture.await_popup(0, 1);
    assert_eq!(second, (CANVAS - after.x - 60, 20, 60, 40));
    assert_eq!(
        fixture.popup_count(1),
        1,
        "the non-reactive twin should stay quiet through the same scroll"
    );
}

/// A re-constrain must not fight a `reposition` the client has not acked:
/// while the new positioner is in flight the popup is skipped, and once it
/// lands the re-constrain answers from it.
#[test]
fn a_reconstrain_does_not_fight_an_unacked_reposition() {
    let mut fixture = Fixture::one_output();
    let middle = three_columns(&mut fixture);
    let slid = menu_near_right_edge(&fixture, middle).adjust(Adjust::SlideX);
    let before = fixture.rect_of(middle);

    fixture.popup(Parent::Window(middle), slid.reactive());
    // A new positioner, lower down and further left so both its answer and
    // the next re-constrain's are distinguishable: still `SlideX`, still
    // reactive, still crossing the edge.
    let right = fixture.output_rect(1).right() - before.x;
    let moved = Spec::menu_at(right - 30, 50, 60, 40)
        .adjust(Adjust::SlideX)
        .reactive();
    let repositioned = fixture.reposition_without_ack(0, moved, 7);
    assert_eq!(
        repositioned,
        (CANVAS - before.x - 60, 50, 60, 40),
        "the reposition answers slid, from the new positioner (sanity)"
    );

    // Scroll while the new positioner is in flight: the re-constrain must
    // leave it alone rather than write the old positioner's answer over it.
    fixture.state.act(Action::FocusColumn(Horizontal::Left));
    fixture.settle();
    fixture.state.act(Action::FocusColumn(Horizontal::Left));
    fixture.settle();
    assert_ne!(
        fixture.rect_of(middle),
        before,
        "the scroll should have moved the parent (sanity)"
    );
    assert_eq!(
        fixture.popup_count(0),
        2,
        "an in-flight reposition must not earn a configure past its own answer"
    );

    // Once acked, a resize re-constrains from the new positioner: the
    // strip has only two views, so scrolling back would return to the
    // conditions the reposition already answered and owe nothing.
    fixture.done(Step::AckPopup { popup: 0 });
    assert!(
        fixture.state.resize_output(260, 200),
        "the output should resize"
    );
    fixture.settle();
    let after = fixture.rect_of(middle);
    // `SlideX` pulls left only as far as it must: the requested x, or the
    // new target's right edge minus the width, whichever is smaller.
    let target_right = 260 - after.x;
    let slid = (target_right - 60).min(right - 30);
    let second = fixture.await_popup(0, 2);
    assert_eq!(
        second,
        (slid, 50, 60, 40),
        "the re-constrain should answer from the repositioned positioner"
    );
}
