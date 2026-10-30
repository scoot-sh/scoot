//! Cards in motion: entrances, exits and the restack between layouts.
//!
//! The layout stays the truth. [`layout`](super::layout) says where every
//! card belongs; this module only decides where a card is *drawn* between
//! two layouts, the way the compositor's slide animation moves a window
//! while the arrangement is already at its target. Nothing here is hit by
//! the bus: a closed notification is closed to every client the moment it
//! closes, and its exit is the picture catching up.
//!
//! # What moves
//!
//! - A **new card** enters (`card.enter`): it fades in and, by style, slides
//!   in from the anchored edge, grows, or drops in.
//! - A **closed card** leaves (`card.exit`) as a *ghost*: a picture of the
//!   card as it was, run backwards through the same style while the layout
//!   has already moved on. A ghost is never hit-tested and has no input
//!   region.
//! - A **card whose place moved** (a neighbor closed or arrived) settles
//!   from where it was seen to where the layout wants it, on the look's
//!   curve. A new target mid-flight continues from what is on screen.
//!
//! # How it is drawn
//!
//! A moving card is rasterized once into a pooled [`Layer`] when its motion
//! begins (or when its content or hover changes), then composited each
//! frame with an offset, a scale and an alpha: no glyph work per frame. A
//! card at rest is not here at all and paints through the unchanged static
//! path, so a look with `motion = none`, and a stack between animations,
//! paint the exact bytes they always did.
//!
//! # Coordinates
//!
//! Positions that must survive the surface changing height are kept
//! relative to the edge the stack is anchored to (`e`: the card's top,
//! counted from the surface top for a top anchor, and from the surface
//! bottom, so negative, for a bottom one). While anything moves the
//! surface holds the height it needs for the whole motion
//! ([`Motion::hold`]) and drops back to the layout's when it ends.
//!
//! # Allocation
//!
//! [`Motion::retarget`] allocates (clones, layers): it runs on a content
//! change. Everything a frame calls does not, and a test pins it:
//! [`settle`](Motion::settle) (while something is still moving),
//! [`pose`](Motion::pose), [`paint_layers`](Motion::paint_layers) when no
//! layer is stale, [`compose`](Motion::compose),
//! [`input_rect`](Motion::input_rect) and [`unproject`](Motion::unproject).
//! Landing frees the pool once, on the frame after the last card stops.
//!
//! Nothing is read from a clock: every query takes `now`. The popup passes
//! the frame it is drawing, so a late frame jumps forward and a frame past
//! the deadline lands.

use std::time::Instant;

use scootui::motion::{Settle, Timing};
use scootui::paint::{Canvas, Layer};

use super::layout::{Card, Rect, Stack};
use crate::config::Anchor;
use crate::look::{CardMotion, MotionTokens};

/// How far a sliding card travels, in logical pixels.
pub const SLIDE_PX: f32 = 40.0;
/// How far a dropping card falls, in logical pixels.
pub const DROP_PX: f32 = 24.0;
/// The size a scaling card grows from (a fraction of its own).
pub const SCALE_FROM: f32 = 0.88;
/// At most this many leaving cards are kept: a flood of closes snaps the
/// oldest ghost rather than holding an unbounded number of pictures.
pub const MAX_GHOSTS: usize = 8;
/// The spring's peak past its target (about 9.5%) plus a little slack: the
/// surface holds this much more than the distance a card travels.
const OVERSHOOT: f32 = 0.11;

/// Where one card is drawn relative to where the layout put it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Horizontal offset, in logical pixels.
    pub dx: i32,
    /// Vertical offset, in logical pixels.
    pub dy: i32,
    /// Opacity, 0 to 255.
    pub alpha: u8,
    /// Size about the card's center, `1.0` at rest.
    pub scale: f32,
}

impl Pose {
    /// At rest: where the layout put it, whole and opaque.
    pub const REST: Pose = Pose {
        dx: 0,
        dy: 0,
        alpha: 255,
        scale: 1.0,
    };

    /// The pose of a card `away` of the way to its off-stage state: `0.0`
    /// is at rest, `1.0` is fully gone. A spring's overshoot makes `away`
    /// briefly negative, which pushes the card a little past rest (the
    /// opacity stays clamped).
    fn of(style: CardMotion, away: f32, right: bool) -> Pose {
        let alpha = ((1.0 - away.clamp(0.0, 1.0)) * 255.0).round() as u8;
        let side = if right { 1.0 } else { -1.0 };
        let mut pose = Pose {
            alpha,
            ..Pose::REST
        };
        match style {
            CardMotion::Fade => {}
            CardMotion::Slide => pose.dx = (side * SLIDE_PX * away).round() as i32,
            CardMotion::Drop => pose.dy = (-DROP_PX * away).round() as i32,
            CardMotion::Scale => pose.scale = (1.0 - (1.0 - SCALE_FROM) * away).max(0.05),
        }
        pose
    }

    /// Whether this is the pose of a card that has not moved.
    pub fn is_rest(&self) -> bool {
        *self == Pose::REST
    }
}

/// One card whose motion is in flight.
struct Track {
    id: u32,
    /// Where the layout wants the card's top, relative to the anchored
    /// edge.
    e: i32,
    /// How far the card is drawn from `e`, settling to zero.
    off: Settle,
    /// When its entrance began, while it runs.
    enter: Option<Instant>,
    picture: Picture,
}

/// A card leaving.
struct Ghost {
    /// Where its top was drawn when it left, relative to the anchored
    /// edge.
    e: i32,
    start: Instant,
    /// How far gone it already was (it closed mid-enter): the exit
    /// continues from there.
    away0: f32,
    style: CardMotion,
    picture: Picture,
}

/// A card rasterized into a layer, and what it was rasterized with.
struct Picture {
    /// The card with its origin at the layer's corner.
    local: Card,
    /// The layer's corner in the stack's coordinates.
    origin: (u32, u32),
    layer: Layer,
    /// The button the layer was painted with hovered, if any.
    painted_hover: Option<usize>,
    /// Whether the layer must be painted again before it is composited.
    stale: bool,
}

impl Picture {
    /// Shapes the picture for `card` as `stack` lays it out: the layer
    /// sized to the card and its shadow's room, the card translated to the
    /// layer's corner, and the layer due a repaint. Called when the card
    /// first moves and again when a new layout changes its place or its
    /// size (a replace can add a line). False when the card is too large
    /// to move.
    fn refit(&mut self, card: &Card, stack: &Stack) -> bool {
        let pad = stack.pad;
        let (Some(width), Some(height)) = (
            card.rect
                .width
                .checked_add(pad.left)
                .and_then(|w| w.checked_add(pad.right)),
            card.rect
                .height
                .checked_add(pad.top)
                .and_then(|h| h.checked_add(pad.bottom)),
        ) else {
            return false;
        };
        if !self.layer.begin(width, height) {
            return false;
        }
        self.origin = (
            card.rect.x.saturating_sub(pad.left),
            card.rect.y.saturating_sub(pad.top),
        );
        self.local = card.translated(self.origin.0, self.origin.1);
        self.painted_hover = None;
        self.stale = true;
        true
    }
}

/// The motion state of one stack: what is in flight and the layers it is
/// drawn through. Empty (and holding no memory) whenever nothing moves.
pub struct Motion {
    timing: Timing,
    exit_timing: Timing,
    enter: CardMotion,
    exit: CardMotion,
    bottom: bool,
    right: bool,
    tracks: Vec<Track>,
    ghosts: Vec<Ghost>,
    /// Layers not in use, kept while something moves.
    pool: Vec<Layer>,
    /// The surface height the current motion needs.
    hold: u32,
    /// The height of the surface the cards are drawn on, `0` until the
    /// popup says ([`fit`](Self::fit)): a card settling stops at the
    /// surface's edge instead of springing past it.
    surface: u32,
}

impl Motion {
    /// The motion `tokens` ask for on a stack anchored at `anchor`.
    pub fn new(tokens: &MotionTokens, anchor: Anchor) -> Self {
        let timing = tokens.timing();
        Self {
            timing,
            exit_timing: timing.without_overshoot(),
            enter: tokens.enter,
            exit: tokens.exit,
            bottom: matches!(anchor, Anchor::BottomLeft | Anchor::BottomRight),
            right: matches!(anchor, Anchor::TopRight | Anchor::BottomRight),
            tracks: Vec::new(),
            ghosts: Vec::new(),
            pool: Vec::new(),
            hold: 0,
            surface: 0,
        }
    }

    /// Tells the motion how tall the surface it draws on is. A spring
    /// overshoots its target; toward the edge the stack is anchored to
    /// there is no surface left to overshoot into, so a card is held at
    /// the edge there (the far side has the room `hold` reserves).
    pub fn fit(&mut self, surface: u32) {
        self.surface = surface;
    }

    /// Whether the look moves anything at all. When false, every other
    /// method is a no-op and nothing was ever allocated.
    pub fn enabled(&self) -> bool {
        self.timing.active()
    }

    /// Whether something is in flight: the popup keeps asking for frames
    /// while this holds, and stops the moment it does not.
    pub fn active(&self) -> bool {
        !self.tracks.is_empty() || !self.ghosts.is_empty()
    }

    /// The surface height the motion in flight needs, `0` at rest. The
    /// popup makes the surface at least this tall while [`active`](Self::active).
    pub fn hold(&self) -> u32 {
        if self.active() { self.hold } else { 0 }
    }

    /// Whether the stack is anchored to the bottom edge.
    pub fn bottom(&self) -> bool {
        self.bottom
    }

    /// Drops everything in flight and every pooled layer: the motion
    /// snaps. For a surface that went away, a re-theme, a new anchor.
    pub fn reset(&mut self) {
        self.tracks.clear();
        self.ghosts.clear();
        self.pool.clear();
        self.hold = 0;
        self.surface = 0;
    }

    /// A card's top relative to the anchored edge.
    fn edge(&self, rect_y: u32, stack_height: u32) -> i32 {
        let y = i32::try_from(rect_y).unwrap_or(i32::MAX);
        if self.bottom {
            y.saturating_sub(i32::try_from(stack_height).unwrap_or(i32::MAX))
        } else {
            y
        }
    }

    /// A layer sized for `card` and its shadow's room, from the pool.
    /// `None` when the card is too large to move (it then snaps).
    fn picture(&mut self, card: &Card, stack: &Stack) -> Option<Picture> {
        let layer = self.pool.pop().unwrap_or_default();
        let mut picture = Picture {
            local: card.clone(),
            origin: (0, 0),
            layer,
            painted_hover: None,
            stale: true,
        };
        picture.refit(card, stack).then_some(picture)
    }

    /// Takes the layer back into the pool, capped so a burst does not
    /// leave a pile of large buffers behind.
    fn recycle(&mut self, picture: Picture) {
        if self.pool.len() < MAX_GHOSTS + 8 {
            self.pool.push(picture.layer);
        }
    }

    /// Notes that the layout changed from `old` to `new` at `now`: new
    /// cards enter, cards that left become ghosts, cards that moved settle
    /// from where they are seen. Allocates (clones, layers): this runs on
    /// a content change, never per frame.
    pub fn retarget(&mut self, old: &Stack, new: &Stack, now: Instant) {
        if !self.enabled() {
            return;
        }
        let tracks = std::mem::take(&mut self.tracks);
        let mut previous: Vec<Option<Track>> = tracks.into_iter().map(Some).collect();
        for card in &new.cards {
            let e = self.edge(card.rect.y, new.height);
            let existing = previous
                .iter_mut()
                .find(|track| track.as_ref().is_some_and(|track| track.id == card.id))
                .and_then(Option::take);
            let track = match existing {
                Some(mut track) => {
                    // Continue from what is on screen, not from where the
                    // card was headed.
                    let seen = track.e as f32 + track.off.value(&self.timing, now);
                    track.e = e;
                    track.off = Settle::new(seen - e as f32, now);
                    if track.picture.refit(card, new) {
                        Some(track)
                    } else {
                        // Grew too large to move: it snaps.
                        self.recycle(track.picture);
                        None
                    }
                }
                None => match old.cards.iter().find(|before| before.id == card.id) {
                    Some(before) => {
                        let from = self.edge(before.rect.y, old.height);
                        if from == e {
                            None
                        } else {
                            self.picture(card, new).map(|picture| Track {
                                id: card.id,
                                e,
                                off: Settle::new((from - e) as f32, now),
                                enter: None,
                                picture,
                            })
                        }
                    }
                    None => self.picture(card, new).map(|picture| Track {
                        id: card.id,
                        e,
                        off: Settle::new(0.0, now),
                        enter: Some(now),
                        picture,
                    }),
                },
            };
            if let Some(track) = track {
                self.tracks.push(track);
            }
        }
        // What was in flight or on screen and is no longer in the stack
        // leaves.
        for before in &old.cards {
            if new.cards.iter().any(|card| card.id == before.id) {
                continue;
            }
            let track = previous
                .iter_mut()
                .find(|track| track.as_ref().is_some_and(|track| track.id == before.id))
                .and_then(Option::take);
            let (e, away0) = match &track {
                Some(track) => {
                    let e = track.e as f32 + track.off.value(&self.timing, now);
                    let away = track
                        .enter
                        .map_or(0.0, |start| 1.0 - self.timing.eased(start, now));
                    (e.round() as i32, away.clamp(0.0, 1.0))
                }
                None => (self.edge(before.rect.y, old.height), 0.0),
            };
            let picture = match track {
                Some(track) => Some(track.picture),
                None => self.picture(before, old),
            };
            if let Some(picture) = picture {
                if self.ghosts.len() >= MAX_GHOSTS {
                    let oldest = self.ghosts.remove(0);
                    self.recycle(oldest.picture);
                }
                self.ghosts.push(Ghost {
                    e,
                    start: now,
                    away0,
                    style: self.exit,
                    picture,
                });
            }
        }
        for track in previous.into_iter().flatten() {
            self.recycle(track.picture);
        }
        self.hold = self.hold.max(self.reach(old, new));
    }

    /// The surface height every card in flight needs at either end of its
    /// travel, with the spring's overshoot.
    fn reach(&self, old: &Stack, new: &Stack) -> u32 {
        let mut need = i64::from(old.height.max(new.height));
        let pad = new.pad;
        let mut take = |e: f32, height: u32, slack: f32| {
            let e = i64::from(e.round() as i32);
            let slack = i64::from(slack.round() as i32);
            let needed = if self.bottom {
                i64::from(pad.top) - e
            } else {
                e + i64::from(height) + i64::from(pad.bottom)
            };
            need = need.max(needed + slack);
        };
        for track in &self.tracks {
            let height = track.picture.local.rect.height;
            let travel = track.off.from();
            let slack = travel.abs() * OVERSHOOT;
            take(track.e as f32, height, slack);
            take(track.e as f32 + travel, height, slack);
        }
        for ghost in &self.ghosts {
            take(ghost.e as f32, ghost.picture.local.rect.height, 0.0);
        }
        u32::try_from(need.clamp(0, i64::from(u32::MAX))).unwrap_or(u32::MAX)
    }
}

impl Motion {
    /// Drops what has landed at `now` and returns its layers to the pool.
    /// When nothing is left, the pool and the held height go too: an idle
    /// stack holds no motion memory. Returns whether anything landed.
    pub fn settle(&mut self, now: Instant) -> bool {
        let in_flight = self.tracks.len() + self.ghosts.len();
        let timing = self.timing;
        let exit_timing = self.exit_timing;
        let mut index = 0;
        while index < self.tracks.len() {
            let track = &self.tracks[index];
            let entering = track.enter.is_some_and(|start| !timing.settled(start, now));
            if entering || !track.off.settled(&timing, now) {
                index += 1;
            } else {
                let track = self.tracks.remove(index);
                self.recycle(track.picture);
            }
        }
        let mut index = 0;
        while index < self.ghosts.len() {
            if exit_timing.settled(self.ghosts[index].start, now) {
                let ghost = self.ghosts.remove(index);
                self.recycle(ghost.picture);
            } else {
                index += 1;
            }
        }
        if !self.active() {
            self.pool.clear();
            self.pool.shrink_to_fit();
            self.hold = 0;
        }
        self.tracks.len() + self.ghosts.len() != in_flight
    }

    /// How a card in flight is drawn at `now`, or `None` for a card at
    /// rest (it paints through the static path).
    pub fn pose(&self, id: u32, now: Instant) -> Option<Pose> {
        let track = self.tracks.iter().find(|track| track.id == id)?;
        let mut pose = match track.enter {
            Some(start) if !self.timing.settled(start, now) => {
                Pose::of(self.enter, 1.0 - self.timing.eased(start, now), self.right)
            }
            _ => Pose::REST,
        };
        let off = track.off.value(&self.timing, now).round() as i32;
        pose.dy = pose.dy.saturating_add(self.keep_on_surface(track, off));
        Some(pose)
    }

    /// `off` limited so the card, at its target plus `off`, stays on the
    /// surface. Only a spring's overshoot toward the anchored edge ever
    /// reaches the limit: a card never travels farther than the surface
    /// held for it.
    fn keep_on_surface(&self, track: &Track, off: i32) -> i32 {
        if self.surface == 0 {
            return off;
        }
        let surface = i64::from(self.surface);
        let top = if self.bottom {
            surface + i64::from(track.e)
        } else {
            i64::from(track.e)
        };
        let height = i64::from(track.picture.local.rect.height);
        let (lowest, highest) = (-top, surface - top - height);
        if lowest > highest {
            return off;
        }
        i64::from(off).clamp(lowest, highest) as i32
    }

    /// Whether `id` is drawn through a layer this frame.
    pub fn moving(&self, id: u32) -> bool {
        self.tracks.iter().any(|track| track.id == id)
    }

    /// Paints every layer that is stale or was painted with a different
    /// hover through `paint`, which receives the card (at the layer's
    /// origin), the layer to clear and paint into, and the hovered button
    /// of this card. Allocates nothing itself.
    pub fn paint_layers(
        &mut self,
        hover: Option<(u32, Option<usize>)>,
        mut paint: impl FnMut(&Card, &mut Layer, Option<usize>),
    ) {
        let hovered = |id: u32| match hover {
            Some((hovered, button)) if hovered == id => button,
            _ => None,
        };
        let pictures = self
            .tracks
            .iter_mut()
            .map(|track| (&mut track.picture, hovered(track.id)))
            .chain(
                self.ghosts
                    .iter_mut()
                    .map(|ghost| (&mut ghost.picture, None)),
            );
        for (picture, button) in pictures {
            if !picture.stale && picture.painted_hover == button {
                continue;
            }
            let (width, height) = (picture.layer.width(), picture.layer.height());
            if !picture.layer.begin(width, height) {
                continue;
            }
            paint(&picture.local, &mut picture.layer, button);
            picture.painted_hover = button;
            picture.stale = false;
        }
    }

    /// Composites the leaving cards, then the moving ones, onto `canvas`
    /// at `now`. The static cards were painted before this.
    pub fn compose(&self, canvas: &mut Canvas<'_>, now: Instant) {
        let surface = canvas.height();
        for ghost in &self.ghosts {
            let leaving = if self.exit_timing.settled(ghost.start, now) {
                1.0
            } else {
                ghost.away0 + (1.0 - ghost.away0) * self.exit_timing.eased(ghost.start, now)
            };
            let pose = Pose::of(ghost.style, leaving, self.right);
            blit(
                canvas,
                &ghost.picture,
                &pose,
                self.layer_top(ghost.e, &ghost.picture, surface),
            );
        }
        for track in &self.tracks {
            let Some(pose) = self.pose(track.id, now) else {
                continue;
            };
            blit(
                canvas,
                &track.picture,
                &pose,
                self.layer_top(track.e, &track.picture, surface),
            );
        }
    }

    /// Where a picture's layer top lies on a surface `surface` pixels
    /// tall, for a card whose top is `e` from the anchored edge: the
    /// card's own top less the shadow room above it.
    fn layer_top(&self, e: i32, picture: &Picture, surface: u32) -> i32 {
        let top = if self.bottom {
            i32::try_from(surface).unwrap_or(i32::MAX).saturating_add(e)
        } else {
            e
        };
        top.saturating_sub(i32::try_from(picture.local.rect.y).unwrap_or(0))
    }

    /// How far a static card lies below its layout coordinates on a
    /// surface `surface` pixels tall: the held height a bottom-anchored
    /// surface has over the layout's own.
    pub fn shift(&self, surface: u32, stack_height: u32) -> u32 {
        if self.bottom {
            surface.saturating_sub(stack_height)
        } else {
            0
        }
    }

    /// Maps a pointer position on the surface to the layout coordinates of
    /// the card `card` is drawn as, undoing its pose (and a bottom
    /// anchor's held height), so the layout's own rectangles and buttons
    /// hit-test the card where it is seen. A card at rest only loses the
    /// shift.
    pub fn unproject(&self, card: &Card, px: i64, py: i64, shift: u32, now: Instant) -> (i64, i64) {
        let py = py - i64::from(shift);
        let Some(pose) = self.pose(card.id, now) else {
            return (px, py);
        };
        let rect = card.rect;
        let (cx, cy) = (
            i64::from(rect.x) + i64::from(rect.width) / 2,
            i64::from(rect.y) + i64::from(rect.height) / 2,
        );
        let scale = f64::from(pose.scale).max(0.05);
        let unscale =
            |p: i64, c: i64, d: i32| c + (((p - i64::from(d) - c) as f64) / scale).round() as i64;
        (unscale(px, cx, pose.dx), unscale(py, cy, pose.dy))
    }

    /// The rectangle `card` covers on the surface at `now`, clipped to
    /// `surface` (width, height), for the input region: the pose applied
    /// to the layout rectangle, plus a bottom anchor's shift. `None` when
    /// nothing of it is on the surface.
    pub fn input_rect(
        &self,
        card: &Card,
        shift: u32,
        surface: (u32, u32),
        now: Instant,
    ) -> Option<Rect> {
        let rect = card.rect;
        let pose = self.pose(card.id, now).unwrap_or(Pose::REST);
        let scale = f64::from(pose.scale).max(0.05);
        let (w, h) = (
            (f64::from(rect.width) * scale).round() as i64,
            (f64::from(rect.height) * scale).round() as i64,
        );
        let x = i64::from(rect.x) + i64::from(pose.dx) + (i64::from(rect.width) - w) / 2;
        let y = i64::from(rect.y)
            + i64::from(pose.dy)
            + i64::from(shift)
            + (i64::from(rect.height) - h) / 2;
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = (
            (x + w).min(i64::from(surface.0)),
            (y + h).min(i64::from(surface.1)),
        );
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        Some(Rect {
            x: x0 as u32,
            y: y0 as u32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        })
    }
}

/// Composites `picture` at its pose, its layer's top at `top` (before the
/// pose's own vertical offset).
fn blit(canvas: &mut Canvas<'_>, picture: &Picture, pose: &Pose, top: i32) {
    let left = i32::try_from(picture.origin.0).unwrap_or(0);
    picture.layer.composite(
        canvas,
        left.saturating_add(pose.dx),
        top.saturating_add(pose.dy),
        pose.scale,
        pose.alpha,
    );
}

#[cfg(test)]
mod tests;
