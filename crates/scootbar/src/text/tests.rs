//! Text through the seven-segment test font: measured exactly, drawn crisp
//! enough to read back, and cached within bounds.

use ab_glyph::FontVec;

use super::{MAX_ARENA, MAX_GLYPHS, Text};
use crate::color::Color;
use crate::paint::{Canvas, Span};
use crate::testfont;

const FG: Color = Color {
    r: 0xff,
    g: 0xff,
    b: 0xff,
};

fn text() -> Text {
    Text::new(ab_glyph::FontArc::new(
        FontVec::try_from_vec(testfont::build()).unwrap(),
    ))
}

#[test]
fn the_test_font_measures_exactly() {
    let text = text();
    // 600 units wide each, 1000 to the em: 30 pixels at a 50-pixel em.
    assert_eq!(text.measure(None, "3:07", 50.0), 120);
    assert_eq!(text.measure(None, "3:07 pm", 50.0), 210);
    // An icon and the space after it.
    assert_eq!(text.measure(Some('8'), "1", 50.0), 90);
    assert_eq!(text.measure(Some('8'), "", 50.0), 30);
    assert_eq!(text.measure(None, "", 50.0), 0);
    // Control characters take no space; unknown ones take `.notdef`'s.
    assert_eq!(text.measure(None, "1\n\t2\u{7}", 50.0), 60);
    assert_eq!(text.measure(None, "Ω", 50.0), 30);
    // Fractional sizes round up.
    assert_eq!(text.measure(None, "1", 21.0), 13);
    let metrics = text.metrics(50.0);
    assert!((metrics.ascent - 40.0).abs() < 1e-3, "{metrics:?}");
    assert!((metrics.descent + 10.0).abs() < 1e-3, "{metrics:?}");
    // A 50-pixel line centered in 60 rows: 5 above, baseline at 45.
    assert_eq!(metrics.baseline(60), 45);
    // Taller than the bar: centered all the same (15 rows above the top),
    // and clipped when drawn.
    assert_eq!(metrics.baseline(20), 25);
}

fn draw(text: &mut Text, s: &str, em: f32, width: u32, height: u32) -> Vec<u8> {
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, width, height).unwrap();
    let baseline = text.metrics(em).baseline(height);
    text.draw(
        &mut canvas,
        None,
        s,
        em,
        10,
        baseline,
        FG,
        Span { x: 0, width },
    );
    pixels
}

fn decode(pixels: &[u8], width: u32, height: u32, baseline: i64, em: f64) -> String {
    let ink = |x: i64, y: i64| -> bool {
        if x < 0 || y < 0 || x >= i64::from(width) || y >= i64::from(height) {
            return false;
        }
        pixels[((y as u32 * width + x as u32) * 4) as usize] > 128
    };
    let columns: Vec<i64> = (0..i64::from(width))
        .filter(|&x| (0..i64::from(height)).any(|y| ink(x, y)))
        .collect();
    testfont::decode(ink, columns[0], *columns.last().unwrap(), baseline, em)
}

/// The paint test of digits: every digit, the colon and the letters of
/// `am`/`pm` drawn and read back from the pixels.
#[test]
fn digits_drawn_read_back() {
    let mut text = text();
    for s in ["0123456789", "3:07 pm", "12:59 am", "8", "1:11", "?Ω"] {
        let pixels = draw(&mut text, s, 50.0, 400, 60);
        let expected: String = s
            .chars()
            .filter(|c| *c != ' ')
            .map(|c| if "0123456789:apm".contains(c) { c } else { '?' })
            .collect();
        assert_eq!(decode(&pixels, 400, 60, 45, 50.0), expected, "{s}");
    }
}

/// At a fractional scale's em (1.5 × 14 = 21 pixels) the edges fall
/// between pixels, so the coverage is gray there, and the text still reads.
#[test]
fn a_fractional_size_is_antialiased_and_still_legible() {
    let mut text = text();
    let em = 21.0 * 2.0;
    let pixels = draw(&mut text, "3:07", em, 300, 50);
    let grays = pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 0 && p[0] < 255)
        .count();
    assert!(grays > 0, "no antialiasing at a fractional size");
    let baseline = text.metrics(em).baseline(50);
    assert_eq!(decode(&pixels, 300, 50, baseline, f64::from(em)), "3:07");
}

#[test]
fn drawing_is_clipped_to_its_span() {
    let mut text = text();
    let mut pixels = vec![0u8; 200 * 60 * 4];
    let mut canvas = Canvas::new(&mut pixels, 200, 60).unwrap();
    // Clip to 40..70: only what falls there is drawn.
    text.draw(
        &mut canvas,
        None,
        "8888",
        50.0,
        10,
        45,
        FG,
        Span { x: 40, width: 30 },
    );
    for (i, p) in pixels.chunks_exact(4).enumerate() {
        let x = i as u32 % 200;
        if !(40..70).contains(&x) {
            assert_eq!(p[0], 0, "ink at column {x}");
        }
    }
    // Off the canvas entirely: nothing, and no panic.
    let mut canvas = Canvas::new(&mut pixels, 200, 60).unwrap();
    text.draw(
        &mut canvas,
        None,
        "8",
        50.0,
        -1_000_000,
        i64::MAX / 2,
        FG,
        Span {
            x: 0,
            width: u32::MAX,
        },
    );
}

#[test]
fn the_cache_fills_lazily_and_stays_bounded() {
    let mut text = text();
    assert_eq!(text.cached(), (0, 0));
    draw(&mut text, "3:07", 50.0, 300, 60);
    // Four glyphs cached (3, the colon, 0 and 7), none for anything not
    // drawn.
    let (glyphs, bytes) = text.cached();
    assert_eq!(glyphs, 4);
    assert!(bytes > 0);
    // Drawn again: nothing new.
    draw(&mut text, "3:07", 50.0, 300, 60);
    assert_eq!(text.cached(), (glyphs, bytes));
    // Every size a new glyph: the cache drops and refills, never past its
    // bounds.
    for size in 0..(MAX_GLYPHS * 2) {
        let em = 8.0 + size as f32 * 0.25;
        draw(&mut text, "8", em, 300, 60);
        let (glyphs, bytes) = text.cached();
        assert!(glyphs <= MAX_GLYPHS && bytes <= MAX_ARENA);
    }
    // A glyph too big to cache is drawn all the same.
    let before = text.cached();
    let pixels = draw(&mut text, "8", 1000.0, 700, 1100);
    assert_eq!(text.cached(), before);
    assert!(pixels.chunks_exact(4).any(|p| p[0] == 255));
}
