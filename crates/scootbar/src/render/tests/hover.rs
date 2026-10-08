//! Hover: the module under the pointer repaints in the accent token, and
//! only it: the span it left and the span it entered, on this output and
//! in whichever buffer is drawn, never a neighbour and never another
//! output. The pointer's place is the scene's, not a module's revision.

use super::*;
use crate::render::Member;

/// A bar of hoverable modules (each `text` in its section).
fn bar(modules: &[(Section, &str)]) -> Bar {
    let mut bar = Bar::new(modules);
    let sections: Vec<_> = modules.iter().map(|m| m.0).collect();
    bar.scene = Scene::all_hoverable(&sections, &vec![true; modules.len()]);
    bar
}

fn accent(pixels: &[u8], span: Span, bar: &Bar) -> bool {
    let accent = rgb(bar.style.theme.accent);
    (0..HEIGHT).any(|y| (span.x..span.end()).any(|x| pixel(pixels, x, y) == accent))
}

fn foreground(pixels: &[u8], span: Span, bar: &Bar) -> bool {
    let fg = rgb(bar.style.theme.foreground);
    (0..HEIGHT).any(|y| (span.x..span.end()).any(|x| pixel(pixels, x, y) == fg))
}

fn middle(span: Span) -> u32 {
    span.x + span.width / 2
}

/// Draws the bar as the daemon does: paint the buffer, damage the surface.
struct Drawn {
    pixels: Vec<u8>,
    record: Record,
    shown: Record,
    spans: Vec<Span>,
    whole: bool,
}

impl Drawn {
    fn new(modules: usize) -> Self {
        Self {
            pixels: buffer(),
            record: Record::new(modules),
            shown: Record::new(modules),
            spans: Vec::new(),
            whole: false,
        }
    }

    fn draw(&mut self, bar: &mut Bar) {
        bar.paint(&mut self.pixels, &mut self.record);
        self.whole = damage(&mut self.shown, &bar.scene, frame(), &mut self.spans);
    }
}

#[test]
fn hover_repaints_and_damages_only_the_module_entered() {
    let mut bar = bar(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let mut drawn = Drawn::new(2);
    drawn.draw(&mut bar);
    assert!(drawn.whole);
    assert!(!bar.scene.stale(&bar.placed, &drawn.shown));
    let spans = bar.scene.spans().to_vec();
    assert!(!accent(&drawn.pixels, spans[0], &bar));

    // Scribble on the other module: a partial repaint must leave it alone.
    let marker = (spans[1].x + 1, 1);
    let i = ((marker.1 * WIDTH + marker.0) * 4) as usize;
    drawn.pixels[i..i + 4].copy_from_slice(&[1, 2, 3, 0xff]);

    bar.scene.set_pointer(Some(middle(spans[0])));
    assert!(
        bar.scene.stale(&bar.placed, &drawn.shown),
        "a hover is a draw"
    );
    drawn.draw(&mut bar);
    assert!(!drawn.whole);
    assert_eq!(drawn.spans, [spans[0]], "damage is the one span");
    assert!(accent(&drawn.pixels, spans[0], &bar));
    assert!(!foreground(&drawn.pixels, spans[0], &bar));
    assert_eq!(pixel(&drawn.pixels, marker.0, marker.1), [3, 2, 1]);
    let background = rgb(bar.style.theme.background);
    assert_eq!(read(&drawn.pixels, spans[0], background), "1:00");
    assert!(
        !bar.scene.stale(&bar.placed, &drawn.shown),
        "drawn, so settled"
    );
}

#[test]
fn moving_between_modules_repaints_the_one_left_and_the_one_entered() {
    let mut bar = bar(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let mut drawn = Drawn::new(2);
    drawn.draw(&mut bar);
    let spans = bar.scene.spans().to_vec();

    bar.scene.set_pointer(Some(middle(spans[0])));
    drawn.draw(&mut bar);
    bar.scene.set_pointer(Some(middle(spans[1])));
    drawn.draw(&mut bar);
    assert_eq!(drawn.spans, [spans[0], spans[1]]);
    assert!(
        foreground(&drawn.pixels, spans[0], &bar),
        "the left is back"
    );
    assert!(!accent(&drawn.pixels, spans[0], &bar));
    assert!(accent(&drawn.pixels, spans[1], &bar));

    // Leaving the bar repaints the last one only.
    bar.scene.set_pointer(None);
    drawn.draw(&mut bar);
    assert_eq!(drawn.spans, [spans[1]]);
    assert!(!accent(&drawn.pixels, spans[1], &bar));
}

#[test]
fn moving_inside_one_module_draws_nothing() {
    let mut bar = bar(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let mut drawn = Drawn::new(2);
    drawn.draw(&mut bar);
    let span = bar.scene.spans()[0];
    bar.scene.set_pointer(Some(span.x + 1));
    drawn.draw(&mut bar);
    for x in span.x..span.end() {
        bar.scene.set_pointer(Some(x));
        assert!(!bar.scene.stale(&bar.placed, &drawn.shown), "x {x}");
    }
}

#[test]
fn a_hover_does_not_touch_a_module_or_ask_its_view_again() {
    let mut bar = bar(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let mut drawn = Drawn::new(2);
    drawn.draw(&mut bar);
    let spans = bar.scene.spans().to_vec();
    let before = (bar.asked[0].get(), bar.asked[1].get());
    bar.scene.set_pointer(Some(middle(spans[0])));
    drawn.draw(&mut bar);
    bar.scene.set_pointer(Some(middle(spans[1])));
    drawn.draw(&mut bar);
    assert_eq!((bar.asked[0].get(), bar.asked[1].get()), before);
    // The revisions are the modules' own, shared by every output: a hover
    // on this one must not look like a change to another.
    assert!(bar.placed.iter().all(|placed| placed.revision == 0));
}

#[test]
fn a_hover_is_this_outputs_alone() {
    let mut bar = bar(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let mut other = Scene::all_hoverable(&[Section::Left, Section::Right], &[true, true]);
    bar.update();
    other.update(
        &bar.placed,
        &OutputView { name: None },
        Some(&bar.text),
        &bar.style,
        Scale::Integer(1),
        Size {
            width: WIDTH,
            height: HEIGHT,
        },
    );
    let mut on_a = Drawn::new(2);
    let mut on_b = Drawn::new(2);
    on_a.draw(&mut bar);
    let _ = damage(&mut on_b.shown, &other, frame(), &mut on_b.spans);
    let span = bar.scene.spans()[0];
    bar.scene.set_pointer(Some(middle(span)));
    assert!(bar.scene.stale(&bar.placed, &on_a.shown));
    assert!(
        !other.stale(&bar.placed, &on_b.shown),
        "the other output is untouched"
    );
}

#[test]
fn a_buffer_that_missed_a_hover_catches_up() {
    let mut bar = bar(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let (mut a, mut b) = (buffer(), buffer());
    let (mut record_a, mut record_b) = (Record::new(2), Record::new(2));
    bar.paint(&mut a, &mut record_a);
    bar.paint(&mut b, &mut record_b);
    let spans = bar.scene.spans().to_vec();
    // The hover is drawn into `b` only; then it moves, and `a` is next.
    bar.scene.set_pointer(Some(middle(spans[0])));
    bar.paint(&mut b, &mut record_b);
    bar.scene.set_pointer(Some(middle(spans[1])));
    bar.paint(&mut a, &mut record_a);
    assert!(
        !accent(&a, spans[0], &bar),
        "the left was never tinted in `a`"
    );
    assert!(accent(&a, spans[1], &bar));
    // And `b`, drawn again, drops the hover it still shows on the left.
    bar.paint(&mut b, &mut record_b);
    assert!(!accent(&b, spans[0], &bar));
    assert!(accent(&b, spans[1], &bar));
    assert_eq!(a, b);
}

#[test]
fn a_module_with_no_binding_is_never_tinted() {
    let mut bar = Bar::new(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let mut drawn = Drawn::new(2);
    drawn.draw(&mut bar);
    let span = bar.scene.spans()[0];
    bar.scene.set_pointer(Some(middle(span)));
    assert!(!bar.scene.stale(&bar.placed, &drawn.shown));
    drawn.draw(&mut bar);
    assert!(drawn.spans.is_empty());
    assert!(!accent(&drawn.pixels, span, &bar));
}

#[test]
fn hover_follows_a_module_that_grows_under_the_pointer() {
    let mut bar = bar(&[(Section::Left, "1"), (Section::Left, "2")]);
    let mut drawn = Drawn::new(2);
    drawn.draw(&mut bar);
    let spans = bar.scene.spans().to_vec();
    // Over the second module; the first grows and pushes it right.
    let x = middle(spans[1]);
    bar.scene.set_pointer(Some(x));
    drawn.draw(&mut bar);
    bar.set(0, "1111", Class::Normal);
    drawn.draw(&mut bar);
    let moved = bar.scene.spans().to_vec();
    assert_ne!(moved[1], spans[1]);
    // `x` is now inside whichever module holds it, and only that is tinted.
    let under = bar.scene.member_at(x).expect("something is there");
    assert_eq!(accent(&drawn.pixels, moved[0], &bar), under == 0);
    assert_eq!(accent(&drawn.pixels, moved[1], &bar), under == 1);
}

#[test]
fn a_module_that_vanishes_under_the_pointer_loses_the_hover() {
    let mut bar = bar(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let mut drawn = Drawn::new(2);
    drawn.draw(&mut bar);
    let spans = bar.scene.spans().to_vec();
    bar.scene.set_pointer(Some(middle(spans[1])));
    drawn.draw(&mut bar);
    assert!(accent(&drawn.pixels, spans[1], &bar));
    // It shows nothing now: no span, nothing to hit or tint, and no panic.
    bar.set(1, "", Class::Normal);
    drawn.draw(&mut bar);
    assert_eq!(bar.scene.spans()[1].width, 0);
    assert_eq!(bar.scene.member_at(middle(spans[1])), None);
    assert!(!bar.scene.stale(&bar.placed, &drawn.shown));
    let background = rgb(bar.style.theme.background);
    for y in 0..HEIGHT {
        assert_eq!(pixel(&drawn.pixels, middle(spans[1]), y), background);
    }
    // It comes back, and the pointer is still over it: tinted again.
    bar.set(1, "7:07", Class::Normal);
    drawn.draw(&mut bar);
    assert!(accent(&drawn.pixels, bar.scene.spans()[1], &bar));
}

#[test]
fn a_rebuilt_scene_holds_no_hover() {
    // A reload builds new scenes: nothing is hovered until the pointer says.
    let mut bar = bar(&[(Section::Left, "1:00")]);
    bar.update();
    bar.scene.set_pointer(Some(1));
    let members = [Member {
        module: 0,
        section: Section::Left,
        margin: 0,
        hover: true,
        separator_before: false,
    }];
    let fresh = Scene::with_members(&members);
    assert_eq!(fresh.hover, None);
    assert_eq!(fresh.pointer, None);
}

#[test]
fn a_press_is_routed_by_the_layout_on_screen_until_the_next_draw() {
    // A module's width changes (a revision bump) but nothing is drawn yet:
    // the spans are still the ones the user saw, so a click lands on what
    // was on screen. The next draw moves them.
    let mut bar = bar(&[(Section::Left, "1"), (Section::Left, "2")]);
    bar.update();
    let before = bar.scene.spans().to_vec();
    let x = middle(before[1]);
    assert_eq!(bar.scene.member_at(x), Some(1));
    bar.set(0, "1111", Class::Normal);
    assert_eq!(bar.scene.member_at(x), Some(1), "no draw yet");
    assert_eq!(bar.scene.spans(), before);
    bar.update();
    assert_ne!(bar.scene.spans(), before);
}

#[test]
fn member_at_holds_the_left_edge_and_not_the_right() {
    let mut bar = bar(&[
        (Section::Left, "1"),
        (Section::Left, ""),
        (Section::Left, "2"),
    ]);
    bar.update();
    let spans = bar.scene.spans().to_vec();
    assert_eq!(bar.scene.member_at(spans[0].x), Some(0));
    assert_eq!(bar.scene.member_at(spans[0].end() - 1), Some(0));
    assert_eq!(
        bar.scene.member_at(spans[0].end()),
        Some(2),
        "the empty one holds none"
    );
    assert_eq!(bar.scene.member_at(WIDTH), None);
    assert_eq!(bar.scene.member_at(u32::MAX), None);
}

#[test]
fn a_warm_hover_allocates_nothing() {
    // A hover is the pointer's hot path: a repaint per module entered or
    // left, at the rate the pointer moves. After the first draw (which
    // sizes the pooled vectors) nothing is allocated.
    let mut bar = bar(&[(Section::Left, "1:00"), (Section::Right, "7:07")]);
    let mut drawn = Drawn::new(2);
    drawn.draw(&mut bar);
    let spans = bar.scene.spans().to_vec();
    // Warm: one hover each way.
    for span in [spans[0], spans[1]] {
        bar.scene.set_pointer(Some(middle(span)));
        drawn.draw(&mut bar);
    }
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for round in 0..100 {
            let x = match round % 3 {
                0 => middle(spans[0]),
                1 => middle(spans[1]),
                _ => 0,
            };
            bar.scene.set_pointer(Some(x));
            let _ = bar.scene.stale(&bar.placed, &drawn.shown);
            let _ = bar.scene.member_at(x);
            drawn.draw(&mut bar);
        }
        bar.scene.set_pointer(None);
        drawn.draw(&mut bar);
    });
    assert_eq!(allocations, 0);
}
