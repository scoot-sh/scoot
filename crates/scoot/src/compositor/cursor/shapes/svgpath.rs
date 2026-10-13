//! A tiny SVG path-data reader: turns the `d` strings of the vendored Mocu
//! cursor art ([`super::mocu`]) into flattened polygons the rasterizer fills.
//!
//! Only what that art uses is supported -- `M L H V C A Z` in absolute and
//! relative form, implicit command repetition, and the compact number syntax
//! SVG allows (`-0.5-0.5`, `.875`, `1e2`). Curves and arcs are flattened into
//! line segments with a tolerance given in path units, so the caller picks it
//! from the target bitmap size: a 16px cursor flattens a circle into a few
//! dozen segments, a 256px one into a few hundred, and both stay within a
//! tenth of a pixel of the true curve.
//!
//! Runs once per shape per bitmap size at startup or reload, never per frame.
//! Malformed data ends the parse at the bad token and keeps what came before
//! (a partial shape, never a panic); the art is compile-time constant and the
//! tests assert every shape parses completely.

/// One closed polygon, in path units.
pub(crate) type Subpath = Vec<(f64, f64)>;

/// A cursor of the pull parser over path data.
struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Tokens<'a> {
    fn new(d: &'a str) -> Self {
        Self {
            s: d.as_bytes(),
            i: 0,
        }
    }

    fn skip_separators(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b',' | b'\n' | b'\t' | b'\r')
        {
            self.i += 1;
        }
    }

    /// The next command letter, if the next token is one.
    fn command(&mut self) -> Option<u8> {
        self.skip_separators();
        let c = *self.s.get(self.i)?;
        if c.is_ascii_alphabetic() && c != b'e' && c != b'E' {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    /// Whether a number (rather than a command or the end) comes next.
    fn number_follows(&mut self) -> bool {
        self.skip_separators();
        matches!(self.s.get(self.i), Some(b'0'..=b'9' | b'-' | b'+' | b'.'))
    }

    fn number(&mut self) -> Option<f64> {
        self.skip_separators();
        let start = self.i;
        let mut seen_dot = false;
        let mut seen_exp = false;
        if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
            self.i += 1;
        }
        while let Some(&c) = self.s.get(self.i) {
            match c {
                b'0'..=b'9' => self.i += 1,
                b'.' if !seen_dot && !seen_exp => {
                    seen_dot = true;
                    self.i += 1;
                }
                b'e' | b'E' if !seen_exp => {
                    seen_exp = true;
                    self.i += 1;
                    if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse()
            .ok()
    }

    /// An arc flag: a single `0` or `1`, which SVG allows unseparated.
    fn flag(&mut self) -> Option<bool> {
        self.skip_separators();
        let c = *self.s.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }

    fn done(&mut self) -> bool {
        self.skip_separators();
        self.i >= self.s.len()
    }
}

/// Parses `d` into closed polygons, flattening curves to within `tol` path
/// units. Returns `None` if the data is malformed or ends early (the
/// complete-parse check the tests use); see [`parse`] for the forgiving form.
#[cfg(test)]
pub(crate) fn parse_strict(d: &str, tol: f64) -> Option<Vec<Subpath>> {
    let mut out = Vec::new();
    let ok = parse_into(d, tol, &mut out);
    ok.then_some(out)
}

/// Parses `d` into closed polygons, flattening curves to within `tol` path
/// units. Malformed data keeps whatever parsed before it.
pub(crate) fn parse(d: &str, tol: f64) -> Vec<Subpath> {
    let mut out = Vec::new();
    parse_into(d, tol, &mut out);
    out
}

fn parse_into(d: &str, tol: f64, out: &mut Vec<Subpath>) -> bool {
    let tol = tol.max(1e-4);
    let mut t = Tokens::new(d);
    let (mut x, mut y) = (0.0f64, 0.0f64);
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    let mut cur: Subpath = Vec::new();
    let mut cmd: Option<u8> = None;
    let flush = |cur: &mut Subpath, out: &mut Vec<Subpath>| {
        if cur.len() >= 3 {
            out.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    loop {
        if t.done() {
            break;
        }
        if let Some(c) = t.command() {
            cmd = Some(c);
        } else if !t.number_follows() {
            return false;
        }
        let Some(c) = cmd else { return false };
        let rel = c.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        match c.to_ascii_uppercase() {
            b'M' => {
                let (Some(nx), Some(ny)) = (t.number(), t.number()) else {
                    return false;
                };
                flush(&mut cur, out);
                x = ox + nx;
                y = oy + ny;
                sx = x;
                sy = y;
                cur.push((x, y));
                // Further pairs after a moveto are implicit linetos.
                cmd = Some(if rel { b'l' } else { b'L' });
            }
            b'L' => {
                let (Some(nx), Some(ny)) = (t.number(), t.number()) else {
                    return false;
                };
                x = ox + nx;
                y = oy + ny;
                cur.push((x, y));
            }
            b'H' => {
                let Some(nx) = t.number() else { return false };
                x = ox + nx;
                cur.push((x, y));
            }
            b'V' => {
                let Some(ny) = t.number() else { return false };
                y = oy + ny;
                cur.push((x, y));
            }
            b'C' => {
                let mut v = [0.0; 6];
                for slot in &mut v {
                    let Some(n) = t.number() else { return false };
                    *slot = n;
                }
                let p1 = (ox + v[0], oy + v[1]);
                let p2 = (ox + v[2], oy + v[3]);
                let p3 = (ox + v[4], oy + v[5]);
                cubic(&mut cur, (x, y), p1, p2, p3, tol);
                (x, y) = p3;
            }
            b'A' => {
                let (Some(rx), Some(ry), Some(rot)) = (t.number(), t.number(), t.number()) else {
                    return false;
                };
                let (Some(large), Some(sweep)) = (t.flag(), t.flag()) else {
                    return false;
                };
                let (Some(nx), Some(ny)) = (t.number(), t.number()) else {
                    return false;
                };
                let end = (ox + nx, oy + ny);
                arc(&mut cur, (x, y), rx, ry, rot, large, sweep, end, tol);
                (x, y) = end;
            }
            b'Z' => {
                flush(&mut cur, out);
                x = sx;
                y = sy;
                // A command after `z` without a moveto starts at the
                // subpath's start point.
                cur.push((x, y));
                cmd = None;
            }
            _ => return false,
        }
    }
    flush(&mut cur, out);
    true
}

/// Appends a cubic Bezier from `p0` (already in `cur`) to `p3`, flattened by
/// uniform subdivision sized from the control polygon's length.
fn cubic(
    cur: &mut Subpath,
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    tol: f64,
) {
    let len = dist(p0, p1) + dist(p1, p2) + dist(p2, p3);
    let n = ((len / tol).sqrt().ceil() as usize).clamp(2, 64);
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let mt = 1.0 - t;
        let a = mt * mt * mt;
        let b = 3.0 * mt * mt * t;
        let c = 3.0 * mt * t * t;
        let d = t * t * t;
        cur.push((
            a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
            a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
        ));
    }
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Appends an elliptical arc from `p0` to `p1`, per the endpoint-to-centre
/// conversion in SVG 1.1 appendix F.6.5, flattened so the chord error stays
/// under `tol`.
#[allow(clippy::too_many_arguments)]
fn arc(
    cur: &mut Subpath,
    p0: (f64, f64),
    rx: f64,
    ry: f64,
    rot_deg: f64,
    large: bool,
    sweep: bool,
    p1: (f64, f64),
    tol: f64,
) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || p0 == p1 {
        cur.push(p1);
        return;
    }
    let phi = rot_deg.to_radians();
    let (sin_p, cos_p) = phi.sin_cos();
    let dx = (p0.0 - p1.0) / 2.0;
    let dy = (p0.1 - p1.1) / 2.0;
    let x1 = cos_p * dx + sin_p * dy;
    let y1 = -sin_p * dx + cos_p * dy;
    // Radii too small to reach: scale them up (F.6.6).
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = (num / den).max(0.0).sqrt();
    if large == sweep {
        coef = -coef;
    }
    let cx1 = coef * rx * y1 / ry;
    let cy1 = -coef * ry * x1 / rx;
    let cx = cos_p * cx1 - sin_p * cy1 + (p0.0 + p1.0) / 2.0;
    let cy = sin_p * cx1 + cos_p * cy1 + (p0.1 + p1.1) / 2.0;
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| -> f64 {
        (ux * vy - uy * vx).atan2(ux * vx + uy * vy)
    };
    let ux = (x1 - cx1) / rx;
    let uy = (y1 - cy1) / ry;
    let vx = (-x1 - cx1) / rx;
    let vy = (-y1 - cy1) / ry;
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f64::consts::TAU;
    }
    // Segments so the sagitta r(1 - cos(step/2)) stays under `tol`.
    let r = rx.max(ry);
    let step = if tol >= r {
        std::f64::consts::FRAC_PI_2
    } else {
        2.0 * (1.0 - tol / r).acos()
    };
    let n = ((delta.abs() / step.max(1e-3)).ceil() as usize).clamp(1, 512);
    for i in 1..=n {
        if i == n {
            cur.push(p1);
            break;
        }
        let th = theta1 + delta * i as f64 / n as f64;
        let (s, c) = th.sin_cos();
        cur.push((
            cos_p * rx * c - sin_p * ry * s + cx,
            sin_p * rx * c + cos_p * ry * s + cy,
        ));
    }
}
