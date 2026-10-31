//! Times the card stack's repaint and layout under each card style (the
//! accent marker, the pill, the per-app accent, the title weight and
//! spacing): the evidence behind "a look that sets none costs what it did,
//! and a style costs a few microseconds a repaint".
//!
//! ```sh
//! cargo run --release -p scootnotify --example bench_styles -- --font /path/to/font.ttf [--bold-font /path/to/bold.ttf]
//! ```
//!
//! Dev tooling only: never part of the shipped binary. Release builds only:
//! a debug build says nothing about a repaint.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ab_glyph::{FontArc, FontVec};
use scootnotify::bus::hints::Urgency;
use scootnotify::bus::notify::Action;
use scootnotify::config::Config;
use scootnotify::look::{CardMark, CardShape, Look, ShadowStyle};
use scootnotify::model::expiry::Clock;
use scootnotify::model::store::{Icon, Notification};
use scootnotify::view::layout;
use scootnotify::view::render::paint;
use scootui::paint::{Canvas, Pad, Shadow};
use scootui::text::Text;

fn note(id: u32, summary: &str, body: &str) -> Notification {
    Notification {
        id,
        app_name: ["Firefox", "Slack", "Calendar", "Mail", "Files"][id as usize % 5].to_owned(),
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

/// Five visible cards, the way a busy desktop shows them: mixed heights, a
/// progress bar, buttons, three urgencies.
fn notes() -> Vec<Notification> {
    let mut notes: Vec<Notification> = (1..=5)
        .map(|id| {
            note(
                id,
                &format!("Message {id}"),
                "A body long enough to wrap onto a second line in a 360 px card",
            )
        })
        .collect();
    notes[1].urgency = Urgency::Low;
    notes[4].urgency = Urgency::Critical;
    notes[4].value = Some(60);
    notes[4].actions = vec![
        Action {
            key: "open".to_owned(),
            label: "Open".to_owned(),
        },
        Action {
            key: "later".to_owned(),
            label: "Later".to_owned(),
        },
    ];
    notes
}

/// The median and the 5th percentile of `samples`: the median is the
/// typical frame, the 5th percentile what the code costs when the machine
/// is not interrupting it (a shared VM's noise comes in bursts, which move
/// the median and leave the low tail alone).
fn median_and_p5(samples: &mut [Duration]) -> (Duration, Duration) {
    samples.sort_unstable();
    (samples[samples.len() / 2], samples[samples.len() / 20])
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut font_path = None;
    let mut bold_path = None;
    let mut iters = 1000usize;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--font" => font_path = args.next(),
            "--bold-font" => bold_path = args.next(),
            "--iters" => iters = args.next().and_then(|v| v.parse().ok()).unwrap_or(iters),
            other => panic!("unknown argument {other}"),
        }
    }
    let font_bytes = match font_path {
        Some(path) => std::fs::read(path).expect("the font reads"),
        None => scootui::testfont::build(),
    };
    let font = FontArc::new(FontVec::try_from_vec(font_bytes).expect("a usable font"));
    let bold = bold_path.map(|path| {
        let bytes = std::fs::read(path).expect("the bold font reads");
        FontArc::new(FontVec::try_from_vec(bytes).expect("a usable bold font"))
    });
    // A text with the bold face when one was given, as the daemon builds it.
    let text_of = |font: &FontArc| match &bold {
        Some(bold) => Text::new(font.clone()).with_bold(bold.clone()),
        None => Text::new(font.clone()),
    };
    let config = Config::default();
    let notes = notes();
    let live: Vec<&Notification> = notes.iter().collect();
    let urgencies: HashMap<u32, Urgency> = notes.iter().map(|n| (n.id, n.urgency)).collect();
    let icons = HashMap::new();

    let styles: Vec<(&str, Look)> = vec![
        ("default", Look::default()),
        (
            "stripe",
            Look {
                mark: CardMark::Stripe,
                ..Look::default()
            },
        ),
        (
            "dot",
            Look {
                mark: CardMark::Dot,
                ..Look::default()
            },
        ),
        (
            "badge",
            Look {
                mark: CardMark::Badge,
                ..Look::default()
            },
        ),
        (
            "badge+app-accent",
            Look {
                mark: CardMark::Badge,
                app_accent: true,
                ..Look::default()
            },
        ),
        (
            "pill",
            Look {
                shape: CardShape::Pill,
                ..Look::default()
            },
        ),
        (
            "pill+soft+badge+accent",
            Look {
                shape: CardShape::Pill,
                shadow: ShadowStyle::Soft,
                mark: CardMark::Badge,
                app_accent: true,
                ..Look::default()
            },
        ),
        (
            "title-regular",
            Look {
                title_weight: 400,
                ..Look::default()
            },
        ),
        (
            "title-heavy+spacing",
            Look {
                title_weight: 900,
                title_spacing: 2,
                ..Look::default()
            },
        ),
    ];
    println!("style surface paint-median-us paint-p5-us layout-median-us (best of 9 rounds)");
    let mut best: Vec<Option<(Duration, Duration, Duration)>> = vec![None; styles.len()];
    let mut size = vec![(0u32, 0u32); styles.len()];
    for _round in 0..9 {
        for (slot, (_, look)) in styles.iter().enumerate() {
            let text = text_of(&font);
            let mut shadow = look.shadow.spec().map(Shadow::new);
            let pad = shadow.as_ref().map_or(Pad::ZERO, Shadow::pad);
            // Layout: what a content change costs (the accent's hash and
            // contrast search run here, once per card).
            let mut layout_samples = Vec::with_capacity(200);
            let mut stack = None;
            for _ in 0..200 {
                let start = Instant::now();
                let laid =
                    layout::layout_styled(&live, &config, &text, look.font_size_px, 2_000, look);
                layout_samples.push(start.elapsed());
                stack = Some(laid);
            }
            layout_samples.sort_unstable();
            let layout_median = layout_samples[layout_samples.len() / 2];
            let stack = layout::inset(stack.expect("laid out"), pad);
            if let Some(shadow) = shadow.as_mut() {
                shadow.prepare(
                    look.card_radius(config.radius),
                    stack.cards.iter().map(|c| (c.rect.width, c.rect.height)),
                );
            }
            let mut text = text_of(&font);
            let mut pixels = vec![0u8; stack.width as usize * stack.height as usize * 4];
            let mut run = |pixels: &mut Vec<u8>| {
                let mut canvas = Canvas::new(pixels, stack.width, stack.height).expect("canvas");
                paint(
                    &mut canvas,
                    &mut text,
                    &stack,
                    &config,
                    look,
                    &urgencies,
                    None,
                    &icons,
                    shadow.as_ref(),
                );
            };
            for _ in 0..20 {
                run(&mut pixels);
            }
            let mut samples = Vec::with_capacity(iters);
            for _ in 0..iters {
                let start = Instant::now();
                run(&mut pixels);
                samples.push(start.elapsed());
            }
            let (median, p5) = median_and_p5(&mut samples);
            best[slot] = Some(match best[slot] {
                Some((m, p, l)) => (m.min(median), p.min(p5), l.min(layout_median)),
                None => (median, p5, layout_median),
            });
            size[slot] = (stack.width, stack.height);
        }
    }
    for (slot, (name, _)) in styles.iter().enumerate() {
        let (median, p5, layout) = best[slot].expect("measured");
        println!(
            "{name} {}x{} {:.1} {:.1} {:.1}",
            size[slot].0,
            size[slot].1,
            median.as_secs_f64() * 1e6,
            p5.as_secs_f64() * 1e6,
            layout.as_secs_f64() * 1e6,
        );
    }
}
