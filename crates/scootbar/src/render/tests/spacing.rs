//! What the corner clearance, a module's margin and the separators do to
//! the layout and the pixels.

use super::*;
use crate::render::Member;

fn dim() -> [u8; 3] {
    rgb(Theme::default().dim)
}

fn background() -> [u8; 3] {
    rgb(Theme::default().background)
}

/// A bar of `modules` with `margins` (logical, per module) for its scene,
/// every gap holding a separator (as a section list with no mark does).
fn bar_with(modules: &[(Section, &str)], margins: &[u32]) -> Bar {
    let mut bar = Bar::new(modules);
    let members: Vec<Member> = modules
        .iter()
        .zip(margins)
        .enumerate()
        .map(|(module, (&(section, _), &margin))| Member {
            hover: false,
            module,
            section,
            margin,
            separator_before: module > 0 && modules[module - 1].0 == section,
        })
        .collect();
    bar.scene = Scene::with_members(&members);
    bar
}

#[test]
fn a_rounded_bar_keeps_its_first_and_last_module_out_of_the_corners() {
    let mut bar = Bar::new(&[(Section::Left, "1"), (Section::Right, "2")]);
    // Padding 10 a side: 20 in `update`, a quarter of it 5 off the radius.
    bar.style.radius = 20;
    bar.update();
    assert_eq!(
        bar.scene.spans(),
        [Span { x: 15, width: 50 }, Span { x: 535, width: 50 }]
    );
    // The first ink starts at the padding past that: clear of the corner
    // square's 20 columns; the pill, half a padding out, starts at the
    // radius exactly.
    let ink = bar.scene.spans()[0].x + 10;
    assert!(ink > 20);
    assert_eq!(bar.scene.spans()[0].x + 5, 20);
    // Square, nothing moves.
    bar.style.radius = 0;
    bar.scene = Scene::all(&[Section::Left, Section::Right]);
    bar.update();
    assert_eq!(
        bar.scene.spans(),
        [Span { x: 0, width: 50 }, Span { x: 550, width: 50 }]
    );
}

#[test]
fn a_radius_the_bar_cannot_hold_is_cut_back_before_it_clears_anything() {
    // Radius 500 in a 60-high bar draws as 30: the clearance is 30 - 5.
    let mut bar = Bar::new(&[(Section::Left, "1")]);
    bar.style.radius = 500;
    bar.update();
    assert_eq!(bar.scene.spans(), [Span { x: 25, width: 50 }]);
}

#[test]
fn a_margin_moves_its_module_and_its_neighbours() {
    let mut bar = bar_with(&[(Section::Left, "1"), (Section::Left, "2")], &[0, 7]);
    bar.style.spacing = 4;
    bar.update();
    // 0..50, then 4 of spacing and 7 of margin before the second, whose
    // 7 more after it has nothing to move.
    assert_eq!(
        bar.scene.spans(),
        [Span { x: 0, width: 50 }, Span { x: 61, width: 50 }]
    );
    // At scale 2 the margin doubles (14): 100 wide, 8 spacing, 14 margin.
    bar.scene.update(
        &bar.placed,
        &OutputView { name: None },
        Some(&bar.text),
        &bar.style,
        Scale::Integer(2),
        Size {
            width: WIDTH * 2,
            height: HEIGHT * 2,
        },
    );
    assert_eq!(bar.scene.spans()[1].x, 100 + 8 + 14);
}

/// Every pixel of column `x`, for whether it is a separator's.
fn column(pixels: &[u8], x: u32) -> Vec<[u8; 3]> {
    (0..HEIGHT).map(|y| pixel(pixels, x, y)).collect()
}

#[test]
fn a_separator_is_a_line_centered_in_the_gap_between_neighbours() {
    let mut bar = Bar::new(&[
        (Section::Left, "1"),
        (Section::Left, "2"),
        (Section::Right, "3"),
    ]);
    bar.style.spacing = 20;
    bar.style.separator = 4;
    let mut pixels = buffer();
    let mut record = Record::new(3);
    bar.paint(&mut pixels, &mut record);
    // Left: 0..50 and 70..120; the gap 50..70 has the line at 58..62, from
    // a quarter of the height (15) to three quarters (45).
    for x in 58..62 {
        for y in 0..HEIGHT {
            let want = if (15..45).contains(&y) {
                dim()
            } else {
                background()
            };
            assert_eq!(pixel(&pixels, x, y), want, "({x},{y})");
        }
    }
    for x in [50, 57, 62, 69] {
        assert!(!column(&pixels, x).contains(&dim()), "column {x}");
    }
    // One separator only: none between the sections, none at the edges.
    let lines = (0..WIDTH)
        .filter(|&x| column(&pixels, x).contains(&dim()))
        .count();
    assert_eq!(lines, 4);
}

#[test]
fn a_line_wider_than_its_gap_is_cut_to_it_and_never_touches_a_module() {
    let mut bar = Bar::new(&[(Section::Left, "1"), (Section::Left, "2")]);
    bar.style.spacing = 6;
    bar.style.separator = 30;
    let mut pixels = buffer();
    let mut record = Record::new(2);
    bar.paint(&mut pixels, &mut record);
    let lines: Vec<u32> = (0..WIDTH)
        .filter(|&x| column(&pixels, x).contains(&dim()))
        .collect();
    assert_eq!(lines, (50..56).collect::<Vec<_>>());
}

#[test]
fn no_separator_is_drawn_beside_an_empty_module_or_with_none_asked() {
    let mut bar = Bar::new(&[
        (Section::Left, "1"),
        (Section::Left, ""),
        (Section::Left, "2"),
    ]);
    bar.style.spacing = 10;
    bar.style.separator = 2;
    let mut pixels = buffer();
    let mut record = Record::new(3);
    bar.paint(&mut pixels, &mut record);
    // One gap between the two that show (50..60); the empty one adds none.
    let lines: Vec<u32> = (0..WIDTH)
        .filter(|&x| column(&pixels, x).contains(&dim()))
        .collect();
    assert_eq!(lines, [54, 55]);
    bar.style.separator = 0;
    let mut pixels = buffer();
    let mut record = Record::new(3);
    bar.paint(&mut pixels, &mut record);
    assert!(!(0..WIDTH).any(|x| column(&pixels, x).contains(&dim())));
}

#[test]
fn a_module_repaint_leaves_the_separator_alone() {
    let mut bar = Bar::new(&[(Section::Left, "1"), (Section::Left, "2")]);
    bar.style.spacing = 10;
    bar.style.separator = 2;
    let mut pixels = buffer();
    let mut record = Record::new(2);
    bar.paint(&mut pixels, &mut record);
    let before = pixels.clone();
    // The first module changes to a glyph of the same width: only its span
    // is repainted, and the line beside it stays.
    bar.set(0, "7", Class::Warn);
    bar.paint(&mut pixels, &mut record);
    for x in [54, 55] {
        assert_eq!(column(&pixels, x), column(&before, x), "column {x}");
    }
    assert_ne!(pixels, before, "the module did change");
}

#[test]
fn separators_draw_only_where_a_mark_stands() {
    // Three on the left, `["1", "2", "|", "3"]`: the first two are one
    // group (no line), the last gap holds one.
    let mut bar = bar_with(
        &[
            (Section::Left, "1"),
            (Section::Left, "2"),
            (Section::Left, "3"),
        ],
        &[0, 0, 0],
    );
    let members = [
        Member {
            hover: false,
            module: 0,
            section: Section::Left,
            margin: 0,
            separator_before: false,
        },
        Member {
            hover: false,
            module: 1,
            section: Section::Left,
            margin: 0,
            separator_before: false,
        },
        Member {
            hover: false,
            module: 2,
            section: Section::Left,
            margin: 0,
            separator_before: true,
        },
    ];
    bar.scene = Scene::with_members(&members);
    bar.style.spacing = 20;
    bar.style.separator = 4;
    let mut pixels = buffer();
    let mut record = Record::new(3);
    bar.paint(&mut pixels, &mut record);
    // 0..50, 70..120, 140..190: the first gap (50..70) is clear, the
    // second (120..140) holds the line at 128..132.
    let lines: Vec<u32> = (0..WIDTH)
        .filter(|&x| column(&pixels, x).contains(&dim()))
        .collect();
    assert_eq!(lines, [128, 129, 130, 131]);
}

#[test]
fn a_mark_beside_an_empty_module_marks_the_visible_gap() {
    // `["1", "|", "", "2"]`: the empty middle takes no space, so the line
    // the mark asks for goes between the two that show.
    let mut bar = bar_with(
        &[
            (Section::Left, "1"),
            (Section::Left, ""),
            (Section::Left, "2"),
        ],
        &[0, 0, 0],
    );
    let members = [
        Member {
            hover: false,
            module: 0,
            section: Section::Left,
            margin: 0,
            separator_before: false,
        },
        Member {
            hover: false,
            module: 1,
            section: Section::Left,
            margin: 0,
            separator_before: true,
        },
        Member {
            hover: false,
            module: 2,
            section: Section::Left,
            margin: 0,
            separator_before: false,
        },
    ];
    bar.scene = Scene::with_members(&members);
    bar.style.spacing = 10;
    bar.style.separator = 2;
    let mut pixels = buffer();
    let mut record = Record::new(3);
    bar.paint(&mut pixels, &mut record);
    let lines: Vec<u32> = (0..WIDTH)
        .filter(|&x| column(&pixels, x).contains(&dim()))
        .collect();
    assert_eq!(lines, [54, 55]);
    // The same bar with no mark anywhere near draws nothing.
    let members = [
        Member {
            hover: false,
            module: 0,
            section: Section::Left,
            margin: 0,
            separator_before: false,
        },
        Member {
            hover: false,
            module: 1,
            section: Section::Left,
            margin: 0,
            separator_before: false,
        },
        Member {
            hover: false,
            module: 2,
            section: Section::Left,
            margin: 0,
            separator_before: false,
        },
    ];
    bar.scene = Scene::with_members(&members);
    let mut pixels = buffer();
    let mut record = Record::new(3);
    bar.paint(&mut pixels, &mut record);
    assert!(!(0..WIDTH).any(|x| column(&pixels, x).contains(&dim())));
}
