use super::*;

#[test]
fn the_outputs_a_surface_is_on_decide_the_integer_scale() {
    let mut outputs = OutputScales::<u8>::default();
    assert_eq!(outputs.current(), None, "on no output, nothing is guessed");
    outputs.bound(1);
    outputs.bound(2);
    outputs.bound(1);
    outputs.set_scale(&1, 2);
    outputs.set_scale(&2, 3);
    assert_eq!(outputs.current(), None, "bound is not entered");
    outputs.enter(&1);
    assert_eq!(outputs.current(), Some(2));
    outputs.enter(&2);
    outputs.enter(&2);
    assert_eq!(outputs.current(), Some(3), "the larger of the two");
    outputs.leave(&2);
    assert_eq!(outputs.current(), Some(2));
    outputs.set_scale(&1, 0);
    outputs.set_scale(&1, -4);
    outputs.set_scale(&9, 5);
    assert_eq!(
        outputs.current(),
        Some(2),
        "a broken factor changes nothing"
    );
    outputs.leave(&1);
    assert_eq!(outputs.current(), None);
    outputs.enter(&1);
    outputs.leave_all();
    assert_eq!(outputs.current(), None, "a destroyed surface is on none");
    // An output entered before it was known counts as 1.
    outputs.enter(&7);
    assert_eq!(outputs.current(), Some(1));
}

#[test]
fn a_removed_output_is_neither_known_nor_entered() {
    let mut outputs = OutputScales::<u8>::default();
    outputs.bound(1);
    outputs.bound(2);
    outputs.set_scale(&1, 2);
    outputs.set_scale(&2, 3);
    outputs.enter(&1);
    outputs.enter(&2);
    assert_eq!(outputs.current(), Some(3));
    outputs.remove(&2);
    assert_eq!(outputs.current(), Some(2), "the surface is on the one left");
    outputs.remove(&1);
    assert_eq!(outputs.current(), None);
    // Removing what was never there changes nothing, and a re-bound
    // output starts again at scale 1.
    outputs.remove(&9);
    outputs.bound(2);
    outputs.enter(&2);
    assert_eq!(outputs.current(), Some(1));
}
