//! Times the card stack's repaint under each shadow style, and the one-off
//! cost of building a shadow's tables: the evidence behind "`shadow =
//! "none"` costs what it did, and a cached shadow is cheap per frame".
//!
//! ```sh
//! cargo run --release -p scootnotify --example bench_depth -- --font /path/to/font.ttf
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
use scootnotify::look::{FillStyle, Look, ShadowStyle};
use scootnotify::model::expiry::Clock;
use scootnotify::model::store::{Icon, Notification};
use scootnotify::view::layout;
use scootnotify::view::render::paint;
use scootui::paint::{Canvas, Pad, Shadow};
use scootui::text::Text;

fn note(id: u32, summary: &str, body: &str) -> Notification {
    Notification {
        id,
        app_name: "bench".to_owned(),
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
    let mut iters = 1000usize;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--font" => font_path = args.next(),
            "--iters" => iters = args.next().and_then(|v| v.parse().ok()).unwrap_or(iters),
            other => panic!("unknown argument {other}"),
        }
    }
    let font_bytes = match font_path {
        Some(path) => std::fs::read(path).expect("the font reads"),
        None => scootui::testfont::build(),
    };
    let font = FontArc::new(FontVec::try_from_vec(font_bytes).expect("a usable font"));
    let config = Config::default();
    let notes = notes();
    let live: Vec<&Notification> = notes.iter().collect();
    let urgencies: HashMap<u32, Urgency> = notes.iter().map(|n| (n.id, n.urgency)).collect();
    let icons = HashMap::new();

    println!(
        "style fill surface surface-px median-us(best of 9 rounds) p5-us(best of 9 rounds) layout+tables-us"
    );
    // Three interleaved rounds per style, to keep thermal and neighbor
    // noise from favoring whichever style ran first.
    let styles = [
        (ShadowStyle::None, FillStyle::Solid),
        (ShadowStyle::Soft, FillStyle::Solid),
        (ShadowStyle::Deep, FillStyle::Solid),
        (ShadowStyle::Glow, FillStyle::Solid),
        (ShadowStyle::None, FillStyle::Gradient),
        (ShadowStyle::Soft, FillStyle::Gradient),
    ];
    let mut best: Vec<Option<(Duration, Duration)>> = vec![None; styles.len()];
    let mut report: Vec<(u32, u32, Duration)> = vec![(0, 0, Duration::ZERO); styles.len()];
    for _round in 0..9 {
        for (slot, (shadow_style, fill)) in styles.iter().enumerate() {
            let look = Look {
                shadow: *shadow_style,
                fill: *fill,
                ..Look::default()
            };
            let build_start = Instant::now();
            let mut shadow = look.shadow.spec().map(Shadow::new);
            let pad = shadow.as_ref().map_or(Pad::ZERO, Shadow::pad);
            let text = Text::new(font.clone());
            let stack = layout::inset(
                layout::layout(&live, &config, &text, look.font_size_px, 2_000),
                pad,
            );
            if let Some(shadow) = shadow.as_mut() {
                shadow.prepare(
                    config.radius,
                    stack.cards.iter().map(|c| (c.rect.width, c.rect.height)),
                );
            }
            // Layout plus tables: what a content change costs (layout is
            // the same for every style; the difference is the tables).
            let build = build_start.elapsed();
            let mut text = Text::new(font.clone());
            let mut pixels = vec![0u8; stack.width as usize * stack.height as usize * 4];
            let mut run = |pixels: &mut Vec<u8>| {
                let mut canvas = Canvas::new(pixels, stack.width, stack.height).expect("canvas");
                paint(
                    &mut canvas,
                    &mut text,
                    &stack,
                    &config,
                    &look,
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
            best[slot] = Some(best[slot].map_or((median, p5), |(m, p)| (m.min(median), p.min(p5))));
            report[slot] = (stack.width, stack.height, build);
        }
    }
    for (slot, (shadow_style, fill)) in styles.iter().enumerate() {
        let (w, h, build) = report[slot];
        println!(
            "{:?} {:?} {}x{} {} {:.1} {:.1} {:.1}",
            shadow_style,
            fill,
            w,
            h,
            u64::from(w) * u64::from(h),
            best[slot].expect("measured").0.as_secs_f64() * 1e6,
            best[slot].expect("measured").1.as_secs_f64() * 1e6,
            build.as_secs_f64() * 1e6,
        );
    }
}
