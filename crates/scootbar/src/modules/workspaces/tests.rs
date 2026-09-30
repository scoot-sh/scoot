//! The workspaces module through the harness: views, batches, pills and
//! hit tests, with the protocol side injected as plain data (building real
//! `ext-workspace-v1` objects needs a compositor; the integration tests in
//! `tests/workspaces.rs` drive those). Every test starts the module as the
//! bar does, through [`Harness::start`].

use ab_glyph::{FontArc, FontVec};

use super::{Group, Link, MAX_WORKSPACES, Ws, hit_index, item_span, parse_coord, parse_number};
use crate::density::Scale;
use crate::modules::harness::Harness;
use crate::modules::{ClickCtx, CustomDraw, MAX_TEXT, Module, OutputView, Update, find};
use crate::paint::{Canvas, Span};
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };
const EM: f32 = 50.0;
const PAD: u32 = 8;

fn text() -> Text {
    let font = FontArc::new(FontVec::try_from_vec(testfont::build()).unwrap());
    Text::new(font)
}

/// The module as the bar starts it, with its shared state to inject
/// batches through.
fn started() -> (Harness, Link) {
    let spec = find("workspaces").expect("the workspaces module is built");
    let settings = super::super::Settings::default();
    let link = settings.workspaces.link.clone();
    let harness = Harness::start(spec, &settings).expect("workspaces starts anywhere");
    (harness, link)
}

/// Commits one output's workspaces as a single `done` would: `items` are
/// `(number, coord, active)`, sorted by `coord` at commit like the real
/// path.
fn commit(link: &Link, name: &str, items: &[(u32, u32, bool)]) {
    let mut shared = link.0.borrow_mut();
    let index = match shared.groups[..shared.groups_len]
        .iter()
        .position(|group| group.name() == name)
    {
        Some(index) => index,
        None => {
            assert!(shared.groups_len < super::MAX_GROUPS);
            let index = shared.groups_len;
            shared.groups[index] = Group::default();
            shared.groups_len += 1;
            index
        }
    };
    let group = &mut shared.groups[index];
    let bytes = name.as_bytes();
    let len = bytes.len().min(super::MAX_NAME);
    group.name[..len].copy_from_slice(&bytes[..len]);
    group.name_len = len;
    group.staged_len = 0;
    for &(number, coord, active) in items {
        assert!(group.staged_len < MAX_WORKSPACES);
        group.staged[group.staged_len] = Ws {
            handle: None,
            number,
            coord,
            active,
        };
        group.staged_len += 1;
    }
    shared.on_done();
}

#[test]
fn unavailable_hardware_is_an_empty_view_not_an_error() {
    let (harness, _) = started();
    assert_eq!(harness.source_count(), 0);
    assert!(harness.view().is_empty());
    assert!(harness.view_on(Some("DP-1")).is_empty());
}

#[test]
fn a_name_s_leading_number_is_what_shows() {
    assert_eq!(parse_number("1"), 1);
    assert_eq!(parse_number("2 DP-1"), 2);
    assert_eq!(parse_number("10"), 10);
    assert_eq!(parse_number("2:web"), 2);
    assert_eq!(parse_number(""), 0);
    assert_eq!(parse_number("web"), 0);
    assert_eq!(parse_number("99999999999"), 999999999);
}

#[test]
fn coordinates_are_the_first_native_u32() {
    assert_eq!(parse_coord(&1u32.to_ne_bytes()), 1);
    assert_eq!(parse_coord(&[1, 0, 0, 0, 99]), 1);
    assert_eq!(parse_coord(&[]), 0);
    assert_eq!(parse_coord(&[7]), 0);
}

#[test]
fn each_output_shows_its_own_numbers_sorted_by_coordinates() {
    let (harness, link) = started();
    // Arrival order is not sorted order; coordinates rule.
    commit(&link, "DP-1", &[(10, 10, false), (2, 2, true)]);
    commit(&link, "HDMI-1", &[(1, 1, true)]);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "2 10");
    assert_eq!(harness.view_on(Some("HDMI-1")).text(), "1");
    // No group here, and no name at all: nothing to show, taking no space.
    assert!(harness.view_on(Some("DP-2")).is_empty());
    assert!(harness.view_on(None).is_empty());
    assert!(harness.view().is_empty());
}

#[test]
fn a_workspace_never_named_shows_its_position() {
    let (harness, link) = started();
    // Sorted by coordinates first: `(3, 3)` stays first, `(0, 5)` shows
    // its 1-based position, 2.
    commit(&link, "DP-1", &[(0, 5, false), (3, 3, true)]);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "3 2");
}

#[test]
fn a_group_with_zero_workspaces_shows_nothing() {
    let (harness, link) = started();
    commit(&link, "DP-1", &[]);
    assert!(harness.view_on(Some("DP-1")).is_empty());
}

#[test]
fn a_done_reports_changed_once_then_quiet() {
    let (mut harness, link) = started();
    assert_eq!(harness.dispatch(), Update::Unchanged);
    commit(&link, "DP-1", &[(1, 1, true)]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.dispatch(), Update::Unchanged);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "1");
}

#[test]
fn a_batch_without_done_never_redraws() {
    let (mut harness, link) = started();
    commit(&link, "DP-1", &[(1, 1, true)]);
    assert_eq!(harness.dispatch(), Update::Changed);
    // A whole batch staged with no `done`: the view stays, quietly.
    {
        let mut shared = link.0.borrow_mut();
        let group = &mut shared.groups[0];
        group.staged[group.staged_len] = Ws {
            number: 2,
            coord: 2,
            ..Ws::default()
        };
        group.staged_len += 1;
    }
    assert_eq!(harness.dispatch(), Update::Unchanged);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "1");
}

#[test]
fn a_removed_workspace_is_not_redrawn_nor_clicked_before_its_done() {
    let (mut harness, link) = started();
    commit(&link, "DP-1", &[(1, 1, false), (2, 2, true)]);
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "1 2");
    // `Removed` for workspace 1, with no `done` after it: the staged
    // half of what `on_workspace_removed` does, hand-rolled (the event
    // method takes a proxy, which no compositor-free test can fabricate;
    // the integration suite drives the real path, and its click test
    // passes through the guarded send on every run).
    {
        let mut shared = link.0.borrow_mut();
        let group = &mut shared.groups[0];
        group.staged.swap(0, group.staged_len - 1);
        group.staged_len -= 1;
    }
    // The divergence the click guard reads: committed still shows both,
    // staged no longer holds workspace 1.
    {
        let shared = link.0.borrow();
        let group = &shared.groups[0];
        assert_eq!(group.committed_len, 2);
        assert_eq!(group.staged_len, 1);
        assert!(group.committed[..2].iter().any(|ws| ws.number == 1));
        assert!(group.staged[..1].iter().all(|ws| ws.number != 1));
    }
    // No redraw: the batch is not whole, and the drawn view still shows
    // both workspaces.
    assert_eq!(harness.dispatch(), Update::Unchanged);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "1 2");
    // A press on the removed pill sends nothing. (Without a manager no
    // request could go out in any case; what this pins alongside the
    // above is that the press path stays quiet while the batch is open.)
    let font = text();
    let view = harness.view_on(Some("DP-1"));
    let (start, end) = item_span(&font, "1 2", EM, i64::from(PAD), 0).unwrap();
    let ctx = ClickCtx {
        output: DP1,
        x: (start + end) / 2,
        view: &view,
        text: &font,
        em: EM,
        padding: PAD,
    };
    assert_eq!(harness.click(&ctx), Update::Unchanged);
}

#[test]
fn finished_mid_batch_is_never_half_drawn() {
    let (mut harness, link) = started();
    commit(&link, "DP-1", &[(1, 1, true)]);
    assert_eq!(harness.dispatch(), Update::Changed);
    // Half a batch staged, then the manager goes: the staged half is
    // dropped, nothing reports, the last commit still shows.
    {
        let mut shared = link.0.borrow_mut();
        {
            let group = &mut shared.groups[0];
            group.staged[group.staged_len] = Ws {
                number: 2,
                coord: 2,
                ..Ws::default()
            };
            group.staged_len += 1;
            assert_eq!(group.committed_len, 1);
        }
        shared.on_finished();
        assert_eq!(shared.groups[0].staged_len, 1);
    }
    assert_eq!(harness.dispatch(), Update::Unchanged);
    assert_eq!(harness.view_on(Some("DP-1")).text(), "1");
    assert!(!link.0.borrow().live);
}

#[test]
fn a_flood_of_batches_is_one_redraw() {
    let (mut harness, link) = started();
    for round in 0..50 {
        commit(&link, "DP-1", &[(1 + (round % 2) as u32, 1, true)]);
    }
    // Fifty `done`s since the last turn: one report, then quiet. The loop
    // draws once a turn whatever arrived, so a flood costs one frame.
    assert_eq!(harness.dispatch(), Update::Changed);
    assert_eq!(harness.dispatch(), Update::Unchanged);
}

#[test]
fn hostile_counts_and_names_stay_within_the_view_bound() {
    let (harness, link) = started();
    let big: Vec<(u32, u32, bool)> = (0..MAX_WORKSPACES as u32)
        .map(|i| (u32::MAX - i, i + 1, i == 0))
        .collect();
    commit(&link, "DP-1", &big);
    let view = harness.view_on(Some("DP-1"));
    assert!(view.text().len() <= MAX_TEXT);
    assert!(view.was_cut());
    assert!(!view.text().chars().any(char::is_control));
}

/// The item spans tile the text with no gaps and no overlap, and the hit
/// test finds each item by its middle and nothing past the end.
#[test]
fn pills_tile_the_text_and_the_hit_test_finds_each() {
    let font = text();
    let full = "1 2 10";
    let x0 = i64::from(PAD);
    let mut end = 0;
    for (item, number) in ["1", "2", "10"].iter().enumerate() {
        let (start, stop) = item_span(&font, full, EM, x0, item).unwrap();
        assert!(start >= end, "item {item} overlaps the last");
        assert_eq!(&full[span_of(full, item)], *number);
        let middle = (start + stop) / 2;
        assert_eq!(hit_index(&font, full, EM, PAD, 3, middle), Some(item));
        end = stop;
    }
    assert!(item_span(&font, full, EM, x0, 3).is_none());
    // Past the end: nothing.
    assert_eq!(hit_index(&font, full, EM, PAD, 3, 10_000), None);
}

/// The byte range of the `n`th item of a space-separated view text.
fn span_of(full: &str, want: usize) -> std::ops::Range<usize> {
    let mut start = 0;
    for (index, part) in full.split(' ').enumerate() {
        if index == want {
            return start..start + part.len();
        }
        start += part.len() + 1;
    }
    panic!("no item {want} in {full:?}");
}

#[test]
fn a_click_without_a_manager_sends_nothing() {
    let (mut harness, link) = started();
    commit(&link, "DP-1", &[(1, 1, false), (2, 2, true)]);
    let font = text();
    let view = harness.view_on(Some("DP-1"));
    assert_eq!(view.text(), "1 2");
    // No manager is bound in a unit test, so no request can go out
    // whatever is clicked: the inactive pill, the active one (already
    // there), a gap, or nowhere near the text.
    let (first, first_end) = item_span(&font, "1 2", EM, i64::from(PAD), 0).unwrap();
    let (second, second_end) = item_span(&font, "1 2", EM, i64::from(PAD), 1).unwrap();
    for x in [
        0,
        (first + first_end) / 2,
        (first_end + second) / 2,
        (second + second_end) / 2,
        second_end + 100,
    ] {
        let ctx = ClickCtx {
            output: DP1,
            x,
            view: &view,
            text: &font,
            em: EM,
            padding: PAD,
        };
        assert_eq!(harness.click(&ctx), Update::Unchanged, "x {x}");
    }
}

/// The pill draws through the module as the render path calls it: an
/// accent fill behind the active item only, its digits in the background
/// color, the inactive digit on plain background.
#[test]
fn custom_draw_marks_only_the_active_item() {
    let (harness, link) = started();
    commit(&link, "DP-1", &[(1, 1, false), (2, 2, true)]);
    let view = harness.view_on(Some("DP-1"));
    let theme = Theme::default();
    let mut font = text();
    let span = Span { x: 0, width: 400 };
    let baseline = font.metrics(EM).baseline(60);
    let mut pixels = vec![0u8; 400 * 60 * 4];
    let mut canvas = Canvas::new(&mut pixels, 400, 60).unwrap();
    canvas.fill_span(span, theme.background);
    let module = super::Workspaces {
        link: link.clone(),
        pill: super::Pill::default(),
        seen: 0,
    };
    let mut custom = CustomDraw {
        output: DP1,
        view: &view,
        canvas: &mut canvas,
        text: &mut font,
        span,
        em: EM,
        baseline,
        padding: PAD,
        scale: Scale::Integer(1),
        theme: &theme,
    };
    assert!(module.custom_draw(&mut custom));
    let at = |x: u32, y: u32| -> [u8; 3] {
        let i = ((y * 400 + x) * 4) as usize;
        [pixels[i + 2], pixels[i + 1], pixels[i]]
    };
    let accent = [theme.accent.r, theme.accent.g, theme.accent.b];
    let bg = [theme.background.r, theme.background.g, theme.background.b];
    // The active item's span, from the same walk the module uses.
    let font = text();
    let (start, end) = item_span(&font, "1 2", EM, i64::from(PAD), 1).unwrap();
    // The fill is behind the number, full height: accent just outside the
    // digits on both sides.
    assert_eq!(at(start - 2, 30), accent);
    assert_eq!(at(end + 1, 30), accent);
    // The digit itself is drawn in the background color on the pill...
    let middle = (start + end) / 2;
    assert!(
        (0..60).any(|y| at(middle, y) == bg),
        "no background-colored digit on the pill"
    );
    // ...and the inactive digit sits on plain background.
    let (first, _) = item_span(&font, "1 2", EM, i64::from(PAD), 0).unwrap();
    assert_eq!(at(first.saturating_sub(2), 30), bg);
}

#[test]
fn without_a_group_custom_draw_keeps_the_plain_draw() {
    let (harness, _) = started();
    let view = harness.view_on(Some("DP-1"));
    let theme = Theme::default();
    let mut font = text();
    let span = Span { x: 0, width: 400 };
    let baseline = font.metrics(EM).baseline(60);
    let mut pixels = vec![0u8; 400 * 60 * 4];
    let mut canvas = Canvas::new(&mut pixels, 400, 60).unwrap();
    let module = super::Workspaces {
        link: super::Link::default(),
        pill: super::Pill::default(),
        seen: 0,
    };
    let mut custom = CustomDraw {
        output: DP1,
        view: &view,
        canvas: &mut canvas,
        text: &mut font,
        span,
        em: EM,
        baseline,
        padding: PAD,
        scale: Scale::Integer(1),
        theme: &theme,
    };
    assert!(!module.custom_draw(&mut custom));
}

/// The pill drawn as the render path calls it with `pill`'s shape, for
/// workspace 2 of 2 active: the pixels of a 400x60 bar, and the item's
/// span.
fn pill_drawn(pill: super::Pill) -> (Vec<u8>, u32, u32) {
    let (harness, link) = started();
    commit(&link, "DP-1", &[(1, 1, false), (2, 2, true)]);
    let view = harness.view_on(Some("DP-1"));
    let theme = Theme::default();
    let mut font = text();
    let span = Span { x: 0, width: 400 };
    let baseline = font.metrics(EM).baseline(60);
    let mut pixels = vec![0u8; 400 * 60 * 4];
    let mut canvas = Canvas::new(&mut pixels, 400, 60).unwrap();
    canvas.fill_span(span, theme.background);
    let module = super::Workspaces {
        link,
        pill,
        seen: 0,
    };
    let mut custom = CustomDraw {
        output: DP1,
        view: &view,
        canvas: &mut canvas,
        text: &mut font,
        span,
        em: EM,
        baseline,
        padding: PAD,
        scale: Scale::Integer(1),
        theme: &theme,
    };
    assert!(module.custom_draw(&mut custom));
    let (start, end) = item_span(&text(), "1 2", EM, i64::from(PAD), 1).unwrap();
    (pixels, start, end)
}

fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * 400 + x) * 4) as usize;
    [pixels[i + 2], pixels[i + 1], pixels[i]]
}

#[test]
fn a_rounded_pill_cuts_its_corners_and_a_square_one_does_not() {
    let theme = Theme::default();
    let accent = [theme.accent.r, theme.accent.g, theme.accent.b];
    let bg = [theme.background.r, theme.background.g, theme.background.b];
    // The pill spans start - 4 .. end + 4 (half the padding), full height.
    let (square, start, end) = pill_drawn(super::Pill::default());
    let (left, right) = (start - PAD / 2, end + PAD / 2 - 1);
    for (x, y) in [(left, 0), (right, 0), (left, 59), (right, 59)] {
        assert_eq!(at(&square, x, y), accent, "square corner ({x},{y})");
    }
    // Radius 4 is a quarter circle of 4 pixels in each corner: the corner
    // pixel is the bar's, the edge midpoints are still the pill's.
    let (round, ..) = pill_drawn(super::Pill {
        radius: 4,
        inset: 0,
    });
    for (x, y) in [(left, 0), (right, 0), (left, 59), (right, 59)] {
        assert_eq!(at(&round, x, y), bg, "round corner ({x},{y})");
    }
    for (x, y) in [(left, 30), (right, 30), ((left + right) / 2, 0)] {
        assert_eq!(at(&round, x, y), accent, "edge ({x},{y})");
    }
    // Beyond half the pill's short side, the radius is cut back, not
    // wrapped: a pill of a huge radius still draws and keeps its middle.
    let (huge, ..) = pill_drawn(super::Pill {
        radius: u32::MAX,
        inset: 0,
    });
    assert_eq!(at(&huge, (left + right) / 2, 30), accent);
}

#[test]
fn an_inset_lifts_the_pill_off_the_bar_and_never_past_the_text() {
    let theme = Theme::default();
    let accent = [theme.accent.r, theme.accent.g, theme.accent.b];
    let bg = [theme.background.r, theme.background.g, theme.background.b];
    let (pixels, start, end) = pill_drawn(super::Pill {
        radius: 0,
        inset: 3,
    });
    let x = start - 2;
    assert_eq!(at(&pixels, x, 2), bg, "above the pill");
    assert_eq!(at(&pixels, x, 3), accent, "the pill's first row");
    assert_eq!(at(&pixels, x, 56), accent, "the pill's last row");
    assert_eq!(at(&pixels, x, 57), bg, "below the pill");
    assert_eq!(at(&pixels, end + 1, 30), accent);
    // An inset past what the line leaves is cut back so the number stays
    // on the pill: the pill still covers the digit's rows.
    let (pixels, start, _) = pill_drawn(super::Pill {
        radius: 0,
        inset: 1000,
    });
    let x = start - 2;
    let rows: Vec<u32> = (0..60).filter(|&y| at(&pixels, x, y) == accent).collect();
    assert!(!rows.is_empty(), "the pill vanished");
    let line = {
        let m = text().metrics(EM);
        (m.ascent - m.descent).ceil() as u32
    };
    assert!(
        rows.len() as u32 >= line,
        "{} rows for a {line}-row line",
        rows.len()
    );
}
