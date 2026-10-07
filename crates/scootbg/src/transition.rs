//! Transitions between wallpapers: the kinds, the easing, and the
//! per-frame math, with no Wayland objects, so each is a unit test. The
//! daemon (`daemon::transition`) carries them out.
//!
//! A transition blends what an output shows now (a solid color, or the
//! pixels of its buffer) with what it should show next (likewise) into a
//! third buffer, one frame at a time, paced by frame callbacks with a
//! timer fallback (`FRAME_STEP_MS`). The eased progress `t` comes from the
//! clock, so a slow frame drops frames rather than falling behind: `t`
//! jumps to where the clock says. A frame that costs more than
//! [`FRAME_BUDGET_MS`] of CPU skips the next one, so the compositor is
//! never stalled by a wallpaper.
//!
//! ## Kinds
//!
//! - [`Kind::Fade`]: every pixel lerps from old to new by `t`.
//! - [`Kind::Wipe`]: a straight leading edge sweeps across at `angle_deg`
//!   (clockwise from the positive x-axis on screen, y down: 0 shows the new
//!   wallpaper from the left edge, 90 from the top, 180 from the right, 270
//!   from the bottom). Behind the edge is new, ahead of it old.
//! - [`Kind::Grow`]: a disc of the new wallpaper grows from `pos` (fractions
//!   of the width and height; the center by default) until it covers the
//!   output.
//! - [`Kind::None`]: no animation; the change lands at once.
//!
//! At `t = 0` every pixel is old, at `t = 1` every pixel is new, exactly:
//! the edge and the radius are inset by one pixel past each end so no
//! endpoint shows a stray line of the other side.
//!
//! ## Damage
//!
//! [`damage`] returns the smallest axis-aligned cover of what changed
//! between two eased progresses, so a commit damages only that: the whole
//! buffer for a fade (every pixel moves), the sweeping band for a wipe
//! (tight for axis-aligned angles, its bounding box otherwise), the disc's
//! bounding box for a grow. No heap is allocated for any of it: at most
//! four rects on the stack.
//!
//! ## Memory bound
//!
//! A transition holds at most one extra full-size buffer per output (the
//! frame being drawn: [`frame_bytes`], 33,177,600 B at 3840×2160), plus, on
//! a mid-transition restart only, one snapshot of what is on screen (the
//! memcpy the restart starts from). The old and new endpoints are shared
//! references to pixels that already exist, never copies. Everything extra
//! is freed when the transition finishes, so an idle daemon costs what a
//! static wallpaper costs. An allocation that fails ends the transition at
//! once (the final wallpaper is shown the normal way), never wedged and
//! never worse than the OOM abort `scaler-oom-abort.md` knows.
//!
//! ## No per-frame allocation
//!
//! [`Sweep::new`] precomputes one frame's geometry on the stack;
//! [`blend_row`] writes one row from shared slices into a reused buffer.
//! A solid-color endpoint fills one reused scanline per transition, not per
//! frame.

#[cfg(test)]
mod tests;

#[cfg(test)]
mod bench;

/// How often the timer prods a running transition: about 60 Hz. The eased
/// progress still comes from the clock, so this is a cadence, not a step
/// count: a missed wake just renders a later `t`.
pub const FRAME_STEP_MS: u64 = 16;

/// The CPU time one frame may cost, in milliseconds. A frame past it skips
/// the next one. Measured under pixman at 4K on the Asahi M2, release
/// build (see `transition::bench`): a 1080p fade blends in ~1 ms, a 4K
/// fade in ~4 ms, a 4K wipe in ~6 ms; 8 ms leaves headroom for a loaded
/// machine while staying inside one 60 Hz interval.
pub const FRAME_BUDGET_MS: u64 = 8;

/// The duration a request gets when it names none, in milliseconds.
pub const DEFAULT_DURATION_MS: u32 = 500;

/// The longest transition taken, in milliseconds. Past it the request is
/// refused: a longer one would hold its buffers and its timer past any
/// reasonable change, against the zero-idle rule.
pub const MAX_DURATION_MS: u32 = 60_000;

/// What a transition looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// No animation: the change lands at once (the default).
    #[default]
    None,
    /// Every pixel lerps from old to new.
    Fade,
    /// A straight edge sweeps across at an angle.
    Wipe,
    /// A disc of the new wallpaper grows from a point.
    Grow,
}

impl Kind {
    /// Every kind, for parsing and for help text.
    pub const ALL: [Self; 4] = [Self::None, Self::Fade, Self::Wipe, Self::Grow];

    /// The name a request uses.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Fade => "fade",
            Self::Wipe => "wipe",
            Self::Grow => "grow",
        }
    }

    /// The kind named `name`, if it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// Parses a transition kind, saying what it takes.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        Self::from_name(text).ok_or_else(|| ParseError::UnknownKind(text.to_owned()))
    }
}

/// How the eased progress moves through a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    /// Straight through (the honest default for measurement).
    Linear,
    /// Slow start.
    EaseIn,
    /// Fast start, settling in (the default for a change).
    #[default]
    EaseOut,
    /// Slow at both ends.
    EaseInOut,
    /// Smoothstep: slow at both ends, steeper in the middle.
    Smooth,
}

impl Easing {
    /// Every curve, for parsing and for help text.
    pub const ALL: [Self; 5] = [
        Self::Linear,
        Self::EaseIn,
        Self::EaseOut,
        Self::EaseInOut,
        Self::Smooth,
    ];

    /// The name a request uses.
    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::EaseIn => "ease-in",
            Self::EaseOut => "ease-out",
            Self::EaseInOut => "ease-in-out",
            Self::Smooth => "smooth",
        }
    }

    /// The curve named `name`, if it is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|easing| easing.name() == name)
    }

    /// Parses an easing curve, saying what it takes.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        Self::from_name(text).ok_or_else(|| ParseError::UnknownEasing(text.to_owned()))
    }
}

/// Why transition parameters were refused. Each says what was wrong and
/// what is taken instead; the caller (the CLI, the protocol, the section)
/// adds where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnknownKind(String),
    UnknownEasing(String),
    BadDuration(String),
    BadAngle(String),
    BadPosition(String),
    /// A transition parameter without `transition`: the parameter's name
    /// as the surface spells it (`--duration-ms` on the command line,
    /// `duration-ms` in a section).
    Orphan(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKind(text) => {
                write!(f, "unknown transition {text:?}: none, fade, wipe or grow")
            }
            Self::UnknownEasing(text) => write!(
                f,
                "unknown easing {text:?}: linear, ease-in, ease-out, ease-in-out or smooth"
            ),
            Self::BadDuration(text) => write!(
                f,
                "bad duration {text:?}: milliseconds as digits, 0 to {MAX_DURATION_MS}"
            ),
            Self::BadAngle(text) => write!(
                f,
                "bad angle {text:?}: degrees as a number (0 wipes in from the left, 90 from \
                 the top, 180 from the right, 270 from the bottom)"
            ),
            Self::BadPosition(text) => write!(
                f,
                "bad position {text:?}: two fractions of the width and height as `X,Y`, each \
                 0 to 1 (`0.5,0.5` is the center)"
            ),
            Self::Orphan(name) => write!(f, "`{name}` applies to a transition, and there is none"),
        }
    }
}

impl std::error::Error for ParseError {}

/// A transition as a request names it: validated, ready to run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    pub kind: Kind,
    pub duration_ms: u32,
    pub easing: Easing,
    /// Wipe direction in degrees, normalized to `[0, 360)`.
    pub angle_deg: f64,
    /// Grow center as fractions of the width and height.
    pub pos: (f64, f64),
}

impl Default for Spec {
    /// No animation: what an omitted transition means everywhere.
    fn default() -> Self {
        Self {
            kind: Kind::None,
            duration_ms: DEFAULT_DURATION_MS,
            easing: Easing::EaseOut,
            angle_deg: 0.0,
            pos: (0.5, 0.5),
        }
    }
}

impl Spec {
    /// No animation.
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether the change lands at once: `none`, or a zero duration with
    /// any kind.
    pub fn is_instant(&self) -> bool {
        self.kind == Kind::None || self.duration_ms == 0
    }
}

/// Assembles a [`Spec`] from already-looked-up parameters: `kind` parsed,
/// the rest raw. Without a kind, any other parameter is refused (it would
/// otherwise be silently not applied); with an explicit `none`, the rest
/// are ignored, as on the wire. `spell` names the offending parameter as
/// the surface spells it, for the refusal.
pub fn assemble(
    kind: Option<Kind>,
    duration_ms: Option<&str>,
    easing: Option<&str>,
    angle: Option<&str>,
    position: Option<&str>,
    spell: impl Fn(&str) -> String,
) -> Result<Spec, ParseError> {
    let kind = match kind {
        None => {
            for (key, given) in [
                ("duration-ms", duration_ms),
                ("easing", easing),
                ("angle", angle),
                ("position", position),
            ] {
                if given.is_some() {
                    return Err(ParseError::Orphan(spell(key)));
                }
            }
            return Ok(Spec::none());
        }
        Some(Kind::None) => return Ok(Spec::none()),
        Some(kind) => kind,
    };
    Ok(Spec {
        kind,
        duration_ms: match duration_ms {
            None => DEFAULT_DURATION_MS,
            Some(text) => parse_duration_ms(text)?,
        },
        easing: match easing {
            None => Easing::default(),
            Some(text) => Easing::parse(text)?,
        },
        angle_deg: match angle {
            None => 0.0,
            Some(text) => parse_angle_deg(text)?,
        },
        pos: match position {
            None => (0.5, 0.5),
            Some(text) => parse_position(text)?,
        },
    })
}

/// Parses a duration in milliseconds: digits only, `0` to
/// [`MAX_DURATION_MS`].
pub fn parse_duration_ms(text: &str) -> Result<u32, ParseError> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ParseError::BadDuration(text.to_owned()));
    }
    text.parse::<u64>()
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|&value| value <= MAX_DURATION_MS)
        .ok_or_else(|| ParseError::BadDuration(text.to_owned()))
}

/// Parses a wipe angle in degrees: any finite number, normalized to
/// `[0, 360)`. Huge values are normalized, not refused; NaN, infinities
/// and non-numbers are refused.
pub fn parse_angle_deg(text: &str) -> Result<f64, ParseError> {
    if text != text.trim() {
        return Err(ParseError::BadAngle(text.to_owned()));
    }
    let value: f64 = text
        .parse()
        .map_err(|_| ParseError::BadAngle(text.to_owned()))?;
    if !value.is_finite() {
        return Err(ParseError::BadAngle(text.to_owned()));
    }
    Ok(normalize_angle(value))
}

/// Normalizes degrees to `[0, 360)`.
pub fn normalize_angle(deg: f64) -> f64 {
    if !deg.is_finite() {
        return 0.0;
    }
    let wrapped = deg % 360.0;
    if wrapped < 0.0 {
        wrapped + 360.0
    } else if wrapped == 0.0 {
        // Collapses `-0.0`, so equal inputs compare equal downstream.
        0.0
    } else {
        wrapped
    }
}

/// Parses a grow center: `X,Y`, two finite fractions in `[0, 1]`.
pub fn parse_position(text: &str) -> Result<(f64, f64), ParseError> {
    let refused = || ParseError::BadPosition(text.to_owned());
    let Some((x, y)) = text.split_once(',') else {
        return Err(refused());
    };
    if text.contains(char::is_whitespace) {
        return Err(refused());
    }
    let number = |part: &str| {
        let value: f64 = part.parse().map_err(|_| refused())?;
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(value)
        } else {
            Err(refused())
        }
    };
    Ok((number(x)?, number(y)?))
}

/// The eased progress for `easing` at linear `t`: `0` at `0`, `1` at `1`,
/// monotonic between. `t` outside `[0, 1]` is clamped, so a clock that
/// overshoots still ends exactly.
pub fn ease(easing: Easing, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => t,
        Easing::EaseIn => t * t * t,
        Easing::EaseOut => 1.0 - (1.0 - t).powi(3),
        Easing::EaseInOut => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        Easing::Smooth => t * t * (3.0 - 2.0 * t),
    }
}

/// The eased progress `elapsed_ms` into `spec`: `0` before the start, `1`
/// at and past the end. A zero duration is done at once.
pub fn progress(spec: &Spec, elapsed_ms: u64) -> f64 {
    if spec.duration_ms == 0 {
        return 1.0;
    }
    ease(spec.easing, elapsed_ms as f64 / f64::from(spec.duration_ms))
}

/// How many frames a transition of `duration_ms` renders at the frame
/// cadence: at least one, so even the shortest transition shows its end.
pub fn total_steps(duration_ms: u32) -> u32 {
    u64::from(duration_ms)
        .div_ceil(FRAME_STEP_MS)
        .max(1)
        .min(u64::from(u32::MAX)) as u32
}

/// Which frame `elapsed_ms` into a transition of `duration_ms` is showing:
/// `0` before the first cadence, `total_steps` at and past the end. The
/// driver renders only when this moves, so a timer that fires early or a
/// frame callback and a timer racing never render twice.
pub fn step_for_elapsed(duration_ms: u32, elapsed_ms: u64) -> u32 {
    (elapsed_ms / FRAME_STEP_MS).min(u64::from(total_steps(duration_ms))) as u32
}

/// How many extra frames to skip after one cost `measured_ms` of CPU:
/// none within budget, one past it. Skipping advances the eased progress
/// without rendering, so the animation keeps time instead of lagging.
pub fn skip_for(measured_ms: u64) -> u32 {
    u32::from(measured_ms > FRAME_BUDGET_MS)
}

/// The bytes of shared memory one transition frame buffer of `dims`
/// needs (`XRGB8888`): `None` on overflow, which ends the transition at
/// once rather than allocating.
pub fn frame_bytes(dims: (u32, u32)) -> Option<u64> {
    u64::from(dims.0)
        .checked_mul(u64::from(dims.1))?
        .checked_mul(4)
}

/// A buffer-pixel rectangle, in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// The whole buffer.
    pub fn whole(dims: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            w: dims.0,
            h: dims.1,
        }
    }
}

/// What changed between two frames: at most four rectangles on the stack,
/// never a heap allocation.
#[derive(Debug, Clone, Copy)]
pub struct Damage {
    pub rects: [Rect; 4],
    pub n: u8,
}

impl Damage {
    /// Nothing changed.
    pub fn none() -> Self {
        Self {
            rects: [Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            }; 4],
            n: 0,
        }
    }

    /// One rectangle.
    pub fn one(rect: Rect) -> Self {
        let mut damage = Self::none();
        damage.rects[0] = rect;
        damage.n = 1;
        damage
    }

    /// The whole buffer.
    pub fn whole(dims: (u32, u32)) -> Self {
        Self::one(Rect::whole(dims))
    }

    /// The rectangles to damage, in order.
    pub fn iter(&self) -> impl Iterator<Item = Rect> {
        self.rects[..usize::from(self.n)].iter().copied()
    }

    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// One frame's geometry, precomputed on the stack: what [`blend_row`]
/// needs per pixel without recomputing trigonometry or ranges.
#[derive(Debug, Clone, Copy)]
pub struct Sweep {
    kind: Kind,
    eased: f64,
    w: u32,
    /// Wipe: the unit direction and the leading edge's projection.
    dx: f64,
    dy: f64,
    edge: f64,
    /// Grow: the center in pixels and the radius.
    cx: f64,
    cy: f64,
    radius: f64,
}

impl Sweep {
    /// Precomputes the frame at eased progress `eased` (clamped to
    /// `[0, 1]`) on a buffer of `dims`, for `spec`'s kind, angle and
    /// center.
    pub fn new(spec: &Spec, eased: f64, dims: (u32, u32)) -> Self {
        let eased = eased.clamp(0.0, 1.0);
        let (dx, dy) = direction(spec.angle_deg);
        let (_, _, min, span) = wipe_range(dx, dy, dims);
        let (cx, cy, maxr) = grow_range(spec.pos, dims);
        Self {
            kind: spec.kind,
            eased,
            w: dims.0,
            dx,
            dy,
            // Inset one pixel past each end so `t = 0` is all old and
            // `t = 1` all new, exactly.
            edge: min - 1.0 + eased * (span + 2.0),
            cx,
            cy,
            radius: eased * (maxr + 2.0) - 1.0,
        }
    }

    /// The new wallpaper's weight at pixel `(x, y)`: `0` is old, `1` new.
    /// A fade is uniform; a wipe and a grow are hard edges (no antialias:
    /// the damage stays exactly computable).
    #[inline]
    pub fn weight(&self, x: u32, y: u32) -> f64 {
        match self.kind {
            Kind::None => 1.0,
            Kind::Fade => self.eased,
            Kind::Wipe => {
                if f64::from(x).mul_add(self.dx, f64::from(y) * self.dy) <= self.edge {
                    1.0
                } else {
                    0.0
                }
            }
            Kind::Grow => {
                let dx = f64::from(x) - self.cx;
                let dy = f64::from(y) - self.cy;
                if dx.hypot(dy) <= self.radius {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// The wipe direction for normalized degrees: clockwise from the positive
/// x-axis on screen (y down), so 0 is left-to-right, 90 top-to-bottom.
fn direction(angle_deg: f64) -> (f64, f64) {
    let radians = normalize_angle(angle_deg).to_radians();
    (radians.cos(), radians.sin())
}

/// The projection range of a `dims` buffer along `(dx, dy)`: the direction
/// back, and the minimum corner projection and the span over the four
/// corners.
fn wipe_range(dx: f64, dy: f64, dims: (u32, u32)) -> (f64, f64, f64, f64) {
    let (w, h) = (f64::from(dims.0), f64::from(dims.1));
    let corners = [0.0, w * dx, h * dy, w.mul_add(dx, h * dy)];
    let min = corners.iter().copied().fold(f64::INFINITY, f64::min);
    let max = corners.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (dx, dy, min, max - min)
}

/// A grow's center in pixels and the farthest corner's distance from it.
fn grow_range(pos: (f64, f64), dims: (u32, u32)) -> (f64, f64, f64) {
    let (w, h) = (f64::from(dims.0), f64::from(dims.1));
    let (cx, cy) = (pos.0 * w, pos.1 * h);
    let maxr = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)]
        .into_iter()
        .map(|(x, y)| (x - cx).hypot(y - cy))
        .fold(0.0_f64, f64::max);
    (cx, cy, maxr)
}

/// Blends one row: `out[i] = old[i] + (new[i] - old[i]) * weight` per
/// channel, with the unused byte set opaque. Rows are `w * 4` bytes of
/// `XRGB8888` (`old`, `new`, `out`); the fade reads no geometry per pixel.
/// Values stay in `[0, 255]` by construction (a lerp of two bytes);
/// rounding picks the nearest byte, and the endpoints are exact.
pub fn blend_row(sweep: &Sweep, y: u32, old: &[u8], new: &[u8], out: &mut [u8]) {
    let w = sweep.w as usize;
    debug_assert!(old.len() >= w * 4 && new.len() >= w * 4 && out.len() >= w * 4);
    if sweep.kind == Kind::Fade {
        let weight = sweep.eased;
        for x in 0..w {
            let o = x * 4;
            for c in 0..3 {
                let value = f64::from(old[o + c])
                    + (f64::from(new[o + c]) - f64::from(old[o + c])) * weight;
                out[o + c] = value.round().clamp(0.0, 255.0) as u8;
            }
            out[o + 3] = 0xff;
        }
        return;
    }
    for x in 0..w {
        let weight = sweep.weight(x as u32, y);
        let o = x * 4;
        for c in 0..3 {
            let value =
                f64::from(old[o + c]) + (f64::from(new[o + c]) - f64::from(old[o + c])) * weight;
            out[o + c] = value.round().clamp(0.0, 255.0) as u8;
        }
        out[o + 3] = 0xff;
    }
}

/// The smallest axis-aligned cover of what changed between eased
/// progresses `prev` and `now` (in either order) on a buffer of `dims`:
/// the whole buffer for a fade, the sweeping band for a wipe, the grown
/// disc's bounding box for a grow, nothing when they are equal. `angle_deg`
/// and `pos` are the spec's (already validated). Always a superset of the
/// changed pixels: over-covering by a pixel is correct, under-covering is
/// not.
pub fn damage(
    kind: Kind,
    prev: f64,
    now: f64,
    dims: (u32, u32),
    angle_deg: f64,
    pos: (f64, f64),
) -> Damage {
    if kind == Kind::None || prev == now {
        return Damage::none();
    }
    let (lo, hi) = if prev < now { (prev, now) } else { (now, prev) };
    let lo = lo.clamp(0.0, 1.0);
    let hi = hi.clamp(0.0, 1.0);
    if lo == hi {
        return Damage::none();
    }
    match kind {
        Kind::None => Damage::none(),
        Kind::Fade => Damage::whole(dims),
        Kind::Wipe => wipe_damage(lo, hi, dims, angle_deg),
        Kind::Grow => grow_damage(hi, dims, pos),
    }
}

/// The band between the wipe edges at `lo` and `hi`: the bounding box of
/// the rect corners inside the band plus the edges' intersections with the
/// rect's sides, clipped to the buffer. Tight for axis-aligned angles.
fn wipe_damage(lo: f64, hi: f64, dims: (u32, u32), angle_deg: f64) -> Damage {
    let (dx, dy) = direction(angle_deg);
    let (_, _, min, span) = wipe_range(dx, dy, dims);
    let edge = |e: f64| min - 1.0 + e * (span + 2.0);
    let (s0, s1) = (edge(lo), edge(hi));
    let (w, h) = (f64::from(dims.0), f64::from(dims.1));
    let proj = |x: f64, y: f64| x.mul_add(dx, y * dy);
    // Corners inside the band, then each edge's intersections with the
    // four sides: at most twelve points, on the stack, never the heap.
    let mut points = [(0.0_f64, 0.0_f64); 12];
    let mut n = 0;
    let mut push = |x: f64, y: f64| {
        if n < points.len() {
            points[n] = (x, y);
            n += 1;
        }
    };
    // Corners inside the band.
    for (x, y) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
        let p = proj(x, y);
        if p >= s0 && p <= s1 {
            push(x, y);
        }
    }
    // Each edge's intersections with the four sides.
    for s in [s0, s1] {
        // x = 0 and x = w (needs dy != 0).
        if dy.abs() > 1e-9 {
            for x in [0.0, w] {
                let y = (s - x * dx) / dy;
                if y >= 0.0 && y <= h {
                    push(x, y);
                }
            }
        }
        // y = 0 and y = h (needs dx != 0).
        if dx.abs() > 1e-9 {
            for y in [0.0, h] {
                let x = (s - y * dy) / dx;
                if x >= 0.0 && x <= w {
                    push(x, y);
                }
            }
        }
    }
    if n == 0 {
        return Damage::none();
    }
    let points = &points[..n];
    let min_x = points
        .iter()
        .map(|&(x, _)| x)
        .fold(f64::INFINITY, f64::min)
        .max(0.0);
    let max_x = points
        .iter()
        .map(|&(x, _)| x)
        .fold(f64::NEG_INFINITY, f64::max)
        .min(w);
    let min_y = points
        .iter()
        .map(|&(_, y)| y)
        .fold(f64::INFINITY, f64::min)
        .max(0.0);
    let max_y = points
        .iter()
        .map(|&(_, y)| y)
        .fold(f64::NEG_INFINITY, f64::max)
        .min(h);
    // Pixel coverage: any pixel the band touches is damaged. Floor the low
    // side, ceil the high side; an empty cover means no pixel center moved
    // sides, so nothing changed.
    let x0 = (min_x.floor() as u32).min(dims.0);
    let x1 = (max_x.ceil() as u32).min(dims.0);
    let y0 = (min_y.floor() as u32).min(dims.1);
    let y1 = (max_y.ceil() as u32).min(dims.1);
    if x1 <= x0 || y1 <= y0 {
        return Damage::none();
    }
    Damage::one(Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}

/// The grown disc's bounding box at `hi` (the ring between `lo` and `hi`
/// sits inside it): a superset of the changed pixels, tight within a
/// pixel.
fn grow_damage(hi: f64, dims: (u32, u32), pos: (f64, f64)) -> Damage {
    let (cx, cy, maxr) = grow_range(pos, dims);
    let r = hi * (maxr + 2.0) - 1.0;
    if r <= 0.0 {
        return Damage::none();
    }
    let (w, h) = (f64::from(dims.0), f64::from(dims.1));
    let x0 = ((cx - r).floor().max(0.0) as u32).min(dims.0);
    let x1 = ((cx + r).ceil().min(w) as u32).min(dims.0);
    let y0 = ((cy - r).floor().max(0.0) as u32).min(dims.1);
    let y1 = ((cy + r).ceil().min(h) as u32).min(dims.1);
    if x1 <= x0 || y1 <= y0 {
        return Damage::none();
    }
    Damage::one(Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    })
}
