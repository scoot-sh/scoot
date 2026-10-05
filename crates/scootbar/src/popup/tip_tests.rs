//! A tooltip's pure half: when it is due (`Hover`), how its text wraps
//! (`Content::wrap_tooltip`) and how it is laid out.

use std::time::{Duration, Instant};

use super::wrap::MAX_LINES;
use super::{Content, Hover, Key, Layout, MAX_TEXT, MAX_WIDGETS, Step};
use crate::outputs::{OutputId, Outputs};

const DELAY: Duration = Duration::from_millis(500);

fn keys() -> (Key, Key, Key) {
    let mut outputs: Outputs<()> = Outputs::default();
    let one = outputs.add(1, |_| ());
    let two = outputs.add(2, |_| ());
    let key = |output: OutputId, module| Key { output, module };
    (key(one, 0), key(one, 1), key(two, 0))
}

/// A clock the test sets, and counts the reads of.
struct Clock {
    now: Instant,
    reads: u32,
}

impl Clock {
    fn new() -> Self {
        Self {
            now: Instant::now(),
            reads: 0,
        }
    }
    fn advance(&mut self, by: Duration) {
        self.now += by;
    }
    fn read(&mut self) -> Instant {
        self.reads += 1;
        self.now
    }
}

fn update(hover: &mut Hover, clock: &mut Clock, hovered: Option<Key>, blocked: bool) -> Step {
    hover.update(hovered, blocked, &mut || clock.read())
}

fn wait(hover: &Hover, clock: &mut Clock) -> Option<Duration> {
    hover.wait(&mut || clock.read())
}

#[test]
fn no_timer_and_no_clock_read_unless_the_pointer_rests_on_a_tooltip() {
    let (a, ..) = keys();
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    // Nothing hovered: nothing waits, and the clock is never read.
    for _ in 0..100 {
        assert_eq!(update(&mut hover, &mut clock, None, false), Step::Nothing);
    }
    assert_eq!(wait(&hover, &mut clock), None);
    assert!(hover.is_idle());
    assert_eq!(clock.reads, 0, "an idle bar reads no clock");
    // Resting on a module that has one: a deadline, the clock read once to
    // set it.
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Nothing
    );
    assert_eq!(clock.reads, 1);
    assert_eq!(wait(&hover, &mut clock), Some(DELAY));
}

#[test]
fn the_tooltip_shows_when_the_delay_passes_and_then_nothing_waits() {
    let (a, ..) = keys();
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    update(&mut hover, &mut clock, Some(a), false);
    clock.advance(DELAY - Duration::from_millis(1));
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Nothing
    );
    assert_eq!(wait(&hover, &mut clock), Some(Duration::from_millis(1)));
    clock.advance(Duration::from_millis(1));
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Show(a)
    );
    assert!(hover.is_shown());
    // Shown: no deadline, no clock read, no repeat show.
    let reads = clock.reads;
    assert_eq!(wait(&hover, &mut clock), None);
    clock.advance(Duration::from_secs(60));
    for _ in 0..10 {
        assert_eq!(
            update(&mut hover, &mut clock, Some(a), false),
            Step::Nothing
        );
    }
    assert_eq!(clock.reads, reads);
}

#[test]
fn leaving_hides_it_and_cancels_a_pending_one() {
    let (a, ..) = keys();
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    update(&mut hover, &mut clock, Some(a), false);
    assert_eq!(update(&mut hover, &mut clock, None, false), Step::Nothing);
    assert_eq!(wait(&hover, &mut clock), None, "left: no timer");
    assert!(hover.is_idle());
    // Shown, then left.
    update(&mut hover, &mut clock, Some(a), false);
    clock.advance(DELAY);
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Show(a)
    );
    assert_eq!(update(&mut hover, &mut clock, None, false), Step::Hide);
    assert!(hover.is_idle());
    // Back: a whole new delay.
    update(&mut hover, &mut clock, Some(a), false);
    assert_eq!(wait(&hover, &mut clock), Some(DELAY));
}

#[test]
fn moving_to_another_module_hides_and_waits_the_whole_delay_again() {
    let (a, b, other_output) = keys();
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    update(&mut hover, &mut clock, Some(a), false);
    clock.advance(Duration::from_millis(400));
    // Slid onto b before a's was due: a's is forgotten, b's starts now.
    assert_eq!(
        update(&mut hover, &mut clock, Some(b), false),
        Step::Nothing
    );
    assert_eq!(wait(&hover, &mut clock), Some(DELAY));
    clock.advance(DELAY);
    assert_eq!(
        update(&mut hover, &mut clock, Some(b), false),
        Step::Show(b)
    );
    // From a shown tooltip to a module on another output: hide the first.
    assert_eq!(
        update(&mut hover, &mut clock, Some(other_output), false),
        Step::Hide
    );
    assert_eq!(wait(&hover, &mut clock), Some(DELAY));
}

#[test]
fn motion_within_a_module_does_not_restart_the_delay() {
    let (a, ..) = keys();
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    update(&mut hover, &mut clock, Some(a), false);
    for _ in 0..4 {
        clock.advance(Duration::from_millis(100));
        assert_eq!(
            update(&mut hover, &mut clock, Some(a), false),
            Step::Nothing
        );
    }
    clock.advance(Duration::from_millis(100));
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Show(a)
    );
}

#[test]
fn a_click_popup_wins_and_the_tooltip_does_not_return_under_a_still_pointer() {
    let (a, ..) = keys();
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    update(&mut hover, &mut clock, Some(a), false);
    clock.advance(DELAY);
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Show(a)
    );
    // A popup opens on the module: the tooltip goes.
    assert_eq!(update(&mut hover, &mut clock, Some(a), true), Step::Hide);
    assert_eq!(wait(&hover, &mut clock), None);
    // The popup closes, the pointer has not moved: no tooltip, ever, until
    // it leaves.
    clock.advance(Duration::from_secs(10));
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Nothing
    );
    assert_eq!(wait(&hover, &mut clock), None);
    update(&mut hover, &mut clock, None, false);
    update(&mut hover, &mut clock, Some(a), false);
    assert_eq!(wait(&hover, &mut clock), Some(DELAY));
}

#[test]
fn no_tooltip_is_due_while_a_popup_is_open_and_one_is_armed_when_it_closes() {
    let (a, b, _) = keys();
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    // Hovered with a popup already open: nothing armed, no clock read.
    assert_eq!(update(&mut hover, &mut clock, Some(a), true), Step::Nothing);
    assert_eq!(wait(&hover, &mut clock), None);
    clock.advance(DELAY * 4);
    assert_eq!(update(&mut hover, &mut clock, Some(a), true), Step::Nothing);
    assert_eq!(clock.reads, 0);
    // Crossing another module meanwhile arms nothing either.
    assert_eq!(update(&mut hover, &mut clock, Some(b), true), Step::Nothing);
    assert_eq!(wait(&hover, &mut clock), None);
    // The popup closes under a pointer that rests on a module no tooltip was
    // pending for: its tooltip is due a delay from then.
    assert_eq!(
        update(&mut hover, &mut clock, Some(b), false),
        Step::Nothing
    );
    assert_eq!(wait(&hover, &mut clock), Some(DELAY));
}

#[test]
fn a_press_or_scroll_dismisses_until_the_pointer_leaves_the_module() {
    let (a, b, _) = keys();
    for pending in [false, true] {
        let mut hover = Hover::new(DELAY);
        let mut clock = Clock::new();
        update(&mut hover, &mut clock, Some(a), false);
        if !pending {
            clock.advance(DELAY);
            assert_eq!(
                update(&mut hover, &mut clock, Some(a), false),
                Step::Show(a)
            );
        }
        hover.dismiss(None);
        assert_eq!(wait(&hover, &mut clock), None);
        clock.advance(DELAY * 10);
        assert_eq!(
            update(&mut hover, &mut clock, Some(a), false),
            Step::Nothing
        );
        // Another module has its own.
        update(&mut hover, &mut clock, Some(b), false);
        assert_eq!(wait(&hover, &mut clock), Some(DELAY));
    }
}

#[test]
fn what_the_compositor_took_back_is_a_dismissal_and_a_failed_open_too() {
    let (a, ..) = keys();
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    update(&mut hover, &mut clock, Some(a), false);
    clock.advance(DELAY);
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Show(a)
    );
    // `popup_done`: held off, not reopened a turn later.
    hover.gone();
    clock.advance(DELAY * 3);
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Nothing
    );
    assert_eq!(wait(&hover, &mut clock), None);
    // Dismissing what is idle is nothing.
    let mut idle = Hover::new(DELAY);
    idle.dismiss(None);
    assert!(idle.is_idle());
}

#[test]
fn zero_is_off_and_a_reset_forgets_what_was_hovered() {
    let (a, ..) = keys();
    let mut off = Hover::new(Duration::ZERO);
    let mut clock = Clock::new();
    assert!(!off.enabled());
    assert_eq!(update(&mut off, &mut clock, Some(a), false), Step::Nothing);
    assert_eq!(wait(&off, &mut clock), None);
    assert_eq!(clock.reads, 0);

    let mut hover = Hover::new(DELAY);
    update(&mut hover, &mut clock, Some(a), false);
    hover.reset(Duration::from_millis(50));
    assert!(hover.is_idle());
    update(&mut hover, &mut clock, Some(a), false);
    assert_eq!(wait(&hover, &mut clock), Some(Duration::from_millis(50)));
    hover.reset(Duration::ZERO);
    assert!(!hover.enabled());
}

#[test]
fn a_delay_that_overflows_the_clock_is_a_dismissal_not_a_panic() {
    let (a, ..) = keys();
    let mut hover = Hover::new(Duration::MAX);
    let mut clock = Clock::new();
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Nothing
    );
    assert_eq!(wait(&hover, &mut clock), None);
}

/// Every character one tenth of an em wide: a line of `n` characters is `n`
/// pixels.
fn width(line: &str) -> u32 {
    line.chars().count() as u32
}

fn lines(content: &Content) -> Vec<&str> {
    content
        .widgets()
        .iter()
        .map(|widget| content.label(widget))
        .collect()
}

fn wrapped(tip: &str, max: u32) -> Content {
    let mut content = Content::default();
    content.wrap_tooltip(tip, max, width);
    content
}

#[test]
fn a_short_tooltip_is_one_line_and_a_long_one_wraps_at_spaces() {
    assert_eq!(lines(&wrapped("Charging 80%", 40)), ["Charging 80%"]);
    assert_eq!(
        lines(&wrapped("the quick brown fox jumps", 10)),
        ["the quick", "brown fox", "jumps"]
    );
    // Exactly the width fits.
    assert_eq!(lines(&wrapped("abcde fghij", 11)), ["abcde fghij"]);
    assert_eq!(lines(&wrapped("abcde fghij", 10)), ["abcde", "fghij"]);
}

#[test]
fn a_newline_breaks_a_line_and_control_characters_and_blank_text_vanish() {
    assert_eq!(lines(&wrapped("one\ntwo", 40)), ["one", "two"]);
    assert_eq!(lines(&wrapped("a\n\nb", 40)), ["a", "b"]);
    assert_eq!(lines(&wrapped("a\u{1b}[31mb", 40)), ["a[31mb"]);
    assert!(wrapped("", 40).is_empty());
    assert!(wrapped("   \n \t ", 40).is_empty());
}

#[test]
fn a_word_wider_than_the_line_is_broken_where_it_fills_it() {
    assert_eq!(lines(&wrapped("abcdefghij", 4)), ["abcd", "efgh", "ij"]);
    assert_eq!(
        lines(&wrapped("x abcdefghij", 4)),
        ["x", "abcd", "efgh", "ij"]
    );
    // Multi-byte characters are never split mid-character.
    assert_eq!(lines(&wrapped("ééééé", 2)), ["éé", "éé", "é"]);
    // Narrower than one character: still one character a line, no hang.
    assert_eq!(lines(&wrapped("abc", 0)), ["a", "b", "c"]);
}

#[test]
fn the_lines_are_capped_with_an_ellipsis_on_the_last() {
    let tip = "a b c d e f g h i j k l m n o p";
    let content = wrapped(tip, 1);
    let shown = lines(&content);
    assert_eq!(shown.len(), MAX_LINES);
    assert_eq!(shown[MAX_LINES - 1], "…");
    // Exactly the cap is not cut.
    let exact = wrapped("a b c d e f", 1);
    assert_eq!(lines(&exact), ["a", "b", "c", "d", "e", "f"]);
    // The ellipsis fits the width too: a long last line loses characters.
    let long = wrapped("ab cd ef gh ij kl mn op qr st uv wx yz", 5);
    assert!(
        lines(&long).iter().all(|line| width(line) <= 5),
        "{:?}",
        lines(&long)
    );
    assert!(lines(&long).last().unwrap().ends_with('…'));
}

#[test]
fn the_bounds_hold_for_hostile_input_and_the_storage_is_reused() {
    let huge = "word ".repeat(10_000);
    let content = wrapped(&huge, 12);
    assert!(content.widgets().len() <= MAX_LINES.min(MAX_WIDGETS));
    assert!(content.text.len() <= MAX_TEXT + '…'.len_utf8());
    let mut content = content;
    let (widgets, text) = (content.widgets.capacity(), content.text.capacity());
    for tip in ["a few words here", "again and again and again", "x"] {
        content.wrap_tooltip(tip, 8, width);
        assert!(content.widgets.capacity() <= widgets.max(MAX_LINES));
        assert_eq!(content.text.capacity(), text, "refilling reuses the string");
    }
    // Every character a different width class, and one that never fits.
    let lone = wrapped("\u{10ffff}", 0);
    assert_eq!(lone.widgets().len(), 1);
}

#[test]
fn a_tooltip_layout_is_as_wide_as_its_widest_line_and_rows_stack() {
    use ab_glyph::{FontArc, FontVec};
    let text = crate::text::Text::new(FontArc::new(
        FontVec::try_from_vec(crate::testfont::build()).unwrap(),
    ));
    let em = 20.0;
    let content = {
        let mut content = Content::default();
        content.wrap_tooltip("one two three", 10_000, |line| text.measure(None, line, em));
        content
    };
    let mut layout = Layout::default();
    layout.compute_tooltip(&content, &text, em, 8, 1);
    let line = text.measure(None, "one two three", em);
    assert_eq!(layout.width, line + 16 + 2);
    assert_eq!(layout.rows().len(), 1);
    let row = layout.rows()[0];
    assert_eq!(row.y0, 1 + 4);
    assert_eq!(layout.height, row.y1 + 1 + 4);
    // Two lines: two rows, the second right under the first.
    let mut two = Content::default();
    two.wrap_tooltip("one\ntwo", 10_000, |line| text.measure(None, line, em));
    layout.compute_tooltip(&two, &text, em, 8, 1);
    assert_eq!(layout.rows().len(), 2);
    assert_eq!(layout.rows()[0].y1, layout.rows()[1].y0);
    // Empty: a frame and its padding, no panic.
    layout.compute_tooltip(&Content::default(), &text, em, 8, 1);
    assert!(layout.rows().is_empty());
    layout.compute_tooltip(&two, &text, f32::NAN, 8, 1);
    assert!(layout.width > 0 && layout.height > 0);
}

#[test]
fn a_press_on_a_module_nothing_was_armed_for_still_holds_its_tooltip_off() {
    let (a, b, _) = keys();
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    // The pointer arrives with a popup open and presses (closing it): the
    // module's tooltip is not armed after the click.
    assert_eq!(update(&mut hover, &mut clock, Some(a), true), Step::Nothing);
    hover.dismiss(Some(a));
    assert_eq!(
        update(&mut hover, &mut clock, Some(a), false),
        Step::Nothing
    );
    assert_eq!(wait(&hover, &mut clock), None);
    assert_eq!(clock.reads, 0);
    // Another module has its own, and so has this one after a leave.
    update(&mut hover, &mut clock, Some(b), false);
    assert_eq!(wait(&hover, &mut clock), Some(DELAY));
    update(&mut hover, &mut clock, None, false);
    update(&mut hover, &mut clock, Some(a), false);
    assert_eq!(wait(&hover, &mut clock), Some(DELAY));
    // A press over no module with a tooltip changes nothing.
    let mut hover = Hover::new(DELAY);
    hover.dismiss(None);
    assert!(hover.is_idle());
}

#[test]
fn a_warm_hover_and_a_shown_tooltip_allocate_nothing() {
    use crate::paint::Canvas;
    use crate::theme::Theme;
    use ab_glyph::{FontArc, FontVec};

    let (a, b, _) = keys();
    let text = crate::text::Text::new(FontArc::new(
        FontVec::try_from_vec(crate::testfont::build()).unwrap(),
    ));
    let mut text = text;
    let em = 20.0;
    let mut hover = Hover::new(DELAY);
    let mut clock = Clock::new();
    let mut spare = Content::default();
    let mut shown = Content::default();
    let mut layout = Layout::default();
    let theme = Theme::default();
    let tips = [
        "a window title that wraps onto a second line because it is long",
        "Charging 80%",
        "one\ntwo\nthree",
    ];
    let max = (em * super::TIP_MAX_EM) as u32;
    // Warm: both contents sized by the longest text (they swap, so each is
    // the spare in turn), then one of every text for the layout and the glyph
    // cache, and a canvas to paint into.
    for content in [&mut spare, &mut shown] {
        content.wrap_tooltip(tips[0], max, |line| text.measure(None, line, em));
    }
    let mut pixels = Vec::new();
    let sharp = super::Round::new(0, 1, 1, 1);
    for tip in tips {
        spare.wrap_tooltip(tip, max, |line| text.measure(None, line, em));
        std::mem::swap(&mut spare, &mut shown);
        layout.compute_tooltip(&shown, &text, em, 8, 1);
        pixels.resize(
            pixels
                .len()
                .max((layout.width * layout.height * 4) as usize),
            0,
        );
        let mut canvas = Canvas::new(&mut pixels, layout.width, layout.height).unwrap();
        super::paint(
            &mut canvas,
            &mut text,
            &theme,
            &shown,
            &layout,
            &super::Interaction::default(),
            em,
            &sharp,
        );
    }
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for round in 0..300usize {
            // What a turn does with the pointer on the bar: the state machine,
            // moving between two modules and resting (the clock read only
            // while a deadline is pending), a due tooltip, a leave.
            let hovered = [Some(a), Some(a), Some(b), None][round % 4];
            let _ = hover.update(hovered, round % 7 == 6, &mut || clock.read());
            let _ = hover.wait(&mut || clock.read());
            clock.advance(Duration::from_millis(200));
            if round % 5 == 0 {
                hover.dismiss(Some(a));
            }
            // What a shown tooltip does when its text changes: wrapped into the
            // spare, compared, laid out into the same layout, painted.
            spare.wrap_tooltip(tips[round % 3], max, |line| text.measure(None, line, em));
            if spare != shown {
                std::mem::swap(&mut spare, &mut shown);
                layout.compute_tooltip(&shown, &text, em, 8, 1);
                let mut canvas = Canvas::new(&mut pixels, layout.width, layout.height).unwrap();
                super::paint(
                    &mut canvas,
                    &mut text,
                    &theme,
                    &shown,
                    &layout,
                    &super::Interaction::default(),
                    em,
                    &sharp,
                );
            }
        }
    });
    assert_eq!(allocations, 0, "a warm tooltip allocated");
}
