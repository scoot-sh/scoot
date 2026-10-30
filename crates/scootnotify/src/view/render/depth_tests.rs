//! The depth tokens' pixels: a golden per shadow style and per fill, what
//! a shadow does and does not touch, and the zero-allocation pin with a
//! shadow on. The default look's own bytes are pinned in `tests`.

use std::collections::HashMap;

use scootui::color::Color;
use scootui::paint::{Canvas, Pad, Shadow};

use super::tests::{busy_notes, make_text};
use super::*;
use crate::look::{FillStyle, ShadowStyle};
use crate::model::store::Notification;
use crate::view::layout;

/// A styled stack: the layout padded for the look's shadow, the shadow's
/// tables sized to its cards, as the popup does on a content change.
fn styled(look: &Look, config: &Config, notes: &[Notification]) -> (layout::Stack, Option<Shadow>) {
    let live: Vec<&Notification> = notes.iter().collect();
    let mut shadow = look.shadow.spec().map(Shadow::new);
    let pad = shadow.as_ref().map_or(Pad::ZERO, Shadow::pad);
    let stack = layout::inset(
        layout::layout(&live, config, &make_text(), look.font_size_px, 2_000),
        pad,
    );
    if let Some(shadow) = shadow.as_mut() {
        shadow.prepare(
            config.radius,
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

fn look_with(shadow: ShadowStyle, fill: FillStyle) -> Look {
    Look {
        shadow,
        fill,
        ..Look::default()
    }
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

/// One golden per style over the busy stack (seven notifications, three
/// urgencies, a progress bar, buttons, the pill). A change to any of
/// these is a change to what a look that opts in paints: re-record only on
/// purpose, and look at the screenshots first.
#[test]
fn each_style_paints_its_golden() {
    let config = Config::default();
    let notes = busy_notes();
    for (shadow, fill, golden, size) in [
        (
            ShadowStyle::Soft,
            FillStyle::Solid,
            0x42f1_b47c_5e3b_713e,
            (384, 418),
        ),
        (
            ShadowStyle::Deep,
            FillStyle::Solid,
            0x8005_0a3e_8b2b_29a9,
            (416, 450),
        ),
        (
            ShadowStyle::Glow,
            FillStyle::Solid,
            0xde67_f051_7807_38ff,
            (392, 426),
        ),
        (
            ShadowStyle::None,
            FillStyle::Gradient,
            0xac90_d9e5_ca77_cecf,
            (360, 394),
        ),
        (
            ShadowStyle::Soft,
            FillStyle::Gradient,
            0x5946_f9de_4b55_b3a9,
            (384, 418),
        ),
    ] {
        let look = look_with(shadow, fill);
        let (stack, tables) = styled(&look, &config, &notes);
        let pixels = paint_styled(&look, &config, &notes, &stack, tables.as_ref());
        assert_eq!((stack.width, stack.height), size, "{shadow:?} {fill:?}");
        assert_eq!(fingerprint(&pixels), golden, "{shadow:?} {fill:?}");
    }
}

/// Only the shadow differs between a shadowed stack and the same padded
/// stack without one: every pixel a card covers outright is identical, so
/// neither a card's own shadow nor its neighbor's lands on a card.
#[test]
fn a_shadow_never_lands_on_a_card() {
    let config = Config::default();
    let notes = busy_notes();
    for opacity in [1.0, 0.5] {
        let mut look = look_with(ShadowStyle::Deep, FillStyle::Solid);
        look.opacity = opacity;
        let (stack, tables) = styled(&look, &config, &notes);
        let with = paint_styled(&look, &config, &notes, &stack, tables.as_ref());
        let without = paint_styled(&look, &config, &notes, &stack, None);
        assert_ne!(with, without, "the shadow painted something");
        let radius = config.radius;
        for card in &stack.cards {
            let r = card.rect;
            for y in r.y..r.y + r.height {
                for x in r.x + radius..r.x + r.width - radius {
                    assert_eq!(
                        pixel(&with, stack.width, x, y),
                        pixel(&without, stack.width, x, y),
                        "opacity {opacity}: card {} at ({x}, {y})",
                        card.id
                    );
                }
            }
        }
    }
}

/// Depth is below the cards and around them: the gaps and the room around
/// the stack hold the shadow, the input region stays the cards, and the
/// shadow falls down.
#[test]
fn the_shadow_fills_the_room_around_the_cards_and_clicks_through() {
    let config = Config::default();
    let notes = busy_notes();
    let look = look_with(ShadowStyle::Soft, FillStyle::Solid);
    let (stack, tables) = styled(&look, &config, &notes);
    let pixels = paint_styled(&look, &config, &notes, &stack, tables.as_ref());
    assert_eq!(
        stack.input,
        stack.cards.iter().map(|card| card.rect).collect::<Vec<_>>()
    );
    let first = stack.cards[0].rect;
    let middle = first.x + first.width / 2;
    // The room above the first card holds a faint shadow (the soft style
    // falls 4 px): premultiplied black, partial alpha.
    let above = pixel(&pixels, stack.width, middle, first.y - 2);
    let below = pixel(&pixels, stack.width, middle, first.y + first.height + 2);
    assert_eq!(
        &above[..3],
        &[0, 0, 0],
        "a black shadow is premultiplied zeros"
    );
    assert!(
        above[3] > 0 && below[3] > above[3],
        "above {above:?} below {below:?}"
    );
    // The far corner of the room is beyond the reach: transparent.
    assert_eq!(pixel(&pixels, stack.width, 0, 0), [0; 4]);
    // The gap between two cards is shadow, never solid card.
    let gap_y = first.y + first.height + 4;
    assert!(pixel(&pixels, stack.width, middle, gap_y)[3] > 0);
}

#[test]
fn a_glow_takes_each_cards_ring_color() {
    let config = Config::default();
    let notes = busy_notes();
    let look = look_with(ShadowStyle::Glow, FillStyle::Solid);
    let (stack, tables) = styled(&look, &config, &notes);
    let pixels = paint_styled(&look, &config, &notes, &stack, tables.as_ref());
    // Just left of each card's vertical middle: the glow in the card's
    // ring color (critical: urgent, low: border, normal: focus), premultiplied.
    let ring = |urgency| match urgency {
        Urgency::Critical => look.theme.urgent,
        Urgency::Low => look.border,
        Urgency::Normal => look.focus,
    };
    for card in &stack.cards {
        let urgency = notes[card.id as usize - 1].urgency;
        let tint: Color = ring(urgency);
        let r = card.rect;
        let at = pixel(&pixels, stack.width, r.x - 2, r.y + r.height / 2);
        let alpha = u32::from(at[3]);
        assert!(alpha > 0, "card {} glows", card.id);
        // Premultiplied: channel = tint × alpha / 255, to a rounding step.
        for (got, want) in [(at[2], tint.r), (at[1], tint.g), (at[0], tint.b)] {
            let expected = u32::from(want) * alpha / 255;
            assert!(
                u32::from(got).abs_diff(expected) <= 1,
                "card {} {urgency:?}: {at:?} vs tint {tint:?}",
                card.id
            );
        }
    }
}

#[test]
fn the_pill_sits_under_the_cards_not_in_the_shadow_room() {
    let config = Config {
        max_visible: 2,
        ..Config::default()
    };
    let notes = busy_notes();
    let look = look_with(ShadowStyle::Deep, FillStyle::Solid);
    let (stack, tables) = styled(&look, &config, &notes);
    assert!(stack.hidden > 0);
    let pixels = paint_styled(&look, &config, &notes, &stack, tables.as_ref());
    let dim = look.theme.dim;
    let is_dim = |x: u32, y: u32| {
        let at = pixel(&pixels, stack.width, x, y);
        at == [dim.b, dim.g, dim.r, 255]
    };
    let cards_w = stack.width - stack.pad.left - stack.pad.right;
    let middle = stack.pad.left + cards_w / 2;
    let last_card = stack.cards.last().unwrap().rect;
    let below_cards = last_card.y + last_card.height;
    let pill_rows: Vec<u32> = (below_cards..stack.height)
        .filter(|&y| is_dim(middle - 3, y))
        .collect();
    assert!(!pill_rows.is_empty(), "the pill is painted below the cards");
    let bottom = *pill_rows.last().unwrap();
    assert!(
        bottom < stack.height - stack.pad.bottom,
        "the pill ends above the bottom room: {bottom} of {}",
        stack.height
    );
}

/// `shadow = "none"` is no table and no pad: the layout is the bare
/// layout, byte for byte, and the paint is the default look's.
#[test]
fn shadow_none_changes_nothing() {
    let config = Config::default();
    let notes = busy_notes();
    let look = look_with(ShadowStyle::None, FillStyle::Solid);
    let (stack, tables) = styled(&look, &config, &notes);
    assert!(tables.is_none() && stack.pad.is_zero());
    let live: Vec<&Notification> = notes.iter().collect();
    assert_eq!(
        stack,
        layout::layout(&live, &config, &make_text(), look.font_size_px, 2_000)
    );
}

/// A repaint allocates nothing after warm-up with each depth token on:
/// the tables were built when the stack changed, the gradient is
/// per-row arithmetic. Linux-only, like the counting allocator.
#[cfg(target_os = "linux")]
#[test]
fn a_repaint_with_depth_allocates_nothing_after_warm_up() {
    let config = Config::default();
    let notes = busy_notes();
    for (shadow, fill) in [
        (ShadowStyle::Soft, FillStyle::Solid),
        (ShadowStyle::Deep, FillStyle::Gradient),
        (ShadowStyle::Glow, FillStyle::Gradient),
    ] {
        let look = look_with(shadow, fill);
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
        assert_eq!(allocs, 0, "{shadow:?} {fill:?}: a repaint allocates");
    }
}

/// The five styles as PNGs, composited over a mid-gray desktop, for a
/// human to look at before re-recording the goldens:
/// `SCOOT_DEPTH_DUMP=/some/dir cargo test -p scootnotify dump_the_styles -- --ignored`.
#[test]
#[ignore = "writes files for a human; run with SCOOT_DEPTH_DUMP set"]
fn dump_the_styles() {
    let Some(dir) = std::env::var_os("SCOOT_DEPTH_DUMP") else {
        return;
    };
    let config = Config::default();
    let notes = busy_notes();
    for (name, shadow, fill) in [
        ("none", ShadowStyle::None, FillStyle::Solid),
        ("soft", ShadowStyle::Soft, FillStyle::Solid),
        ("deep", ShadowStyle::Deep, FillStyle::Solid),
        ("glow", ShadowStyle::Glow, FillStyle::Solid),
        ("gradient", ShadowStyle::None, FillStyle::Gradient),
        ("soft-gradient", ShadowStyle::Soft, FillStyle::Gradient),
    ] {
        let look = look_with(shadow, fill);
        let (stack, tables) = styled(&look, &config, &notes);
        let pixels = paint_styled(&look, &config, &notes, &stack, tables.as_ref());
        // Premultiplied BGRA over a flat desktop color, to RGB.
        let desktop = [0x4a_u32, 0x5a, 0x6e];
        let mut rgb = Vec::with_capacity(pixels.len() / 4 * 3);
        for px in pixels.as_chunks::<4>().0 {
            let behind = 255 - u32::from(px[3]);
            for (channel, base) in [
                (px[2], desktop[0]),
                (px[1], desktop[1]),
                (px[0], desktop[2]),
            ] {
                rgb.push((u32::from(channel) + base * behind / 255).min(255) as u8);
            }
        }
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        let file = std::fs::File::create(&path).expect("create png");
        let mut encoder = png::Encoder::new(file, stack.width, stack.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("png header")
            .write_image_data(&rgb)
            .expect("png data");
    }
}
