//! Stepping left and right across outputs, wrapping around the ring.
//!
//! The ring is geometry order, not creation order: outputs sort by the x of
//! their whole area, then by y, with their position in the output list
//! (creation order) breaking what is still tied. The two orders coincide
//! today -- outputs pack left to right -- and stating the geometry rule here
//! (rather than where the actions are handled) is what keeps the stepping
//! right once outputs become placeable (see
//! `docs/backlog/core/output-position-and-live-mode.md`, out of scope for
//! the stepping itself).
//!
//! The whole area sorts, not the usable one: a bar mapping or unmapping
//! re-measures the usable area, and the ring must not reorder under a held
//! key because of it.

use super::tree::Output;
use crate::messages::Horizontal;
use crate::types::OutputId;

/// The output left or right of `focused`, wrapping around.
///
/// `outputs` is the output list in creation order. Answers `None` when
/// there is nothing to step to: fewer than two outputs (with one output
/// either key is a no-op, and with two either key names the other), or a
/// `focused` id the list doesn't hold.
///
/// Pure over the list -- no [`World`](super::World), no Wayland, no I/O --
/// so the wrap and the ordering are testable here rather than against a
/// live session.
pub(super) fn neighbour_output(
    outputs: &[Output],
    focused: OutputId,
    direction: Horizontal,
) -> Option<OutputId> {
    if outputs.len() < 2 {
        return None;
    }
    let from = outputs.iter().position(|o| o.id == focused)?;
    // Geometry order: left to right, then top to bottom, by the whole
    // area's top-left corner; the creation position breaks ties, so every
    // key is distinct and identical geometries still cycle deterministically.
    let key = |index: usize| {
        let area = outputs[index].area;
        (area.x, area.y, index)
    };
    let from_key = key(from);
    // The nearest output on the asked-for side, when one exists; otherwise
    // the far end of the ring (the wrap). `from` itself is skipped
    // throughout: an output is never its own neighbour.
    let mut nearer: Option<((i32, i32, usize), OutputId)> = None;
    let mut far: Option<((i32, i32, usize), OutputId)> = None;
    for (index, output) in outputs.iter().enumerate() {
        if index == from {
            continue;
        }
        let candidate = key(index);
        let on_side = match direction {
            Horizontal::Left => candidate < from_key,
            Horizontal::Right => candidate > from_key,
        };
        if on_side {
            let better = match nearer {
                None => true,
                Some((best, _)) => match direction {
                    Horizontal::Left => candidate > best,
                    Horizontal::Right => candidate < best,
                },
            };
            if better {
                nearer = Some((candidate, output.id));
            }
        }
        let farther = match far {
            None => true,
            Some((best, _)) => match direction {
                Horizontal::Left => candidate > best,
                Horizontal::Right => candidate < best,
            },
        };
        if farther {
            far = Some((candidate, output.id));
        }
    }
    nearer.or(far).map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;

    const LEFT: Horizontal = Horizontal::Left;
    const RIGHT: Horizontal = Horizontal::Right;

    fn outputs(areas: &[(u64, Rect)]) -> Vec<Output> {
        areas
            .iter()
            .map(|&(id, area)| Output::new(OutputId(id), area))
            .collect()
    }

    #[test]
    fn no_outputs_step_nowhere() {
        let outputs = outputs(&[]);
        for dir in [LEFT, RIGHT] {
            assert_eq!(neighbour_output(&outputs, OutputId(1), dir), None);
        }
    }

    #[test]
    fn one_output_is_a_no_op_either_way() {
        let outputs = outputs(&[(1, Rect::new(0, 0, 1000, 600))]);
        for dir in [LEFT, RIGHT] {
            assert_eq!(neighbour_output(&outputs, OutputId(1), dir), None);
        }
    }

    #[test]
    fn two_outputs_each_name_the_other() {
        let outputs = outputs(&[
            (1, Rect::new(0, 0, 1000, 600)),
            (2, Rect::new(1000, 0, 1000, 600)),
        ]);
        assert_eq!(
            neighbour_output(&outputs, OutputId(1), RIGHT),
            Some(OutputId(2))
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(1), LEFT),
            Some(OutputId(2))
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(2), LEFT),
            Some(OutputId(1))
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(2), RIGHT),
            Some(OutputId(1))
        );
    }

    #[test]
    fn three_outputs_wrap_at_both_ends() {
        let outputs = outputs(&[
            (1, Rect::new(0, 0, 1000, 600)),
            (2, Rect::new(1000, 0, 1000, 600)),
            (3, Rect::new(2000, 0, 1000, 600)),
        ]);
        assert_eq!(
            neighbour_output(&outputs, OutputId(1), LEFT),
            Some(OutputId(3)),
            "left of the leftmost wraps to the rightmost"
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(3), RIGHT),
            Some(OutputId(1)),
            "right of the rightmost wraps to the leftmost"
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(2), LEFT),
            Some(OutputId(1))
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(2), RIGHT),
            Some(OutputId(3))
        );
    }

    #[test]
    fn geometry_order_wins_over_creation_order() {
        // Added middle-last: creation order is 1, 2, 3 but the ring runs
        // 1, 3, 2 by x. (Today the platform still packs left to right, so
        // this arrangement only arises in the test -- it is what placeable
        // outputs will produce for real.)
        let outputs = outputs(&[
            (1, Rect::new(0, 0, 1000, 600)),
            (2, Rect::new(2000, 0, 1000, 600)),
            (3, Rect::new(1000, 0, 1000, 600)),
        ]);
        assert_eq!(
            neighbour_output(&outputs, OutputId(1), RIGHT),
            Some(OutputId(3)),
            "right of 1 is the middle screen by x, not the later creation"
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(2), LEFT),
            Some(OutputId(3))
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(2), RIGHT),
            Some(OutputId(1)),
            "right of the rightmost by x wraps to the leftmost"
        );
    }

    #[test]
    fn identical_x_orders_by_y() {
        let outputs = outputs(&[
            (1, Rect::new(0, 600, 1000, 600)),
            (2, Rect::new(0, 0, 1000, 600)),
        ]);
        assert_eq!(
            neighbour_output(&outputs, OutputId(2), RIGHT),
            Some(OutputId(1)),
            "same x: the lower screen is right of the upper one"
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(1), LEFT),
            Some(OutputId(2))
        );
    }

    #[test]
    fn identical_geometry_falls_back_to_creation_order() {
        // Overlapping outputs (a mirror pair, or a mode still being
        // settled): the ring still has to be total, so creation order
        // decides.
        let outputs = outputs(&[
            (1, Rect::new(0, 0, 1000, 600)),
            (2, Rect::new(0, 0, 1000, 600)),
        ]);
        assert_eq!(
            neighbour_output(&outputs, OutputId(1), RIGHT),
            Some(OutputId(2))
        );
        assert_eq!(
            neighbour_output(&outputs, OutputId(1), LEFT),
            Some(OutputId(2))
        );
    }

    #[test]
    fn an_unknown_focused_id_steps_nowhere() {
        let outputs = outputs(&[
            (1, Rect::new(0, 0, 1000, 600)),
            (2, Rect::new(1000, 0, 1000, 600)),
        ]);
        for dir in [LEFT, RIGHT] {
            assert_eq!(neighbour_output(&outputs, OutputId(99), dir), None);
        }
    }
}
