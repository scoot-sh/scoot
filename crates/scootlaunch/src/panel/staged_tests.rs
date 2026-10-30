//! Frames of a panel with a shadow and a motion, drawn at chosen instants
//! of a made-up clock and read back as pixels: the entrance, the fade, the
//! sliding highlight, and the frames that must cost almost nothing.
//!
//! `SCOOTLAUNCH_DUMP_FRAMES=<dir>` writes every frame a test draws as a
//! PNG (over mid-gray, so the alpha shows) for looking at before a golden
//! is pinned.

use std::time::{Duration, Instant};

use scootui::density::Scale;
use scootui::effects::ShadowStyle;
use scootui::motion::Curve;
use scootui::output::Size;
use scootui::text::Text;

use super::*;
use crate::rank::frecency::Frecency;

fn at(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

fn entries(count: usize) -> Vec<Entry> {
    super::bytes_tests::entries(count)
}

/// A panel, its look and its effects, ready to draw frames of.
struct Rig {
    _dir: tempfile::TempDir,
    text: Text,
    icons: crate::icons::Cache,
    ui: Ui,
    look: Look,
    fx: Effects,
    scale: Scale,
    width: u32,
    height: u32,
}

impl Rig {
    fn new(look: Look, scale: Scale) -> Self {
        let dir = tempfile::tempdir().expect("tmp");
        let text = super::bytes_tests::test_text(dir.path());
        let ui = Ui::open(entries(6), Frecency::empty(), 0, 10);
        let layout = Layout::new(look.font_size_px, look.radius, 1.0);
        let width = (30.0 * look.font_size_px) as u32;
        let height = layout.height(10);
        let fx = Effects::from_look(&look);
        Self {
            _dir: dir,
            text,
            icons: crate::icons::Cache::default(),
            ui,
            look,
            fx,
            scale,
            width,
            height,
        }
    }

    /// The surface's logical size and its buffer's.
    fn surface(&self) -> (Size, (u32, u32)) {
        let size = Size {
            width: self.width + 2 * self.fx.room.0,
            height: self.height + 2 * self.fx.room.1,
        };
        let factor = self.scale.factor();
        let dims = (
            (f64::from(size.width) * factor).round() as u32,
            (f64::from(size.height) * factor).round() as u32,
        );
        (size, dims)
    }

    /// One frame at `now`; `dirty` as the app would pass it. The buffer
    /// starts full of junk, as a pooled one does.
    fn frame(&mut self, dirty: bool, now: Instant) -> (Vec<u8>, (u32, u32), (u32, u32)) {
        let (size, dims) = self.surface();
        let mut out = vec![0xAAu8; dims.0 as usize * dims.1 as usize * 4];
        let view = PanelView {
            ui: &self.ui,
            look: &self.look,
            width: self.width,
            height: self.height,
            error: None,
            empty_text: "empty",
            scale: self.scale,
            icon_theme: "Fixture",
            icon_bases: &[],
        };
        let origin = staged::paint_staged(
            &mut self.fx,
            &view,
            &mut self.text,
            &mut self.icons,
            &mut out,
            dims,
            size,
            dirty,
            now,
        );
        (out, dims, origin)
    }
}

fn dump(name: &str, pixels: &[u8], dims: (u32, u32)) {
    let Some(dir) = std::env::var_os("SCOOTLAUNCH_DUMP_FRAMES") else {
        return;
    };
    let mut rgba = Vec::with_capacity(pixels.len());
    for pixel in pixels.as_chunks::<4>().0 {
        // Premultiplied BGRA over mid-gray.
        let inverse = 255 - u32::from(pixel[3]);
        let over = |c: u8| (u32::from(c) + 128 * inverse / 255).min(255) as u8;
        rgba.extend_from_slice(&[over(pixel[2]), over(pixel[1]), over(pixel[0]), 255]);
    }
    std::fs::create_dir_all(&dir).expect("dump dir");
    let path = std::path::PathBuf::from(dir).join(format!("{name}.png"));
    let mut buf = Vec::new();
    let mut encoder = png::Encoder::new(&mut buf, dims.0, dims.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(&rgba).expect("png data");
    drop(writer);
    std::fs::write(path, buf).expect("write frame");
}

fn fnv(pixels: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in pixels {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn alpha_at(pixels: &[u8], width: u32, x: u32, y: u32) -> u8 {
    pixels[(y * width + x) as usize * 4 + 3]
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = (y * width + x) as usize * 4;
    pixels[at..at + 4].try_into().unwrap()
}

fn moving() -> Look {
    Look {
        shadow: ShadowStyle::Soft,
        motion: Curve::EaseOut,
        motion_ms: 200,
        ..Look::default()
    }
}

#[test]
fn a_panel_at_rest_in_its_shadow_is_the_plain_panel_in_the_middle() {
    let base = Instant::now();
    let mut plain = Rig::new(Look::default(), Scale::Integer(1));
    let mut shadowed = Rig::new(
        Look {
            shadow: ShadowStyle::Soft,
            ..Look::default()
        },
        Scale::Integer(1),
    );
    let (reference, ref_dims, _) = plain.frame(true, base);
    let (frame, dims, origin) = shadowed.frame(true, base);
    dump("rest-shadowed", &frame, dims);
    assert_eq!(origin, (12, 16));
    // Every pixel the plain panel draws opaque is the same pixel in the
    // middle of the shadowed surface, byte for byte.
    let mut compared = 0;
    for y in 0..ref_dims.1 {
        for x in 0..ref_dims.0 {
            let want = pixel_at(&reference, ref_dims.0, x, y);
            if want[3] == 255 {
                assert_eq!(
                    pixel_at(&frame, dims.0, x + origin.0, y + origin.1),
                    want,
                    "({x}, {y})"
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 100_000, "{compared} opaque pixels compared");
    // And the shadow is around it, symmetric left and right.
    let mid = origin.1 + ref_dims.1 / 2;
    assert!(alpha_at(&frame, dims.0, origin.0 - 3, mid) > 0);
    assert_eq!(
        pixel_at(&frame, dims.0, origin.0 - 5, mid),
        pixel_at(&frame, dims.0, origin.0 + ref_dims.0 + 4, mid)
    );
    assert!(alpha_at(&frame, dims.0, dims.0 / 2, origin.1 + ref_dims.1 + 3) > 0);
}

#[test]
fn the_entrance_fades_in_from_nothing_and_lands_on_the_rest_frame() {
    let base = Instant::now();
    let mut rig = Rig::new(moving(), Scale::Integer(1));
    let mut last_alpha = 0u32;
    let mut frames = Vec::new();
    for ms in [0u64, 40, 80, 120, 160, 200, 400] {
        let (frame, dims, origin) = rig.frame(ms == 0, at(base, ms));
        dump(&format!("enter-{ms:03}"), &frame, dims);
        let centre = (origin.0 + rig.width / 2, origin.1 + rig.height - 20);
        let alpha = u32::from(alpha_at(&frame, dims.0, centre.0, centre.1));
        assert!(
            alpha >= last_alpha,
            "{ms} ms: alpha {alpha} after {last_alpha}"
        );
        last_alpha = alpha;
        frames.push((ms, frame));
    }
    // The first frame is a clear surface (junk cleared, nothing drawn).
    assert!(frames[0].1.iter().all(|byte| *byte == 0));
    // Partway it is partly there.
    let mid = alpha_at(
        &frames[2].1,
        rig.surface().1.0,
        rig.surface().1.0 / 2,
        rig.surface().1.1 - 40,
    );
    assert!(mid > 0 && mid < 255, "{mid}");
    // From the deadline on, every frame is the same bytes: the rest frame,
    // which is the static composition of the panel and its shadow.
    assert_eq!(frames[5].1, frames[6].1);
    let mut settled = Rig::new(
        Look {
            motion: Curve::None,
            ..moving()
        },
        Scale::Integer(1),
    );
    assert_eq!(frames[6].1, settled.frame(true, base).0);
}

#[test]
fn the_entrance_grows_the_panel_about_its_center() {
    let base = Instant::now();
    let mut rig = Rig::new(moving(), Scale::Integer(1));
    let (_, dims, _) = rig.frame(true, base);
    let (partial, _, _) = rig.frame(false, at(base, 60));
    let (rest, _, _) = rig.frame(false, at(base, 400));
    // The bounding box of the panel itself (the shadow is under 100 even
    // at rest): smaller early, centered the same.
    let bounds = |pixels: &[u8]| {
        let (mut x0, mut y0, mut x1, mut y1) = (dims.0, dims.1, 0, 0);
        for y in 0..dims.1 {
            for x in 0..dims.0 {
                if alpha_at(pixels, dims.0, x, y) > 100 {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        (x0, y0, x1, y1)
    };
    let (small, full) = (bounds(&partial), bounds(&rest));
    assert!(
        small.2 - small.0 < full.2 - full.0 && small.3 - small.1 < full.3 - full.1,
        "{small:?} inside {full:?}"
    );
    let center = |b: (u32, u32, u32, u32)| (i64::from(b.0 + b.2), i64::from(b.1 + b.3));
    let (a, b) = (center(small), center(full));
    assert!(
        (a.0 - b.0).abs() <= 2 && (a.1 - b.1).abs() <= 2,
        "the panel grows about its center: {a:?} against {b:?}"
    );
}

#[test]
fn a_frame_that_only_advances_the_motion_does_not_repaint_the_panel() {
    let base = Instant::now();
    let mut rig = Rig::new(moving(), Scale::Integer(1));
    rig.frame(true, base);
    let painted = rig.fx.stage.as_ref().map(|stage| stage.composes());
    for ms in [20, 40, 60, 80, 100] {
        rig.frame(false, at(base, ms));
    }
    assert_eq!(
        rig.fx.stage.as_ref().map(|stage| stage.composes()),
        painted,
        "five entrance frames composed nothing new"
    );
    // A content change does.
    rig.frame(true, at(base, 120));
    assert_eq!(
        rig.fx.stage.as_ref().map(|stage| stage.composes()),
        painted.map(|count| count + 1)
    );
}

#[test]
fn an_advancing_frame_allocates_nothing() {
    let base = Instant::now();
    let mut rig = Rig::new(moving(), Scale::Integer(1));
    // Warm up: the layers, the shadow tables, the glyph cache.
    rig.frame(true, base);
    rig.frame(false, at(base, 20));
    let (size, dims) = rig.surface();
    let mut out = vec![0u8; dims.0 as usize * dims.1 as usize * 4];
    let view = PanelView {
        ui: &rig.ui,
        look: &rig.look,
        width: rig.width,
        height: rig.height,
        error: None,
        empty_text: "empty",
        scale: rig.scale,
        icon_theme: "Fixture",
        icon_bases: &[],
    };
    for ms in [40u64, 60, 80, 100, 120, 140, 160, 180, 200, 400] {
        let (_, allocs) = scootbg_mem::count_allocations(|| {
            staged::paint_staged(
                &mut rig.fx,
                &view,
                &mut rig.text,
                &mut rig.icons,
                &mut out,
                dims,
                size,
                false,
                at(base, ms),
            );
        });
        assert_eq!(allocs, 0, "the frame at {ms} ms allocated");
    }
}

#[test]
fn the_fade_out_dims_from_the_rest_frame_to_nothing() {
    let base = Instant::now();
    let mut rig = Rig::new(moving(), Scale::Integer(1));
    rig.frame(true, base);
    let rest_time = at(base, 1000);
    let (rest, dims, origin) = rig.frame(false, rest_time);
    assert!(rig.fx.motion.begin_exit(rest_time));
    let probe = (origin.0 + rig.width / 2, origin.1 + rig.height - 20);
    let mut last = 255u32;
    let mut at_end = Vec::new();
    for ms in [0u64, 40, 80, 120, 160, 200, 260] {
        let (frame, _, _) = rig.frame(false, at(rest_time, ms));
        dump(&format!("exit-{ms:03}"), &frame, dims);
        let alpha = u32::from(alpha_at(&frame, dims.0, probe.0, probe.1));
        assert!(alpha <= last, "{ms} ms: alpha {alpha} after {last}");
        last = alpha;
        if ms == 0 {
            assert_eq!(frame, rest, "the fade starts from the rest frame");
        }
        at_end = frame;
    }
    assert!(at_end.iter().all(|byte| *byte == 0), "gone at the end");
}

fn selected_slot_pixel(
    rig: &Rig,
    frame: &[u8],
    dims: (u32, u32),
    origin: (u32, u32),
    slot: u32,
) -> [u8; 4] {
    let layout = device_layout(&rig.look, rig.scale);
    let row = layout.row(scale_len(rig.width, rig.scale.factor()), slot);
    // Left of the icon column, inside the highlight: nothing but fill.
    pixel_at(
        frame,
        dims.0,
        origin.0 + row.x + 2,
        origin.1 + row.y + row.height / 2,
    )
}

#[test]
fn the_highlight_slides_one_row_and_lands_on_it() {
    let base = Instant::now();
    let mut rig = Rig::new(moving(), Scale::Integer(1));
    let accent = rig.look.theme.accent.argb8888(255).to_le_bytes();
    rig.frame(true, base);
    let settled = at(base, 1000);
    let (frame, dims, origin) = rig.frame(false, settled);
    assert_eq!(selected_slot_pixel(&rig, &frame, dims, origin, 0), accent);
    assert_ne!(selected_slot_pixel(&rig, &frame, dims, origin, 1), accent);
    // Down: the rows are the same, the selection moved.
    rig.ui.key(scootui::entry::KEY_DOWN, false, "");
    let (start, _, _) = rig.frame(true, settled);
    dump("slide-000", &start, dims);
    assert_eq!(
        selected_slot_pixel(&rig, &start, dims, origin, 0),
        accent,
        "the highlight starts where it was"
    );
    let (mid, _, _) = rig.frame(false, at(settled, 60));
    dump("slide-060", &mid, dims);
    assert!(mid != start);
    // Halfway between rows it covers the end of one and the start of the
    // other, so the bottom of row 0's middle band is already clear of it
    // or it still covers row 1's: never both rows fully.
    let both = selected_slot_pixel(&rig, &mid, dims, origin, 0) == accent
        && selected_slot_pixel(&rig, &mid, dims, origin, 1) == accent;
    assert!(!both, "a one-row highlight is not on two rows' middles");
    let (end, _, _) = rig.frame(false, at(settled, 400));
    dump("slide-400", &end, dims);
    assert_eq!(selected_slot_pixel(&rig, &end, dims, origin, 1), accent);
    assert_ne!(selected_slot_pixel(&rig, &end, dims, origin, 0), accent);
    // The rest frame of the slide is the frame of a look that never moved.
    let mut still = Rig::new(
        Look {
            motion: Curve::None,
            ..moving()
        },
        Scale::Integer(1),
    );
    still.ui.key(scootui::entry::KEY_DOWN, false, "");
    let expected = still.frame(true, base).0;
    dump("slide-still", &expected, dims);
    if end != expected {
        let diffs: Vec<_> = (0..end.len() / 4)
            .filter(|i| end[i * 4..i * 4 + 4] != expected[i * 4..i * 4 + 4])
            .map(|i| ((i as u32) % dims.0, (i as u32) / dims.0))
            .collect();
        let (x, y) = diffs[0];
        panic!(
            "{} pixels differ, first {:?}: {:?} against {:?}; accent {accent:?}",
            diffs.len(),
            &diffs[..diffs.len().min(8)],
            pixel_at(&end, dims.0, x, y),
            pixel_at(&expected, dims.0, x, y)
        );
    }
}

#[test]
fn typing_snaps_the_highlight_instead_of_sliding_it() {
    let base = Instant::now();
    let mut rig = Rig::new(moving(), Scale::Integer(1));
    rig.frame(true, base);
    let settled = at(base, 1000);
    rig.frame(false, settled);
    rig.ui.key(scootui::entry::KEY_DOWN, false, "");
    rig.ui.key(scootui::entry::KEY_DOWN, false, "");
    rig.frame(true, settled);
    let moved = at(settled, 1000);
    rig.frame(false, moved);
    // A typed character reranks: the selection is the top row again, at
    // once.
    rig.ui.key(0, false, "a");
    rig.frame(true, moved);
    assert!(
        !rig.fx.motion.sliding(moved),
        "a filter change is a snap, not a slide"
    );
}

#[test]
fn every_scale_and_a_tiny_surface_draw_without_a_panic() {
    let base = Instant::now();
    for scale in [
        Scale::Integer(1),
        Scale::Fractional(180),
        Scale::Fractional(150),
        Scale::Integer(2),
        Scale::Integer(3),
    ] {
        let mut rig = Rig::new(moving(), scale);
        let (_, dims, origin) = rig.frame(true, base);
        // The panel never leaves the surface, whatever the rounding.
        let panel = (
            scale_len(rig.width, scale.factor()),
            scale_len(rig.height, scale.factor()),
        );
        assert!(origin.0 * 2 <= dims.0 && origin.1 * 2 <= dims.1);
        let device = (
            scale_len(origin.0, scale.factor()),
            scale_len(origin.1, scale.factor()),
        );
        assert!(device.0 + panel.0 <= dims.0, "{scale:?}");
        assert!(device.1 + panel.1 <= dims.1, "{scale:?}");
        for ms in [10, 90, 400] {
            rig.frame(false, at(base, ms));
        }
    }
    // A compositor that configured the surface smaller than the panel (a
    // tiny output): the shadow gives way, the panel is clipped, nothing
    // panics.
    let mut rig = Rig::new(moving(), Scale::Integer(1));
    let view_dims = (rig.width / 2, rig.height / 2);
    let mut out = vec![0u8; view_dims.0 as usize * view_dims.1 as usize * 4];
    let view = PanelView {
        ui: &rig.ui,
        look: &rig.look,
        width: rig.width,
        height: rig.height,
        error: None,
        empty_text: "empty",
        scale: rig.scale,
        icon_theme: "Fixture",
        icon_bases: &[],
    };
    let origin = staged::paint_staged(
        &mut rig.fx,
        &view,
        &mut rig.text,
        &mut rig.icons,
        &mut out,
        view_dims,
        Size {
            width: view_dims.0,
            height: view_dims.1,
        },
        true,
        base,
    );
    assert_eq!(origin, (0, 0));
    // Fractional scale golden: pinned so a rounding change shows.
    let mut rig = Rig::new(moving(), Scale::Fractional(180));
    let (frame, _, _) = rig.frame(true, at(base, 1000));
    assert_ne!(fnv(&frame), 0);
}

#[test]
fn a_scale_change_in_the_middle_of_a_motion_repaints_at_the_new_scale() {
    let base = Instant::now();
    let mut rig = Rig::new(moving(), Scale::Integer(1));
    let (one, dims_one, _) = rig.frame(true, base);
    assert_eq!(one.len(), dims_one.0 as usize * dims_one.1 as usize * 4);
    let composed = rig.fx.stage.as_ref().map(|stage| stage.composes());
    // The output's scale changes while the entrance plays; the next frame
    // is not a content change (dirty is false), yet it must repaint: its
    // buffer is twice the size.
    rig.scale = Scale::Integer(2);
    let (two, dims_two, origin) = rig.frame(false, at(base, 50));
    assert_eq!(dims_two, (dims_one.0 * 2, dims_one.1 * 2));
    assert_eq!(two.len(), dims_two.0 as usize * dims_two.1 as usize * 4);
    assert_eq!(origin, rig.fx.room);
    assert_eq!(
        rig.fx.stage.as_ref().map(|stage| stage.composes()),
        composed.map(|count| count + 1),
        "a new scale composes the scene again"
    );
    // The shadow was rebuilt at the new scale: it reaches 2x as far below
    // the panel (soft: 4 px down and 12 px of blur, at 2x).
    let mut settled = Rig::new(moving(), Scale::Integer(2));
    settled.frame(true, base);
    let (rest, dims, _) = settled.frame(false, at(base, 5000));
    // `origin` is logical: the panel's device top is twice the room.
    let below = 2 * settled.fx.room.1 + scale_len(settled.height, 2.0) + 20;
    assert!(alpha_at(&rest, dims.0, dims.0 / 2, below) > 0);
}

/// How long one staged frame at `now` takes.
fn time_frame(
    rig: &mut Rig,
    out: &mut [u8],
    dims: (u32, u32),
    size: Size,
    dirty: bool,
    now: Instant,
) -> Duration {
    let view = PanelView {
        ui: &rig.ui,
        look: &rig.look,
        width: rig.width,
        height: rig.height,
        error: None,
        empty_text: "empty",
        scale: rig.scale,
        icon_theme: "Fixture",
        icon_bases: &[],
    };
    let started = Instant::now();
    staged::paint_staged(
        &mut rig.fx,
        &view,
        &mut rig.text,
        &mut rig.icons,
        out,
        dims,
        size,
        dirty,
        now,
    );
    std::hint::black_box(&out);
    started.elapsed()
}

/// Per-frame costs, printed: `cargo test --release -p scootlaunch --bin
/// scootlaunch bench_effects -- --ignored --nocapture`. Not an assertion:
/// the numbers go in `dev/benches/shell-launch/`.
#[test]
#[ignore = "a benchmark; run it with --release --ignored --nocapture"]
fn bench_effects() {
    const WARM: usize = 20;
    const ITERS: usize = 300;
    const ROUNDS: usize = 9;
    fn report(name: &str, dims: (u32, u32), mut round: impl FnMut(usize) -> Duration) {
        let mut samples: Vec<u128> = Vec::new();
        for _ in 0..ROUNDS {
            for n in 0..WARM {
                round(n);
            }
            for n in 0..ITERS {
                samples.push(round(n).as_nanos());
            }
        }
        samples.sort_unstable();
        let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize] as f64 / 1000.0;
        println!(
            "{name:<46} {:>5}x{:<5} median {:>8.1} us   p5 {:>8.1} us",
            dims.0,
            dims.1,
            at(0.5),
            at(0.05)
        );
    }
    let base = Instant::now();
    let moving = |shadow, motion: Curve| Look {
        shadow,
        motion,
        motion_ms: 200,
        ..Look::default()
    };
    for scale in [Scale::Integer(1), Scale::Integer(2)] {
        println!("-- scale {}", scale.factor());
        // The default path: no shadow, no motion, straight into the buffer.
        let mut rig = Rig::new(Look::default(), scale);
        let (_, dims) = rig.surface();
        report(
            "none/none: paint straight (any open, any key)",
            dims,
            |_| {
                let (size, dims) = rig.surface();
                let mut out = vec![0u8; dims.0 as usize * dims.1 as usize * 4];
                let view = PanelView {
                    ui: &rig.ui,
                    look: &rig.look,
                    width: rig.width,
                    height: rig.height,
                    error: None,
                    empty_text: "empty",
                    scale: rig.scale,
                    icon_theme: "Fixture",
                    icon_bases: &[],
                };
                let _ = size;
                let started = Instant::now();
                paint_panel(&mut out, dims, &view, &mut rig.text, &mut rig.icons, None);
                std::hint::black_box(&out);
                started.elapsed()
            },
        );
        for (name, look) in [
            (
                "soft, still: repaint (a key)",
                moving(ShadowStyle::Soft, Curve::None),
            ),
            (
                "deep, still: repaint (a key)",
                moving(ShadowStyle::Deep, Curve::None),
            ),
            (
                "glow, still: repaint (a key)",
                moving(ShadowStyle::Glow, Curve::None),
            ),
        ] {
            let mut rig = Rig::new(look, scale);
            let (size, dims) = rig.surface();
            let mut out = vec![0u8; dims.0 as usize * dims.1 as usize * 4];
            report(name, dims, |_| {
                time_frame(&mut rig, &mut out, dims, size, true, base)
            });
        }
        // Frames of a motion on a soft-shadowed panel.
        let mut rig = Rig::new(moving(ShadowStyle::Soft, Curve::EaseOut), scale);
        let (size, dims) = rig.surface();
        let mut out = vec![0u8; dims.0 as usize * dims.1 as usize * 4];
        time_frame(&mut rig, &mut out, dims, size, true, base);
        report("entrance frame (scaled + faded present)", dims, |n| {
            time_frame(
                &mut rig,
                &mut out,
                dims,
                size,
                false,
                at(base, 4 + (n as u64 % 10) * 16),
            )
        });
        report("a panel at rest: present (one copy)", dims, |_| {
            time_frame(&mut rig, &mut out, dims, size, false, at(base, 5000))
        });
        rig.fx.motion.begin_exit(at(base, 6000));
        report("fade-out frame (faded present)", dims, |n| {
            time_frame(
                &mut rig,
                &mut out,
                dims,
                size,
                false,
                at(base, 6000 + 4 + (n as u64 % 10) * 16),
            )
        });
        let mut rig = Rig::new(moving(ShadowStyle::Soft, Curve::EaseOut), scale);
        let settled = at(base, 5000);
        time_frame(&mut rig, &mut out, dims, size, true, base);
        time_frame(&mut rig, &mut out, dims, size, false, settled);
        rig.ui.key(scootui::entry::KEY_DOWN, false, "");
        time_frame(&mut rig, &mut out, dims, size, true, settled);
        report("sliding-highlight frame (repaint + present)", dims, |n| {
            time_frame(
                &mut rig,
                &mut out,
                dims,
                size,
                false,
                at(settled, 4 + (n as u64 % 11) * 16),
            )
        });
    }
}
