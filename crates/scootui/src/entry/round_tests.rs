//! The focus ring and the inverted match chip follow
//! [`Geometry::radius`]: square at 0 (the dmenu look), rounded to the
//! highlight's radius otherwise, and never past the row's text clip.

use ab_glyph::{FontArc, FontVec};

use super::line::Entry;
use super::list::List;
use super::paint::{Geometry, paint_entry, paint_list_over};
use crate::color::Color;
use crate::paint::{Canvas, Span};
use crate::region::Rect;
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

const EM: f32 = 20.0;
const ROW_H: u32 = 28;
const WIDTH: u32 = 400;
const WASH: Color = Color {
    r: 255,
    g: 0,
    b: 255,
};

fn text() -> Text {
    Text::new(FontArc::new(
        FontVec::try_from_vec(testfont::build()).unwrap(),
    ))
}

fn geo(radius: u32) -> Geometry {
    Geometry {
        x: 8,
        y: 8,
        width: WIDTH - 16,
        row_height: ROW_H,
        em: EM,
        radius,
    }
}

fn rgb(color: Color) -> [u8; 3] {
    [color.r, color.g, color.b]
}

fn washed(pixels: &mut [u8], height: u32) -> Canvas<'_> {
    let mut canvas = Canvas::new(pixels, WIDTH, height).unwrap();
    canvas.fill_rect(Span { x: 0, width: WIDTH }, 0, height, WASH);
    canvas
}

#[test]
fn the_focus_ring_is_square_at_radius_zero_and_rounded_above() {
    let theme = Theme::default();
    let height = ROW_H + 16;
    let mut text = text();
    let entry = Entry::new(64);
    for radius in [0, 6, ROW_H / 2] {
        let geo = geo(radius);
        let mut pixels = vec![0u8; (WIDTH * height) as usize * 4];
        let mut canvas = washed(&mut pixels, height);
        paint_entry(&mut canvas, &mut text, &theme, &entry, &geo, true, "");
        let corners = [
            (geo.x, geo.y),
            (geo.x + geo.width - 1, geo.y),
            (geo.x, geo.y + ROW_H - 1),
            (geo.x + geo.width - 1, geo.y + ROW_H - 1),
        ];
        for (x, y) in corners {
            let ring = canvas.at(x, y) == rgb(theme.accent);
            assert_eq!(ring, radius == 0, "radius {radius}: corner ({x}, {y})");
        }
        // The straight sides are the 2 px accent bars either way.
        let mid = geo.x + geo.width / 2;
        for dy in [0, 1, ROW_H - 2, ROW_H - 1] {
            assert_eq!(
                canvas.at(mid, geo.y + dy),
                rgb(theme.accent),
                "radius {radius}"
            );
        }
        assert_eq!(canvas.at(mid, geo.y + 2), rgb(WASH), "radius {radius}");
        assert_eq!(canvas.at(geo.x + 1, geo.y + ROW_H / 2), rgb(theme.accent));
        assert_eq!(canvas.at(geo.x + 4, geo.y + ROW_H / 2), rgb(WASH));
    }
}

/// A row of `em` text `row_h` tall, for the sizes a 1x, 2x and 3x panel
/// draws.
#[derive(Clone, Copy)]
struct Size {
    em: f32,
    row_h: u32,
}

const SIZES: [Size; 3] = [
    Size {
        em: 14.0,
        row_h: 23,
    },
    Size {
        em: 28.0,
        row_h: 46,
    },
    Size {
        em: 42.0,
        row_h: 69,
    },
];

/// One selected row with `spans` matched, drawn over a pill of
/// `pill_radius` the way the launcher draws it, the chip at `chip_radius`;
/// returns the canvas bytes and the chip's top-left corner (the span
/// `(2, 6)`'s box, when that is the span).
fn selected_row(
    size: Size,
    pill_radius: u32,
    chip_radius: u32,
    spans: &[(u32, u32)],
    label: &str,
) -> Vec<u8> {
    let theme = Theme::default();
    let mut text = text();
    let mut pixels = vec![0u8; (WIDTH * (size.row_h + 16)) as usize * 4];
    let mut canvas = washed(&mut pixels, size.row_h + 16);
    let geo = Geometry {
        row_height: size.row_h,
        em: size.em,
        radius: chip_radius,
        ..geo(0)
    };
    let pill = Rect {
        x: geo.x,
        y: geo.y,
        width: geo.width,
        height: size.row_h,
    };
    canvas.fill_rounded_gradient(pill, pill_radius, theme.accent, theme.accent, 255);
    let mut list = List::new(1);
    list.set_count(1);
    list.select(0);
    list.set_highlights(&[(0, spans)]);
    paint_list_over(&mut canvas, &mut text, &theme, &list, &geo, &|_| label);
    pixels
}

fn at(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [pixels[i + 2], pixels[i + 1], pixels[i]]
}

#[test]
fn the_inverted_chip_rounds_its_corners_to_the_highlights_radius() {
    let theme = Theme::default();
    let size = Size {
        em: EM,
        row_h: ROW_H,
    };
    let geo = geo(10);
    let chip_x = geo.text_x() + text().measure(None, "ww", EM);
    let chip_w = text().measure(None, "wwww", EM);
    let square = selected_row(size, 10, 0, &[(2, 6)], "wwwwwwwwww");
    let round = selected_row(size, 10, 10, &[(2, 6)], "wwwwwwwwww");
    // Square: the chip's top-left pixel is the text ink (`background`).
    assert_eq!(at(&square, chip_x, geo.y), rgb(theme.background));
    // Rounded: that corner is the highlight's fill again, the chip's
    // straight top is still the ink, and so is its middle.
    assert_eq!(at(&round, chip_x, geo.y), rgb(theme.accent));
    assert_eq!(
        at(&round, chip_x + chip_w - 1, geo.y + ROW_H - 1),
        rgb(theme.accent)
    );
    assert_eq!(
        at(&round, chip_x + chip_w / 2, geo.y),
        rgb(theme.background)
    );
    assert_eq!(
        at(&round, chip_x + chip_w / 2, geo.y + ROW_H - 1),
        rgb(theme.background)
    );
}

#[test]
fn a_chip_past_the_text_clip_is_cut_at_it() {
    let geo = geo(10);
    let clip_end = geo.clip().end();
    // 60 characters in a 384 px row: the span outruns the clip.
    let label = "w".repeat(60);
    let theme = Theme::default();
    let size = Size {
        em: EM,
        row_h: ROW_H,
    };
    for radius in [0, 10] {
        let pixels = selected_row(size, 10, radius, &[(0, 60)], &label);
        for y in geo.y..geo.y + ROW_H {
            for x in clip_end..geo.x + geo.width {
                assert_ne!(
                    at(&pixels, x, y),
                    rgb(theme.background),
                    "radius {radius}: chip ink past the clip at ({x}, {y})"
                );
            }
        }
    }
}

/// A chip pixel counts as chip, for a glyph over it, from this coverage:
/// the corner circle's antialiased rim shaves a percent or two off the
/// pixel a glyph's edge shares with it, which no eye reads as loss.
const MOSTLY: u8 = 192;

/// A shape pixel at least this covered is on the shape: the select-all
/// fill's own rim touches the ring's antialiased rim only at its faint
/// edge (measured: an 18% blend on a 70% rim pixel).
const HALF: u8 = 128;

/// The matched glyphs draw in `accent` over the chip, so a glyph pixel in
/// the chip's cut corner would vanish into the highlight. Every pixel the
/// square chip's glyphs ink is (nearly) whole chip when rounded, at the sizes and
/// radii a 1x to 3x panel draws (up to the pill: half a row).
#[test]
fn a_rounded_chip_loses_no_glyph_pixel_to_its_corners() {
    let theme = Theme::default();
    for size in SIZES {
        let geo = Geometry {
            row_height: size.row_h,
            em: size.em,
            ..geo(0)
        };
        let text = text();
        let chip_x = geo.text_x() + text.measure(None, "ww", size.em);
        let chip_w = text.measure(None, "wwww", size.em);
        for radius in [size.row_h / 4, size.row_h / 2] {
            let square = selected_row(size, radius, 0, &[(2, 6)], "wwwwwwwwww");
            // The chip alone: its coverage, on nothing.
            let mut mask = vec![0u8; (WIDTH * (size.row_h + 16)) as usize * 4];
            let mut canvas = Canvas::new(&mut mask, WIDTH, size.row_h + 16).unwrap();
            let chip = Rect {
                x: chip_x,
                y: geo.y,
                width: chip_w,
                height: size.row_h,
            };
            let chip_radius =
                super::paint::text_box_radius(&Geometry { radius, ..geo }, text.metrics(size.em));
            canvas.fill_rounded_gradient(
                chip,
                chip_radius,
                theme.background,
                theme.background,
                255,
            );
            for y in geo.y..geo.y + size.row_h {
                for x in chip_x..chip_x + chip_w {
                    let inked = at(&square, x, y) != rgb(theme.background);
                    let covered = mask[((y * WIDTH + x) * 4 + 3) as usize];
                    assert!(
                        !inked || covered >= MOSTLY,
                        "em {} radius {radius}: a glyph pixel at ({x}, {y}) falls in the cut corner ({covered}/255)",
                        size.em
                    );
                }
            }
        }
    }
}

#[test]
fn select_all_fills_the_rounded_ring_not_its_corners() {
    use super::line::InsertOutcome;
    let theme = Theme::default();
    let mut text = text();
    let mut entry = Entry::new(64);
    let word = "w".repeat(40);
    assert_eq!(entry.insert_text(&word), InsertOutcome::Inserted);
    entry.select_all();
    for radius in [0, 10, ROW_H / 2] {
        let geo = geo(radius);
        let mut pixels = vec![0u8; (WIDTH * (ROW_H + 16)) as usize * 4];
        let mut canvas = washed(&mut pixels, ROW_H + 16);
        paint_entry(&mut canvas, &mut text, &theme, &entry, &geo, true, "");
        // The fill is the accent, same as the ring, so the only thing
        // the corner shows is whether it was cut: the ring's corner
        // pixel is not accent when rounded, filled or not.
        for (x, y) in [
            (geo.x, geo.y),
            (geo.x + geo.width - 1, geo.y),
            (geo.x, geo.y + ROW_H - 1),
            (geo.x + geo.width - 1, geo.y + ROW_H - 1),
        ] {
            assert_eq!(
                canvas.at(x, y) == rgb(theme.accent),
                radius == 0,
                "radius {radius}"
            );
        }
        // The fill stops at the text clip: nothing accent between the
        // clip's end and the ring's right side.
        let clip_end = geo.clip().end();
        for y in geo.y + radius.max(2)..geo.y + ROW_H - radius.max(2) {
            for x in clip_end..geo.x + geo.width - 2 {
                assert_ne!(
                    canvas.at(x, y),
                    rgb(theme.accent),
                    "radius {radius}: ({x}, {y})"
                );
            }
        }
    }
}

/// The select-all fill stays inside the ring's rounded shape: nothing but
/// pixels the ring's own shape (nearly) covers changes when the selection is
/// added, at every size and up to the pill.
#[test]
fn the_select_all_fill_stays_inside_the_ring() {
    use super::line::InsertOutcome;
    let theme = Theme::default();
    let mut text = text();
    let word = "w".repeat(6);
    for size in SIZES {
        for radius in [size.row_h / 4, size.row_h / 2] {
            let geo = Geometry {
                row_height: size.row_h,
                em: size.em,
                radius,
                ..geo(0)
            };
            let height = size.row_h + 16;
            let paint = |text: &mut Text, selected: bool| {
                let mut entry = Entry::new(64);
                assert_eq!(entry.insert_text(&word), InsertOutcome::Inserted);
                if selected {
                    entry.select_all();
                } else {
                    entry.move_end();
                }
                let mut pixels = vec![0u8; (WIDTH * height) as usize * 4];
                let mut canvas = washed(&mut pixels, height);
                paint_entry(&mut canvas, text, &theme, &entry, &geo, false, "");
                pixels
            };
            let (plain, selected) = (paint(&mut text, false), paint(&mut text, true));
            let mut mask = vec![0u8; (WIDTH * height) as usize * 4];
            let mut canvas = Canvas::new(&mut mask, WIDTH, height).unwrap();
            let field = Rect {
                x: geo.x,
                y: geo.y,
                width: geo.width,
                height: size.row_h,
            };
            canvas.fill_rounded_gradient(field, radius, theme.accent, theme.accent, 255);
            let mut moved = 0;
            for y in 0..height {
                for x in 0..WIDTH {
                    let i = ((y * WIDTH + x) * 4) as usize;
                    let changed = plain[i..i + 4] != selected[i..i + 4];
                    assert!(
                        !changed || mask[i + 3] >= HALF,
                        "em {} radius {radius}: ({x}, {y}) changed outside the shape",
                        size.em
                    );
                    moved += usize::from(changed);
                }
            }
            assert!(
                moved > 0,
                "em {} radius {radius}: no selection drawn",
                size.em
            );
        }
    }
}
