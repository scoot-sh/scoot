//! The dmenu panel on the stage: the shadow, the fill gradient and the
//! entrance a look gives the apps panel, drawn at chosen instants of a
//! made-up clock and read back as pixels, and the scales the painter must
//! get right on its own (the launcher draws at scale 1 today, so scale 2
//! is covered here and nowhere live).
//!
//! `SCOOTLAUNCH_DUMP_FRAMES=<dir>` writes every frame as a PNG, as
//! `staged_tests` does.

use std::time::{Duration, Instant};

use scootui::color::Color;
use scootui::density::Scale;
use scootui::effects::{FillStyle, ShadowStyle};
use scootui::motion::Curve;
use scootui::output::Size;
use scootui::text::Text;

use super::staged::{Shape, stage_frame};
use super::*;
use crate::dmenu::DmenuUi;
use crate::stage::Effects;

fn at(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

fn look(shadow: ShadowStyle, fill: FillStyle, motion: Curve) -> Look {
    Look {
        shadow,
        fill,
        motion,
        motion_ms: 200,
        radius: 12,
        ..Look::default()
    }
}

/// How many list rows the rig's panel holds.
const ROWS: u32 = 6;

/// A dmenu panel, its look and its effects, ready to draw frames of.
struct Rig {
    _dir: tempfile::TempDir,
    text: Text,
    dui: DmenuUi,
    spec: DmenuSpec,
    look: Look,
    fx: Effects,
    scale: Scale,
    width: u32,
    height: u32,
    /// The list rows `height` holds.
    rows: u32,
}

impl Rig {
    fn new(look: Look, scale: Scale) -> Self {
        let dir = tempfile::tempdir().expect("tmp");
        let text = super::bytes_tests::test_text(dir.path());
        let lines: Vec<String> = (0..8).map(|n| format!("line {n}")).collect();
        let dui = DmenuUi::open(lines, ROWS as usize, false, false);
        let layout = Layout::dmenu(look.font_size_px);
        let width = (30.0 * look.font_size_px) as u32;
        let height = layout.height(ROWS);
        let spec = DmenuSpec {
            prompt: String::from("> "),
            placeholder: String::new(),
            index: false,
            prompt_color: look.theme.foreground,
            prompt_only: false,
        };
        let fx = Effects::from_look(&look);
        Self {
            _dir: dir,
            text,
            dui,
            spec,
            look,
            fx,
            scale,
            width,
            height,
            rows: ROWS,
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

    /// One staged frame at `now`, into `out`, the way `App::draw_frame`
    /// does it.
    fn frame_into(&mut self, out: &mut [u8], dirty: bool, now: Instant) -> (u32, u32) {
        let (size, dims) = self.surface();
        let shape = Shape {
            size: (self.width, self.height),
            radius: self.look.radius,
            tint: self.look.focus,
            scale: self.scale,
        };
        let (dui, text, look, spec) = (&self.dui, &mut self.text, &self.look, &self.spec);
        let (width, height, rows, scale) = (self.width, self.height, self.rows, self.scale);
        stage_frame(
            &mut self.fx,
            &shape,
            None,
            out,
            dims,
            size,
            dirty,
            now,
            |pixels, panel_dims, _| {
                paint_dmenu(
                    pixels, panel_dims, dui, text, look, width, height, rows, scale, spec,
                );
            },
        )
    }

    /// One frame at `now`; the buffer starts full of junk, as a pooled
    /// one does.
    fn frame(&mut self, dirty: bool, now: Instant) -> (Vec<u8>, (u32, u32), (u32, u32)) {
        let (_, dims) = self.surface();
        let mut out = vec![0xAAu8; dims.0 as usize * dims.1 as usize * 4];
        let origin = self.frame_into(&mut out, dirty, now);
        (out, dims, origin)
    }

    /// The plain panel's pixels (no stage), at this rig's size and scale.
    fn plain(&mut self) -> (Vec<u8>, (u32, u32)) {
        let factor = self.scale.factor();
        let dims = (
            (f64::from(self.width) * factor).round() as u32,
            (f64::from(self.height) * factor).round() as u32,
        );
        let mut out = vec![0u8; dims.0 as usize * dims.1 as usize * 4];
        paint_dmenu(
            &mut out,
            dims,
            &self.dui,
            &mut self.text,
            &self.look,
            self.width,
            self.height,
            self.rows,
            self.scale,
            &self.spec,
        );
        (out, dims)
    }
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = (y * width + x) as usize * 4;
    pixels[at..at + 4].try_into().unwrap()
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

#[test]
fn a_dmenu_at_rest_in_its_shadow_is_the_plain_panel_in_the_middle() {
    let base = Instant::now();
    let mut rig = Rig::new(
        look(ShadowStyle::Soft, FillStyle::Solid, Curve::None),
        Scale::Integer(1),
    );
    let (reference, ref_dims) = rig.plain();
    let (frame, dims, origin) = rig.frame(true, base);
    dump("dmenu-rest-shadowed", &frame, dims);
    assert_eq!(origin, rig.fx.room);
    assert!(origin.0 > 0 && origin.1 > 0, "a soft shadow has room");
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
    assert!(compared > 50_000, "{compared} opaque pixels compared");
    // The shadow is around it: translucent black beside the panel, and
    // clear at the surface's edge.
    let mid = origin.1 + ref_dims.1 / 2;
    let beside = pixel_at(&frame, dims.0, origin.0 - 3, mid);
    assert!(beside[3] > 0 && beside[3] < 255, "{beside:?}");
    assert_eq!(&beside[..3], &[0, 0, 0], "a shadow is black");
    assert_eq!(pixel_at(&frame, dims.0, 0, mid)[3], 0);
}

#[test]
fn a_gradient_fill_runs_from_light_at_the_top_to_dark_at_the_bottom() {
    let base = Instant::now();
    let flat = {
        let mut rig = Rig::new(
            look(ShadowStyle::None, FillStyle::Solid, Curve::None),
            Scale::Integer(1),
        );
        rig.plain().0
    };
    let mut rig = Rig::new(
        look(ShadowStyle::None, FillStyle::Gradient, Curve::None),
        Scale::Integer(1),
    );
    let (graded, dims) = rig.plain();
    assert_eq!(rig.fx.room, (0, 0));
    assert!(rig.fx.stage.is_none(), "no shadow and no motion: no stage");
    // A fill pixel between the border and the entry, top and bottom.
    let x = 40;
    let (top, bottom) = (6, dims.1 - 6);
    assert_ne!(
        pixel_at(&graded, dims.0, x, top),
        pixel_at(&flat, dims.0, x, top)
    );
    assert_ne!(
        pixel_at(&graded, dims.0, x, top),
        pixel_at(&graded, dims.0, x, bottom),
        "the fill is not flat"
    );
    let luma = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
    assert!(
        luma(pixel_at(&graded, dims.0, x, top)) > luma(pixel_at(&graded, dims.0, x, bottom)),
        "the light is at the top"
    );
    // And the staged panel is the same gradient.
    let mut staged = Rig::new(
        look(ShadowStyle::Soft, FillStyle::Gradient, Curve::None),
        Scale::Integer(1),
    );
    let (frame, fdims, origin) = staged.frame(true, base);
    assert_eq!(
        pixel_at(&frame, fdims.0, x + origin.0, top + origin.1),
        pixel_at(&graded, dims.0, x, top)
    );
}

#[test]
fn the_entrance_fades_in_and_lands_on_the_rest_frame() {
    let base = Instant::now();
    let moving = look(ShadowStyle::Soft, FillStyle::Gradient, Curve::EaseOut);
    let mut rig = Rig::new(moving.clone(), Scale::Integer(1));
    let mut last = 0u32;
    let mut frames = Vec::new();
    for ms in [0u64, 40, 80, 120, 160, 200, 400] {
        let (frame, dims, origin) = rig.frame(ms == 0, at(base, ms));
        dump(&format!("dmenu-enter-{ms:03}"), &frame, dims);
        let probe = (origin.0 + rig.width / 2, origin.1 + rig.height - 12);
        let alpha = u32::from(pixel_at(&frame, dims.0, probe.0, probe.1)[3]);
        assert!(alpha >= last, "{ms} ms: alpha {alpha} after {last}");
        last = alpha;
        frames.push(frame);
    }
    assert!(
        frames[0].iter().all(|byte| *byte == 0),
        "the first frame is clear"
    );
    assert_eq!(frames[5], frames[6], "settled by the deadline");
    let mut still = Rig::new(
        Look {
            motion: Curve::None,
            ..moving
        },
        Scale::Integer(1),
    );
    assert_eq!(frames[6], still.frame(true, base).0, "the rest frame");
}

#[test]
fn a_dmenu_has_no_sliding_highlight() {
    let base = Instant::now();
    let mut rig = Rig::new(
        look(ShadowStyle::Soft, FillStyle::Solid, Curve::EaseOut),
        Scale::Integer(1),
    );
    rig.frame(true, base);
    let settled = at(base, 1000);
    rig.frame(false, settled);
    // Down: the selection is on the next row at once, on this very frame,
    // and nothing is left moving.
    rig.dui.key(scootui::entry::KEY_DOWN, false, "");
    let (moved, _, _) = rig.frame(true, settled);
    assert!(!rig.fx.motion.sliding(settled));
    assert!(!rig.fx.motion.animating(settled), "at rest after one frame");
    let mut still = Rig::new(
        look(ShadowStyle::Soft, FillStyle::Solid, Curve::None),
        Scale::Integer(1),
    );
    still.dui.key(scootui::entry::KEY_DOWN, false, "");
    assert_eq!(moved, still.frame(true, base).0);
}

#[test]
fn a_frame_that_only_advances_the_entrance_does_not_repaint_the_panel() {
    let base = Instant::now();
    let mut rig = Rig::new(
        look(ShadowStyle::Soft, FillStyle::Solid, Curve::EaseOut),
        Scale::Integer(1),
    );
    rig.frame(true, base);
    let painted = rig.fx.stage.as_ref().map(|stage| stage.composes());
    for ms in [20, 40, 60, 80, 100] {
        rig.frame(false, at(base, ms));
    }
    assert_eq!(rig.fx.stage.as_ref().map(|stage| stage.composes()), painted);
    // A keystroke does.
    rig.dui.key(0, false, "l");
    rig.frame(true, at(base, 120));
    assert_eq!(
        rig.fx.stage.as_ref().map(|stage| stage.composes()),
        painted.map(|count| count + 1)
    );
}

#[test]
fn a_warm_dmenu_frame_allocates_nothing_in_the_entrance_and_at_rest() {
    // At every scale a compositor sends, and after the scale moved: the
    // layers and tables are rebuilt once for the new size, then reused.
    for (first, scale) in [
        (Scale::Integer(1), Scale::Integer(1)),
        (Scale::Integer(1), Scale::Fractional(180)),
        (Scale::Fractional(150), Scale::Integer(2)),
        (Scale::Integer(2), Scale::Fractional(210)),
    ] {
        let mut rig = Rig::new(
            look(ShadowStyle::Soft, FillStyle::Gradient, Curve::EaseOut),
            first,
        );
        let (_, dims) = rig.surface();
        let mut out = vec![0u8; dims.0 as usize * dims.1 as usize * 4];
        rig.frame_into(&mut out, true, Instant::now());
        rig.scale = scale;
        warm_dmenu_frames_allocate_nothing(rig);
    }
}

fn warm_dmenu_frames_allocate_nothing(mut rig: Rig) {
    let base = Instant::now();
    let (_, dims) = rig.surface();
    let mut out = vec![0u8; dims.0 as usize * dims.1 as usize * 4];
    // Warm up: the layers, the shadow tables, the glyph cache.
    rig.frame_into(&mut out, true, base);
    rig.frame_into(&mut out, false, at(base, 20));
    for ms in [40u64, 60, 80, 100, 120, 140, 160, 180, 200, 400, 800] {
        let (_, allocs) = scootbg_mem::count_allocations(|| {
            rig.frame_into(&mut out, false, at(base, ms));
        });
        assert_eq!(allocs, 0, "the frame at {ms} ms allocated");
    }
    // A keystroke's repaint, warmed up by one of the same.
    rig.dui.key(0, false, "l");
    rig.frame_into(&mut out, true, at(base, 900));
    rig.dui.key(0, false, "i");
    let (_, allocs) = scootbg_mem::count_allocations(|| {
        rig.frame_into(&mut out, true, at(base, 920));
    });
    assert_eq!(allocs, 0, "a repainted frame allocated");
}

#[test]
fn the_dmenu_draws_at_a_device_scale_with_the_border_scaled() {
    let base = Instant::now();
    // The border color is the look's focus; count its run along the
    // middle row from the panel's left edge.
    let border_run = |pixels: &[u8], width: u32, y: u32, from: u32| {
        let focus = Look::default().focus;
        let want = focus.argb8888(255).to_le_bytes();
        (from..width)
            .take_while(|x| pixel_at(pixels, width, *x, y) == want)
            .count() as u32
    };
    for (scale, factor) in [
        (Scale::Integer(1), 1.0),
        (Scale::Fractional(180), 1.5),
        (Scale::Integer(2), 2.0),
        (Scale::Fractional(240), 2.0),
        (Scale::Integer(3), 3.0),
    ] {
        let mut rig = Rig::new(
            look(ShadowStyle::None, FillStyle::Solid, Curve::None),
            scale,
        );
        let (frame, dims) = rig.plain();
        dump(&format!("dmenu-scale-{factor}"), &frame, dims);
        // The panel fills the device buffer: edge to edge.
        assert_eq!(
            dims,
            (
                (f64::from(rig.width) * factor).round() as u32,
                (f64::from(rig.height) * factor).round() as u32
            )
        );
        let mid = dims.1 / 2;
        assert_eq!(
            border_run(&frame, dims.0, mid, 0),
            (2.0 * factor).round() as u32,
            "the border at {factor}x"
        );
        // The prompt and entry are drawn, so the top row of widgets has
        // ink: the text is scaled too (a 1x draw in the corner would leave
        // the lower right of a scaled buffer empty).
        let layout = dmenu_layout(&rig.look, scale, rig.height, rig.rows);
        let last_row_y = layout.list_y() + 5 * layout.row_h + layout.row_h / 2;
        assert!(last_row_y < dims.1, "{factor}x: the rows fit the buffer");
        let selected_row_y = layout.list_y() + layout.row_h / 2;
        // The first row's highlight is the accent across the inner box.
        let accent = rig.look.theme.accent.argb8888(255).to_le_bytes();
        assert_eq!(
            pixel_at(&frame, dims.0, layout.inner() + 3, selected_row_y),
            accent,
            "{factor}x: the highlight is on the first row at its device place"
        );
    }
    // The stage at a device scale: the panel lands inside its room, scaled.
    for scale in [Scale::Fractional(180), Scale::Integer(2)] {
        let mut rig = Rig::new(
            look(ShadowStyle::Soft, FillStyle::Gradient, Curve::EaseOut),
            scale,
        );
        let (frame, dims, origin) = rig.frame(true, at(base, 0));
        let (settled, _, _) = rig.frame(false, at(base, 600));
        dump("dmenu-scale-staged", &settled, dims);
        assert!(
            frame.iter().all(|byte| *byte == 0),
            "{scale:?}: first frame"
        );
        let panel = (
            scale_len(rig.width, scale.factor()),
            scale_len(rig.height, scale.factor()),
        );
        let device = (
            scale_len(origin.0, scale.factor()),
            scale_len(origin.1, scale.factor()),
        );
        assert!(device.0 + panel.0 <= dims.0 && device.1 + panel.1 <= dims.1);
        // The panel's own edge is opaque, the shadow beside it is not.
        let mid = device.1 + panel.1 / 2;
        assert_eq!(pixel_at(&settled, dims.0, device.0 + 3, mid)[3], 255);
        let beside = pixel_at(&settled, dims.0, device.0 - 2, mid)[3];
        assert!(beside > 0 && beside < 255, "{scale:?}: shadow {beside}");
    }
}

#[test]
fn an_empty_prompt_only_or_tiny_dmenu_draws_without_a_panic() {
    let base = Instant::now();
    for (lines, prompt_only) in [(Vec::new(), false), (vec![String::from("a")], true)] {
        let mut rig = Rig::new(
            look(ShadowStyle::Deep, FillStyle::Gradient, Curve::Spring),
            Scale::Integer(1),
        );
        rig.dui = DmenuUi::open(lines, 6, false, prompt_only);
        rig.spec.prompt_only = prompt_only;
        rig.rows = if prompt_only { 0 } else { 6 };
        rig.height = Layout::dmenu(rig.look.font_size_px).height(rig.rows);
        for ms in [0, 30, 90, 600] {
            let (_, _, origin) = rig.frame(ms == 0, at(base, ms));
            assert_eq!(origin, rig.fx.room);
        }
    }
    // A surface the compositor made smaller than the panel (a tiny
    // output): the shadow gives way and the panel is clipped.
    let mut rig = Rig::new(
        look(ShadowStyle::Soft, FillStyle::Solid, Curve::EaseOut),
        Scale::Integer(1),
    );
    let dims = (rig.width / 2, rig.height / 2);
    let mut out = vec![0u8; dims.0 as usize * dims.1 as usize * 4];
    let shape = Shape {
        size: (rig.width, rig.height),
        radius: rig.look.radius,
        tint: Color { r: 1, g: 2, b: 3 },
        scale: rig.scale,
    };
    let (dui, text, look, spec) = (&rig.dui, &mut rig.text, &rig.look, &rig.spec);
    let origin = stage_frame(
        &mut rig.fx,
        &shape,
        None,
        &mut out,
        dims,
        Size {
            width: dims.0,
            height: dims.1,
        },
        true,
        base,
        |pixels, panel_dims, _| {
            paint_dmenu(
                pixels,
                panel_dims,
                dui,
                text,
                look,
                shape.size.0,
                shape.size.1,
                rig.rows,
                Scale::Integer(1),
                spec,
            );
        },
    );
    assert_eq!(origin, (0, 0));
}
