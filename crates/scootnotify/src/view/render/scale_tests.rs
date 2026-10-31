//! The repaint at the compositor's scale: the layout stays in logical
//! pixels, the buffer is exactly the logical size times the scale, and a
//! card is the same card at every scale, only sharper. Pixels use the
//! seven-segment test font.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use scootui::density::Scale;
use scootui::motion::Curve;
use scootui::paint::{Canvas, Pad, Shadow};

use super::tests::{busy_notes, make_text, note};
use super::*;
use crate::config::Anchor;
use crate::look::{CardMotion, MotionTokens, ShadowStyle};
use crate::model::store::Notification;
use crate::view::layout;
use crate::view::motion::Motion;

/// The scales a compositor sends: the 1x it always drew at, the fractional
/// ones laptops ship with, the integers, and one nothing rounds evenly at.
fn scales() -> [Scale; 7] {
    [
        Scale::Integer(1),
        Scale::Fractional(150),
        Scale::Fractional(180),
        Scale::Fractional(210),
        Scale::Integer(2),
        Scale::Integer(3),
        Scale::Fractional(133),
    ]
}

struct Scene {
    text: Text,
    config: Config,
    look: Look,
    /// The look's shadow at scale 1, for the layout's room.
    logical: Option<Shadow>,
    /// The same shadow at the scale drawn at, with its corner tables.
    shadow: Option<Shadow>,
    urgencies: HashMap<u32, Urgency>,
    scale: Scale,
}

impl Scene {
    fn new(style: ShadowStyle, scale: Scale) -> Self {
        let look = Look {
            shadow: style,
            ..Look::default()
        };
        let dev = Dev::new(scale);
        Self {
            text: make_text(),
            config: Config::default(),
            logical: look.shadow.spec().map(Shadow::new),
            shadow: look.shadow.spec().map(|spec| Shadow::new(dev.shadow(spec))),
            look,
            urgencies: (1..=8).map(|id| (id, Urgency::Normal)).collect(),
            scale,
        }
    }

    fn dev(&self) -> Dev {
        Dev::new(self.scale)
    }

    fn stack(&mut self, notes: &[Notification]) -> layout::Stack {
        let live: Vec<&Notification> = notes.iter().collect();
        let pad = self.logical.as_ref().map_or(Pad::ZERO, Shadow::pad);
        let stack = layout::inset(
            layout::layout(
                &live,
                &self.config,
                &self.text,
                self.look.font_size_px,
                2_000,
            ),
            pad,
        );
        let dev = self.dev();
        if let Some(shadow) = self.shadow.as_mut().filter(|_| !stack.cards.is_empty()) {
            shadow.prepare(
                dev.len(self.look.card_radius(self.config.radius)),
                stack
                    .cards
                    .iter()
                    .map(|card| (dev.len(card.rect.width), dev.len(card.rect.height))),
            );
        }
        stack
    }

    /// The buffer the compositor is sent for `stack`.
    fn buffer(&self, stack: &layout::Stack) -> (u32, u32) {
        self.scale
            .buffer(scootui::output::Size {
                width: stack.width,
                height: stack.height,
            })
            .expect("a small stack fits")
    }

    fn paint(&mut self, stack: &layout::Stack) -> (Vec<u8>, u32, u32) {
        let (w, h) = self.buffer(stack);
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        {
            let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
            let dev = self.dev();
            paint(
                &mut canvas,
                &mut self.text,
                stack,
                &self.config,
                &self.look,
                &self.urgencies,
                None,
                &HashMap::new(),
                self.shadow.as_ref(),
                dev,
            );
        }
        (pixels, w, h)
    }
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = (y as usize * width as usize + x as usize) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

fn bgr(color: scootui::color::Color) -> [u8; 3] {
    [color.b, color.g, color.r]
}

fn rgb_of(pixel: [u8; 4]) -> [u8; 3] {
    [pixel[0], pixel[1], pixel[2]]
}

/// A card's ring, fill and corners sit where the layout says at every
/// scale: its outline is the logical rectangle mapped, one border wide.
#[test]
fn a_card_sits_where_the_layout_puts_it_at_every_scale() {
    let notes = [note(1, "Hello", "World, wrapped a few times over a card")];
    for scale in scales() {
        let mut scene = Scene::new(ShadowStyle::None, scale);
        let stack = scene.stack(&notes);
        let card = &stack.cards[0];
        let (pixels, w, h) = scene.paint(&stack);
        let dev = scene.dev();
        assert_eq!((w, h), scene.buffer(&stack), "{scale}");
        let rect = dev.rect(card.rect);
        let border = dev.thick(scene.config.border_px);
        let middle = rect.x + rect.width / 2;
        let focus = bgr(scene.look.focus);
        // The ring's first and last rows, down the middle.
        assert_eq!(rgb_of(pixel(&pixels, w, middle, rect.y)), focus, "{scale}");
        assert_eq!(
            rgb_of(pixel(&pixels, w, middle, rect.y + border - 1)),
            focus,
            "{scale}"
        );
        assert_eq!(
            rgb_of(pixel(&pixels, w, middle, rect.y + rect.height - 1)),
            focus,
            "{scale}"
        );
        // Inside the ring is the fill, and one row above the card is clear.
        assert_eq!(
            rgb_of(pixel(&pixels, w, middle, rect.y + border + 1)),
            bgr(scene.look.surface),
            "{scale}"
        );
        if rect.y > 0 {
            assert_eq!(pixel(&pixels, w, middle, rect.y - 1)[3], 0, "{scale}");
        }
        // The border is as wide as the config's, scaled: not one pixel
        // thicker.
        assert_ne!(
            rgb_of(pixel(&pixels, w, middle, rect.y + border)),
            focus,
            "{scale}"
        );
        // The card never leaves the buffer it was sized by.
        assert!(
            rect.x + rect.width <= w + 1 && rect.y + rect.height <= h + 1,
            "{scale}"
        );
    }
}

/// Integer scales are the 1x card with every pixel an n x n block of its
/// own geometry: the card is exactly n times as big on each side.
#[test]
fn an_integer_scale_is_the_layout_times_n() {
    let notes = [note(1, "Hello", "World")];
    let mut one = Scene::new(ShadowStyle::None, Scale::Integer(1));
    let stack = one.stack(&notes);
    for factor in [2u32, 3] {
        let mut scene = Scene::new(ShadowStyle::None, Scale::Integer(factor));
        let (_, w, h) = scene.paint(&stack);
        assert_eq!((w, h), (stack.width * factor, stack.height * factor));
        let card = stack.cards[0].rect;
        let rect = scene.dev().rect(card);
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (
                card.x * factor,
                card.y * factor,
                card.width * factor,
                card.height * factor
            )
        );
    }
}

/// Text is drawn at the device size: more ink, in the card, at the pen the
/// layout set, scaled.
#[test]
fn text_is_drawn_at_the_device_size() {
    let notes = [note(1, "Hello", "World")];
    let ink = |scale: Scale| {
        let mut scene = Scene::new(ShadowStyle::None, scale);
        let stack = scene.stack(&notes);
        let (pixels, w, h) = scene.paint(&stack);
        let foreground = bgr(scene.look.theme.foreground);
        let mut count = 0usize;
        let (mut x0, mut x1) = (w, 0);
        for y in 0..h {
            for x in 0..w {
                if rgb_of(pixel(&pixels, w, x, y)) == foreground
                    && pixel(&pixels, w, x, y)[3] == 255
                {
                    count += 1;
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                }
            }
        }
        (count, x0, x1, scene.dev(), stack)
    };
    let (one, a0, a1, _, _) = ink(Scale::Integer(1));
    let (two, b0, b1, dev, stack) = ink(Scale::Integer(2));
    assert!(one > 20, "the 1x summary has ink");
    assert!(
        two > one * 3 && two < one * 5,
        "twice the size is about four times the ink: {one} then {two}"
    );
    // The ink starts where the layout's pen is, scaled, and stays inside
    // the card.
    let pen = dev.x(stack.cards[0].summary[0].x);
    assert!(i64::from(b0) >= pen - 1, "{b0} against the pen at {pen}");
    assert!(b1 < dev.rect(stack.cards[0].rect).x + dev.rect(stack.cards[0].rect).width);
    assert!(i64::from(b1 - b0) >= i64::from(a1 - a0) * 2 - 6);
}

/// A card painted into a layer and composited at rest is the card painted
/// in place, to the byte, at every scale and under every shadow.
#[test]
fn a_layer_at_rest_is_the_static_card() {
    let notes = [note(
        1,
        "Layer",
        "A body long enough to wrap onto a second line",
    )];
    for style in [ShadowStyle::None, ShadowStyle::Soft, ShadowStyle::Deep] {
        for scale in scales() {
            let mut scene = Scene::new(style, scale);
            let stack = scene.stack(&notes);
            let (expected, w, h) = scene.paint(&stack);
            let dev = scene.dev();

            let mut motion = Motion::new(&MotionTokens::default(), Anchor::TopRight);
            motion.set_scale(scale);
            let mut layer = scootui::paint::Layer::new();
            let card = &stack.cards[0];
            let origin = (
                card.rect.x.saturating_sub(stack.pad.left),
                card.rect.y.saturating_sub(stack.pad.top),
            );
            let local = card.translated(origin.0, origin.1);
            let size = (
                card.rect.width + stack.pad.left + stack.pad.right,
                card.rect.height + stack.pad.top + stack.pad.bottom,
            );
            let (lw, lh) = if dev.is_unit() {
                size
            } else {
                (dev.len(size.0) + 1, dev.len(size.1) + 1)
            };
            assert!(layer.begin(lw, lh));
            {
                let mut canvas = layer.canvas().unwrap();
                paint_layer(
                    &mut canvas,
                    &mut scene.text,
                    &local,
                    &scene.config,
                    &scene.look,
                    Urgency::Normal,
                    None,
                    &HashMap::new(),
                    scene.shadow.as_ref(),
                    dev.from_origin(origin.0, origin.1),
                );
            }
            let mut pixels = vec![0u8; w as usize * h as usize * 4];
            let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
            layer.composite(
                &mut canvas,
                dev.x(i64::from(origin.0)) as i32,
                dev.y(i64::from(origin.1)) as i32,
                1.0,
                255,
            );
            assert!(
                pixels == expected,
                "{style:?} at {scale}: the layer differs from the static card"
            );
            drop(motion);
        }
    }
}

/// The shadow's reach in device pixels stays inside the room the layout
/// left for it, rounded: the surface is never clipped short of it.
#[test]
fn a_shadow_fits_the_room_the_layout_left() {
    for style in [ShadowStyle::Soft, ShadowStyle::Deep, ShadowStyle::Glow] {
        let spec = Look {
            shadow: style,
            ..Look::default()
        }
        .shadow
        .spec()
        .expect("a shadow");
        let room = Shadow::new(spec).pad();
        for scale in scales() {
            let dev = Dev::new(scale);
            let scaled = Shadow::new(dev.shadow(spec)).pad();
            for (got, logical, side) in [
                (scaled.top, room.top, "top"),
                (scaled.right, room.right, "right"),
                (scaled.bottom, room.bottom, "bottom"),
                (scaled.left, room.left, "left"),
            ] {
                assert!(
                    got <= dev.len(logical) + 1,
                    "{style:?} {side} at {scale}: reach {got}, room {} (+1 kept)",
                    dev.len(logical)
                );
                // And the shadow is the one asked for, not one cut short by
                // the blur cap.
                assert!(
                    got + 2 >= dev.len(logical),
                    "{style:?} {side} at {scale}: reach {got} of room {}",
                    dev.len(logical)
                );
            }
        }
    }
}

/// A stack in motion paints the static cards through the same path: with
/// nothing in flight it is the static bytes, at every scale.
#[test]
fn the_moving_path_with_nothing_in_flight_is_the_static_paint() {
    let notes = busy_notes();
    for scale in scales() {
        let mut scene = Scene::new(ShadowStyle::Soft, scale);
        let stack = scene.stack(&notes);
        let (expected, w, h) = scene.paint(&stack);
        let mut motion = Motion::new(&MotionTokens::default(), Anchor::TopRight);
        motion.set_scale(scale);
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
            &HashMap::new(),
            scene.shadow.as_ref(),
            &motion,
            Instant::now(),
        );
        assert!(pixels == expected, "{scale}");
    }
}

fn at(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

fn motion_tokens() -> MotionTokens {
    MotionTokens {
        curve: Curve::Spring,
        duration_ms: 200,
        bounce: 1.0,
        enter: CardMotion::Scale,
        exit: CardMotion::Fade,
    }
}

/// A whole animated frame (settle, stale-layer check, static cards,
/// composites) and a whole static repaint allocate nothing once warm, at a
/// fractional and an integer scale, and after the scale changed under them.
#[test]
fn a_warm_frame_allocates_nothing_after_a_scale_change() {
    let before = busy_notes();
    let after: Vec<Notification> = before.iter().skip(1).cloned().collect();
    for scale in [Scale::Fractional(180), Scale::Integer(2)] {
        let mut scene = Scene::new(ShadowStyle::Soft, Scale::Integer(1));
        let old = scene.stack(&before);
        let t0 = Instant::now();
        let mut motion = Motion::new(&motion_tokens(), Anchor::BottomRight);
        // Start at 1x, then change scale under the running motion, as a
        // monitor swap would.
        let new = {
            scene.scale = scale;
            scene.shadow = scene
                .look
                .shadow
                .spec()
                .map(|spec| Shadow::new(scene.dev().shadow(spec)));
            scene.stack(&after)
        };
        motion.retarget(&old, &new, t0);
        motion.set_scale(scale);
        let dev = scene.dev();
        let paint_layers = |scene: &mut Scene, motion: &mut Motion| {
            let Scene {
                text,
                config,
                look,
                shadow,
                urgencies,
                ..
            } = scene;
            motion.paint_layers(None, |card, layer, button, dev| {
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
                    &HashMap::new(),
                    shadow.as_ref(),
                    dev,
                );
            });
        };
        paint_layers(&mut scene, &mut motion);
        let surface = new.height.max(motion.hold());
        let (w, h) = scale
            .buffer(scootui::output::Size {
                width: new.width,
                height: surface,
            })
            .unwrap();
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        let icons = HashMap::new();
        let mut frame = |scene: &mut Scene, motion: &mut Motion, ms: u64| {
            let now = at(t0, ms);
            motion.settle(now);
            paint_layers(scene, motion);
            let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
            paint_moving(
                &mut canvas,
                &mut scene.text,
                &new,
                &scene.config,
                &scene.look,
                &scene.urgencies,
                None,
                &icons,
                scene.shadow.as_ref(),
                motion,
                now,
            );
        };
        // Warm: glyphs rasterized, layers painted.
        frame(&mut scene, &mut motion, 10);
        let ((), allocations) = scootbg_mem::count_allocations(|| {
            for ms in [20, 40, 60, 80, 100] {
                frame(&mut scene, &mut motion, ms);
            }
        });
        assert_eq!(allocations, 0, "an animated frame at {scale}");
        // The static repaint too.
        let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
        let run = |scene: &mut Scene, canvas: &mut Canvas<'_>| {
            paint(
                canvas,
                &mut scene.text,
                &new,
                &scene.config,
                &scene.look,
                &scene.urgencies,
                None,
                &icons,
                scene.shadow.as_ref(),
                dev,
            );
        };
        run(&mut scene, &mut canvas);
        let ((), allocations) = scootbg_mem::count_allocations(|| {
            for _ in 0..3 {
                run(&mut scene, &mut canvas);
            }
        });
        assert_eq!(allocations, 0, "a repaint at {scale}");
    }
}

/// A layer too large at the new scale is dropped instead of kept: the card
/// snaps to its place, and the motion goes on with what still fits.
#[test]
fn a_card_too_large_to_move_at_the_new_scale_snaps() {
    // A card whose layer fits at 1x and not at 8x (the pool limit is
    // 2 Mpx): the widest card the config allows.
    let mut scene = Scene::new(ShadowStyle::None, Scale::Integer(1));
    scene.config.width_px = 1024;
    let notes = [note(1, "Wide", "A body")];
    let old = scene.stack(&[]);
    let new = scene.stack(&notes);
    let mut motion = Motion::new(&motion_tokens(), Anchor::TopRight);
    motion.retarget(&old, &new, Instant::now());
    assert!(motion.active(), "it moves at 1x");
    motion.set_scale(Scale::Integer(8));
    assert!(
        !motion.active(),
        "an 8x layer of that card is over the limit"
    );
    motion.set_scale(Scale::Integer(1));
}

/// Icons decode at the slot's device size, never under 96 and never over
/// the cap.
#[test]
fn the_icon_side_follows_the_scale() {
    assert_eq!(icon_side(Scale::Integer(1)), ICON_SIDE);
    assert_eq!(icon_side(Scale::Integer(2)), ICON_SIDE);
    assert_eq!(icon_side(Scale::Fractional(150)), ICON_SIDE);
    assert_eq!(icon_side(Scale::Integer(3)), 144);
    assert_eq!(icon_side(Scale::Fractional(300)), 120);
    assert_eq!(icon_side(Scale::Integer(16)), MAX_ICON_SIDE);
}

/// A card fading in is drawn on the pixels the settled card will have, at
/// every scale and under any held height, for either anchor: the fully
/// covered pixels of the layer are exactly the static card's. A bottom
/// anchor holds the surface taller than the stack while cards move; what
/// is inside the stack must not shift with that height (it would twitch
/// every card when the hold drops).
#[test]
fn a_moving_card_is_drawn_where_the_static_card_lands() {
    let notes = [note(
        1,
        "Landing",
        "A body that wraps onto a second line, so odd",
    )];
    let tokens = MotionTokens {
        curve: Curve::EaseOut,
        duration_ms: 400,
        bounce: 1.0,
        enter: CardMotion::Fade,
        exit: CardMotion::Fade,
    };
    for anchor in [Anchor::TopRight, Anchor::BottomRight] {
        for scale in scales() {
            for held in [0u32, 7, 13, 41] {
                let mut scene = Scene::new(ShadowStyle::None, scale);
                let old = scene.stack(&[]);
                let stack = scene.stack(&notes);
                let t0 = Instant::now();
                let mut motion = Motion::new(&tokens, anchor);
                motion.set_scale(scale);
                motion.retarget(&old, &stack, t0);
                let surface = stack.height + held;
                let (w, h) = scale
                    .buffer(scootui::output::Size {
                        width: stack.width,
                        height: surface,
                    })
                    .unwrap();
                let draw = |scene: &mut Scene, motion: &mut Motion, now: Instant| {
                    let mut pixels = vec![0u8; w as usize * h as usize * 4];
                    let Scene {
                        text,
                        config,
                        look,
                        shadow,
                        urgencies,
                        ..
                    } = scene;
                    motion.paint_layers(None, |card, layer, button, dev| {
                        let mut canvas = layer.canvas().expect("layer");
                        paint_layer(
                            &mut canvas,
                            text,
                            card,
                            config,
                            look,
                            Urgency::Normal,
                            button,
                            &HashMap::new(),
                            shadow.as_ref(),
                            dev,
                        );
                    });
                    let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
                    paint_moving(
                        &mut canvas,
                        text,
                        &stack,
                        config,
                        look,
                        urgencies,
                        None,
                        &HashMap::new(),
                        shadow.as_ref(),
                        motion,
                        now,
                    );
                    pixels
                };
                let moving = draw(&mut scene, &mut motion, at(t0, 120));
                assert!(
                    motion.active(),
                    "{anchor:?} {scale} held {held}: still fading"
                );
                let landed = {
                    let mut idle = Motion::new(&tokens, anchor);
                    idle.set_scale(scale);
                    draw(&mut scene, &mut idle, at(t0, 120))
                };
                let alpha = |pixels: &[u8]| {
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| p[3])
                        .collect::<Vec<u8>>()
                };
                let (moving, landed) = (alpha(&moving), alpha(&landed));
                let peak = *moving.iter().max().unwrap();
                assert!(
                    peak > 0 && peak < 255,
                    "{anchor:?} {scale}: a fade, peak {peak}"
                );
                let covered_moving: Vec<usize> =
                    (0..moving.len()).filter(|&i| moving[i] == peak).collect();
                let covered_landed: Vec<usize> =
                    (0..landed.len()).filter(|&i| landed[i] == 255).collect();
                assert!(
                    covered_moving == covered_landed,
                    "{anchor:?} {scale} held {held}: {} covered pixels moving, {} landed",
                    covered_moving.len(),
                    covered_landed.len()
                );
            }
        }
    }
}
