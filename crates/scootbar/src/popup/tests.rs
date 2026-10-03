//! The pure popup: content bounds, layout arithmetic, the pointer state
//! machine, and what is painted, read back from pixels.

use ab_glyph::{FontArc, FontVec};

use super::{Activate, Content, Interaction, Kind, Layout, MAX_TEXT, MAX_WIDGETS, Wheel, paint};
use crate::paint::Canvas;
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

const EM: f32 = 20.0;
const PAD: u32 = 8;
const FRAME: u32 = 1;

fn text() -> Text {
    Text::new(FontArc::new(
        FontVec::try_from_vec(testfont::build()).unwrap(),
    ))
}

/// A text line, a slider (0 to 100) at 40, a button and a selected button.
fn sample() -> Content {
    let mut content = Content::default();
    assert!(content.text(format_args!("12 34")));
    assert!(content.slider(40, 100, "set"));
    assert!(content.button(format_args!("a"), "toggle", None, false, false));
    assert!(content.button(format_args!("p"), "pick", Some(7), true, false));
    content
}

fn laid(content: &Content) -> Layout {
    let mut layout = Layout::default();
    layout.compute(content, &text(), EM, PAD, FRAME);
    layout
}

/// The middle of a row's height.
fn mid(layout: &Layout, row: usize) -> i64 {
    let row = layout.rows()[row];
    i64::from(row.y0 + (row.y1 - row.y0) / 2)
}

#[test]
fn content_holds_labels_in_one_string_and_keeps_its_memory_when_cleared() {
    let mut content = sample();
    assert_eq!(content.widgets().len(), 4);
    assert_eq!(content.label(&content.widgets()[0]), "12 34");
    assert_eq!(content.label(&content.widgets()[1]), "");
    assert_eq!(content.label(&content.widgets()[3]), "p");
    let (widgets, text) = (content.widgets.capacity(), content.text.capacity());
    content.clear();
    assert!(content.is_empty());
    // Refilling after a clear does not grow what the first fill sized.
    let content = {
        let mut again = content;
        again.text(format_args!("x"));
        again
    };
    assert_eq!(content.widgets.capacity(), widgets);
    assert_eq!(content.text.capacity(), text);
}

#[test]
fn a_slider_value_is_held_to_its_max_and_a_zero_max_to_one() {
    let mut content = Content::default();
    content.slider(500, 100, "set");
    content.slider(3, 0, "set");
    assert!(matches!(
        content.widgets()[0].kind,
        Kind::Slider {
            value: 100,
            max: 100,
            ..
        }
    ));
    assert!(matches!(
        content.widgets()[1].kind,
        Kind::Slider {
            value: 1,
            max: 1,
            ..
        }
    ));
}

#[test]
fn widgets_and_text_are_bounded() {
    let mut content = Content::default();
    for _ in 0..MAX_WIDGETS {
        assert!(content.text(format_args!("row")));
    }
    assert!(!content.text(format_args!("one too many")));
    assert_eq!(content.widgets().len(), MAX_WIDGETS);

    let mut content = Content::default();
    let long = "é".repeat(MAX_TEXT);
    assert!(content.text(format_args!("{long}")));
    // Cut on a character boundary, at the bound, never past it.
    assert!(content.label(&content.widgets()[0]).len() <= MAX_TEXT);
    assert!(
        content
            .label(&content.widgets()[0])
            .chars()
            .all(|c| c == 'é')
    );
    // The text is full: nothing more is taken.
    assert!(!content.text(format_args!("more")));
}

#[test]
fn a_layout_stacks_equal_rows_inside_the_frame_and_is_at_least_the_minimum_wide() {
    let content = sample();
    let layout = laid(&content);
    let rows = layout.rows();
    assert_eq!(rows.len(), 4);
    let height = rows[0].y1 - rows[0].y0;
    assert_eq!(height, (EM * 1.9).ceil() as u32);
    for pair in rows.windows(2) {
        assert_eq!(pair[0].y1, pair[1].y0, "rows touch");
        assert_eq!(pair[1].y1 - pair[1].y0, height);
    }
    assert_eq!(rows[0].y0, FRAME + PAD / 2);
    assert_eq!(layout.height, rows[3].y1 + FRAME + PAD / 2);
    assert!(layout.width >= (EM * 13.0) as u32 + 2 * FRAME);
}

#[test]
fn a_longer_label_widens_the_popup_and_is_capped() {
    let mut content = Content::default();
    content.text(format_args!("{}", "0".repeat(10_000)));
    let layout = laid(&content);
    // 40 ems of label, padding and frame at most.
    assert!(layout.width <= (EM * 40.0) as u32 + 2 * PAD + 2 * FRAME);
    let mut short = Content::default();
    short.text(format_args!("1"));
    assert!(layout.width > laid(&short).width);
}

#[test]
fn an_empty_content_is_a_frame_and_nothing_in_it() {
    let layout = laid(&Content::default());
    assert!(layout.rows().is_empty());
    assert_eq!(layout.hit(10, 10), None);
    assert!(layout.height > 0 && layout.width > 0);
}

#[test]
fn degenerate_ems_do_not_panic() {
    for em in [0.0, -3.0, f32::NAN, f32::INFINITY, 1e30] {
        let mut layout = Layout::default();
        layout.compute(&sample(), &text(), em, PAD, FRAME);
        assert!(!layout.rows().is_empty());
    }
    let mut layout = Layout::default();
    layout.compute(&sample(), &text(), EM, u32::MAX, u32::MAX);
    let _ = layout.hit(0, 0);
    let _ = layout.track(1);
}

#[test]
fn a_hit_is_a_row_inside_the_frame_only() {
    let layout = laid(&sample());
    let x = i64::from(layout.width / 2);
    assert_eq!(layout.hit(x, mid(&layout, 0)), Some(0));
    assert_eq!(layout.hit(x, mid(&layout, 3)), Some(3));
    assert_eq!(layout.hit(x, -1), None);
    assert_eq!(layout.hit(x, i64::from(layout.height)), None);
    assert_eq!(layout.hit(0, mid(&layout, 1)), None, "the frame is no row");
    assert_eq!(layout.hit(-5, mid(&layout, 1)), None);
    assert_eq!(layout.hit(i64::from(layout.width), mid(&layout, 1)), None);
}

#[test]
fn a_slider_maps_x_to_a_value_both_ways_and_clamps_to_the_track() {
    let layout = laid(&sample());
    let track = layout.track(1);
    assert!(track.width > 0);
    let at = |x: i64| layout.value_at(1, x, 100);
    assert_eq!(at(i64::from(track.x)), 0);
    assert_eq!(at(i64::from(track.end())), 100);
    assert_eq!(at(-1000), 0);
    assert_eq!(at(1_000_000), 100);
    assert_eq!(at(i64::from(track.x + track.width / 2)), 50);
    // The knob's pixel for a value maps back to it.
    for value in [0, 1, 37, 99, 100] {
        let x = layout.knob_x(1, value, 100);
        assert_eq!(at(i64::from(x)), value, "value {value}");
    }
    // A huge max does not overflow.
    assert_eq!(
        layout.value_at(1, i64::from(track.end()), u32::MAX),
        u32::MAX
    );
}

#[test]
fn a_track_too_narrow_is_empty_and_gives_zero() {
    let mut content = Content::default();
    content.slider(5, 10, "set");
    let mut layout = Layout::default();
    layout.compute(&content, &text(), EM, PAD, FRAME);
    layout.width = 4;
    assert_eq!(layout.track(0).width, 0);
    assert_eq!(layout.value_at(0, 3, 10), 0);
    assert_eq!(layout.track(9).width, 0, "no such row");
}

#[test]
fn a_press_on_a_slider_acts_and_each_new_value_acts_once() {
    let content = sample();
    let layout = laid(&content);
    let mut state = Interaction::default();
    let y = mid(&layout, 1);
    let track = layout.track(1);
    let x = |value: u32| i64::from(layout.knob_x(1, value, 100));
    let _ = track;

    let down = state.press(&content, &layout, x(25), y);
    assert_eq!(
        down.activate,
        Some(Activate {
            action: "set",
            arg: Some(25),
            closes: false,
        })
    );
    assert_eq!(state.dragging(), Some((1, 25)));

    // The same value again: nothing is asked of the module.
    let same = state.motion(&content, &layout, x(25), y);
    assert_eq!(same.activate, None);
    assert!(!same.redraw);

    // The pointer wanders above the row: the value follows x, y no longer
    // matters (the compositor ends a drag that leaves the popup).
    let moved = state.motion(&content, &layout, x(80), -500);
    assert_eq!(moved.activate.map(|a| a.arg), Some(Some(80)));
    assert_eq!(state.dragging(), Some((1, 80)));
    // Far past the end clamps to the maximum.
    let end = state.motion(&content, &layout, 99_999, y);
    assert_eq!(end.activate.map(|a| a.arg), Some(Some(100)));

    let up = state.release(&content, &layout, 99_999, y);
    assert_eq!(up.activate, None, "the drag acted as it went");
    assert!(up.redraw);
    assert_eq!(state.dragging(), None);
}

#[test]
fn a_button_fires_on_release_over_the_button_it_was_pressed_on() {
    let content = sample();
    let layout = laid(&content);
    let x = i64::from(layout.width / 2);
    let mut state = Interaction::default();

    let down = state.press(&content, &layout, x, mid(&layout, 3));
    assert_eq!(down.activate, None, "a press only arms");
    let up = state.release(&content, &layout, x, mid(&layout, 3));
    assert_eq!(
        up.activate,
        Some(Activate {
            action: "pick",
            arg: Some(7),
            closes: false,
        })
    );

    // Pressed on one, released on another: nothing.
    state.press(&content, &layout, x, mid(&layout, 2));
    let off = state.release(&content, &layout, x, mid(&layout, 3));
    assert_eq!(off.activate, None);
    // Released outside the popup: nothing.
    state.press(&content, &layout, x, mid(&layout, 2));
    assert_eq!(state.release(&content, &layout, -4, -4).activate, None);
    // A release with nothing pressed is nothing, and does not redraw.
    let idle = state.release(&content, &layout, x, mid(&layout, 2));
    assert_eq!((idle.activate, idle.redraw), (None, false));
}

#[test]
fn a_press_on_text_or_outside_does_nothing_and_a_second_press_is_not_a_click() {
    let content = sample();
    let layout = laid(&content);
    let x = i64::from(layout.width / 2);
    let mut state = Interaction::default();
    assert_eq!(
        state.press(&content, &layout, x, mid(&layout, 0)),
        Default::default()
    );
    assert_eq!(state.press(&content, &layout, -1, 0), Default::default());
    assert!(!state.is_held());
    state.press(&content, &layout, x, mid(&layout, 2));
    // A second button while one is held: ignored, the first still armed.
    assert_eq!(
        state.press(&content, &layout, x, mid(&layout, 3)),
        Default::default()
    );
    assert_eq!(
        state
            .release(&content, &layout, x, mid(&layout, 2))
            .activate
            .map(|a| a.action),
        Some("toggle")
    );
}

#[test]
fn hover_marks_buttons_only_and_asks_for_a_redraw_only_when_it_changes() {
    let content = sample();
    let layout = laid(&content);
    let x = i64::from(layout.width / 2);
    let mut state = Interaction::default();
    assert!(state.motion(&content, &layout, x, mid(&layout, 2)).redraw);
    assert_eq!(state.hover(), Some(2));
    assert!(
        !state
            .motion(&content, &layout, x + 1, mid(&layout, 2))
            .redraw
    );
    // Text and slider rows are not hover targets.
    assert!(state.motion(&content, &layout, x, mid(&layout, 1)).redraw);
    assert_eq!(state.hover(), None);
    assert!(!state.motion(&content, &layout, x, mid(&layout, 0)).redraw);
    assert!(state.motion(&content, &layout, x, mid(&layout, 3)).redraw);
    assert!(state.clear());
    assert!(!state.clear());
}

#[test]
fn content_that_changes_under_a_drag_or_a_hover_drops_what_no_longer_fits() {
    let content = sample();
    let layout = laid(&content);
    let x = i64::from(layout.knob_x(1, 10, 100));
    let mut state = Interaction::default();
    state.press(&content, &layout, x, mid(&layout, 1));
    state.motion(&content, &layout, x, mid(&layout, 1));
    // The slider became text: the drag is over.
    let mut changed = Content::default();
    changed.text(format_args!("gone"));
    changed.text(format_args!("gone"));
    state.retain(&changed, &layout);
    assert!(!state.is_held());
    // A hover on a row that is gone.
    let mut state = Interaction::default();
    state.motion(
        &content,
        &layout,
        i64::from(layout.width / 2),
        mid(&layout, 3),
    );
    let mut short = Content::default();
    short.button(format_args!("a"), "toggle", None, false, false);
    state.retain(&short, &layout);
    assert_eq!(state.hover(), None);
    // And a motion with a drag whose widget is gone mid-flight is safe.
    let mut state = Interaction::default();
    state.press(&content, &layout, x, mid(&layout, 1));
    let out = state.motion(&changed, &layout, x + 30, mid(&layout, 1));
    assert_eq!(out.activate, None);
    assert!(!state.is_held());
}

#[test]
fn a_closing_button_asks_to_close_and_a_plain_one_does_not() {
    let mut content = Content::default();
    content.button(format_args!("stay"), "menu", None, false, false);
    content.button(format_args!("go"), "connect", Some(2), false, true);
    let layout = laid(&content);
    let x = i64::from(layout.width / 2);
    let mut state = Interaction::default();
    state.press(&content, &layout, x, mid(&layout, 0));
    let stay = state.release(&content, &layout, x, mid(&layout, 0));
    assert_eq!(
        stay.activate.map(|a| (a.action, a.arg, a.closes)),
        Some(("menu", None, false))
    );
    state.press(&content, &layout, x, mid(&layout, 1));
    let go = state.release(&content, &layout, x, mid(&layout, 1));
    assert_eq!(
        go.activate.map(|a| (a.action, a.arg, a.closes)),
        Some(("connect", Some(2), true))
    );
}

/// Ten rows with two visible (the compositor's configure came back
/// shorter than asked): the scroll, the clamp and what a press hits.
fn tall() -> (Content, Layout, u32) {
    let mut content = Content::default();
    for row in 0..10 {
        content.button(format_args!("net{row}"), "connect", Some(row), false, true);
    }
    let mut layout = laid(&content);
    let (y0, y1, second_y1) = {
        let rows = layout.rows();
        (rows[0].y0, rows[0].y1, rows[1].y1)
    };
    let edge = layout.frame + PAD / 2;
    // Two rows visible, the rest below.
    layout.height = second_y1 + edge;
    let row_h = y1 - y0;
    (content, layout, row_h)
}

#[test]
fn scrolling_moves_a_row_a_notch_and_holds_to_what_fits() {
    let (content, layout, row_h) = tall();
    assert_eq!(layout.max_scroll(), row_h * 8);
    let x = i64::from(layout.width / 2);
    let mut state = Interaction::default();
    assert_eq!(state.scroll(), 0);
    // One notch down: one row.
    assert!(state.scroll_by(&layout, 1));
    assert_eq!(state.scroll(), row_h);
    // Past the end clamps to the last row; back past the start to zero.
    assert!(state.scroll_by(&layout, 100));
    assert_eq!(state.scroll(), row_h * 8);
    assert!(!state.scroll_by(&layout, 1));
    assert!(state.scroll_by(&layout, -100));
    assert_eq!(state.scroll(), 0);
    assert!(!state.scroll_by(&layout, -1));
    assert!(!state.scroll_by(&layout, 0));
    // Scrolled one row, the first visible row is the second network: a
    // press at its middle hits row 1, and the release names it.
    assert!(state.scroll_by(&layout, 1));
    let y = mid(&layout, 1) - row_h as i64;
    state.press(&content, &layout, x, y);
    let up = state.release(&content, &layout, x, y);
    assert_eq!(
        up.activate.map(|a| (a.action, a.arg)),
        Some(("connect", Some(1)))
    );
    // What fits needs no scroll.
    let mut state = Interaction::default();
    let uncut = laid(&content);
    assert_eq!(uncut.max_scroll(), 0);
    assert!(!state.scroll_by(&uncut, 1));
}

#[test]
fn a_scrolled_away_press_is_dropped_and_a_new_content_holds_the_scroll() {
    let (content, layout, _) = tall();
    let x = i64::from(layout.width / 2);
    let mut state = Interaction::default();
    state.press(&content, &layout, x, mid(&layout, 0));
    // The wheel moves what the press armed on away: the release is
    // nothing, not a wrong row.
    assert!(state.scroll_by(&layout, 2));
    let up = state.release(&content, &layout, x, mid(&layout, 0));
    assert_eq!(up.activate, None);
    // A shorter content holds a scroll past its end to what fits.
    let mut short = Content::default();
    short.button(format_args!("only"), "connect", Some(0), false, true);
    let mut shrunk = Layout::default();
    shrunk.compute(&short, &text(), EM, PAD, FRAME);
    shrunk.height = shrunk.rows()[0].y1 + shrunk.frame + PAD / 2;
    assert_eq!(shrunk.max_scroll(), 0);
    state.retain(&short, &shrunk);
    assert_eq!(state.scroll(), 0);
    // Leaving clears the scroll with the rest.
    assert!(state.scroll_by(&layout, 1));
    assert!(state.clear());
    assert_eq!(state.scroll(), 0);
}

#[test]
fn a_wheel_groups_a_frame_into_rows() {
    let mut wheel = Wheel::default();
    wheel.axis_value120(120);
    assert_eq!(wheel.frame(), 1);
    // Two halves of a notch add up; a third waits.
    wheel.axis_value120(60);
    wheel.axis_value120(60);
    assert_eq!(wheel.frame(), 1);
    wheel.axis_value120(60);
    assert_eq!(wheel.frame(), 0);
    wheel.axis_value120(60);
    assert_eq!(wheel.frame(), 1);
    // A direction change drops the leftover.
    wheel.axis_value120(60);
    assert_eq!(wheel.frame(), 0);
    wheel.axis_value120(-120);
    assert_eq!(wheel.frame(), -1);
    // Continuous pixels, fifteen a notch.
    wheel.axis(15.0);
    assert_eq!(wheel.frame(), 1);
    wheel.axis(7.5);
    assert_eq!(wheel.frame(), 0);
    wheel.axis(7.5);
    assert_eq!(wheel.frame(), 1);
    // Discrete wins where both arrive in one frame.
    wheel.axis(150.0);
    wheel.axis_value120(120);
    assert_eq!(wheel.frame(), 1);
    wheel.axis_discrete(2);
    assert_eq!(wheel.frame(), 2);
    // Hostile input is nothing.
    wheel.axis(f64::NAN);
    wheel.axis(f64::INFINITY);
    assert_eq!(wheel.frame(), 0);
    wheel.axis_stop();
    assert_eq!(wheel.frame(), 0);
}

#[test]
fn a_long_label_is_cut_with_an_ellipsis() {
    let text = text();
    let mut kept = [0u8; MAX_TEXT];
    let cut = super::paint::cut;
    assert_eq!(cut(&text, "Wimbly", EM, 10_000, &mut kept), "Wimbly");
    let wide = "0".repeat(1000);
    let shown = cut(&text, &wide, EM, 100, &mut kept);
    assert!(shown.ends_with('…'), "{shown}");
    assert!(text.measure(None, shown, EM) <= 100);
    // Even the ellipsis does not fit: nothing.
    assert_eq!(cut(&text, &wide, EM, 0, &mut kept), "");
}

fn render(content: &Content, interaction: &Interaction) -> (Layout, Vec<u8>, u32) {
    let layout = laid(content);
    let (w, h) = (layout.width, layout.height);
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
    paint(
        &mut canvas,
        &mut text(),
        &Theme::default(),
        content,
        &layout,
        interaction,
        EM,
    );
    (layout, pixels, w)
}

fn rgb(pixels: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * w + x) * 4) as usize;
    [pixels[at + 2], pixels[at + 1], pixels[at]]
}

fn color(c: crate::color::Color) -> [u8; 3] {
    [c.r, c.g, c.b]
}

#[test]
fn the_popup_is_a_dim_frame_around_the_background() {
    let (layout, pixels, w) = render(&sample(), &Interaction::default());
    let theme = Theme::default();
    assert_eq!(rgb(&pixels, w, 0, 0), color(theme.dim));
    assert_eq!(rgb(&pixels, w, w - 1, layout.height - 1), color(theme.dim));
    assert_eq!(rgb(&pixels, w, 1, 1), color(theme.background));
    assert_eq!(
        rgb(&pixels, w, w - 2, layout.height - 2),
        color(theme.background)
    );
}

#[test]
fn the_slider_fills_to_its_value_and_a_drag_shows_the_pointers() {
    let content = sample();
    let theme = Theme::default();
    let (layout, pixels, w) = render(&content, &Interaction::default());
    let y = mid(&layout, 1) as u32;
    let track = layout.track(1);
    // Left of the knob the rail is the accent, right of it dim.
    let knob = layout.knob_x(1, 40, 100);
    let left = rgb(&pixels, w, track.x + 2, y);
    let right = rgb(&pixels, w, track.end() - 2, y);
    assert_eq!(left, color(theme.accent));
    assert_eq!(right, color(theme.dim));
    assert_eq!(rgb(&pixels, w, knob, y), color(theme.foreground));

    // Dragged to 90: the rail's accent reaches past where 40 ended.
    let mut state = Interaction::default();
    state.press(
        &content,
        &layout,
        i64::from(layout.knob_x(1, 90, 100)),
        y.into(),
    );
    let (_, dragged, _) = render(&content, &state);
    let probe = layout.knob_x(1, 70, 100);
    assert_ne!(rgb(&pixels, w, probe, y), color(theme.accent));
    // `render` lays out again, identically.
    assert_eq!(rgb(&dragged, w, probe, y), color(theme.accent));
}

#[test]
fn a_hovered_button_is_filled_and_a_selected_one_is_in_the_accent() {
    let content = sample();
    let theme = Theme::default();
    let layout = laid(&content);
    let mut state = Interaction::default();
    state.motion(
        &content,
        &layout,
        i64::from(layout.width / 2),
        mid(&layout, 2),
    );
    let (layout, pixels, w) = render(&content, &state);
    let row = layout.rows()[2];
    assert_eq!(rgb(&pixels, w, w - 3, row.y0 + 1), color(theme.dim));
    // The unhovered second button's row is the background.
    let other = layout.rows()[3];
    assert_eq!(
        rgb(&pixels, w, w - 3, other.y0 + 1),
        color(theme.background)
    );
    // Its label ("p", selected) is drawn in the accent somewhere, not the
    // foreground; the first button's ("a") is the foreground.
    let ink = |row: super::layout::Row, wanted: [u8; 3]| {
        (0..w).any(|x| (row.y0..row.y1).any(|y| rgb(&pixels, w, x, y) == wanted))
    };
    assert!(ink(layout.rows()[3], color(theme.accent)));
    assert!(!ink(layout.rows()[3], color(theme.foreground)));
    assert!(ink(layout.rows()[2], color(theme.foreground)));
}

#[test]
fn painting_into_a_canvas_smaller_than_the_layout_clips() {
    let content = sample();
    let layout = laid(&content);
    let mut pixels = vec![0u8; 10 * 10 * 4];
    let mut canvas = Canvas::new(&mut pixels, 10, 10).unwrap();
    paint(
        &mut canvas,
        &mut text(),
        &Theme::default(),
        &content,
        &layout,
        &Interaction::default(),
        EM,
    );
}

#[test]
fn a_warm_popup_allocates_nothing() {
    // What an open popup does on every turn it is busy: the module's content
    // refilled into the spare and compared, the layout redone, a motion or a
    // drag step through the state machine, and the whole popup painted. After
    // the first of each (which sizes the pooled vectors and fills the glyph
    // cache) nothing is allocated.
    let mut text = text();
    let mut spare = Content::default();
    let mut shown = Content::default();
    let mut layout = Layout::default();
    let mut state = Interaction::default();
    let theme = Theme::default();
    let fill = |content: &mut Content, level: u32| {
        content.clear();
        content.text(format_args!("12 34  {level}%"));
        content.slider(level, 100, "set");
        content.button(format_args!("a"), "toggle", None, false, false);
        content.button(format_args!("p"), "pick", Some(7), true, false);
    };
    fill(&mut shown, 40);
    layout.compute(&shown, &text, EM, PAD, FRAME);
    let (w, h) = (layout.width, layout.height);
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let x = i64::from(layout.knob_x(1, 50, 100));
    // Warm: one of everything, including a different level so the glyphs
    // of both are cached.
    for level in [41, 40, 99] {
        fill(&mut spare, level);
        let _ = spare != shown;
        std::mem::swap(&mut spare, &mut shown);
        layout.compute(&shown, &text, EM, PAD, FRAME);
        state.press(&shown, &layout, x, mid(&layout, 1));
        state.release(&shown, &layout, x, mid(&layout, 1));
        state.scroll_by(&layout, 1);
        state.scroll_by(&layout, -1);
        let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
        paint(&mut canvas, &mut text, &theme, &shown, &layout, &state, EM);
    }
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for round in 0..200u32 {
            fill(&mut spare, 40 + round % 3);
            if spare != shown {
                std::mem::swap(&mut spare, &mut shown);
            }
            layout.compute(&shown, &text, EM, PAD, FRAME);
            let y = mid(&layout, 1 + (round as usize % 3));
            state.motion(&shown, &layout, x + i64::from(round), y);
            state.press(&shown, &layout, x, y);
            state.motion(&shown, &layout, x + 3, y);
            state.release(&shown, &layout, x, y);
            state.retain(&shown, &layout);
            state.scroll_by(&layout, 1);
            state.scroll_by(&layout, -1);
            let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
            paint(&mut canvas, &mut text, &theme, &shown, &layout, &state, EM);
        }
    });
    assert_eq!(allocations, 0);
}
