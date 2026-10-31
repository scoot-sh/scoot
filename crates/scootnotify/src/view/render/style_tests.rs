//! The card styles' pixels: a golden per mark, for the pill and for the
//! title weights and spacing; the mark sitting inside the card's own shape
//! and moving with it; and the zero-allocation pin with every style on.
//! The default look's own bytes are pinned in `tests`, `depth_tests` and
//! `motion_tests`, none of which changed with these tokens.

use std::collections::HashMap;

use scootui::color::Color;
use scootui::paint::{Canvas, Pad, Shadow};

use super::tests::{make_text, note};
use super::*;
use crate::look::style::{PILL_INSET, PILL_RADIUS, STRIPE_PX};
use crate::look::{CardMark, CardShape, ShadowStyle};
use crate::model::store::Notification;
use crate::view::layout;

/// Four apps, three urgencies, an action row and a progress bar, so a mark
/// shows on cards of different heights and colors.
fn apps() -> Vec<Notification> {
    let mut notes: Vec<Notification> = ["Firefox", "Slack", "Calendar", "Mail"]
        .iter()
        .zip(1..)
        .map(|(app, id)| {
            let mut card = note(id, &format!("Card {id}"), "A body that wraps a few times");
            card.app_name = (*app).to_owned();
            card
        })
        .collect();
    notes[1].urgency = Urgency::Critical;
    notes[2].urgency = Urgency::Low;
    notes[3].value = Some(40);
    notes[3].actions = vec![crate::bus::notify::Action {
        key: "open".to_owned(),
        label: "Open".to_owned(),
    }];
    notes
}

fn styled(look: &Look, config: &Config, notes: &[Notification]) -> (layout::Stack, Option<Shadow>) {
    let live: Vec<&Notification> = notes.iter().collect();
    let mut shadow = look.shadow.spec().map(Shadow::new);
    let pad = shadow.as_ref().map_or(Pad::ZERO, Shadow::pad);
    let stack = layout::inset(
        layout::layout_styled(&live, config, &make_text(), look.font_size_px, 2_000, look),
        pad,
    );
    if let Some(shadow) = shadow.as_mut() {
        shadow.prepare(
            look.card_radius(config.radius),
            stack
                .cards
                .iter()
                .map(|card| (card.rect.width, card.rect.height)),
        );
    }
    (stack, shadow)
}

fn paint_styled(
    look: &Look,
    config: &Config,
    notes: &[Notification],
    stack: &layout::Stack,
    shadow: Option<&Shadow>,
) -> Vec<u8> {
    let urgencies = notes.iter().map(|n| (n.id, n.urgency)).collect();
    let mut pixels = vec![0u8; stack.width as usize * stack.height as usize * 4];
    let mut canvas = Canvas::new(&mut pixels, stack.width, stack.height).unwrap();
    paint(
        &mut canvas,
        &mut make_text(),
        stack,
        config,
        look,
        &urgencies,
        None,
        &HashMap::new(),
        shadow,
    );
    pixels
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

fn look_with(edit: impl FnOnce(&mut Look)) -> Look {
    let mut look = Look::default();
    edit(&mut look);
    look
}

/// The styles under test, by name.
fn styles() -> Vec<(&'static str, Look)> {
    vec![
        ("stripe", look_with(|l| l.mark = CardMark::Stripe)),
        ("dot", look_with(|l| l.mark = CardMark::Dot)),
        ("badge", look_with(|l| l.mark = CardMark::Badge)),
        (
            "stripe-app-accent",
            look_with(|l| {
                l.mark = CardMark::Stripe;
                l.app_accent = true;
            }),
        ),
        (
            "dot-app-accent",
            look_with(|l| {
                l.mark = CardMark::Dot;
                l.app_accent = true;
            }),
        ),
        (
            "badge-app-accent",
            look_with(|l| {
                l.mark = CardMark::Badge;
                l.app_accent = true;
            }),
        ),
        ("pill", look_with(|l| l.shape = CardShape::Pill)),
        (
            "pill-stripe-soft",
            look_with(|l| {
                l.shape = CardShape::Pill;
                l.mark = CardMark::Stripe;
                l.shadow = ShadowStyle::Soft;
            }),
        ),
        (
            "pill-badge-accent-glow",
            look_with(|l| {
                l.shape = CardShape::Pill;
                l.mark = CardMark::Badge;
                l.app_accent = true;
                l.shadow = ShadowStyle::Glow;
            }),
        ),
        ("title-regular", look_with(|l| l.title_weight = 400)),
        (
            "title-heavy-spaced",
            look_with(|l| {
                l.title_weight = 900;
                l.title_spacing = 2;
            }),
        ),
        ("title-tight", look_with(|l| l.title_spacing = -1)),
    ]
}

/// One golden per style over the four-app stack. A change to any is a
/// change to what a look that opts in paints: re-record only on purpose,
/// and look at the frames first (`SCOOT_DUMP_FRAMES=dir`).
#[test]
fn each_style_paints_its_golden() {
    let config = Config::default();
    let notes = apps();
    let mut seen = Vec::new();
    for (name, look) in styles() {
        let (stack, tables) = styled(&look, &config, &notes);
        let pixels = paint_styled(&look, &config, &notes, &stack, tables.as_ref());
        dump(name, &pixels, stack.width, stack.height);
        seen.push((name, fingerprint(&pixels)));
    }
    assert_eq!(seen, GOLDENS);
}

const GOLDENS: [(&str, u64); 12] = [
    ("stripe", 0xbb89_1289_8441_2a24),
    ("dot", 0xeac6_05e2_7fda_b4d8),
    ("badge", 0x1df3_cefe_ba38_0b9c),
    ("stripe-app-accent", 0x7dca_180b_2fdd_d584),
    ("dot-app-accent", 0x5e8d_cc6d_067c_a768),
    ("badge-app-accent", 0xb148_649a_b219_7ace),
    ("pill", 0x68de_bbe5_b85b_6ec0),
    ("pill-stripe-soft", 0x3428_0477_a5c3_d638),
    ("pill-badge-accent-glow", 0xad80_648b_238f_db72),
    ("title-regular", 0x3002_2ae5_b365_81d4),
    ("title-heavy-spaced", 0x661c_ae31_7328_34f6),
    ("title-tight", 0x267d_d806_c490_5518),
];

#[test]
fn every_style_differs_from_the_default_and_from_each_other() {
    let config = Config::default();
    let notes = apps();
    let (stack, _) = styled(&Look::default(), &config, &notes);
    let base = fingerprint(&paint_styled(
        &Look::default(),
        &config,
        &notes,
        &stack,
        None,
    ));
    let mut prints = vec![base];
    for (name, look) in styles() {
        let (stack, tables) = styled(&look, &config, &notes);
        let print = fingerprint(&paint_styled(
            &look,
            &config,
            &notes,
            &stack,
            tables.as_ref(),
        ));
        assert!(!prints.contains(&print), "{name} repeats another style");
        prints.push(print);
    }
}

#[test]
fn the_default_look_lays_out_and_paints_what_it_always_did() {
    let config = Config::default();
    let notes = apps();
    let live: Vec<&Notification> = notes.iter().collect();
    let look = Look::default();
    let plain = layout::layout(&live, &config, &make_text(), look.font_size_px, 2_000);
    let (styled_stack, _) = styled(&look, &config, &notes);
    assert_eq!(plain, styled_stack);
    assert!(
        plain
            .cards
            .iter()
            .all(|c| c.accent.is_none() && c.dot.is_none())
    );
}

/// The stripe is `STRIPE_PX` columns inside the ring on the leading edge,
/// opaque in the card's accent, and clipped by the card's own shape: the
/// pill's cut corner stays clear.
#[test]
fn the_stripe_sits_inside_the_ring_and_inside_the_shape() {
    let config = Config::default();
    let notes = apps();
    for shape in [CardShape::Rounded, CardShape::Pill] {
        let look = look_with(|l| {
            l.mark = CardMark::Stripe;
            l.shape = shape;
            // Apart from the ring (`focus` defaults to the same accent).
            l.theme.accent = Color {
                r: 200,
                g: 30,
                b: 200,
            };
        });
        let (stack, _) = styled(&look, &config, &notes);
        let pixels = paint_styled(&look, &config, &notes, &stack, None);
        let accent = look.theme.accent;
        let want = [accent.b, accent.g, accent.r, 255];
        for card in &stack.cards {
            let (x, y) = (card.rect.x, card.rect.y);
            let middle = y + card.rect.height / 2;
            let border = config.border_px;
            for column in 0..STRIPE_PX {
                // The pill's arc touches the stripe's first column only at
                // the very middle, where its edge pixel is a hair short of
                // full coverage.
                let got = pixel(&pixels, stack.width, x + border + column, middle);
                assert!(
                    got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2),
                    "{shape:?} card {} column {column}: {got:?}",
                    card.id
                );
            }
            // Just past the stripe is the card fill, not the accent.
            assert_ne!(
                pixel(&pixels, stack.width, x + border + STRIPE_PX + 1, middle),
                want
            );
            // The ring is still the ring outside the stripe.
            assert_ne!(pixel(&pixels, stack.width, x, middle), want);
            // The cut corner is not painted by the stripe or the fill.
            assert_eq!(pixel(&pixels, stack.width, x, y), [0, 0, 0, 0]);
        }
    }
}

/// A card painted into its motion layer carries its mark: the layer's
/// pixels inside the card are the static paint's, shifted to the layer's
/// corner. Marks, the pill and the badge tile are all in the card's own
/// coordinates.
#[test]
fn a_mark_is_painted_with_its_card_into_a_motion_layer() {
    let config = Config::default();
    let notes = apps();
    for (name, look) in styles() {
        let (stack, tables) = styled(&look, &config, &notes);
        let whole = paint_styled(&look, &config, &notes, &stack, tables.as_ref());
        let card = &stack.cards[0];
        // A layer the card's size plus its shadow room.
        let (dx, dy) = (
            card.rect.x.min(stack.pad.left),
            card.rect.y.min(stack.pad.top),
        );
        let (ox, oy) = (card.rect.x - dx, card.rect.y - dy);
        let moved = card.translated(ox, oy);
        let (lw, lh) = (
            moved.rect.x + moved.rect.width + stack.pad.right,
            moved.rect.y + moved.rect.height + stack.pad.bottom,
        );
        let mut layer = vec![0u8; lw as usize * lh as usize * 4];
        let mut canvas = Canvas::new(&mut layer, lw, lh).unwrap();
        let urgency = notes[0].urgency;
        paint_layer(
            &mut canvas,
            &mut make_text(),
            &moved,
            &config,
            &look,
            urgency,
            None,
            &HashMap::new(),
            None,
        );
        // Compare the card's rows away from the neighbors' shadows (none
        // painted here): the first card is the newest, painted last, so
        // nothing of another card lies over it.
        for row in 0..card.rect.height {
            for column in 0..card.rect.width {
                let (sx, sy) = (card.rect.x + column, card.rect.y + row);
                let (lx, ly) = (moved.rect.x + column, moved.rect.y + row);
                let (inside, outside) = (
                    pixel(&layer, lw, lx, ly),
                    pixel(&whole, stack.width, sx, sy),
                );
                // Edge pixels carry the neighbors' shadow in the whole
                // stack and none here: only pixels the card covers outright
                // are the same.
                if inside[3] == 255 || outside[3] == 255 {
                    assert_eq!(inside, outside, "{name}: column {column} row {row}");
                }
            }
        }
    }
}

#[test]
fn the_dot_makes_room_on_the_meta_line_and_only_there() {
    let config = Config::default();
    let notes = apps();
    let live: Vec<&Notification> = notes.iter().collect();
    let plain = layout::layout(&live, &config, &make_text(), 13.0, 2_000);
    let dotted_look = look_with(|l| l.mark = CardMark::Dot);
    let dotted = layout::layout_styled(&live, &config, &make_text(), 13.0, 2_000, &dotted_look);
    for (a, b) in plain.cards.iter().zip(&dotted.cards) {
        assert_eq!(a.rect, b.rect, "the card does not grow");
        assert_eq!(a.summary, b.summary);
        assert_eq!(a.body, b.body);
        assert_eq!(b.meta.x - a.meta.x, 14, "the meta text steps past the dot");
        let dot = b.dot.expect("a dot");
        assert_eq!((dot.width, dot.height), (8, 8));
        assert!(
            i64::from(dot.x + dot.width) < b.meta.x,
            "the dot ends before the text"
        );
        let baseline = b.meta.baseline;
        assert!(
            i64::from(dot.y) < baseline && i64::from(dot.y + dot.height) <= baseline,
            "the dot sits above the baseline"
        );
    }
}

#[test]
fn a_badge_gives_every_card_the_icon_slot() {
    let config = Config::default();
    let notes = apps();
    let live: Vec<&Notification> = notes.iter().collect();
    let plain = layout::layout(&live, &config, &make_text(), 13.0, 2_000);
    assert!(plain.cards.iter().all(|card| !card.has_icon));
    let look = look_with(|l| l.mark = CardMark::Badge);
    let badged = layout::layout_styled(&live, &config, &make_text(), 13.0, 2_000, &look);
    assert!(badged.cards.iter().all(|card| card.has_icon));
    // The slot is the 48 px tile and the text steps past it.
    for (a, b) in plain.cards.iter().zip(&badged.cards) {
        assert!(b.rect.height >= a.rect.height);
        assert_eq!(b.summary[0].x - a.summary[0].x, 58);
    }
}

#[test]
fn the_badge_tile_is_the_accent_with_readable_ink() {
    let config = Config::default();
    let notes = apps();
    let look = look_with(|l| {
        l.mark = CardMark::Badge;
        l.app_accent = true;
    });
    let (stack, _) = styled(&look, &config, &notes);
    let pixels = paint_styled(&look, &config, &notes, &stack, None);
    for card in &stack.cards {
        let accent = card.accent.expect("an app accent");
        assert!(
            look.theme.foreground.contrast_ratio(accent) >= 4.5,
            "{accent} under the ink"
        );
        // A tile corner, inside the 8 px rounding: the plate's color.
        let tile = [
            card.rect.x + config.border_px + layout::PAD + 24,
            card.rect.y + config.border_px + layout::PAD + 2,
        ];
        assert_eq!(
            pixel(&pixels, stack.width, tile[0], tile[1]),
            [accent.b, accent.g, accent.r, 255],
            "card {}",
            card.id
        );
    }
}

#[test]
fn a_pill_card_is_cut_to_half_its_height_up_to_the_cap() {
    let look = look_with(|l| l.shape = CardShape::Pill);
    assert_eq!(look.card_radius(Some(8)), PILL_RADIUS);
    assert_eq!(Look::default().card_radius(Some(8)), 8);
    let config = Config::default();
    let notes = apps();
    let (stack, _) = styled(&look, &config, &notes);
    let pixels = paint_styled(&look, &config, &notes, &stack, None);
    for card in &stack.cards {
        // The content keeps clear of the curved ends.
        assert_eq!(card.pad_x, config.border_px + layout::PAD + PILL_INSET);
        let r = (card.rect.height / 2).min(PILL_RADIUS);
        // The top-left corner square is cut: its corner pixel is clear and
        // the pixel at the circle's center is filled.
        assert_eq!(
            pixel(&pixels, stack.width, card.rect.x, card.rect.y),
            [0; 4]
        );
        assert_eq!(
            pixel(&pixels, stack.width, card.rect.x + r, card.rect.y + r)[3],
            255
        );
    }
}

#[test]
fn the_title_weight_changes_the_summary_and_not_the_body() {
    let config = Config::default();
    let notes = apps();
    let (stack, _) = styled(&Look::default(), &config, &notes);
    let bold = paint_styled(&Look::default(), &config, &notes, &stack, None);
    let regular_look = look_with(|l| l.title_weight = 400);
    let regular = paint_styled(&regular_look, &config, &notes, &stack, None);
    // Same layout, different ink, only on the summary's rows.
    let first = &stack.cards[0];
    let summary_rows = first.summary[0].baseline - 14..=first.summary[0].baseline + 4;
    let mut differing_rows = std::collections::BTreeSet::new();
    for (index, (a, b)) in bold
        .as_chunks::<4>()
        .0
        .iter()
        .zip(regular.as_chunks::<4>().0)
        .enumerate()
    {
        if a != b {
            differing_rows.insert((index as u32 / stack.width) as i64);
        }
    }
    assert!(!differing_rows.is_empty());
    let summaries: Vec<_> = stack
        .cards
        .iter()
        .flat_map(|c| c.summary.iter().map(|l| l.baseline))
        .collect();
    for row in &differing_rows {
        assert!(
            summaries.iter().any(|b| (b - 14..=b + 4).contains(row)),
            "row {row} differs outside every summary (first card's: {summary_rows:?})"
        );
    }
}

#[test]
fn letter_spacing_widens_the_summary_wrap_and_never_the_body() {
    let config = Config::default();
    let mut long = note(1, &"M".repeat(60), &"m".repeat(60));
    long.app_name = "a".to_owned();
    let live = [&long];
    let tight = layout::layout(&live, &config, &make_text(), 13.0, 2_000);
    let look = look_with(|l| l.title_spacing = 4);
    let spaced = layout::layout_styled(&live, &config, &make_text(), 13.0, 2_000, &look);
    let width = |lines: &[layout::Line]| {
        lines
            .iter()
            .map(|l| l.runs.iter().map(|r| r.text.chars().count()).sum::<usize>())
            .collect::<Vec<_>>()
    };
    let (a, b) = (&tight.cards[0], &spaced.cards[0]);
    assert!(
        width(&b.summary)[0] < width(&a.summary)[0],
        "fewer summary characters fit per line when each steps 4 px more"
    );
    assert_eq!(a.body, b.body, "the body is not spaced");
}

/// A repaint allocates nothing after warm-up with every card style on.
#[cfg(target_os = "linux")]
#[test]
fn a_repaint_with_every_style_allocates_nothing_after_warm_up() {
    let config = Config::default();
    let notes = apps();
    for (name, look) in styles() {
        let (stack, tables) = styled(&look, &config, &notes);
        let urgencies: HashMap<u32, Urgency> = notes.iter().map(|n| (n.id, n.urgency)).collect();
        let mut text = make_text();
        let mut pixels = vec![0u8; stack.width as usize * stack.height as usize * 4];
        let mut run = |pixels: &mut Vec<u8>| {
            let mut canvas = Canvas::new(pixels, stack.width, stack.height).unwrap();
            paint(
                &mut canvas,
                &mut text,
                &stack,
                &config,
                &look,
                &urgencies,
                None,
                &HashMap::new(),
                tables.as_ref(),
            );
        };
        for _ in 0..3 {
            run(&mut pixels);
        }
        let (_, allocs) = scootbg_mem::count_allocations(|| run(&mut pixels));
        assert_eq!(allocs, 0, "{name}: a repaint allocates");
    }
}

/// A progress bar runs the card's full content width, so under an icon slot
/// taller than the text beside it (a volume OSD: summary, value, no body)
/// it starts below the slot, not through it.
#[test]
fn a_progress_bar_starts_below_a_taller_icon_slot() {
    let config = Config::default();
    let mut osd = note(1, "Volume", "");
    osd.icon = crate::model::store::Icon::Name("audio-volume-high".to_owned());
    osd.value = Some(30);
    let stack = layout::layout(&[&osd], &config, &make_text(), 13.0, 2_000);
    let card = &stack.cards[0];
    let bar = card.progress_bar.expect("a bar");
    let slot_bottom = card.rect.y + config.border_px + layout::PAD + layout::ICON_PX;
    assert!(bar.y >= slot_bottom, "{} < {slot_bottom}", bar.y);
    assert!(bar.y + bar.height + layout::PAD + config.border_px <= card.rect.y + card.rect.height);
    // A badge gives a card without an icon the same slot, and the same rule.
    let mut plain = note(2, "Volume", "");
    plain.value = Some(30);
    let look = look_with(|l| l.mark = CardMark::Badge);
    let stack = layout::layout_styled(&[&plain], &config, &make_text(), 13.0, 2_000, &look);
    let card = &stack.cards[0];
    assert!(
        card.progress_bar.unwrap().y
            >= card.rect.y + config.border_px + layout::PAD + layout::ICON_PX
    );
}

/// A look that moves and marks: the mark is in the card's layer, so a card
/// in flight carries it (and it stays inside the pill and the shadow's
/// room), and the frame where everything has landed is the static paint.
#[test]
fn a_mark_travels_with_a_card_in_flight_and_lands_on_the_static_paint() {
    use std::time::{Duration, Instant};

    use scootui::motion::Curve;

    use crate::config::Anchor;
    use crate::look::{CardMotion, MotionTokens};
    use crate::view::motion::Motion;

    let config = Config::default();
    let notes = vec![note(1, "Card", "A body that wraps a few times")];
    let tokens = MotionTokens {
        curve: Curve::EaseOut,
        duration_ms: 200,
        bounce: 1.0,
        enter: CardMotion::Slide,
        exit: CardMotion::Fade,
    };
    let frame = |look: &Look, ms: u64| -> (Vec<u8>, layout::Stack) {
        let mut look = look.clone();
        look.motion = tokens;
        let (stack, tables) = styled(&look, &config, &notes);
        let empty = layout::Stack::default_empty();
        let t0 = Instant::now();
        let mut motion = Motion::new(&look.motion, Anchor::TopRight);
        motion.retarget(&empty, &stack, t0);
        let now = t0 + Duration::from_millis(ms);
        motion.settle(now);
        let (w, h) = (stack.width, stack.height.max(motion.hold()).max(1));
        let urgencies: HashMap<u32, Urgency> = notes.iter().map(|n| (n.id, n.urgency)).collect();
        let mut text = make_text();
        let icons = HashMap::new();
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
        if motion.active() {
            motion.paint_layers(None, |card, layer, button| {
                let urgency = urgencies.get(&card.id).copied().unwrap_or(Urgency::Normal);
                let mut canvas = layer.canvas().expect("layer");
                paint_layer(
                    &mut canvas,
                    &mut text,
                    card,
                    &config,
                    &look,
                    urgency,
                    button,
                    &icons,
                    tables.as_ref(),
                );
            });
            paint_moving(
                &mut canvas,
                &mut text,
                &stack,
                &config,
                &look,
                &urgencies,
                None,
                &icons,
                tables.as_ref(),
                &motion,
                now,
            );
        } else {
            paint(
                &mut canvas,
                &mut text,
                &stack,
                &config,
                &look,
                &urgencies,
                None,
                &icons,
                tables.as_ref(),
            );
        }
        (pixels, stack)
    };
    let plain = look_with(|l| {
        l.shape = CardShape::Pill;
        l.shadow = ShadowStyle::Soft;
    });
    let marked = look_with(|l| {
        l.shape = CardShape::Pill;
        l.shadow = ShadowStyle::Soft;
        l.mark = CardMark::Stripe;
    });
    // In flight: the two frames differ, only in a thin band down the
    // card's leading edge, and nowhere near the shadow's room.
    let (a, stack) = frame(&plain, 100);
    let (b, _) = frame(&marked, 100);
    assert_eq!(a.len(), b.len());
    let width = stack.width as usize;
    let (mut left, mut right, mut any) = (usize::MAX, 0usize, false);
    for (index, (x, y)) in a
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .enumerate()
    {
        if x != y {
            any = true;
            left = left.min(index % width);
            right = right.max(index % width);
        }
    }
    assert!(any, "the stripe is drawn while the card moves");
    assert!(
        right - left <= STRIPE_PX as usize + 2,
        "the difference is the stripe's band: columns {left}..={right}"
    );
    // Landed: the static paint, byte for byte.
    let (landed, _) = frame(&marked, 5_000);
    let (laid_out, tables) = styled(&marked, &config, &notes);
    let reference = paint_styled(&marked, &config, &notes, &laid_out, tables.as_ref());
    assert!(landed == reference, "a landed card is the static paint");
}
