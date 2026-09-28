//! The scale a surface is drawn at, as the compositor's events set it:
//! `wl_output.scale`, `wl_surface.preferred_buffer_scale`, and
//! `wp_fractional_scale_v1.preferred_scale`, the best known winning.

use std::sync::Arc;

use super::{Effect, OutputId, Outputs, Size};
use crate::color::Color;
use crate::density::{Buffer, Scale};
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::paint::{Drawn, Plan};
use crate::waiters::Progress;
use crate::wallpaper::{Image, Wallpaper};

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

fn image() -> Wallpaper {
    Wallpaper::Image(Arc::new(Image {
        path: "/a.png".into(),
        look: Look {
            mode: Mode::Fill,
            fill: Color { r: 0, g: 0, b: 0 },
            filter: Filter::Lanczos3,
        },
        serial: 1,
    }))
}

/// scoot at 1.5 on a 1600×1000 mode: `wl_output` says 2.
fn at_one_and_a_half(outputs: &mut Outputs<()>) -> OutputId {
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.stage_mode(true, 1600, 1000);
    output.stage_scale(2);
    output.done();
    assert_eq!(output.settled(), Effect::Create);
    id
}

#[test]
fn the_fractional_scale_draws_at_device_pixels() {
    let mut outputs = Outputs::<()>::default();
    let id = at_one_and_a_half(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(1, 1067, 667);
    // With only `wl_output`'s 2: larger than the output, scaled down.
    assert_eq!(output.scale(), Scale::Integer(2));
    assert_eq!(
        output.full_buffer(),
        Some(Buffer {
            dims: (2134, 1334),
            scale: 2
        })
    );
    // The fraction: 1067 × 1.5 = 1600.5, drawn 1601 wide by the compositor.
    assert!(output.prefer_fractional(180));
    assert_eq!(output.scale(), Scale::Fractional(180));
    assert_eq!(
        output.full_buffer(),
        Some(Buffer {
            dims: (1601, 1001),
            scale: 1
        })
    );
    let wanted = image();
    let target = Drawn {
        content: wanted.clone(),
        size: size(1067, 667),
        scale: Scale::Fractional(180),
    };
    assert_eq!(
        output.plan(Some(&wanted), output.scale()),
        Plan::Show(target.clone())
    );
    output.drew(target);
    assert_eq!(output.plan(Some(&wanted), output.scale()), Plan::Nothing);
    assert_eq!(
        output.progress(Some(&wanted), output.scale()),
        Progress::Done
    );
    // The same scale again changes nothing.
    assert!(!output.prefer_fractional(180));
    // A new one redraws.
    assert!(output.prefer_fractional(150));
    assert!(matches!(
        output.plan(Some(&wanted), output.scale()),
        Plan::Show(_)
    ));
    assert_eq!(
        output.progress(Some(&wanted), output.scale()),
        Progress::Waiting
    );
}

#[test]
fn the_surface_integer_scale_beats_the_output_and_a_fraction_beats_both() {
    let mut outputs = Outputs::<()>::default();
    let id = at_one_and_a_half(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(1, 800, 500);
    assert!(output.prefer_buffer_scale(3));
    assert_eq!(output.scale(), Scale::Integer(3));
    assert_eq!(output.full_buffer().map(|b| b.dims), Some((2400, 1500)));
    assert!(!output.prefer_buffer_scale(0), "ignored");
    assert!(!output.prefer_buffer_scale(-1), "ignored");
    assert!(output.prefer_fractional(240));
    assert_eq!(output.scale(), Scale::Fractional(240));
    assert_eq!(
        output.full_buffer(),
        Some(Buffer {
            dims: (1600, 1000),
            scale: 1
        })
    );
    assert!(!output.prefer_fractional(0), "ignored");
    assert_eq!(output.scale(), Scale::Fractional(240));
}

/// Before the surface is configured, `query`'s `logical` is worked out:
/// with the fraction, 1067×667 rather than `wl_output`'s 800×500; and a
/// surface the compositor leaves the size of (0×0) is sized by it.
#[test]
fn the_fraction_makes_the_estimate_and_sizes_an_unsized_surface() {
    let mut outputs = Outputs::<()>::default();
    let id = at_one_and_a_half(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert_eq!(output.logical(), Some(size(800, 500)));
    assert!(output.prefer_fractional(180));
    assert_eq!(output.logical(), Some(size(1067, 667)));
    let _ = output.configure(1, 0, 0);
    assert_eq!(output.surface_size(), Some(size(1067, 667)));
    assert_eq!(output.full_buffer().map(|b| b.dims), Some((1601, 1001)));
    // One axis given, the other worked out with the fraction. A surface
    // narrower than the output (1000 of 1067) cannot tell a stale fraction
    // from a right one by the mode, so it is drawn at the integer scale:
    // larger, scaled down, never stretched.
    let _ = output.configure(2, 1000, 0);
    assert_eq!(output.surface_size(), Some(size(1000, 667)));
    assert_eq!(output.scale(), Scale::Integer(2));
    assert_eq!(output.full_buffer().map(|b| b.dims), Some((2000, 1334)));
    // Rotated: the axes swap before dividing.
    output.stage_transform(super::Transform::Rotate90);
    output.done();
    let _ = output.configure(3, 0, 0);
    assert_eq!(output.surface_size(), Some(size(667, 1067)));
    assert_eq!(output.full_buffer().map(|b| b.dims), Some((1001, 1601)));
}

/// The scales describe the output the surface is on: kept when the
/// compositor closes the surface and it is made again, so its first draw
/// is at the right size even before the new surface's events.
#[test]
fn a_recreated_surface_keeps_the_scale() {
    let mut outputs = Outputs::<()>::default();
    let id = at_one_and_a_half(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert!(output.prefer_fractional(180));
    let _ = output.configure(1, 1067, 667);
    assert_eq!(output.closed(), Effect::DestroyAndRetry);
    assert_eq!(output.full_buffer(), None, "no surface, no buffer");
    assert_eq!(output.retry(), Effect::Create);
    let _ = output.configure(2, 1067, 667);
    assert_eq!(output.scale(), Scale::Fractional(180));
    output.recreated();
    let _ = output.configure(3, 1067, 667);
    assert_eq!(output.full_buffer().map(|b| b.dims), Some((1601, 1001)));
}

/// 1 → 1.25 → 2 → 1 on a 1600×1000 mode, as scoot sizes the surface at
/// each: every step's buffer is the mode, so an image is never decoded
/// again; but each is a new scale on the surface, so it is redrawn (a new
/// viewport destination over the same buffer).
#[test]
fn a_round_of_scales_keeps_the_buffer_and_redraws() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.stage_mode(true, 1600, 1000);
    output.done();
    let _ = output.settled();
    let wanted = image();
    let mut last: Option<Drawn> = None;
    for (serial, (v120, logical)) in (1..).zip([
        (120, size(1600, 1000)),
        (150, size(1280, 800)),
        (240, size(800, 500)),
        (120, size(1600, 1000)),
    ]) {
        let _ = output.prefer_fractional(v120);
        let _ = output.configure(serial, logical.width, logical.height);
        assert_eq!(
            output.full_buffer(),
            Some(Buffer {
                dims: (1600, 1000),
                scale: 1
            }),
            "{v120}"
        );
        let Plan::Show(target) = output.plan(Some(&wanted), output.scale()) else {
            panic!("{v120}: not redrawn");
        };
        assert_eq!(target.size, logical);
        assert_ne!(Some(&target), last.as_ref());
        output.drew(target.clone());
        last = Some(target);
        assert_eq!(output.plan(Some(&wanted), output.scale()), Plan::Nothing);
    }
}

/// What a draw of an image asks the worker for (`Drawn::buffer`, through
/// `plan`) is exactly what the worker's results are kept for
/// (`Output::full_buffer`, through `daemon::change::image_dims`), on every
/// path and at every scale: if the two ever differed, the render would be
/// dropped as the wrong size and asked for again forever, and the `set`
/// would never be answered.
#[test]
fn a_draw_asks_for_the_size_a_render_is_kept_for() {
    use crate::paint::{Path, scale_for};
    let wanted = image();
    for path in [Path::SinglePixel, Path::ViewportShm, Path::FullShm] {
        for (fractional, buffer_scale, logical) in [
            (None, None, size(800, 500)),
            (None, Some(3), size(800, 500)),
            (Some(180), None, size(1067, 667)),
            (Some(150), Some(2), size(1280, 800)),
            (Some(120), Some(1), size(1600, 1000)),
            (Some(97), None, size(1979, 1237)),
        ] {
            let mut outputs = Outputs::<()>::default();
            let id = at_one_and_a_half(&mut outputs);
            let output = &mut outputs.get_mut(id).unwrap().output;
            if let Some(v120) = fractional {
                assert!(output.prefer_fractional(v120));
            }
            if let Some(factor) = buffer_scale {
                assert!(output.prefer_buffer_scale(factor));
            }
            let _ = output.configure(1, logical.width, logical.height);
            let Plan::Show(target) = output.plan(
                Some(&wanted),
                scale_for(path, Some(&wanted), output.scale()),
            ) else {
                panic!("{path:?}: nothing to draw");
            };
            assert_eq!(
                target.buffer(path),
                output.full_buffer(),
                "{path:?} {fractional:?} {buffer_scale:?}"
            );
        }
    }
}

/// sway at 1.5 on 1600×1000 with a surface still holding the 1.25 it was
/// made with: both round up to `wl_output`'s 2, so only the mode shows the
/// fraction is stale. The integer scale is used (larger, scaled down)
/// until the compositor sends the real one.
#[test]
fn a_fraction_short_of_the_mode_gives_way_to_the_integer() {
    let mut outputs = Outputs::<()>::default();
    let id = at_one_and_a_half(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert!(output.prefer_fractional(150));
    // Not configured: nothing to check against, the fraction stands.
    assert_eq!(output.scale(), Scale::Fractional(150));
    let _ = output.configure(1, 1066, 666);
    assert_eq!(output.scale(), Scale::Integer(2));
    assert_eq!(
        output.full_buffer(),
        Some(Buffer {
            dims: (2132, 1332),
            scale: 2
        })
    );
    assert!(output.prefer_fractional(180));
    assert_eq!(output.scale(), Scale::Fractional(180));
    assert_eq!(output.full_buffer().map(|b| b.dims), Some((1599, 999)));
    // Rotated: checked against the rotated mode.
    output.stage_transform(super::Transform::Rotate90);
    output.done();
    let _ = output.configure(2, 666, 1066);
    assert_eq!(output.scale(), Scale::Fractional(180));
    let _ = output.configure(3, 666, 1066);
    assert!(output.prefer_fractional(150));
    assert_eq!(output.scale(), Scale::Integer(2));
}

/// A draw that failed is not retried in a loop, but a new scale is a new
/// size, so a new chance, as a new `configure` is.
#[test]
fn a_new_scale_retries_a_failed_draw() {
    let mut outputs = Outputs::<()>::default();
    let id = at_one_and_a_half(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(1, 1067, 667);
    let wanted = image();
    output.draw_failed("a test".into());
    assert_eq!(output.plan(Some(&wanted), output.scale()), Plan::Nothing);
    assert_eq!(
        output.progress(Some(&wanted), output.scale()),
        Progress::Failed
    );
    assert!(!output.prefer_fractional(0), "no change, still failed");
    assert_eq!(
        output.progress(Some(&wanted), output.scale()),
        Progress::Failed
    );
    assert!(output.prefer_fractional(180));
    assert!(matches!(
        output.plan(Some(&wanted), output.scale()),
        Plan::Show(_)
    ));
    output.draw_failed("a test".into());
    assert!(output.prefer_buffer_scale(3));
    assert!(matches!(
        output.plan(Some(&wanted), output.scale()),
        Plan::Show(_)
    ));
    output.draw_failed("a test".into());
    // `wl_output.scale` too; a `done` that changes nothing does not.
    output.done();
    assert_eq!(
        output.progress(Some(&wanted), output.scale()),
        Progress::Failed
    );
    output.stage_scale(3);
    output.done();
    assert!(matches!(
        output.plan(Some(&wanted), output.scale()),
        Plan::Show(_)
    ));
}
