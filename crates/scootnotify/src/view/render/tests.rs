//! The repaint's tests: fills, borders, text ink, and the allocation
//! pin. Pixels use the seven-segment test font (stable across font
//! package updates); the integration tests cover the same states with
//! screenshots.

use std::collections::HashMap;

use ab_glyph::{FontArc, FontVec};
use scootui::paint::Canvas;
use scootui::text::Text;

use super::*;
use crate::bus::hints::Urgency;
use crate::config::Config;
use crate::look::Look;
use crate::model::expiry::Clock;
use crate::model::store::{Icon, Notification};
use crate::view::layout;

pub(super) fn make_text() -> Text {
    Text::new(FontArc::new(
        FontVec::try_from_vec(scootui::testfont::build()).unwrap(),
    ))
}

pub(super) fn note(id: u32, summary: &str, body: &str) -> Notification {
    Notification {
        id,
        app_name: "test".to_owned(),
        sender: ":1.1".to_owned(),
        icon: Icon::None,
        summary: summary.to_owned(),
        body: body.to_owned(),
        plain_body: body.to_owned(),
        actions: Vec::new(),
        urgency: Urgency::Normal,
        category: String::new(),
        desktop_entry: String::new(),
        transient: false,
        resident: false,
        keep_history: true,
        output: None,
        value: None,
        seq: u64::from(id),
        timeout_ms: Some(10_000),
        created_ms: 1_000,
        shown_ms: None,
        clock: Clock::waiting(),
    }
}

fn painted(stack: &layout::Stack, look: &Look) -> (Vec<u8>, u32, u32) {
    let mut paint_text = make_text();
    let config = Config::default();
    let urgencies = HashMap::from([(1, Urgency::Normal)]);
    // Device pixels at scale 1: the buffer is the logical size.
    let mut pixels = vec![0u8; stack.width as usize * stack.height as usize * 4];
    let dims = (stack.width, stack.height);
    {
        let mut canvas = Canvas::new(&mut pixels, dims.0, dims.1).unwrap();
        paint(
            &mut canvas,
            &mut paint_text,
            stack,
            &config,
            look,
            &urgencies,
            None,
            &HashMap::new(),
            None,
        );
    }
    (pixels, dims.0, dims.1)
}

fn at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let at = (y as usize * width as usize + x as usize) * 4;
    [pixels[at + 2], pixels[at + 1], pixels[at]]
}

#[test]
fn a_card_has_its_ring_fill_and_ink() {
    let font_text = make_text();
    let config = Config::default();
    let look = Look::default();
    let single = note(1, "Hello", "World");
    let stack = layout::layout(&[&single], &config, &font_text, look.font_size_px, 2_000);
    assert_eq!(stack.cards.len(), 1);
    let (pixels, width, height) = painted(&stack, &look);
    assert!(height > 0);
    // The top edge's middle is the normal-urgency ring (`focus`).
    let focus = look.focus;
    assert_eq!(
        at(&pixels, width, width / 2, 1),
        [focus.r, focus.g, focus.b]
    );
    // The content pad is the surface fill.
    let surface = look.surface;
    assert_eq!(at(&pixels, width, 5, 30), [surface.r, surface.g, surface.b]);
    // The summary row carries foreground ink.
    let foreground = look.theme.foreground;
    let card = &stack.cards[0];
    let row = card.summary[0].baseline as u32;
    let ink = (0..width).any(|x| {
        at(&pixels, width, x, row.saturating_sub(4)) == [foreground.r, foreground.g, foreground.b]
    });
    assert!(ink, "the summary draws foreground ink");
}

#[test]
fn urgency_colors_the_ring() {
    let font_text = make_text();
    let config = Config::default();
    let look = Look::default();
    for (urgency, ring) in [
        (Urgency::Low, look.border),
        (Urgency::Normal, look.focus),
        (Urgency::Critical, look.theme.urgent),
    ] {
        let mut single = note(1, "Hello", "World");
        single.urgency = urgency;
        let stack = layout::layout(&[&single], &config, &font_text, look.font_size_px, 2_000);
        let mut urgencies = HashMap::new();
        urgencies.insert(1, urgency);
        let mut paint_text = make_text();
        let mut pixels = vec![0u8; stack.width as usize * stack.height as usize * 4];
        {
            let mut canvas = Canvas::new(&mut pixels, stack.width, stack.height).unwrap();
            paint(
                &mut canvas,
                &mut paint_text,
                &stack,
                &config,
                &look,
                &urgencies,
                None,
                &HashMap::new(),
                None,
            );
        }
        assert_eq!(
            at(&pixels, stack.width, stack.width / 2, 1),
            [ring.r, ring.g, ring.b],
            "{urgency:?}"
        );
    }
}

/// The repaint allocates nothing after warm-up: layout once, paint
/// until the glyph cache holds every glyph, then count. Mutation-prove
/// by injecting a `format!` into the repaint path and watching this go
/// red (see the report). Linux-only, like the counting allocator behind
/// it (the crate never runs on a Mac).
#[cfg(target_os = "linux")]
#[test]
fn a_repaint_allocates_nothing_after_warm_up() {
    let font_text = make_text();
    let config = Config::default();
    let look = Look::default();
    let mut actions_note = note(1, "Hello", "World *bold* and more words to wrap around");
    actions_note.actions = vec![
        crate::bus::notify::Action {
            key: "open".to_owned(),
            label: "Open".to_owned(),
        },
        crate::bus::notify::Action {
            key: "dismiss".to_owned(),
            label: "Dismiss".to_owned(),
        },
    ];
    actions_note.value = Some(42);
    let stack = layout::layout(
        &[&actions_note],
        &config,
        &font_text,
        look.font_size_px,
        2_000,
    );
    let urgencies = HashMap::from([(1, Urgency::Normal)]);
    let mut paint_text = make_text();
    let mut pixels = vec![0u8; stack.width as usize * stack.height as usize * 4];
    // Warm up: layout-adjacent measures plus every glyph the paint
    // draws, including the pill and the second card state below.
    for _ in 0..3 {
        let mut canvas = Canvas::new(&mut pixels, stack.width, stack.height).unwrap();
        paint(
            &mut canvas,
            &mut paint_text,
            &stack,
            &config,
            &look,
            &urgencies,
            None,
            &HashMap::new(),
            None,
        );
    }
    let (_, allocs) = scootbg_mem::count_allocations(|| {
        let mut canvas = Canvas::new(&mut pixels, stack.width, stack.height).unwrap();
        paint(
            &mut canvas,
            &mut paint_text,
            &stack,
            &config,
            &look,
            &urgencies,
            None,
            &HashMap::new(),
            None,
        );
    });
    assert_eq!(allocs, 0, "a repaint allocates");
}

/// FNV-1a over `bytes`: a stable fingerprint of a painted buffer.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Seven notifications: three urgencies, a body, a progress bar and two
/// buttons on the newest, and more than `max_visible` for the "+N more"
/// pill.
pub(super) fn busy_notes() -> Vec<Notification> {
    let mut notes: Vec<Notification> = (1..=7)
        .map(|id| {
            note(
                id,
                &format!("Card {id}"),
                "A body that wraps around a few times",
            )
        })
        .collect();
    notes[5].urgency = Urgency::Low;
    notes[6].urgency = Urgency::Critical;
    notes[6].value = Some(60);
    notes[6].actions = vec![
        crate::bus::notify::Action {
            key: "open".to_owned(),
            label: "Open".to_owned(),
        },
        crate::bus::notify::Action {
            key: "later".to_owned(),
            label: "Later".to_owned(),
        },
    ];
    notes
}

/// A busy stack painted under the default look, the newest card's first
/// button hovered.
fn default_look_stack() -> (Vec<u8>, u32, u32) {
    let config = Config::default();
    let look = Look::default();
    let notes = busy_notes();
    let live: Vec<&Notification> = notes.iter().collect();
    let font_text = make_text();
    let stack = layout::layout(&live, &config, &font_text, look.font_size_px, 2_000);
    assert!(stack.hidden > 0 && stack.cards.len() > 3);
    let urgencies = notes.iter().map(|n| (n.id, n.urgency)).collect();
    let mut pixels = vec![0u8; stack.width as usize * stack.height as usize * 4];
    {
        let mut canvas = Canvas::new(&mut pixels, stack.width, stack.height).unwrap();
        paint(
            &mut canvas,
            &mut make_text(),
            &stack,
            &config,
            &look,
            &urgencies,
            Some((7, Some(0))),
            &HashMap::new(),
            None,
        );
    }
    (pixels, stack.width, stack.height)
}

#[test]
fn the_default_look_paints_the_bytes_it_always_did() {
    let (pixels, width, height) = default_look_stack();
    // Recorded from the repaint before the depth tokens existed: a look
    // that opts into nothing must keep painting exactly this.
    assert_eq!(
        fingerprint(&pixels),
        0xd689_fb62_65bd_ac76,
        "{width}x{height}"
    );
}
