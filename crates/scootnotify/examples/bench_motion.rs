//! Times the card stack while cards move: one animated frame (settle,
//! stale-layer check, static cards, composites) against the plain repaint
//! it replaces, and the one-off cost of starting a motion (the layers'
//! rasterization). The evidence behind "an animating stack stays in
//! budget on pixman".
//!
//! ```sh
//! cargo run --release -p scootnotify --example bench_motion -- --font /path/to/font.ttf
//! ```
//!
//! Dev tooling only: never part of the shipped binary. Release builds only:
//! a debug build says nothing about a frame.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ab_glyph::{FontArc, FontVec};
use scootnotify::bus::hints::Urgency;
use scootnotify::bus::notify::Action;
use scootnotify::config::{Anchor, Config};
use scootnotify::look::{CardMotion, Look, MotionTokens, ShadowStyle};
use scootnotify::model::expiry::Clock;
use scootnotify::model::store::{Icon, Notification};
use scootnotify::view::layout::{self, Stack};
use scootnotify::view::motion::Motion;
use scootnotify::view::render::{paint, paint_layer, paint_moving};
use scootui::motion::Curve;
use scootui::paint::{Canvas, Pad, Shadow};
use scootui::text::Text;

fn note(id: u32) -> Notification {
    Notification {
        id,
        app_name: "bench".to_owned(),
        sender: ":1.1".to_owned(),
        icon: Icon::None,
        summary: format!("Message {id}"),
        body: "A body long enough to wrap onto a second line in a 360 px card".to_owned(),
        plain_body: String::new(),
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

/// Cards `ids`, oldest first: mixed urgencies, a progress bar and buttons
/// on the critical one, as a busy desktop shows them.
fn notes(ids: &[u32]) -> Vec<Notification> {
    ids.iter()
        .map(|&id| {
            let mut n = note(id);
            if id % 5 == 2 {
                n.urgency = Urgency::Low;
            }
            if id % 5 == 0 {
                n.urgency = Urgency::Critical;
                n.value = Some(60);
                n.actions = vec![
                    Action {
                        key: "open".to_owned(),
                        label: "Open".to_owned(),
                    },
                    Action {
                        key: "later".to_owned(),
                        label: "Later".to_owned(),
                    },
                ];
            }
            n
        })
        .collect()
}

fn median_and_p5(samples: &mut [Duration]) -> (f64, f64) {
    samples.sort_unstable();
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    (
        us(samples[samples.len() / 2]),
        us(samples[samples.len() / 20]),
    )
}

struct Scene {
    text: Text,
    config: Config,
    look: Look,
    shadow: Option<Shadow>,
    urgencies: HashMap<u32, Urgency>,
    icons: HashMap<u32, scootnotify::view::render::IconBits>,
}

impl Scene {
    fn stack(&mut self, ids: &[u32]) -> Stack {
        let notes = notes(ids);
        let live: Vec<&Notification> = notes.iter().collect();
        for n in &notes {
            self.urgencies.insert(n.id, n.urgency);
        }
        let pad = self.shadow.as_ref().map_or(Pad::ZERO, Shadow::pad);
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
        if let Some(shadow) = self.shadow.as_mut().filter(|_| !stack.cards.is_empty()) {
            shadow.prepare(
                self.look.card_radius(self.config.radius),
                stack.cards.iter().map(|c| (c.rect.width, c.rect.height)),
            );
        }
        stack
    }

    fn paint_layers(&mut self, motion: &mut Motion) {
        let Scene {
            text,
            config,
            look,
            shadow,
            urgencies,
            icons,
        } = self;
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
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut font_path = None;
    let mut iters = 400usize;
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
    let look = Look {
        shadow: ShadowStyle::Soft,
        ..Look::default()
    };
    let mut scene = Scene {
        text: Text::new(font),
        config: Config::default(),
        shadow: look.shadow.spec().map(Shadow::new),
        look,
        urgencies: HashMap::new(),
        icons: HashMap::new(),
    };
    let tokens = |enter| MotionTokens {
        curve: Curve::Spring,
        duration_ms: 300,
        bounce: 0.6,
        enter,
        exit: CardMotion::Fade,
    };

    println!("scenario surface-px median-us p5-us");
    // Interleaved rounds, the best of nine, as bench_depth does.
    let mut best: Vec<(String, u32, u32, f64, f64)> = Vec::new();
    for _round in 0..9 {
        let mut row = 0;
        let mut record = |name: &str, w: u32, h: u32, (median, p5): (f64, f64)| {
            if best.len() <= row {
                best.push((name.to_owned(), w, h, median, p5));
            } else {
                best[row].3 = best[row].3.min(median);
                best[row].4 = best[row].4.min(p5);
            }
            row += 1;
        };

        let five = scene.stack(&[1, 2, 3, 4, 5]);
        let (w, h) = (five.width, five.height);

        // The plain repaint: what a stack at rest, or any look with motion
        // none, pays per repaint.
        let mut pixels = vec![0u8; w as usize * h as usize * 4];
        let mut samples = Vec::with_capacity(iters);
        for i in 0..iters + 20 {
            let start = Instant::now();
            let mut canvas = Canvas::new(&mut pixels, w, h).expect("canvas");
            paint(
                &mut canvas,
                &mut scene.text,
                &five,
                &scene.config,
                &scene.look,
                &scene.urgencies,
                None,
                &scene.icons,
                scene.shadow.as_ref(),
            );
            if i >= 20 {
                samples.push(start.elapsed());
            }
        }
        record("static repaint, 5 cards", w, h, median_and_p5(&mut samples));

        // Animated frames mid-flight, per scenario.
        let six = scene.stack(&[1, 2, 3, 4, 5, 6]);
        let four_of_five = scene.stack(&[1, 2, 4, 5]);
        for (name, enter, before, after, anchor) in [
            (
                "enter slide, arrival on 5",
                CardMotion::Slide,
                &five,
                &six,
                Anchor::TopRight,
            ),
            (
                "enter scale, arrival on 5",
                CardMotion::Scale,
                &five,
                &six,
                Anchor::TopRight,
            ),
            (
                "restack, close middle of 5",
                CardMotion::Fade,
                &five,
                &four_of_five,
                Anchor::TopRight,
            ),
            (
                "restack, bottom anchor",
                CardMotion::Fade,
                &five,
                &four_of_five,
                Anchor::BottomRight,
            ),
        ] {
            let t0 = Instant::now();
            let mut motion = Motion::new(&tokens(enter), anchor);
            // Start cost: retarget plus the layers' rasterization.
            let start = Instant::now();
            motion.retarget(before, after, t0);
            scene.paint_layers(&mut motion);
            let started = start.elapsed();
            let (w, h) = (after.width, after.height.max(motion.hold()));
            let mut pixels = vec![0u8; w as usize * h as usize * 4];
            let mut samples = Vec::with_capacity(iters);
            for i in 0..iters + 20 {
                // Spread the frames across the first 150 ms of the 300 ms
                // motion: mid-flight, every card still moving.
                let now = t0 + Duration::from_micros(150_000 * (i % 100) as u64 / 100);
                let start = Instant::now();
                motion.settle(now);
                scene.paint_layers(&mut motion);
                let mut canvas = Canvas::new(&mut pixels, w, h).expect("canvas");
                paint_moving(
                    &mut canvas,
                    &mut scene.text,
                    after,
                    &scene.config,
                    &scene.look,
                    &scene.urgencies,
                    None,
                    &scene.icons,
                    scene.shadow.as_ref(),
                    &motion,
                    now,
                );
                if i >= 20 {
                    samples.push(start.elapsed());
                }
            }
            record(&format!("{name}: frame"), w, h, median_and_p5(&mut samples));
            record(
                &format!("{name}: start (retarget + layers)"),
                w,
                h,
                (started.as_secs_f64() * 1e6, started.as_secs_f64() * 1e6),
            );
        }
    }
    for (name, w, h, median, p5) in best {
        println!("{name} {w}x{h} {median:.1} {p5:.1}");
    }
}
