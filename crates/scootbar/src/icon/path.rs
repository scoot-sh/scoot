//! SVG path data, parsed by hand: the `d` attribute of a `<path>`, as a
//! flat list of absolute [`Seg`]s the rasterizer ([`super::raster`]) fills.
//!
//! Every command of the path grammar is here (`M m L l H h V v C c S s Q q
//! T t A a Z z`), with its implicit repeats (`M 1 2 3 4` is a move and a
//! line), its shorthand numbers (`.5.5`, `1e2`, `-1-2`) and the arc flags
//! written without a separator (`a1 1 0 00.5.5`). Relative commands become
//! absolute, quadratics become cubics, and arcs become at most four cubics
//! each (the endpoint-to-center conversion of the SVG implementation notes,
//! F.6), so what the rasterizer sees is only lines and cubics.
//!
//! ## Hostile input
//!
//! A path comes from a config file, so it is checked, not trusted: at most
//! [`MAX_BYTES`] of text, at most [`MAX_COMMANDS`] commands (argument
//! groups, an implicit repeat counting each time), at most [`MAX_SEGS`]
//! segments once arcs are expanded, and every number finite and within
//! [`MAX_COORD`] (so no later arithmetic overflows `f32`, and none reaches
//! an `as` cast unbounded). Anything else, a stray character included, is an
//! [`Error`] naming the byte it stopped at; nothing is skipped or
//! guessed, and no input panics (`fuzz` in the tests runs a large space of
//! mutated and random strings through it).
//!
//! The path is not required to be closed: a fill closes every subpath, as
//! SVG's does.

use std::fmt;

#[cfg(test)]
mod tests;

/// The longest path data taken, in bytes. Real icon paths run from a few
/// hundred bytes to a few thousand; the config file is at most 64 KiB.
pub const MAX_BYTES: usize = 16 * 1024;
/// The most commands (argument groups) a path may have.
pub const MAX_COMMANDS: usize = 1024;
/// The most segments after arcs become cubics (an arc is at most four).
pub const MAX_SEGS: usize = 4096;
/// The largest magnitude of any number, in the path's own units.
pub const MAX_COORD: f32 = 1.0e6;

/// A point, in the path's own units (or pixels, in the rasterizer).
pub type Point = (f32, f32);

/// One drawing step, with absolute coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    /// Begins a subpath (closing the one before it, in a fill).
    Move(Point),
    Line(Point),
    /// Two control points, then the end.
    Cubic(Point, Point, Point),
    /// A straight line back to where the subpath began.
    Close,
}

/// Why path data (or a viewbox) is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error {
    /// Where it stopped, in bytes from the start.
    pub at: usize,
    pub what: &'static str,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.at, self.what)
    }
}

impl std::error::Error for Error {}

fn fail<T>(at: usize, what: &'static str) -> Result<T, Error> {
    Err(Error { at, what })
}

/// The part of the path's plane an icon is drawn from: SVG's `viewBox`,
/// `min-x min-y width height`. The default is Material's 24 by 24.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for ViewBox {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 24.0,
            height: 24.0,
        }
    }
}

impl ViewBox {
    /// Four numbers, a positive width and height: `"0 0 24 24"`.
    pub fn parse(text: &str) -> Result<Self, Error> {
        if text.len() > 128 {
            return fail(128, "too long for a viewbox");
        }
        let mut scan = Scan::new(text);
        let mut v = [0.0f32; 4];
        for (i, slot) in v.iter_mut().enumerate() {
            if i > 0 {
                scan.separator();
            }
            scan.skip_space();
            *slot = scan.number()?;
        }
        scan.skip_space();
        if !scan.done() {
            return fail(scan.at, "expected four numbers and nothing more");
        }
        let [x, y, width, height] = v;
        // A tiny size would scale a path past `f32`'s comfortable range.
        if width < 1.0e-3 || height < 1.0e-3 {
            return fail(0, "the width and height must be positive (at least 0.001)");
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }
}

/// A parsed path icon: what [`super::Art::Vector`] holds.
#[derive(Debug)]
pub struct Vector {
    /// Unique per parse, so a cache keyed by it never confuses two icons
    /// (an address could be reused after a reload drops one).
    id: u64,
    source: Box<str>,
    view: ViewBox,
    segs: Vec<Seg>,
}

impl Vector {
    /// Parses `d`, to be drawn from `view`.
    pub fn parse(d: &str, view: ViewBox) -> Result<Self, Error> {
        let segs = parse(d)?;
        Ok(Self {
            id: super::next_id(),
            source: d.into(),
            view,
            segs,
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn view(&self) -> ViewBox {
        self.view
    }

    pub fn segs(&self) -> &[Seg] {
        &self.segs
    }
}

/// Two icons are equal when made from the same text and viewbox (bitwise
/// on the numbers, so `Eq` holds): what a reload's comparison of two
/// configs asks.
impl PartialEq for Vector {
    fn eq(&self, other: &Self) -> bool {
        let bits = |v: ViewBox| [v.x, v.y, v.width, v.height].map(f32::to_bits);
        self.source == other.source && bits(self.view) == bits(other.view)
    }
}

impl Eq for Vector {}

/// A cursor over the path text.
struct Scan<'a> {
    s: &'a [u8],
    at: usize,
}

impl<'a> Scan<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            s: text.as_bytes(),
            at: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn done(&self) -> bool {
        self.at >= self.s.len()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0c)) {
            self.at += 1;
        }
    }

    /// Whitespace and at most one comma, between two numbers.
    fn separator(&mut self) {
        self.skip_space();
        if self.peek() == Some(b',') {
            self.at += 1;
        }
        self.skip_space();
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    /// A number: `[+-]? (digits [. digits?] | . digits) ([eE] [+-]? digits)?`.
    fn number(&mut self) -> Result<f32, Error> {
        let start = self.at;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.at += 1;
        }
        let whole = self.digits();
        let mut fraction = 0;
        if self.peek() == Some(b'.') {
            self.at += 1;
            fraction = self.digits();
        }
        if whole == 0 && fraction == 0 {
            return fail(start, "expected a number");
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return fail(self.at, "an exponent needs digits");
            }
        }
        let text = self
            .s
            .get(start..self.at)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or("");
        match text.parse::<f32>() {
            Ok(v) if v.is_finite() && v.abs() <= MAX_COORD => Ok(v),
            Ok(_) => fail(start, "a number out of range (at most 1000000)"),
            Err(_) => fail(start, "expected a number"),
        }
    }

    /// An arc flag: a single `0` or `1`, which may touch what follows.
    fn flag(&mut self) -> Result<bool, Error> {
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.at += 1;
                Ok(true)
            }
            _ => fail(self.at, "an arc flag is 0 or 1"),
        }
    }

    /// Whether another argument group follows a command (a number starts
    /// here, after an optional separator).
    fn more_args(&mut self) -> bool {
        let save = self.at;
        self.separator();
        if matches!(self.peek(), Some(b'0'..=b'9' | b'.' | b'+' | b'-')) {
            true
        } else {
            self.at = save;
            false
        }
    }
}

/// What builds the segment list and tracks the pen.
struct Builder {
    segs: Vec<Seg>,
    commands: usize,
    cur: Point,
    start: Point,
    /// The last cubic control point (for `S`), and quadratic (for `T`),
    /// each only right after that family of command.
    cubic: Option<Point>,
    quad: Option<Point>,
}

impl Builder {
    fn push(&mut self, at: usize, seg: Seg) -> Result<(), Error> {
        // A move right after a move (or the implicit one a close leaves)
        // replaces it: it drew nothing.
        if matches!(seg, Seg::Move(_)) && matches!(self.segs.last(), Some(Seg::Move(_))) {
            self.segs.pop();
        }
        if self.segs.len() >= MAX_SEGS {
            return fail(at, "too many segments (at most 4096)");
        }
        self.segs.push(seg);
        Ok(())
    }

    fn cubic_to(&mut self, at: usize, c1: Point, c2: Point, end: Point) -> Result<(), Error> {
        self.push(at, Seg::Cubic(c1, c2, end))?;
        self.cur = end;
        Ok(())
    }
}

/// The path data as segments.
pub fn parse(d: &str) -> Result<Vec<Seg>, Error> {
    if d.len() > MAX_BYTES {
        return fail(MAX_BYTES, "path data too long (at most 16384 bytes)");
    }
    let mut scan = Scan::new(d);
    let mut b = Builder {
        segs: Vec::new(),
        commands: 0,
        cur: (0.0, 0.0),
        start: (0.0, 0.0),
        cubic: None,
        quad: None,
    };
    scan.skip_space();
    if !matches!(scan.peek(), Some(b'M' | b'm')) {
        return fail(scan.at, "path data starts with M or m");
    }
    loop {
        scan.skip_space();
        let Some(letter) = scan.peek() else { break };
        if !letter.is_ascii_alphabetic() {
            return fail(scan.at, "expected a command letter");
        }
        let at = scan.at;
        if !b"MmLlHhVvCcSsQqTtAaZz".contains(&letter) {
            return fail(at, "not a path command");
        }
        scan.at += 1;
        if matches!(letter, b'Z' | b'z') {
            count(&mut b, at)?;
            b.push(at, Seg::Close)?;
            b.cur = b.start;
            b.cubic = None;
            b.quad = None;
            // A drawing command after Z starts from where the subpath began.
            b.push(at, Seg::Move(b.start))?;
            continue;
        }
        let mut letter = letter;
        loop {
            scan.skip_space();
            let at = scan.at;
            count(&mut b, at)?;
            command(&mut scan, &mut b, letter)?;
            if !scan.more_args() {
                break;
            }
            // More numbers repeat the command; a move's are lines.
            letter = match letter {
                b'M' => b'L',
                b'm' => b'l',
                other => other,
            };
        }
    }
    // A trailing `Move` (from a final Z) draws nothing; leave it out.
    while matches!(b.segs.last(), Some(Seg::Move(_))) {
        b.segs.pop();
    }
    if !b
        .segs
        .iter()
        .any(|s| matches!(s, Seg::Line(_) | Seg::Cubic(..)))
    {
        return fail(d.len(), "the path draws nothing");
    }
    Ok(b.segs)
}

fn count(b: &mut Builder, at: usize) -> Result<(), Error> {
    b.commands += 1;
    if b.commands > MAX_COMMANDS {
        return fail(at, "too many commands (at most 1024)");
    }
    Ok(())
}

/// Reads one argument group of `letter` and applies it.
fn command(scan: &mut Scan<'_>, b: &mut Builder, letter: u8) -> Result<(), Error> {
    let at = scan.at;
    let relative = letter.is_ascii_lowercase();
    let (ox, oy) = if relative { b.cur } else { (0.0, 0.0) };
    let pair = |scan: &mut Scan<'_>, first: bool| -> Result<Point, Error> {
        if !first {
            scan.separator();
        }
        let x = scan.number()?;
        scan.separator();
        let y = scan.number()?;
        Ok((finite(x + ox, at)?, finite(y + oy, at)?))
    };
    let upper = letter.to_ascii_uppercase();
    // Which family of shorthand control points survives this command.
    let (mut keep_cubic, mut keep_quad) = (None, None);
    match upper {
        b'M' => {
            let p = pair(scan, true)?;
            b.push(at, Seg::Move(p))?;
            b.cur = p;
            b.start = p;
        }
        b'L' => {
            let p = pair(scan, true)?;
            b.push(at, Seg::Line(p))?;
            b.cur = p;
        }
        b'H' => {
            let x = scan.number()?;
            let p = (finite(x + ox, at)?, b.cur.1);
            b.push(at, Seg::Line(p))?;
            b.cur = p;
        }
        b'V' => {
            let y = scan.number()?;
            let p = (b.cur.0, finite(y + oy, at)?);
            b.push(at, Seg::Line(p))?;
            b.cur = p;
        }
        b'C' => {
            let c1 = pair(scan, true)?;
            let c2 = pair(scan, false)?;
            let end = pair(scan, false)?;
            b.cubic_to(at, c1, c2, end)?;
            keep_cubic = Some(c2);
        }
        b'S' => {
            let c2 = pair(scan, true)?;
            let end = pair(scan, false)?;
            let c1 = reflect(b.cur, b.cubic);
            b.cubic_to(at, c1, c2, end)?;
            keep_cubic = Some(c2);
        }
        b'Q' => {
            let q = pair(scan, true)?;
            let end = pair(scan, false)?;
            quad_to(b, at, q, end)?;
            keep_quad = Some(q);
        }
        b'T' => {
            let end = pair(scan, true)?;
            let q = reflect(b.cur, b.quad);
            quad_to(b, at, q, end)?;
            keep_quad = Some(q);
        }
        b'A' => {
            let rx = scan.number()?;
            scan.separator();
            let ry = scan.number()?;
            scan.separator();
            let rotation = scan.number()?;
            scan.separator();
            let large = scan.flag()?;
            scan.separator();
            let sweep = scan.flag()?;
            let end = pair(scan, false)?;
            arc(b, at, rx, ry, rotation, large, sweep, end)?;
        }
        _ => return fail(at, "not a path command"),
    }
    b.cubic = keep_cubic;
    b.quad = keep_quad;
    Ok(())
}

fn finite(v: f32, at: usize) -> Result<f32, Error> {
    // Relative moves add up: a sum past the bound is as out of range as a
    // number that big.
    if v.is_finite() && v.abs() <= MAX_COORD * 4.0 {
        Ok(v)
    } else {
        fail(at, "a coordinate out of range")
    }
}

/// `cur` mirrored through the previous control point, or `cur` itself when
/// the previous command was not of the family.
fn reflect(cur: Point, previous: Option<Point>) -> Point {
    match previous {
        Some((x, y)) => (2.0 * cur.0 - x, 2.0 * cur.1 - y),
        None => cur,
    }
}

/// A quadratic as the cubic with the same curve.
fn quad_to(b: &mut Builder, at: usize, q: Point, end: Point) -> Result<(), Error> {
    let p0 = b.cur;
    let third = |from: Point, to: Point| {
        (
            from.0 + 2.0 / 3.0 * (to.0 - from.0),
            from.1 + 2.0 / 3.0 * (to.1 - from.1),
        )
    };
    b.cubic_to(at, third(p0, q), third(end, q), end)
}

/// An elliptical arc from the pen to `end`, as cubics (SVG implementation
/// notes F.6.5 and F.6.6). A zero radius is a line; the same endpoints draw
/// nothing; radii too small to reach are scaled up until they do.
#[allow(clippy::too_many_arguments)]
fn arc(
    b: &mut Builder,
    at: usize,
    rx: f32,
    ry: f32,
    rotation: f32,
    large: bool,
    sweep: bool,
    end: Point,
) -> Result<(), Error> {
    let p0 = b.cur;
    if p0 == end {
        return Ok(());
    }
    let (mut rx, mut ry) = (f64::from(rx.abs()), f64::from(ry.abs()));
    if rx == 0.0 || ry == 0.0 {
        b.push(at, Seg::Line(end))?;
        b.cur = end;
        return Ok(());
    }
    let phi = f64::from(rotation).to_radians();
    let (sin, cos) = phi.sin_cos();
    let (x1, y1) = (f64::from(p0.0), f64::from(p0.1));
    let (x2, y2) = (f64::from(end.0), f64::from(end.1));
    // Step 1: the midpoint, rotated into the ellipse's frame.
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let x1p = cos * dx + sin * dy;
    let y1p = -sin * dx + cos * dy;
    // Radii too small: scale them until the ellipse just reaches.
    let lambda = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }
    // Step 2: the center in that frame.
    let numerator = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let denominator = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut factor = if denominator > 0.0 {
        (numerator / denominator).max(0.0).sqrt()
    } else {
        0.0
    };
    if large == sweep {
        factor = -factor;
    }
    let cxp = factor * rx * y1p / ry;
    let cyp = -factor * ry * x1p / rx;
    // Step 3: the center, back in the plane.
    let cx = cos * cxp - sin * cyp + (x1 + x2) / 2.0;
    let cy = sin * cxp + cos * cyp + (y1 + y2) / 2.0;
    // Step 4: the start angle and the sweep.
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let theta = angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut delta = angle(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    let tau = std::f64::consts::TAU;
    if !sweep && delta > 0.0 {
        delta -= tau;
    } else if sweep && delta < 0.0 {
        delta += tau;
    }
    if !(theta.is_finite() && delta.is_finite() && cx.is_finite() && cy.is_finite()) {
        return fail(at, "an arc that cannot be drawn");
    }
    // Pieces of at most a quarter turn, each a cubic.
    let pieces = (delta.abs() / std::f64::consts::FRAC_PI_2 - 1.0e-9)
        .ceil()
        .clamp(1.0, 4.0) as usize;
    let step = delta / pieces as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let point = |t: f64| {
        let (s, c) = t.sin_cos();
        (rx * c, ry * s)
    };
    let derivative = |t: f64| {
        let (s, c) = t.sin_cos();
        (-rx * s, ry * c)
    };
    let place = |(px, py): (f64, f64)| {
        (
            (cos * px - sin * py + cx) as f32,
            (sin * px + cos * py + cy) as f32,
        )
    };
    for i in 0..pieces {
        let (t0, t1) = (theta + step * i as f64, theta + step * (i + 1) as f64);
        let (a, ad) = (point(t0), derivative(t0));
        let (e, ed) = (point(t1), derivative(t1));
        let c1 = place((a.0 + k * ad.0, a.1 + k * ad.1));
        let c2 = place((e.0 - k * ed.0, e.1 - k * ed.1));
        // The last piece ends exactly where the path says.
        let to = if i + 1 == pieces { end } else { place(e) };
        for p in [c1, c2, to] {
            finite(p.0, at)?;
            finite(p.1, at)?;
        }
        b.cubic_to(at, c1, c2, to)?;
    }
    Ok(())
}
