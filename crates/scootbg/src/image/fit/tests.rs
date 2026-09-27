use super::{Layout, Rect, layout};
use crate::image::Mode;

fn check(layout: &Layout, image: (u32, u32), target: (u32, u32)) {
    assert!(layout.crop.is_inside(image.0, image.1), "{layout:?}");
    let placed = Rect {
        x: layout.at.0,
        y: layout.at.1,
        width: layout.scaled.0,
        height: layout.scaled.1,
    };
    assert!(
        placed.is_inside(target.0, target.1),
        "{layout:?} on {target:?}"
    );
}

#[test]
fn fill_crops_the_overflow_centred() {
    // 6000×4000 onto 3840×2160: a 6000×3375 band, 312 rows off the top.
    let l = layout(Mode::Fill, (6000, 4000), (3840, 2160)).unwrap();
    assert_eq!(
        l.crop,
        Rect {
            x: 0,
            y: 312,
            width: 6000,
            height: 3375
        }
    );
    assert_eq!((l.scaled, l.at, l.tile), ((3840, 2160), (0, 0), false));
    assert!(!l.letterboxed((3840, 2160)));
    // A tall image on a wide output: full width, a band from the middle.
    let l = layout(Mode::Fill, (1000, 3000), (1600, 1000)).unwrap();
    assert_eq!(
        l.crop,
        Rect {
            x: 0,
            y: 1187,
            width: 1000,
            height: 625
        }
    );
    // The same aspect: no crop.
    let l = layout(Mode::Fill, (800, 500), (1600, 1000)).unwrap();
    assert_eq!(l.crop, Rect::whole(800, 500));
}

#[test]
fn fit_letterboxes_centred() {
    let l = layout(Mode::Fit, (6000, 4000), (3840, 2160)).unwrap();
    assert_eq!(l.crop, Rect::whole(6000, 4000));
    assert_eq!((l.scaled, l.at), ((3240, 2160), (300, 0)));
    assert!(l.letterboxed((3840, 2160)));
    let l = layout(Mode::Fit, (4000, 1000), (1600, 1000)).unwrap();
    assert_eq!((l.scaled, l.at), ((1600, 400), (0, 300)));
}

#[test]
fn stretch_scales_the_whole_image_to_the_target() {
    let l = layout(Mode::Stretch, (10, 3000), (1600, 1000)).unwrap();
    assert_eq!(l.crop, Rect::whole(10, 3000));
    assert_eq!((l.scaled, l.at), ((1600, 1000), (0, 0)));
}

#[test]
fn center_does_not_scale_and_crops_what_overflows() {
    let small = layout(Mode::Center, (100, 50), (1600, 1000)).unwrap();
    assert_eq!(small.crop, Rect::whole(100, 50));
    assert_eq!((small.scaled, small.at), ((100, 50), (750, 475)));
    assert!(!small.scales());
    let big = layout(Mode::Center, (2000, 1200), (1600, 1000)).unwrap();
    assert_eq!(
        big.crop,
        Rect {
            x: 200,
            y: 100,
            width: 1600,
            height: 1000
        }
    );
    assert_eq!(big.at, (0, 0));
    // Wider but shorter: cropped one way, letterboxed the other.
    let mixed = layout(Mode::Center, (2000, 500), (1600, 1000)).unwrap();
    assert_eq!(
        mixed.crop,
        Rect {
            x: 200,
            y: 0,
            width: 1600,
            height: 500
        }
    );
    assert_eq!(mixed.at, (0, 250));
}

#[test]
fn tile_reads_one_repeat_from_the_top_left() {
    let l = layout(Mode::Tile, (100, 50), (1600, 1000)).unwrap();
    assert_eq!(
        (l.crop, l.scaled, l.at, l.tile),
        (Rect::whole(100, 50), (100, 50), (0, 0), true)
    );
    assert!(!l.letterboxed((1600, 1000)));
    let big = layout(Mode::Tile, (5000, 50), (1600, 1000)).unwrap();
    assert_eq!(big.crop, Rect::whole(1600, 50));
}

#[test]
fn empty_images_or_targets_have_no_layout() {
    for mode in Mode::ALL {
        for (image, target) in [
            ((0, 10), (10, 10)),
            ((10, 0), (10, 10)),
            ((10, 10), (0, 10)),
            ((10, 10), (10, 0)),
            ((0, 0), (0, 0)),
        ] {
            assert_eq!(
                layout(mode, image, target),
                None,
                "{mode:?} {image:?} {target:?}"
            );
        }
    }
}

#[test]
fn one_pixel_and_extreme_aspects_stay_inside_and_non_empty() {
    let sizes = [
        (1, 1),
        (1, 2),
        (2, 1),
        (1, 20000),
        (20000, 1),
        (3, 16384),
        (16384, 16384),
        (7, 5),
        (1600, 1000),
        (65535, 1),
    ];
    for mode in Mode::ALL {
        for image in sizes {
            for target in sizes {
                let l = layout(mode, image, target).unwrap();
                check(&l, image, target);
                assert!(l.scaled.0 >= 1 && l.scaled.1 >= 1);
                match mode {
                    Mode::Fill | Mode::Stretch => assert_eq!(l.scaled, target),
                    Mode::Fit => assert!(l.scaled.0 == target.0 || l.scaled.1 == target.1),
                    Mode::Center | Mode::Tile => assert!(!l.scales()),
                }
            }
        }
    }
}

#[test]
fn the_largest_sizes_do_not_overflow() {
    let max = (u32::MAX, u32::MAX);
    for mode in Mode::ALL {
        for (image, target) in [
            (max, max),
            (max, (1, 1)),
            ((1, 1), max),
            ((u32::MAX, 1), (1, u32::MAX)),
        ] {
            let l = layout(mode, image, target).unwrap();
            check(&l, image, target);
        }
    }
}

#[test]
fn fill_keeps_the_aspect_to_half_a_pixel() {
    for image in [
        (6000, 4000),
        (4000, 6000),
        (1234, 777),
        (1920, 1080),
        (333, 3333),
    ] {
        for target in [(3840, 2160), (1600, 1000), (1080, 1920), (1, 1000)] {
            let l = layout(Mode::Fill, image, target).unwrap();
            // crop.w / crop.h vs target.w / target.h, on the cropped axis.
            let exact_w = f64::from(l.crop.height) * f64::from(target.0) / f64::from(target.1);
            let exact_h = f64::from(l.crop.width) * f64::from(target.1) / f64::from(target.0);
            let off = (f64::from(l.crop.width) - exact_w)
                .abs()
                .min((f64::from(l.crop.height) - exact_h).abs());
            assert!(
                off <= 0.5 || l.crop.width == 1 || l.crop.height == 1,
                "{image:?} {target:?} {l:?}"
            );
        }
    }
}
