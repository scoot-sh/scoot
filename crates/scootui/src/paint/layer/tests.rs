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

#[test]
fn scaled_composites_reuse_the_pooled_scratch() {
    // The scaled path blends through the layer's pooled scratch row: warm
    // it once, then steady state allocates nothing even with clear rows
    // (a shadow's room) and fractional scales on every frame.
    let mut layer = Layer::new();
    assert!(layer.begin(40, 30));
    for (i, chunk) in layer
        .pixels_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .enumerate()
    {
        if i % 7 == 0 {
            chunk.copy_from_slice(&[0, 0, 0, 0]);
        } else {
            chunk.copy_from_slice(&[
                (i % 251) as u8,
                (i * 3 % 251) as u8,
                (i * 7 % 251) as u8,
                200,
            ]);
        }
    }
    let mut buf = Vec::new();
    let mut dst = canvas_of(&mut buf, 96, 96, [5, 5, 5, 255]);
    layer.composite(&mut dst, 3, 3, 1.7, 255);
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for step in 0..50 {
            layer.composite(&mut dst, 3, 3, 0.8 + step as f32 / 500.0, 255);
            layer.composite(&mut dst, step - 10, step / 2, 2.0, 180);
        }
    });
    assert_eq!(allocations, 0);
}

/// The pre-optimization column-first blend, kept as the byte-for-byte
/// reference for the row-first pooled path: identical integer arithmetic.
#[cfg(test)]
fn old_scaled_pixel(
    upper: &[u8],
    lower: &[u8],
    w: i64,
    tx0: i64,
    tx1: i64,
    wx: u32,
    wy: u32,
) -> [u8; 4] {
    let (at0, at1) = (tx0 as usize * 4, tx1 as usize * 4);
    let (a, b, c, e) = (
        &upper[at0..at0 + 4],
        &upper[at1..at1 + 4],
        &lower[at0..at0 + 4],
        &lower[at1..at1 + 4],
    );
    let _ = w;
    let mut mixed = [0u8; 4];
    for channel in 0..4 {
        let top = u32::from(a[channel]) * (256 - wx) + u32::from(b[channel]) * wx;
        let bottom = u32::from(c[channel]) * (256 - wx) + u32::from(e[channel]) * wx;
        mixed[channel] = ((top * (256 - wy) + bottom * wy + 0x8000) >> 16) as u8;
    }
    mixed
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)] // a test reference mirroring `composite`'s own shape.
fn old_composite(
    src: &[u8],
    sw: u32,
    sh: u32,
    dst: &mut [u8],
    dw_canvas: u32,
    dh_canvas: u32,
    x: i32,
    y: i32,
    scale: f32,
    alpha: u8,
) {
    let (w, h) = (i64::from(sw), i64::from(sh));
    let unscaled = (scale - 1.0).abs() < 1.0 / 256.0;
    let (dw, dh) = if unscaled {
        (w, h)
    } else {
        (
            ((w as f64 * f64::from(scale)).round() as i64).clamp(1, 8192),
            ((h as f64 * f64::from(scale)).round() as i64).clamp(1, 8192),
        )
    };
    let left = i64::from(x) + (w - dw).div_euclid(2);
    let top = i64::from(y) + (h - dh).div_euclid(2);
    let (cw, ch) = (i64::from(dw_canvas), i64::from(dh_canvas));
    let (x0, x1) = (left.max(0), (left + dw).min(cw));
    let (y0, y1) = (top.max(0), (top + dh).min(ch));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let g = u32::from(alpha);
    let src_stride = sw as usize * 4;
    let dst_stride = dw_canvas as usize * 4;
    assert!(dst.len() >= dw_canvas as usize * dh_canvas as usize * 4);
    for row in y0..y1 {
        let out_start = row as usize * dst_stride + x0 as usize * 4;
        let out_len = (x1 - x0) as usize * 4;
        if unscaled {
            let sy = (row - top) as usize;
            let from = sy * src_stride + (x0 - left) as usize * 4;
            let src_slice = &src[from..from + out_len];
            let out = &mut dst[out_start..out_start + out_len];
            for (d, s) in out
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src_slice.as_chunks::<4>().0)
            {
                super::over(d, *s, g);
            }
        } else {
            let step_x = (w << 16) / dw;
            let step_y = (h << 16) / dh;
            let fy = (row - top) * step_y + step_y / 2 - 0x8000;
            let (ty0, ty1, wy) = super::taps(fy, h);
            let upper = &src[ty0 as usize * src_stride..ty0 as usize * src_stride + src_stride];
            let lower = &src[ty1 as usize * src_stride..ty1 as usize * src_stride + src_stride];
            let out = &mut dst[out_start..out_start + out_len];
            for (offset, d) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let ox = x0 - left + offset as i64;
                let fx = ox * step_x + step_x / 2 - 0x8000;
                let (tx0, tx1, wx) = super::taps(fx, w);
                let at0 = tx0 as usize * 4;
                let at1 = tx1 as usize * 4;
                let all_clear = upper[at0..at0 + 4] == [0, 0, 0, 0]
                    && upper[at1..at1 + 4] == [0, 0, 0, 0]
                    && lower[at0..at0 + 4] == [0, 0, 0, 0]
                    && lower[at1..at1 + 4] == [0, 0, 0, 0];
                if all_clear {
                    continue;
                }
                let mixed = old_scaled_pixel(upper, lower, w, tx0, tx1, wx, wy);
                super::over(d, mixed, g);
            }
        }
    }
}

#[test]
fn scaled_matches_the_old_column_first_blend_byte_for_byte() {
    // Patterned content with clear runs (a shadow's room), opaque runs
    // and translucent pixels, over several sizes, scales, alphas and
    // offsets: every destination byte must equal the old blend.
    for (sw, sh) in [(1, 1), (2, 3), (5, 4), (9, 7), (16, 11), (40, 30)] {
        let mut layer = Layer::new();
        assert!(layer.begin(sw, sh));
        for (i, chunk) in layer
            .pixels_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .enumerate()
        {
            let (px, py) = ((i as u32) % sw, (i as u32) / sw);
            if (px + py) % 5 == 0 {
                chunk.copy_from_slice(&[0, 0, 0, 0]);
            } else {
                chunk.copy_from_slice(&[
                    ((px * 37 + py * 91) % 251) as u8,
                    ((px * 53 + py * 29 + 7) % 251) as u8,
                    ((px * 11 + py * 67 + 13) % 251) as u8,
                    [0, 64, 128, 200, 255][(px as usize + py as usize) % 5],
                ]);
            }
        }
        let src_snapshot = layer.pixels().to_vec();
        for scale in [0.5, 0.8, 0.96, 1.5, 1.7, 2.0, 2.96, 3.0] {
            for alpha in [1u8, 64, 128, 200, 255] {
                for (x, y) in [(0, 0), (-3, -2), (2, 1)] {
                    let (cw, ch) = (sw * 3 + 8, sh * 3 + 8);
                    let mut a = vec![0x5au8; cw as usize * ch as usize * 4];
                    let mut b = a.clone();
                    {
                        let mut ca = Canvas::new(&mut a, cw, ch).expect("sized");
                        layer.composite(&mut ca, x, y, scale, alpha);
                    }
                    old_composite(&src_snapshot, sw, sh, &mut b, cw, ch, x, y, scale, alpha);
                    assert_eq!(
                        a, b,
                        "mismatch at {sw}x{sh} scale {scale} alpha {alpha} offset ({x},{y})"
                    );
                }
            }
        }
    }
}

#[test]
fn a_layer_with_a_larger_limit_holds_what_the_default_refuses() {
    let side = 2048u32;
    let mut default = Layer::new();
    assert!(
        !default.begin(side, side),
        "4 Mpx is past the default limit"
    );
    let mut large = Layer::with_limit(side as usize * side as usize);
    assert!(large.begin(side, side));
    assert_eq!(large.pixels_mut().len(), side as usize * side as usize * 4);
    assert!(!large.begin(side, side + 1), "and not one row past its own");
    assert!(large.is_empty());
}

#[test]
fn the_pixel_slice_is_the_layer_and_empty_when_it_is_empty() {
    let mut layer = Layer::new();
    assert!(layer.pixels_mut().is_empty());
    assert!(layer.begin(3, 2));
    assert_eq!(layer.pixels_mut().len(), 24);
    layer.pixels_mut()[0..4].copy_from_slice(&[1, 2, 3, 4]);
    let mut dst = vec![0u8; 3 * 2 * 4];
    let mut canvas = Canvas::new(&mut dst, 3, 2).expect("canvas");
    layer.composite(&mut canvas, 0, 0, 1.0, 255);
    assert_eq!(&dst[0..4], &[1, 2, 3, 4]);
    // A shrinking begin keeps the slice exact, not the allocation's length.
    assert!(layer.begin(1, 1));
    assert_eq!(layer.pixels_mut().len(), 4);
}
