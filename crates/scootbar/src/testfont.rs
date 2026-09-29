//! A tiny TrueType font built in code, for tests: seven-segment digits and
//! a few letters, every glyph a set of axis-aligned rectangles, so text
//! drawn with it can be read back from pixels ([`decode`]).
//!
//! Built rather than checked in: no font file (and no font licence) in the
//! repository, no font needed on the machine, and every pixel predictable.
//! The integration tests write it to a file and pass it as `--font`, then
//! read the time off a compositor's screenshot.
//!
//! The metrics make the arithmetic exact: 1000 units per em, ascent 800,
//! descent -200 (so `ab_glyph`'s scale equals the em), every glyph 600 wide.
//! At a 50-pixel em a unit is 0.05 pixels and every edge lands on a whole
//! pixel, so the shapes are crisp.
//!
//! Glyphs: `0`-`9`, `a` (drawn `A`), `p` (`P`), `m` (an arch: `∩`), `:`,
//! a space, and `.notdef` (a solid block, what any other character draws).
//! Standard library only: `tests/common` compiles this file by `#[path]`.

#![allow(dead_code)] // Each test binary uses a different part.

pub const UNITS_PER_EM: u16 = 1000;
pub const ASCENT: i16 = 800;
pub const DESCENT: i16 = -200;
pub const ADVANCE: u16 = 600;

/// A rectangle in font units: `(x0, y0, x1, y1)`, y up.
type Rect = (i16, i16, i16, i16);

/// The seven segments, `a` to `g` in the usual order.
const SEGMENTS: [Rect; 7] = [
    (200, 600, 400, 700), // a: top
    (400, 400, 500, 600), // b: upper right
    (400, 100, 500, 300), // c: lower right
    (200, 0, 400, 100),   // d: bottom
    (100, 100, 200, 300), // e: lower left
    (100, 400, 200, 600), // f: upper left
    (200, 300, 400, 400), // g: middle
];

/// Below the lower left segment, beside the bottom one: ink only in
/// `.notdef`.
const HOLE: (i16, i16) = (150, 50);

/// Each shape by its segments, as a bit mask (bit 0 is `a`).
const SHAPES: [(char, u8); 13] = [
    ('0', 0b0111111),
    ('1', 0b0000110),
    ('2', 0b1011011),
    ('3', 0b1001111),
    ('4', 0b1100110),
    ('5', 0b1101101),
    ('6', 0b1111101),
    ('7', 0b0000111),
    ('8', 0b1111111),
    ('9', 0b1101111),
    ('a', 0b1110111),
    ('p', 0b1110011),
    ('m', 0b0110111),
];

const COLON: [Rect; 2] = [(250, 150, 350, 250), (250, 450, 350, 550)];
const NOTDEF: Rect = (100, 0, 500, 700);

/// The font's glyphs in id order, with the character each maps from.
fn glyphs() -> Vec<(Option<char>, Vec<Rect>)> {
    let mut glyphs = vec![(None, vec![NOTDEF])];
    for (c, mask) in SHAPES {
        let rects = (0..7)
            .filter(|bit| mask >> bit & 1 == 1)
            .map(|bit| SEGMENTS[bit])
            .collect();
        glyphs.push((Some(c), rects));
    }
    glyphs.push((Some(':'), COLON.to_vec()));
    glyphs.push((Some(' '), Vec::new()));
    glyphs
}

fn u16be(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn i16be(out: &mut Vec<u8>, v: i16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn u32be(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// One simple glyph: a contour of four on-curve points per rectangle.
fn glyph(rects: &[Rect]) -> Vec<u8> {
    let mut out = Vec::new();
    if rects.is_empty() {
        return out;
    }
    let x_min = rects.iter().map(|r| r.0).min().unwrap_or(0);
    let y_min = rects.iter().map(|r| r.1).min().unwrap_or(0);
    let x_max = rects.iter().map(|r| r.2).max().unwrap_or(0);
    let y_max = rects.iter().map(|r| r.3).max().unwrap_or(0);
    i16be(&mut out, rects.len() as i16);
    for v in [x_min, y_min, x_max, y_max] {
        i16be(&mut out, v);
    }
    for i in 0..rects.len() {
        u16be(&mut out, (i * 4 + 3) as u16);
    }
    u16be(&mut out, 0); // no instructions
    let points: Vec<(i16, i16)> = rects
        .iter()
        .flat_map(|&(x0, y0, x1, y1)| [(x0, y0), (x0, y1), (x1, y1), (x1, y0)])
        .collect();
    out.extend(std::iter::repeat_n(0x01u8, points.len())); // on curve, long coordinates
    let (mut x, mut y) = (0i16, 0i16);
    for &(px, _) in &points {
        i16be(&mut out, px - x);
        x = px;
    }
    for &(_, py) in &points {
        i16be(&mut out, py - y);
        y = py;
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

/// The font file's bytes.
pub fn build() -> Vec<u8> {
    let glyphs = glyphs();
    let count = glyphs.len() as u16;

    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for (_, rects) in &glyphs {
        u32be(&mut loca, glyf.len() as u32);
        glyf.extend(glyph(rects));
    }
    u32be(&mut loca, glyf.len() as u32);

    let mut head = Vec::new();
    u16be(&mut head, 1);
    u16be(&mut head, 0);
    u32be(&mut head, 0x0001_0000); // revision
    u32be(&mut head, 0); // checksum adjustment
    u32be(&mut head, 0x5F0F_3CF5); // magic
    u16be(&mut head, 0x000B); // flags
    u16be(&mut head, UNITS_PER_EM);
    head.extend_from_slice(&[0; 16]); // created, modified
    for v in [0, DESCENT, ADVANCE as i16, ASCENT] {
        i16be(&mut head, v);
    }
    u16be(&mut head, 0); // mac style
    u16be(&mut head, 8); // lowest readable size
    i16be(&mut head, 2); // direction hint
    i16be(&mut head, 1); // long `loca`
    i16be(&mut head, 0); // glyph data format

    let mut hhea = Vec::new();
    u16be(&mut hhea, 1);
    u16be(&mut hhea, 0);
    i16be(&mut hhea, ASCENT);
    i16be(&mut hhea, DESCENT);
    i16be(&mut hhea, 0); // line gap
    u16be(&mut hhea, ADVANCE);
    i16be(&mut hhea, 100); // min left side bearing
    i16be(&mut hhea, 100); // min right side bearing
    i16be(&mut hhea, 500); // x max extent
    i16be(&mut hhea, 1); // caret rise
    hhea.extend_from_slice(&[0; 14]); // caret run, offset, reserved x4, format
    u16be(&mut hhea, count);

    let mut hmtx = Vec::new();
    for (_, rects) in &glyphs {
        u16be(&mut hmtx, ADVANCE);
        i16be(&mut hmtx, rects.iter().map(|r| r.0).min().unwrap_or(0));
    }

    let mut maxp = Vec::new();
    u32be(&mut maxp, 0x0001_0000);
    u16be(&mut maxp, count);
    u16be(&mut maxp, 28); // points
    u16be(&mut maxp, 7); // contours
    maxp.extend_from_slice(&[0; 4]);
    u16be(&mut maxp, 2); // zones
    maxp.extend_from_slice(&[0; 16]);

    let mut groups: Vec<(u32, u32)> = glyphs
        .iter()
        .enumerate()
        .filter_map(|(id, (c, _))| Some((u32::from((*c)?), id as u32)))
        .collect();
    groups.sort_unstable();
    let mut cmap = Vec::new();
    u16be(&mut cmap, 0);
    u16be(&mut cmap, 1);
    u16be(&mut cmap, 3); // Windows
    u16be(&mut cmap, 10); // full Unicode
    u32be(&mut cmap, 12);
    u16be(&mut cmap, 12); // format 12
    u16be(&mut cmap, 0);
    u32be(&mut cmap, 16 + 12 * groups.len() as u32);
    u32be(&mut cmap, 0);
    u32be(&mut cmap, groups.len() as u32);
    for (code, id) in groups {
        u32be(&mut cmap, code);
        u32be(&mut cmap, code);
        u32be(&mut cmap, id);
    }

    let tables: [(&[u8; 4], Vec<u8>); 7] = [
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
    ];
    let mut out = Vec::new();
    u32be(&mut out, 0x0001_0000);
    u16be(&mut out, tables.len() as u16);
    u16be(&mut out, 64); // search range
    u16be(&mut out, 2); // entry selector
    u16be(&mut out, 48); // range shift
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        u32be(&mut out, 0); // checksum: not checked by readers
        u32be(&mut out, offset as u32);
        u32be(&mut out, data.len() as u32);
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        offset = 12 + 16 * tables.len() + body.len();
    }
    out.extend(body);
    out
}

/// Reads text drawn in this font back from pixels. `ink(x, y)` says
/// whether a pixel is text; `baseline` is the row of the baseline, `em`
/// the size in pixels; `right` is the rightmost column with ink, which is
/// the last glyph's right edge (every glyph but `:` and the space has one
/// of the right-hand segments), and `left` the leftmost. Spaces are left
/// out of what is returned (they have no ink to tell them by);
/// `.notdef` reads as `?`, anything unreadable as `!`.
pub fn decode(
    ink: impl Fn(i64, i64) -> bool,
    left: i64,
    right: i64,
    baseline: i64,
    em: f64,
) -> String {
    let unit = em / f64::from(UNITS_PER_EM);
    let at = |origin: f64, x: i16, y: i16| -> bool {
        let px = (origin + f64::from(x) * unit).floor() as i64;
        let py = (baseline as f64 - f64::from(y) * unit).floor() as i64;
        ink(px, py)
    };
    let center = |r: Rect| ((r.0 + r.2) / 2, (r.1 + r.3) / 2);
    // The last glyph's origin: its right edge is at 500 units.
    let mut origin = (right + 1) as f64 - 500.0 * unit;
    let advance = f64::from(ADVANCE) * unit;
    let mut read = Vec::new();
    while origin + 500.0 * unit > left as f64 {
        let colon = COLON.iter().all(|&r| {
            let (x, y) = center(r);
            at(origin, x, y)
        });
        let mask = SEGMENTS
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                let (x, y) = center(**r);
                at(origin, x, y)
            })
            .fold(0u8, |mask, (bit, _)| mask | 1 << bit);
        let c = if at(origin, HOLE.0, HOLE.1) {
            '?'
        } else if colon && mask == 0 {
            ':'
        } else if mask == 0 {
            ' '
        } else {
            SHAPES
                .iter()
                .find(|(_, shape)| *shape == mask)
                .map_or('!', |(c, _)| *c)
        };
        if c != ' ' {
            read.push(c);
        }
        origin -= advance;
    }
    read.iter().rev().collect()
}
