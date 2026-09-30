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
    // A glyph too big to cache (800 × 1400 at a 2000-pixel em, past
    // 1 MiB) is drawn all the same.
    let before = text.cached();
    let pixels = draw(&mut text, "8", 2000.0, 1000, 2100);
    assert_eq!(text.cached(), before);
    assert!(pixels.chunks_exact(4).any(|p| p[0] == 255));
}

/// `--font-size 256` at scale 2 is a 512-pixel em: its glyphs are cached,
/// so a redraw rasterizes (and allocates) nothing.
#[test]
fn the_largest_font_size_at_scale_two_is_cached() {
    let mut text = text();
    draw(&mut text, "3:07", 512.0, 1300, 600);
    let filled = text.cached();
    assert_eq!(filled.0, 4, "every glyph cached");
    draw(&mut text, "3:07", 512.0, 1300, 600);
    assert_eq!(text.cached(), filled);
}

/// A hostile font whose glyph bounds would need a rasterizer buffer of
/// gigabytes: the glyph is skipped, quickly, and the rest still draws.
#[test]
fn a_glyph_too_big_to_rasterize_is_skipped() {
    let mut text = Text::new(ab_glyph::FontArc::new(
        FontVec::try_from_vec(testfont::build_hostile()).unwrap(),
    ));
    let start = std::time::Instant::now();
    let pixels = draw(&mut text, "8", 50.0, 300, 60);
    assert!(
        pixels.iter().all(|&b| b == 0),
        "the huge glyph drew something"
    );
    assert_eq!(text.cached(), (0, 0));
    // A glyph within the bounds (the same font's `1`, 312 × 1562) is
    // rasterized and cached as usual.
    draw(&mut text, "1", 50.0, 300, 60);
    assert_eq!(text.cached().0, 1);
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(super::raster_size(f32::INFINITY, 1.0), None);
    assert_eq!(super::raster_size(f32::NAN, 1.0), None);
    assert_eq!(super::raster_size(-1.0, 1.0), None);
    assert_eq!(super::raster_size(4096.0, 1024.0), Some((4096, 1024)));
    assert_eq!(super::raster_size(4097.0, 1.0), None);
    assert_eq!(super::raster_size(4096.0, 1025.0), None);
}

// ---- fallback fonts ----

/// A fallback with `chars`, drawn as bars (`testfont::BAR`).
fn symbols(chars: impl IntoIterator<Item = char>) -> ab_glyph::FontArc {
    ab_glyph::FontArc::new(FontVec::try_from_vec(testfont::build_symbols(chars)).unwrap())
}

fn chain(fallbacks: Vec<ab_glyph::FontArc>) -> Text {
    Text::with_fallbacks(
        ab_glyph::FontArc::new(FontVec::try_from_vec(testfont::build()).unwrap()),
        fallbacks,
    )
}

/// Whether the pixel at font units `(x, y)` in glyph cell `n` (a 50-pixel
/// em, the pen starting at 10) is ink.
fn ink_at(pixels: &[u8], width: u32, height: u32, n: u32, x: f64, y: f64) -> bool {
    let em = 50.0;
    let baseline = chain(vec![]).metrics(em).baseline(height);
    let unit = f64::from(em) / 1000.0;
    let origin = 10.0 + f64::from(n) * 600.0 * unit;
    let px = (origin + x * unit).floor() as u32;
    let py = (baseline as f64 - y * unit).floor() as u32;
    pixels[((py * width + px) * 4) as usize] > 128
}

/// `(a bare bar, .notdef)` at cell `n`: the bar's middle inked with
/// `.notdef`'s hole (150, 50) clear, and the hole inked.
fn probe(pixels: &[u8], width: u32, height: u32, n: u32) -> (bool, bool) {
    let hole = ink_at(pixels, width, height, n, 150.0, 50.0);
    (
        ink_at(pixels, width, height, n, 300.0, 350.0) && !hole,
        hole,
    )
}

#[test]
fn a_fallback_draws_only_what_the_primary_lacks() {
    // The fallback has a bar for `3` (which the primary has, so it must not
    // be used) and for `語` (which it lacks).
    let mut text = chain(vec![symbols(['3', '語'])]);
    let pixels = draw(&mut text, "3語?", 50.0, 300, 60);
    // `3` is the primary's: segment `a`, across the top, which no bar has.
    assert!(
        ink_at(&pixels, 300, 60, 0, 300.0, 650.0),
        "3 is the primary's"
    );
    assert_eq!(probe(&pixels, 300, 60, 1), (true, false), "語: a bar");
    // `?` is in neither font: the primary's `.notdef`, its hole inked.
    assert!(probe(&pixels, 300, 60, 2).1, "? is .notdef");
}

#[test]
fn fallbacks_are_tried_in_order_and_at_most_two_are_kept() {
    let mut text = chain(vec![symbols(['a']), symbols(['a', 'x']), symbols(['y'])]);
    // Only the first two are kept: `y` is in the third, so it is `.notdef`.
    let pixels = draw(&mut text, "xy", 50.0, 300, 60);
    assert_eq!(probe(&pixels, 300, 60, 0), (true, false), "x: second");
    assert!(probe(&pixels, 300, 60, 1).1, "y: .notdef");
    assert_eq!(text.measure(None, "xy", 50.0), 60);
}

/// A title of mixed Latin and CJK, against a chain that covers the CJK: it
/// draws every character and the widths add up, with no blank.
#[test]
fn a_title_of_latin_and_cjk_draws_with_no_blank() {
    let cjk: Vec<char> = "日本語のタイトル中文".chars().collect();
    let mut text = chain(vec![symbols(cjk.iter().copied())]);
    let title = "3:07 日本語のタイトル 中文 a";
    let width = text.measure(None, title, 50.0);
    // Every character is 30 pixels wide, whichever font it is from.
    assert_eq!(width, 30 * title.chars().count() as u32);
    let pixels = draw(&mut text, title, 50.0, width + 20, 60);
    // Every non-space character left ink in its own cell.
    for (n, c) in title.chars().enumerate() {
        if c == ' ' {
            continue;
        }
        let cell = 10 + n * 30;
        let any = (0..60u32).any(|y| {
            (cell..cell + 30).any(|x| pixels[((y * (width + 20) + x as u32) * 4) as usize] > 128)
        });
        assert!(any, "{c:?} at {n} drew nothing");
    }
}

/// With no fallback at all, the CJK title is boxes (`.notdef`), not blanks
/// and not a panic.
#[test]
fn without_a_fallback_a_missing_glyph_is_the_notdef_box() {
    let mut text = text();
    let pixels = draw(&mut text, "日本", 50.0, 100, 60);
    assert!(probe(&pixels, 100, 60, 0).1);
    assert!(probe(&pixels, 100, 60, 1).1);
    assert_eq!(text.cached().0, 1, "one .notdef glyph, shared");
}

/// The icon comes from the chain too: a private-use glyph only the symbol
/// font has, then a space, then the primary's text.
#[test]
fn an_icon_is_drawn_from_the_symbol_font() {
    let icon = '\u{f0e65}';
    let mut text = chain(vec![symbols([icon])]);
    assert_eq!(text.measure(Some(icon), "3", 50.0), 90);
    let (width, height) = (200, 60);
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, width, height).unwrap();
    let baseline = text.metrics(50.0).baseline(height);
    text.draw(
        &mut canvas,
        Some(icon),
        "3",
        50.0,
        10,
        baseline,
        FG,
        Span { x: 0, width },
    );
    assert_eq!(probe(&pixels, width, height, 0), (true, false));
    // The text after the icon and its space is the primary's `3`.
    assert!(ink_at(&pixels, width, height, 2, 300.0, 650.0));
}

/// A stream of arbitrary text (CJK in the fallback's range, any scalar
/// value, ASCII) at several sizes through a chain: the cache never passes
/// its bounds, and nothing panics.
#[test]
fn the_cache_stays_bounded_under_a_fuzzed_title_stream() {
    let mut text = chain(vec![symbols(
        (0x4e00..0x4e00 + 2000).filter_map(char::from_u32),
    )]);
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let (width, height) = (400, 60);
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut peak = 0;
    for round in 0..3000 {
        let title: String = (0..24)
            .map(|_| {
                let r = next();
                let code = match r % 3 {
                    0 => 0x4e00 + (r >> 8) as u32 % 2000,
                    1 => (r >> 8) as u32 % 0x11_0000,
                    _ => 0x20 + (r >> 8) as u32 % 95,
                };
                char::from_u32(code).unwrap_or('\u{fffd}')
            })
            .collect();
        // A few sizes, as scale changes across outputs would make.
        let em = [20.0, 30.0, 45.5, 50.0][round % 4];
        let mut canvas = Canvas::new(&mut pixels, width, height).unwrap();
        text.draw(
            &mut canvas,
            None,
            &title,
            em,
            4,
            40,
            FG,
            Span { x: 0, width },
        );
        let (glyphs, bytes) = text.cached();
        assert!(
            glyphs <= MAX_GLYPHS && bytes <= MAX_ARENA,
            "{glyphs} {bytes}"
        );
        peak = peak.max(glyphs);
    }
    assert!(peak > 100, "the stream did not exercise the cache: {peak}");
}
