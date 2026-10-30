use super::*;

/// A `w` × `h` layer filled with one premultiplied pixel.
fn filled(w: u32, h: u32, pixel: [u8; 4]) -> Layer {
    let mut layer = Layer::new();
    assert!(layer.begin(w, h));
    for chunk in layer.pixels.as_chunks_mut::<4>().0 {
        chunk.copy_from_slice(&pixel);
    }
    layer
}

fn canvas_of(pixels: &mut Vec<u8>, w: u32, h: u32, fill: [u8; 4]) -> Canvas<'_> {
    *pixels = fill.repeat((w * h) as usize);
    Canvas::new(pixels, w, h).expect("sized")
}

fn at(pixels: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = (y * w + x) as usize * 4;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

/// The straightforward per-channel source-over the blit must equal.
fn reference(dst: [u8; 4], src: [u8; 4], g: u32) -> [u8; 4] {
    let div = |v: u32| (v + 127) / 255;
    let s = src.map(|c| div(u32::from(c) * g));
    let inv = 255 - s[3];
    let mut out = [0u8; 4];
    for i in 0..4 {
        out[i] = (s[i] + div(u32::from(dst[i]) * inv)).min(255) as u8;
    }
    out
}

#[test]
fn begin_clears_reshapes_and_refuses_the_absurd() {
    let mut layer = filled(4, 4, [9, 9, 9, 9]);
    assert!(layer.begin(2, 3));
    assert_eq!((layer.width(), layer.height()), (2, 3));
    assert!(layer.pixels.iter().all(|b| *b == 0));
    assert!(!layer.begin(0, 5));
    assert!(layer.is_empty() && layer.canvas().is_none());
    assert!(!layer.begin(u32::MAX, u32::MAX));
    assert!(layer.is_empty());
    assert!(!layer.begin(MAX_LAYER_PIXELS as u32 + 1, 1));
    assert!(layer.begin(MAX_LAYER_PIXELS as u32, 1));
}

#[test]
fn an_opaque_layer_at_full_alpha_is_a_copy() {
    let layer = filled(3, 2, [10, 20, 30, 255]);
    let mut buf = Vec::new();
    let mut dst = canvas_of(&mut buf, 8, 6, [1, 2, 3, 255]);
    layer.composite(&mut dst, 2, 1, 1.0, 255);
    assert_eq!(at(&buf, 8, 2, 1), [10, 20, 30, 255]);
    assert_eq!(at(&buf, 8, 4, 2), [10, 20, 30, 255]);
    assert_eq!(at(&buf, 8, 1, 1), [1, 2, 3, 255]);
    assert_eq!(at(&buf, 8, 5, 1), [1, 2, 3, 255]);
    assert_eq!(at(&buf, 8, 2, 3), [1, 2, 3, 255]);
}

#[test]
fn every_blend_equals_the_reference_source_over() {
    let sources = [
        [0, 0, 0, 0],
        [40, 10, 5, 80],
        [200, 100, 50, 255],
        [7, 7, 7, 7],
    ];
    let dests = [[0, 0, 0, 0], [30, 30, 30, 255], [60, 40, 20, 128]];
    for src in sources {
        for dest in dests {
            for g in [1u32, 64, 127, 200, 254, 255] {
                let layer = filled(1, 1, src);
                let mut buf = Vec::new();
                let mut dst = canvas_of(&mut buf, 1, 1, dest);
                layer.composite(&mut dst, 0, 0, 1.0, g as u8);
                let got = at(&buf, 1, 0, 0);
                let want = reference(dest, src, g);
                for i in 0..4 {
                    assert!(
                        got[i].abs_diff(want[i]) <= 1,
                        "src {src:?} dst {dest:?} g {g}: {got:?} vs {want:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn zero_alpha_and_bad_scales_draw_nothing() {
    let layer = filled(4, 4, [255, 255, 255, 255]);
    for (scale, alpha) in [
        (1.0, 0),
        (0.0, 255),
        (-1.0, 255),
        (f32::NAN, 255),
        (f32::INFINITY, 255),
    ] {
        let mut buf = Vec::new();
        let mut dst = canvas_of(&mut buf, 8, 8, [0, 0, 0, 0]);
        layer.composite(&mut dst, 0, 0, scale, alpha);
        assert!(buf.iter().all(|b| *b == 0), "scale {scale} alpha {alpha}");
    }
    let empty = Layer::new();
    let mut buf = Vec::new();
    let mut dst = canvas_of(&mut buf, 2, 2, [0, 0, 0, 0]);
    empty.composite(&mut dst, 0, 0, 1.0, 255);
}

#[test]
fn clipping_at_every_edge_never_panics_and_keeps_the_visible_part() {
    let layer = filled(6, 6, [50, 60, 70, 255]);
    for (x, y) in [
        (-3, -3),
        (-100, 0),
        (0, -100),
        (5, 5),
        (100, 100),
        (i32::MAX, 0),
        (i32::MIN, i32::MIN),
        (0, i32::MAX),
        (-6, 0),
        (4, 4),
    ] {
        for scale in [1.0, 0.5, 1.5, 0.01, 40.0] {
            let mut buf = Vec::new();
            let mut dst = canvas_of(&mut buf, 8, 8, [0, 0, 0, 0]);
            layer.composite(&mut dst, x, y, scale, 200);
        }
    }
    // Hanging off the top-left corner by three pixels leaves a 3x3 corner.
    let mut buf = Vec::new();
    let mut dst = canvas_of(&mut buf, 8, 8, [0, 0, 0, 0]);
    layer.composite(&mut dst, -3, -3, 1.0, 255);
    assert_eq!(at(&buf, 8, 2, 2), [50, 60, 70, 255]);
    assert_eq!(at(&buf, 8, 3, 3), [0, 0, 0, 0]);
}

#[test]
fn a_scale_shrinks_about_the_center() {
    let layer = filled(10, 10, [100, 100, 100, 255]);
    let mut buf = Vec::new();
    let mut dst = canvas_of(&mut buf, 20, 20, [0, 0, 0, 0]);
    // Placed at (5, 5): a half-size image spans (7.5..12.5), i.e. 5x5 from 7.
    layer.composite(&mut dst, 5, 5, 0.5, 255);
    assert_eq!(at(&buf, 20, 9, 9), [100, 100, 100, 255]);
    assert_eq!(at(&buf, 20, 5, 5), [0, 0, 0, 0]);
    assert_eq!(at(&buf, 20, 13, 13), [0, 0, 0, 0]);
    let covered = (0..400).filter(|i| buf[i * 4 + 3] != 0).count();
    assert_eq!(covered, 25);
}

#[test]
fn a_uniform_layer_stays_uniform_when_scaled() {
    // Bilinear weights sum to one: no edge darkening or channel drift.
    let layer = filled(9, 7, [80, 60, 40, 255]);
    let mut buf = Vec::new();
    let mut dst = canvas_of(&mut buf, 30, 30, [0, 0, 0, 0]);
    layer.composite(&mut dst, 3, 3, 1.7, 255);
    let mut seen = 0;
    for chunk in buf.as_chunks::<4>().0 {
        if chunk[3] != 0 {
            seen += 1;
            assert_eq!(*chunk, [80, 60, 40, 255]);
        }
    }
    assert!(seen > 100);
}

#[test]
fn a_scale_of_almost_one_is_the_exact_copy() {
    let mut layer = Layer::new();
    assert!(layer.begin(5, 5));
    {
        let mut canvas = layer.canvas().expect("canvas");
        canvas.fill_rect(
            super::super::Span { x: 1, width: 3 },
            1,
            4,
            crate::color::Color { r: 9, g: 8, b: 7 },
        );
    }
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut da = canvas_of(&mut a, 9, 9, [3, 3, 3, 255]);
    let mut db = canvas_of(&mut b, 9, 9, [3, 3, 3, 255]);
    layer.composite(&mut da, 2, 2, 1.0, 255);
    layer.composite(&mut db, 2, 2, 1.003, 255);
    assert_eq!(a, b);
}

#[test]
fn compositing_allocates_nothing_and_a_pooled_layer_reuses_its_buffer() {
    let layer = filled(40, 30, [20, 30, 40, 200]);
    let mut buf = Vec::new();
    let mut dst = canvas_of(&mut buf, 64, 64, [5, 5, 5, 255]);
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for step in 0..50 {
            layer.composite(&mut dst, step - 10, step / 2, 1.0, 180);
            layer.composite(&mut dst, 3, 3, 0.8 + step as f32 / 500.0, 255);
        }
    });
    assert_eq!(allocations, 0);
    let mut pooled = filled(40, 30, [1, 1, 1, 1]);
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        assert!(pooled.begin(20, 20));
        assert!(pooled.begin(40, 30));
    });
    assert_eq!(allocations, 0);
}
