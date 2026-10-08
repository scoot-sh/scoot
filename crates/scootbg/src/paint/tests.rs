use std::sync::Arc;

use super::{Drawn, Path, Pick, SLOTS, Slot, pick, scale_for};
use crate::color::Color;
use crate::density::{Buffer, Scale};
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::wallpaper::{Image, Wallpaper};

fn image() -> Wallpaper {
    Wallpaper::Image(Arc::new(Image {
        path: "/a.png".into(),
        look: Look {
            mode: Mode::Fill,
            fill: Color { r: 0, g: 0, b: 0 },
            filter: Filter::Lanczos3,
        },
        animate: true,
        serial: 1,
        fetch: None,
    }))
}
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

/// A color on a viewport path is a 1×1 buffer at scale 1 whatever the
/// surface's scale, so a new scale leaves it alone; an image, and a color
/// on the full-size path, take the surface's scale, fractional included.
#[test]
fn only_a_full_size_buffer_takes_the_surface_scale() {
    let red = Wallpaper::Color(Color { r: 255, g: 0, b: 0 });
    let fraction = Scale::Fractional(180);
    for path in [Path::SinglePixel, Path::ViewportShm] {
        assert_eq!(scale_for(path, Some(&red), fraction), Scale::Integer(1));
        assert_eq!(scale_for(path, None, Scale::Integer(2)), Scale::Integer(1));
        assert_eq!(scale_for(path, Some(&image()), fraction), fraction);
    }
    assert_eq!(scale_for(Path::FullShm, Some(&red), fraction), fraction);
    assert_eq!(
        scale_for(Path::FullShm, Some(&image()), Scale::Integer(3)),
        Scale::Integer(3)
    );
}

#[test]
fn the_buffer_follows_the_path_and_the_scale_and_never_overflows() {
    let drawn = |content: Wallpaper, width, height, scale| Drawn {
        content,
        size: Size { width, height },
        scale,
    };
    let black = || Wallpaper::Color(Color { r: 0, g: 0, b: 0 });
    let one = Buffer {
        dims: (1, 1),
        scale: 1,
    };
    assert_eq!(
        drawn(black(), 1600, 1000, Scale::Integer(1)).buffer(Path::SinglePixel),
        Some(one)
    );
    assert_eq!(
        drawn(black(), 1600, 1000, Scale::Integer(2)).buffer(Path::ViewportShm),
        Some(one)
    );
    assert_eq!(
        drawn(black(), 800, 500, Scale::Integer(2)).buffer(Path::FullShm),
        Some(Buffer {
            dims: (1600, 1000),
            scale: 2
        })
    );
    // A fractional scale: device pixels at buffer scale 1, on any path for
    // an image, and on the full-size path for a color.
    let fractional = Some(Buffer {
        dims: (1601, 1001),
        scale: 1,
    });
    assert_eq!(
        drawn(black(), 1067, 667, Scale::Fractional(180)).buffer(Path::FullShm),
        fractional
    );
    for path in [Path::SinglePixel, Path::ViewportShm, Path::FullShm] {
        assert_eq!(
            drawn(image(), 1067, 667, Scale::Fractional(180)).buffer(path),
            fractional
        );
    }
    assert_eq!(
        drawn(black(), u32::MAX, 1, Scale::Integer(2)).buffer(Path::FullShm),
        None
    );
    assert_eq!(
        drawn(image(), 1, u32::MAX / 2 + 1, Scale::Integer(2)).buffer(Path::SinglePixel),
        None
    );
    assert_eq!(
        drawn(image(), u32::MAX, 1, Scale::Fractional(181)).buffer(Path::SinglePixel),
        None
    );
}

/// A slot for the decision: free or held, its pixels shared or not, and a
/// size.
struct Fake {
    free: bool,
    shared: bool,
    dims: (u32, u32),
}

impl Slot for Fake {
    fn is_free(&self) -> bool {
        self.free
    }
    fn is_writable(&self) -> bool {
        self.free && !self.shared
    }
    fn dims(&self) -> (u32, u32) {
        self.dims
    }
}

fn slot(free: bool, dims: (u32, u32)) -> Option<Fake> {
    Some(Fake {
        free,
        shared: false,
        dims,
    })
}

fn shared(free: bool, dims: (u32, u32)) -> Option<Fake> {
    Some(Fake {
        free,
        shared: true,
        dims,
    })
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
    assert_eq!(pick(&[shared(false, D), None], D), Pick::Fill(1));
}

/// Pixels another output shows are never written, even with this
/// output's own buffer released: the free buffer is dropped and a new one
/// made in its place.
#[test]
fn a_free_buffer_whose_pixels_are_shared_is_replaced_not_reused() {
    const D: (u32, u32) = (1600, 1000);
    assert_eq!(pick(&[shared(true, D), None], D), Pick::Replace(0));
    assert_eq!(pick(&[None, shared(true, D)], D), Pick::Replace(1));
    // An unshared one of the right size still wins.
    assert_eq!(pick(&[shared(true, D), slot(true, D)], D), Pick::Reuse(1));
}

#[test]
fn with_both_buffers_held_the_draw_waits() {
    const D: (u32, u32) = (8, 8);
    assert_eq!(pick(&[slot(false, D), slot(false, D)], D), Pick::Stall);
    assert_eq!(pick(&[slot(false, (4, 4)), slot(false, D)], D), Pick::Stall);
    assert_eq!(pick(&[shared(false, D), slot(false, D)], D), Pick::Stall);
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

/// Whatever the slots hold, the answer is in range, never a held buffer,
/// and never writes shared pixels: every combination of empty, held and
/// free, shared or not, at two sizes.
#[test]
fn every_combination_picks_a_legal_slot() {
    let mut states = vec![None];
    for free in [false, true] {
        for shared in [false, true] {
            for d in [1, 2] {
                states.push(Some((free, shared, d)));
            }
        }
    }
    for &a in &states {
        for &b in &states {
            let make = |s: Option<(bool, bool, u32)>| {
                s.map(|(free, shared, d)| Fake {
                    free,
                    shared,
                    dims: (d, d),
                })
            };
            let slots = [make(a), make(b)];
            let pick = pick(&slots, (1, 1));
            match pick {
                Pick::Reuse(i) => {
                    let s = slots[i].as_ref().unwrap();
                    assert!(s.free && !s.shared && s.dims == (1, 1));
                }
                Pick::Replace(i) => {
                    let s = slots[i].as_ref().unwrap();
                    assert!(s.free && (s.shared || s.dims != (1, 1)));
                    // Only when nothing could be reused.
                    assert!(
                        !slots
                            .iter()
                            .flatten()
                            .any(|s| s.is_writable() && s.dims == (1, 1))
                    );
                }
                Pick::Fill(i) => {
                    assert!(slots[i].is_none());
                    assert!(!slots.iter().flatten().any(Slot::is_free));
                }
                Pick::Stall => assert!(slots.iter().all(|s| s.as_ref().is_some_and(|s| !s.free))),
            }
            if let Pick::Reuse(i) | Pick::Replace(i) | Pick::Fill(i) = pick {
                assert!(i < SLOTS);
            }
        }
    }
}

/// An image is always a full-size buffer at the surface's scale, on every
/// path: only colors take the viewport shortcuts.
#[test]
fn an_image_is_full_size_on_every_path() {
    let color = Wallpaper::Color(Color { r: 1, g: 2, b: 3 });
    for path in [Path::SinglePixel, Path::ViewportShm, Path::FullShm] {
        let two = Scale::Integer(2);
        assert_eq!(scale_for(path, Some(&image()), two), two);
        let expected = if path == Path::FullShm {
            two
        } else {
            Scale::Integer(1)
        };
        assert_eq!(scale_for(path, Some(&color), two), expected);
        let drawn = Drawn {
            content: image(),
            size: Size {
                width: 1600,
                height: 1000,
            },
            scale: two,
        };
        assert_eq!(drawn.buffer(path).map(|b| b.dims), Some((3200, 2000)));
    }
}
