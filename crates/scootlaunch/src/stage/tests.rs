//! The stage's tests: the room a shadow asks for, where the shadow lands
//! around the panel, and the present through a pose.

use super::*;

const PANEL: [u8; 4] = [30, 20, 10, 255];

fn solid(pixels: &mut [u8], _: (u32, u32)) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&PANEL);
    }
}

fn scene_of(panel: (u32, u32), room: (u32, u32)) -> Scene {
    Scene {
        surface: (panel.0 + 2 * room.0, panel.1 + 2 * room.1),
        panel,
        origin: room,
        factor: 1.0,
        radius: 8,
        tint: Color {
            r: 250,
            g: 200,
            b: 40,
        },
    }
}

fn pixel(stage: &Stage, x: u32, y: u32) -> [u8; 4] {
    let width = stage.scene.width();
    let at = (y * width + x) as usize * 4;
    stage.scene.pixels()[at..at + 4].try_into().unwrap()
}

#[test]
fn the_room_is_the_larger_side_of_the_shadow_on_each_axis() {
    assert_eq!(room(ShadowStyle::None), (0, 0));
    // Soft: blur 12 and a 4 px drop reach 12 sideways, 8 above, 16 below.
    assert_eq!(room(ShadowStyle::Soft), (12, 16));
    assert_eq!(room(ShadowStyle::Deep), (28, 38));
    assert_eq!(room(ShadowStyle::Glow), (16, 16));
}

#[test]
fn a_surface_with_no_room_is_painted_straight_and_has_no_shadow() {
    let mut stage = Stage::new(ShadowStyle::Soft);
    let at = scene_of((40, 30), (0, 0));
    assert!(stage.compose(&at, solid));
    for y in 0..30 {
        for x in 0..40 {
            assert_eq!(pixel(&stage, x, y), PANEL);
        }
    }
}

#[test]
fn a_shadow_surrounds_the_panel_and_leaves_its_pixels_alone() {
    let mut stage = Stage::new(ShadowStyle::Soft);
    let room = room(ShadowStyle::Soft);
    let at = scene_of((60, 40), room);
    assert!(stage.compose(&at, solid));
    let (ox, oy) = at.origin;
    // The panel's middle is the panel, byte for byte.
    assert_eq!(pixel(&stage, ox + 30, oy + 20), PANEL);
    // Just below the panel (the drop): black at some strength.
    let below = pixel(&stage, ox + 30, oy + 40 + 2);
    assert!(below[3] > 0 && below[0] == 0 && below[1] == 0 && below[2] == 0);
    // Further out it fades, and the far corner is bare.
    let far = pixel(&stage, ox + 30, oy + 40 + room.1 - 1);
    assert!(far[3] < below[3], "{far:?} against {below:?}");
    assert_eq!(pixel(&stage, 0, 0), [0, 0, 0, 0]);
    // The shadow is symmetric left and right (no sideways offset), so the
    // panel is centered in its surface.
    for k in 0..room.0 {
        assert_eq!(
            pixel(&stage, ox - 1 - k, oy + 20),
            pixel(&stage, ox + 60 + k, oy + 20),
            "{k} px out"
        );
    }
}

#[test]
fn a_glow_takes_the_tint_and_a_soft_shadow_is_black() {
    let mut glow = Stage::new(ShadowStyle::Glow);
    let at = scene_of((60, 40), room(ShadowStyle::Glow));
    assert!(glow.compose(&at, solid));
    let near = pixel(&glow, at.origin.0 + 30, at.origin.1 + 40 + 1);
    // Premultiplied: the tint's channels scaled by the alpha.
    assert!(
        near[3] > 0 && near[2] > near[0] && near[1] > near[0],
        "{near:?}"
    );
}

#[test]
fn a_translucent_panel_does_not_show_its_own_shadow_through() {
    // The panel paints half-covered pixels; the shadow skips only what the
    // panel covers outright, so the interior stays the panel's own bytes.
    let half = [15u8, 10, 5, 128];
    let mut stage = Stage::new(ShadowStyle::Deep);
    let at = scene_of((80, 60), room(ShadowStyle::Deep));
    assert!(stage.compose(&at, |pixels, _| {
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&half);
        }
    }));
    // The translucent layer over a bare interior is the layer itself.
    assert_eq!(pixel(&stage, at.origin.0 + 40, at.origin.1 + 30), half);
}

#[test]
fn a_corner_of_the_panel_keeps_the_shadow_behind_its_cut_pixels() {
    let mut stage = Stage::new(ShadowStyle::Soft);
    let at = scene_of((60, 40), room(ShadowStyle::Soft));
    // A panel that leaves its corner pixel clear: the shadow shows there.
    assert!(stage.compose(&at, |pixels, dims| {
        solid(pixels, dims);
        pixels[0..4].copy_from_slice(&[0, 0, 0, 0]);
    }));
    let corner = pixel(&stage, at.origin.0, at.origin.1);
    assert!(
        corner[3] > 0,
        "{corner:?}: the shadow under a rounded corner"
    );
}

#[test]
fn the_tables_follow_the_scale_and_the_scene_is_the_same_at_the_same_scale() {
    let mut stage = Stage::new(ShadowStyle::Soft);
    let mut at = scene_of((60, 40), room(ShadowStyle::Soft));
    assert!(stage.compose(&at, solid));
    let first = stage.scene.pixels().to_vec();
    assert!(stage.compose(&at, solid));
    assert_eq!(stage.scene.pixels(), &first[..], "a second compose repeats");
    // Twice the scale: twice the blur, so a wider room is drawn.
    at.factor = 2.0;
    let wide = (room(ShadowStyle::Soft).0 * 2, room(ShadowStyle::Soft).1 * 2);
    at = Scene {
        surface: (120 + 2 * wide.0, 80 + 2 * wide.1),
        panel: (120, 80),
        origin: wide,
        radius: 16,
        ..at
    };
    assert!(stage.compose(&at, solid));
    let reach = pixel(&stage, at.origin.0 + 60, at.origin.1 + 80 + 20);
    assert!(reach[3] > 0, "the 2x shadow reaches 20 px below: {reach:?}");
}

#[test]
fn sizes_that_cannot_be_drawn_draw_nothing() {
    let mut stage = Stage::new(ShadowStyle::Soft);
    assert!(!stage.compose(&scene_of((0, 40), (4, 4)), solid));
    assert!(!stage.compose(&scene_of((40, 0), (4, 4)), solid));
    let mut huge = scene_of((40, 40), (4, 4));
    huge.surface = (u32::MAX, u32::MAX);
    assert!(!stage.compose(&huge, solid));
    // A panel larger than its surface (a compositor that configured
    // smaller) is painted and clipped, not a panic.
    let small = Scene {
        surface: (30, 30),
        ..scene_of((60, 40), (4, 4))
    };
    assert!(stage.compose(&small, solid));
}

fn composed(room_style: ShadowStyle) -> (Stage, Scene) {
    let mut stage = Stage::new(room_style);
    let at = scene_of((60, 40), room(room_style));
    assert!(stage.compose(&at, solid));
    (stage, at)
}

#[test]
fn present_at_rest_is_the_scene_and_clears_what_the_pool_left() {
    let (stage, at) = composed(ShadowStyle::Soft);
    let (w, h) = at.surface;
    let mut buffer = vec![0xAAu8; w as usize * h as usize * 4];
    stage.present(&mut buffer, at.surface, Pose::REST);
    assert_eq!(&buffer[..], stage.scene.pixels());
    // A scaled pose clears first: the corner outside the scaled scene is
    // bare, not the stale 0xAA.
    let mut buffer = vec![0xAAu8; w as usize * h as usize * 4];
    stage.present(
        &mut buffer,
        at.surface,
        Pose {
            scale: 0.5,
            alpha: 255,
        },
    );
    assert_eq!(&buffer[..4], &[0, 0, 0, 0]);
}

#[test]
fn present_fades_to_nothing_and_composites_a_half_alpha_half_way() {
    let (stage, at) = composed(ShadowStyle::Soft);
    let (w, h) = at.surface;
    let mut buffer = vec![0xAAu8; w as usize * h as usize * 4];
    stage.present(
        &mut buffer,
        at.surface,
        Pose {
            scale: 1.0,
            alpha: 0,
        },
    );
    assert!(buffer.iter().all(|byte| *byte == 0));
    stage.present(
        &mut buffer,
        at.surface,
        Pose {
            scale: 1.0,
            alpha: 128,
        },
    );
    let middle = ((at.origin.1 + 20) * w + at.origin.0 + 30) as usize * 4;
    let alpha = buffer[middle + 3];
    assert!((126..=130).contains(&alpha), "{alpha}");
}

#[test]
fn present_survives_a_buffer_that_is_not_the_scene() {
    let (stage, at) = composed(ShadowStyle::Soft);
    // A buffer larger than the scene (the compositor resized): composited
    // in the corner, no panic. A short slice draws nothing.
    let mut big = vec![0u8; 200 * 200 * 4];
    stage.present(&mut big, (200, 200), Pose::REST);
    let mut short = vec![0u8; 16];
    stage.present(&mut short, at.surface, Pose::REST);
    stage.present(&mut big, (0, 0), Pose::REST);
    stage.present(&mut big, (u32::MAX, u32::MAX), Pose::REST);
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 1e9] {
        stage.present(&mut big, (200, 200), Pose { scale, alpha: 200 });
    }
}

#[test]
fn a_look_without_a_shadow_or_motion_has_no_effects() {
    let effects = Effects::from_look(&Look::default());
    assert_eq!(effects.room, (0, 0));
    assert!(effects.stage.is_none());
    assert!(!effects.motion.active());
}

#[test]
fn a_shadow_or_motion_alone_turns_the_stage_on() {
    use scootui::motion::Curve;
    let shadowed = Look {
        shadow: ShadowStyle::Soft,
        ..Look::default()
    };
    let effects = Effects::from_look(&shadowed);
    assert_eq!(effects.room, (12, 16));
    assert!(effects.stage.is_some() && !effects.motion.active());
    let moving = Look {
        motion: Curve::EaseOut,
        ..Look::default()
    };
    let effects = Effects::from_look(&moving);
    assert_eq!(effects.room, (0, 0));
    assert!(effects.stage.is_some() && effects.motion.active());
    // A curve with no duration is no motion, and no stage.
    let nothing = Look {
        motion: Curve::Spring,
        motion_ms: 0,
        ..Look::default()
    };
    assert!(Effects::from_look(&nothing).stage.is_none());
}

#[test]
fn the_room_shrinks_to_fit_a_small_surface_and_never_moves_the_panel() {
    // As asked: a surface of the panel plus the room on both sides.
    assert_eq!(fit_room((12, 16), (424, 372), (400, 340)), (12, 16));
    // A surface the compositor held to the panel's own size: no room.
    assert_eq!(fit_room((12, 16), (400, 340), (400, 340)), (0, 0));
    // Smaller than the panel (a tiny output): none, no underflow.
    assert_eq!(fit_room((12, 16), (300, 200), (400, 340)), (0, 0));
    // Room for part of the shadow on one axis only; odd leftovers round down
    // so the panel stays centered.
    assert_eq!(fit_room((12, 16), (411, 372), (400, 340)), (5, 16));
    assert_eq!(fit_room((0, 0), (500, 500), (400, 340)), (0, 0));
    assert_eq!(
        fit_room((u32::MAX, u32::MAX), (u32::MAX, u32::MAX), (0, 0)),
        (u32::MAX / 2, u32::MAX / 2)
    );
}
