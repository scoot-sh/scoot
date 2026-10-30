//! The motion paint's tests: a stack at rest is the static bytes, a golden
//! per entrance style and for a restack, a leaving card, the shadows that
//! fade with their cards, and the zero-allocation pin over a whole animated
//! frame. Time is explicit, so every frame is the same bytes on every run.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use scootui::motion::Curve;
use scootui::paint::{Canvas, Pad, Shadow};

use super::tests::{make_text, note};
use super::*;
use crate::config::Anchor;
use crate::look::{CardMotion, FillStyle, MotionTokens, ShadowStyle};
use crate::model::store::Notification;
use crate::view::layout;
use crate::view::motion::Motion;

struct Scene {
    text: Text,
    config: Config,
    look: Look,
    shadow: Option<Shadow>,
    urgencies: HashMap<u32, Urgency>,
    icons: HashMap<u32, IconBits>,
}

fn scene(shadow: ShadowStyle) -> Scene {
    let look = Look {
        shadow,
        fill: FillStyle::Solid,
        ..Look::default()
    };
    Scene {
        text: make_text(),
        config: Config::default(),
        shadow: look.shadow.spec().map(Shadow::new),
        look,
        urgencies: (1..=8).map(|id| (id, Urgency::Normal)).collect(),
        icons: HashMap::new(),
    }
}

fn notes(ids: &[u32]) -> Vec<Notification> {
    ids.iter()
        .map(|&id| {
            note(
                id,
                &format!("Card {id}"),
                "A body that wraps around a few times",
            )
        })
        .collect()
}

fn stack(scene: &mut Scene, ids: &[u32]) -> layout::Stack {
    let notes = notes(ids);
    let live: Vec<&Notification> = notes.iter().collect();
    let pad = scene.shadow.as_ref().map_or(Pad::ZERO, Shadow::pad);
    let stack = layout::inset(
        layout::layout(
            &live,
            &scene.config,
            &scene.text,
            scene.look.font_size_px,
            2_000,
        ),
        pad,
    );
    // An empty stack sizes no corners and would drop the tables in use.
    if let Some(shadow) = scene.shadow.as_mut().filter(|_| !stack.cards.is_empty()) {
        shadow.prepare(
            scene.config.radius,
            stack
                .cards
                .iter()
                .map(|card| (card.rect.width, card.rect.height)),
        );
    }
    stack
}

fn tokens(curve: Curve, enter: CardMotion) -> MotionTokens {
    MotionTokens {
        curve,
        duration_ms: 200,
        bounce: 1.0,
        enter,
        exit: CardMotion::Fade,
    }
}

fn paint_layers(scene: &mut Scene, motion: &mut Motion) {
    let Scene {
        text,
        config,
        look,
        shadow,
        urgencies,
        icons,
    } = scene;
    motion.paint_layers(None, |card, layer, button| {
        let urgency = urgencies.get(&card.id).copied().unwrap_or(Urgency::Normal);
        let mut canvas = layer.canvas().expect("layer");
        paint_layer(
            &mut canvas,
            text,
            card,
            config,
            look,
            urgency,
            button,
            icons,
            shadow.as_ref(),
        );
    });
}

/// One animated frame: the surface as tall as the layout or the motion
/// needs, painted at `t0 + ms`.
fn frame(
    scene: &mut Scene,
    stack: &layout::Stack,
    motion: &mut Motion,
    t0: Instant,
    ms: u64,
) -> (Vec<u8>, u32, u32) {
    let now = t0 + Duration::from_millis(ms);
    motion.settle(now);
    let (w, h) = (stack.width, stack.height.max(motion.hold()).max(1));
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
    if motion.active() {
        paint_layers(scene, motion);
        paint_moving(
            &mut canvas,
            &mut scene.text,
            stack,
            &scene.config,
            &scene.look,
            &scene.urgencies,
            None,
            &scene.icons,
            scene.shadow.as_ref(),
            motion,
            now,
        );
    } else {
        paint(
            &mut canvas,
            &mut scene.text,
            stack,
            &scene.config,
            &scene.look,
            &scene.urgencies,
            None,
            &scene.icons,
            scene.shadow.as_ref(),
        );
    }
    (pixels, w, h)
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = (y as usize * width as usize + x as usize) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

#[test]
fn a_stack_between_animations_is_the_static_paint_byte_for_byte() {
    for shadow in [ShadowStyle::None, ShadowStyle::Soft] {
        let mut scene = scene(shadow);
        let stack = stack(&mut scene, &[1, 2, 3]);
        let t0 = Instant::now();
        let (static_bytes, w, h) = {
            let mut motion = Motion::new(&MotionTokens::default(), Anchor::TopRight);
            frame(&mut scene, &stack, &mut motion, t0, 0)
        };
        // A motion look whose cards have all landed: same pixels.
        let mut motion = Motion::new(&tokens(Curve::Spring, CardMotion::Slide), Anchor::TopRight);
        motion.retarget(&stack_empty(&mut scene), &stack, t0);
        let (landed, w2, h2) = frame(&mut scene, &stack, &mut motion, t0, 5_000);
        assert_eq!((w, h), (w2, h2));
        assert!(static_bytes == landed, "{shadow:?}");
        // And painting through the moving path with nothing in flight is
        // the same bytes too.
        let mut idle = Motion::new(&tokens(Curve::Spring, CardMotion::Slide), Anchor::TopRight);
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
        paint_moving(
            &mut canvas,
            &mut scene.text,
            &stack,
            &scene.config,
            &scene.look,
            &scene.urgencies,
            None,
            &scene.icons,
            scene.shadow.as_ref(),
            &idle,
            t0,
        );
        assert!(pixels == static_bytes, "{shadow:?}");
        idle.reset();
    }
}

fn stack_empty(scene: &mut Scene) -> layout::Stack {
    stack(scene, &[])
}

/// The frames of one entrance: 0, 70, 140 and 200 ms of a 200 ms ease-out.
fn entrance(style: CardMotion) -> [u64; 4] {
    let mut scene = scene(ShadowStyle::Soft);
    let stack = stack(&mut scene, &[1]);
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, style), Anchor::TopRight);
    motion.retarget(&stack_empty(&mut scene), &stack, t0);
    let mut seen = [0; 4];
    for (slot, ms) in seen.iter_mut().zip([0, 70, 140, 200]) {
        let (pixels, _, _) = frame(&mut scene, &stack, &mut motion, t0, ms);
        *slot = fingerprint(&pixels);
        dump(
            &format!("enter-{style:?}-{ms}"),
            &pixels,
            stack.width,
            stack.height,
        );
    }
    seen
}

/// Writes a frame as a binary PPM when `SCOOT_DUMP_FRAMES` names a
/// directory, for looking at what a golden pins.
fn dump(name: &str, pixels: &[u8], w: u32, h: u32) {
    let Some(dir) = std::env::var_os("SCOOT_DUMP_FRAMES") else {
        return;
    };
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for chunk in pixels.as_chunks::<4>().0 {
        // Over a mid-gray desktop, straight from premultiplied.
        let a = u32::from(chunk[3]);
        for c in [chunk[2], chunk[1], chunk[0]] {
            out.push((u32::from(c) + (128 * (255 - a) + 127) / 255).min(255) as u8);
        }
    }
    let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.ppm")), out);
}

/// One golden set per entrance style. A change to any is a change to what
/// a look that opts into motion paints: re-record only on purpose, and look
/// at the frames first (`SCOOT_DUMP_FRAMES=dir`).
#[test]
fn each_entrance_paints_its_golden_frames() {
    let rest = entrance(CardMotion::Fade)[3];
    for (style, goldens) in [
        (CardMotion::Fade, GOLDEN_FADE),
        (CardMotion::Slide, GOLDEN_SLIDE),
        (CardMotion::Scale, GOLDEN_SCALE),
        (CardMotion::Drop, GOLDEN_DROP),
    ] {
        let seen = entrance(style);
        assert_eq!(seen, goldens, "{style:?}");
        // Every entrance starts invisible and ends on the same bytes.
        assert_eq!(seen[0], entrance(CardMotion::Fade)[0], "{style:?}");
        assert_eq!(seen[3], rest, "{style:?}");
        assert_ne!(seen[1], seen[0], "{style:?} shows something by 70 ms");
    }
}

const GOLDEN_FADE: [u64; 4] = [
    0xfa32_d706_003d_8325,
    0xf41b_fe3e_f92f_7f14,
    0x1eec_1af6_257f_fab5,
    0x1eb7_6945_e99a_2081,
];
const GOLDEN_SLIDE: [u64; 4] = [
    0xfa32_d706_003d_8325,
    0xdfa9_33b3_4082_2940,
    0x57e0_3e11_15ff_1715,
    0x1eb7_6945_e99a_2081,
];
const GOLDEN_SCALE: [u64; 4] = [
    0xfa32_d706_003d_8325,
    0xe2d2_9390_0257_1ecf,
    0x1eec_1af6_257f_fab5,
    0x1eb7_6945_e99a_2081,
];
const GOLDEN_DROP: [u64; 4] = [
    0xfa32_d706_003d_8325,
    0x98f2_2e24_a0c1_ebc4,
    0x17db_b2d0_4215_7ab5,
    0x1eb7_6945_e99a_2081,
];

/// The frames of closing card 2 out of three: card 1 settles up while 2
/// fades. 0, 100 and 200 ms.
fn restack(anchor: Anchor) -> [u64; 3] {
    let mut scene = scene(ShadowStyle::Soft);
    let before = stack(&mut scene, &[1, 2, 3]);
    let after = stack(&mut scene, &[1, 3]);
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), anchor);
    // Painted into the surface the motion holds, as the popup does.
    motion.retarget(&before, &after, t0);
    let mut seen = [0; 3];
    for (slot, ms) in seen.iter_mut().zip([0, 100, 200]) {
        let (pixels, w, h) = frame(&mut scene, &after, &mut motion, t0, ms);
        *slot = fingerprint(&pixels);
        dump(&format!("restack-{anchor:?}-{ms}"), &pixels, w, h);
    }
    seen
}

#[test]
fn a_restack_paints_its_golden_frames_for_both_edges() {
    let top = restack(Anchor::TopRight);
    let bottom = restack(Anchor::BottomRight);
    assert_eq!(
        top,
        [
            0xb2ef_9be1_95db_a1ec,
            0x1f9c_fcc6_0daf_a815,
            0x8ae6_25c6_697a_4bdd
        ]
    );
    assert_eq!(
        bottom,
        [
            0x2d17_0f8c_dc40_c764,
            0x4b8e_99c5_6421_126d,
            0x8ae6_25c6_697a_4bdd
        ]
    );
    // The last frame is the landed stack: the static paint of the layout.
    let mut scene = scene(ShadowStyle::Soft);
    let _ = stack(&mut scene, &[1, 2, 3]);
    let after = stack(&mut scene, &[1, 3]);
    let t0 = Instant::now();
    let mut idle = Motion::new(&MotionTokens::default(), Anchor::TopRight);
    let (rest, _, _) = frame(&mut scene, &after, &mut idle, t0, 0);
    assert_eq!(top[2], fingerprint(&rest));
    assert_eq!(bottom[2], fingerprint(&rest));
    assert_ne!(top[0], top[1]);
    assert_ne!(top[1], top[2]);
}

#[test]
fn a_leaving_card_is_in_the_frame_until_it_lands_and_not_after() {
    let mut scene = scene(ShadowStyle::None);
    let before = stack(&mut scene, &[1]);
    let after = stack(&mut scene, &[]);
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    motion.retarget(&before, &after, t0);
    // The surface is held at the old size for the ghost; the empty
    // layout is zero tall.
    let mut empty = after.clone();
    empty.width = before.width;
    let (first, w, h) = frame(&mut scene, &empty, &mut motion, t0, 0);
    assert_eq!((w, h), (before.width, before.height));
    let covered = |bytes: &[u8]| {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] != 0)
            .count()
    };
    assert!(
        covered(&first) > 10_000,
        "the card is still on screen at 0 ms"
    );
    let (mid, _, _) = frame(&mut scene, &empty, &mut motion, t0, 100);
    assert!(covered(&mid) > 0 && mid != first);
    // Landed: nothing is left to paint, and nothing is in flight.
    let (gone, _, _) = frame(&mut scene, &empty, &mut motion, t0, 200);
    assert!(!motion.active());
    assert_eq!(covered(&gone), 0);
}

#[test]
fn a_fading_card_fades_its_shadow_with_it() {
    let mut scene = scene(ShadowStyle::Deep);
    let stack = stack(&mut scene, &[1]);
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    motion.retarget(&stack_empty(&mut scene), &stack, t0);
    // Under the card, in the shadow's band (below the card's bottom edge).
    let card = &stack.cards[0];
    let (x, y) = (
        card.rect.x + card.rect.width / 2,
        card.rect.y + card.rect.height + 4,
    );
    let (landed, w, _) = {
        let mut idle = Motion::new(&MotionTokens::default(), Anchor::TopRight);
        frame(&mut scene, &stack, &mut idle, t0, 0)
    };
    let full = pixel(&landed, w, x, y)[3];
    assert!(full > 0, "a deep shadow sits below the card");
    let mut last = 0;
    for ms in [20, 60, 100, 140, 180] {
        let (pixels, _, _) = frame(&mut scene, &stack, &mut motion, t0, ms);
        let alpha = pixel(&pixels, w, x, y)[3];
        assert!(alpha >= last && alpha <= full, "{ms} ms: {alpha}");
        last = alpha;
    }
    assert!(last > 0 && last < full + 1);
}

#[test]
fn a_hovered_button_repaints_a_moving_card_once() {
    let mut scene = scene(ShadowStyle::None);
    let mut with_button = notes(&[1]);
    with_button[0].actions = vec![crate::bus::notify::Action {
        key: "open".to_owned(),
        label: "Open".to_owned(),
    }];
    let live: Vec<&Notification> = with_button.iter().collect();
    let stack = layout::layout(&live, &scene.config, &scene.text, 13.0, 2_000);
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    motion.retarget(&stack_empty(&mut scene), &stack, t0);
    let painted = std::cell::Cell::new(0);
    let run = |motion: &mut Motion, hover| {
        motion.paint_layers(hover, |_, layer, _| {
            painted.set(painted.get() + 1);
            assert!(layer.canvas().is_some());
        });
    };
    run(&mut motion, None);
    assert_eq!(painted.get(), 1, "the first paint");
    run(&mut motion, None);
    assert_eq!(painted.get(), 1, "nothing changed: no repaint");
    run(&mut motion, Some((1, Some(0))));
    assert_eq!(painted.get(), 2, "the button became hovered");
    run(&mut motion, Some((1, Some(0))));
    assert_eq!(painted.get(), 2);
    run(&mut motion, Some((9, Some(0))));
    assert_eq!(painted.get(), 3, "another card's hover is not this card's");
}

#[test]
fn an_animated_frame_allocates_nothing() {
    let mut scene = scene(ShadowStyle::Soft);
    let before = stack(&mut scene, &[1, 2, 3]);
    let after = stack(&mut scene, &[1, 3, 4]);
    let t0 = Instant::now();
    let mut motion = Motion::new(
        &tokens(Curve::Spring, CardMotion::Scale),
        Anchor::BottomRight,
    );
    motion.retarget(&before, &after, t0);
    paint_layers(&mut scene, &mut motion);
    let (w, h) = (after.width, after.height.max(motion.hold()));
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
    let run = |scene: &mut Scene, motion: &mut Motion, canvas: &mut Canvas<'_>, ms: u64| {
        let now = t0 + Duration::from_millis(ms);
        motion.settle(now);
        // A whole tick: the layers (none stale), the static cards and the
        // composites.
        motion.paint_layers(None, |_, _, _| unreachable!("nothing is stale"));
        paint_moving(
            canvas,
            &mut scene.text,
            &after,
            &scene.config,
            &scene.look,
            &scene.urgencies,
            None,
            &scene.icons,
            scene.shadow.as_ref(),
            motion,
            now,
        );
    };
    // Warm the glyph cache.
    run(&mut scene, &mut motion, &mut canvas, 0);
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for ms in (16..190).step_by(16) {
            run(&mut scene, &mut motion, &mut canvas, ms);
        }
    });
    assert_eq!(allocations, 0);
}
