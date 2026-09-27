use super::{Drawn, Path, Pick, SLOTS, Slot, pick};
use crate::color::Color;
use crate::outputs::Size;

#[test]
fn the_best_path_the_globals_allow_is_chosen() {
    assert_eq!(Path::choose(true, true, None), Path::SinglePixel);
    assert_eq!(Path::choose(true, false, None), Path::ViewportShm);
    // Single-pixel buffers without a viewporter would be one pixel big.
    assert_eq!(Path::choose(false, true, None), Path::FullShm);
    assert_eq!(Path::choose(false, false, None), Path::FullShm);
}

#[test]
fn forcing_only_picks_a_worse_path_that_is_possible() {
    use Path::*;
    for (viewporter, single_pixel) in [(true, true), (true, false), (false, true), (false, false)] {
        let best = Path::choose(viewporter, single_pixel, None);
        for forced in [SinglePixel, ViewportShm, FullShm] {
            let got = Path::choose(viewporter, single_pixel, Some(forced));
            assert!(got >= best, "never better than the globals allow");
            assert!(got >= forced || got == best);
            if forced >= best {
                assert_eq!(got, forced, "{viewporter} {single_pixel} {forced:?}");
            } else {
                assert_eq!(got, best);
            }
            // Never a path whose globals are missing.
            if got == SinglePixel {
                assert!(viewporter && single_pixel);
            }
            if got == ViewportShm {
                assert!(viewporter);
            }
        }
    }
}

#[cfg(debug_assertions)]
#[test]
fn path_names_round_trip() {
    for path in [Path::SinglePixel, Path::ViewportShm, Path::FullShm] {
        assert_eq!(Path::from_name(path.name()), Some(path));
    }
    assert_eq!(Path::from_name("fast"), None);
    assert_eq!(Path::from_name(""), None);
}

#[test]
fn only_a_full_size_buffer_takes_the_output_scale() {
    assert_eq!(Path::FullShm.buffer_scale(2), 2);
    assert_eq!(Path::FullShm.buffer_scale(0), 1, "never 0");
    assert_eq!(Path::SinglePixel.buffer_scale(3), 1);
    assert_eq!(Path::ViewportShm.buffer_scale(2), 1);
}

#[test]
fn buffer_dims_follow_the_path_and_never_overflow() {
    let drawn = |width, height, scale| Drawn {
        color: Color { r: 0, g: 0, b: 0 },
        size: Size { width, height },
        scale,
    };
    assert_eq!(
        drawn(1600, 1000, 1).buffer_dims(Path::SinglePixel),
        Some((1, 1))
    );
    assert_eq!(
        drawn(1600, 1000, 2).buffer_dims(Path::ViewportShm),
        Some((1, 1))
    );
    assert_eq!(
        drawn(1600, 1000, 2).buffer_dims(Path::FullShm),
        Some((3200, 2000))
    );
    assert_eq!(drawn(u32::MAX, 1, 2).buffer_dims(Path::FullShm), None);
    assert_eq!(
        drawn(1, u32::MAX / 2 + 1, 2).buffer_dims(Path::FullShm),
        None
    );
}

/// A slot for the decision: free or held, and a size.
struct Fake {
    free: bool,
    dims: (u32, u32),
}

impl Slot for Fake {
    fn is_free(&self) -> bool {
        self.free
    }
    fn dims(&self) -> (u32, u32) {
        self.dims
    }
}

fn slot(free: bool, dims: (u32, u32)) -> Option<Fake> {
    Some(Fake { free, dims })
}

#[test]
fn a_released_buffer_of_the_right_size_is_reused_not_reallocated() {
    const D: (u32, u32) = (1, 1);
    assert_eq!(pick(&[slot(true, D), None], D), Pick::Reuse(0));
    assert_eq!(pick(&[slot(false, D), slot(true, D)], D), Pick::Reuse(1));
    // Reuse beats an empty slot.
    assert_eq!(pick(&[None, slot(true, D)], D), Pick::Reuse(1));
}

#[test]
fn a_held_buffer_is_never_picked_so_a_second_one_is_made() {
    const D: (u32, u32) = (1, 1);
    assert_eq!(pick(&[slot(false, D), None], D), Pick::Fill(1));
    assert_eq!(pick(&[None, slot(false, D)], D), Pick::Fill(0));
    assert_eq!(pick::<Fake>(&[None, None], D), Pick::Fill(0));
}

#[test]
fn with_both_buffers_held_the_draw_waits() {
    const D: (u32, u32) = (8, 8);
    assert_eq!(pick(&[slot(false, D), slot(false, D)], D), Pick::Stall);
    assert_eq!(pick(&[slot(false, (4, 4)), slot(false, D)], D), Pick::Stall);
}

#[test]
fn a_free_buffer_of_the_old_size_is_replaced_before_a_new_slot_is_used() {
    let (old, new) = ((1600, 1000), (800, 500));
    assert_eq!(pick(&[slot(true, old), None], new), Pick::Replace(0));
    assert_eq!(
        pick(&[slot(false, old), slot(true, old)], new),
        Pick::Replace(1)
    );
    // The right size still wins over replacing.
    assert_eq!(
        pick(&[slot(true, old), slot(true, new)], new),
        Pick::Reuse(1)
    );
}

/// Whatever the slots hold, the answer is in range and never a held
/// buffer: every combination of empty, held and free, at two sizes.
#[test]
fn every_combination_picks_a_legal_slot() {
    let states = [
        None,
        Some((false, 1)),
        Some((true, 1)),
        Some((false, 2)),
        Some((true, 2)),
    ];
    for a in states {
        for b in states {
            let make = |s: Option<(bool, u32)>| s.map(|(free, d)| Fake { free, dims: (d, d) });
            let slots = [make(a), make(b)];
            let pick = pick(&slots, (1, 1));
            match pick {
                Pick::Reuse(i) => {
                    let s = slots[i].as_ref().unwrap();
                    assert!(s.free && s.dims == (1, 1));
                }
                Pick::Replace(i) => {
                    let s = slots[i].as_ref().unwrap();
                    assert!(s.free && s.dims != (1, 1));
                }
                Pick::Fill(i) => assert!(slots[i].is_none()),
                Pick::Stall => assert!(slots.iter().all(|s| s.as_ref().is_some_and(|s| !s.free))),
            }
            if let Pick::Reuse(i) | Pick::Replace(i) | Pick::Fill(i) = pick {
                assert!(i < SLOTS);
            }
        }
    }
}
