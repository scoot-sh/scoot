//! From modules' views to pixels, per output: measure, lay out, paint, and
//! say what changed. Pure (no Wayland), so every repaint rule is a unit
//! test.
//!
//! ## What is redrawn
//!
//! Each module has a **revision**, bumped when it reports a changed view
//! ([`crate::modules::Update::Changed`]). An output's [`Scene`] holds each
//! module's view as last asked, and the revision it is at; it asks a module
//! for a new view only when its revision moved, or when the output's scale
//! or width changed (everything is measured again then).
//!
//! The layout (each module's [`Span`]) is recomputed after that, and it has
//! a revision of its own, bumped only when a span actually moved: a clock
//! going from `3:07` to `3:08` keeps its width in a font with tabular
//! digits, so its span stays.
//!
//! A buffer's pixels are brought up to date from a [`Record`] of what they
//! show: all of it when the frame or the layout differs (a new size, a
//! module that grew), else only the modules whose revision differs. The
//! compositor is told the same way, against the record of what the surface
//! shows: the whole bar, or only the changed modules' spans. The two
//! records differ once there are two buffers: a buffer written two draws
//! ago has missed one draw's changes, and gets them now.

use crate::density::{DENOMINATOR, Scale, scaled_length};
use crate::layout::{self, Section};
use crate::modules::{OutputView, Placed, View};
use crate::outputs::Frame;
use crate::paint::{Canvas, Span};
use crate::text::Text;
use crate::theme::Theme;

#[cfg(test)]
mod tests;

/// A revision no module has: what a scene or record starts at.
const NEVER: u64 = u64::MAX;

/// How the bar draws modules: colors, text size and gaps, all logical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub theme: Theme,
    /// The em, in logical pixels.
    pub font_size: u32,
    pub padding: u32,
    pub spacing: u32,
}

/// A logical length at `scale`, in device pixels.
/// 0 stays 0 (`scaled_length` makes every other length at least 1).
fn device(length: u32, scale: Scale) -> u32 {
    if length == 0 {
        return 0;
    }
    match scale {
        Scale::Integer(factor) => length.saturating_mul(factor.max(1)),
        Scale::Fractional(v120) => scaled_length(length, v120).unwrap_or(u32::MAX),
    }
}

/// The em at `scale`, in device pixels.
fn em(font_size: u32, scale: Scale) -> f32 {
    let factor = match scale {
        Scale::Integer(factor) => factor.max(1) as f32,
        Scale::Fractional(v120) => v120.max(1) as f32 / DENOMINATOR as f32,
    };
    font_size as f32 * factor
}

/// One output's modules as last measured and laid out.
#[derive(Debug, Default)]
pub struct Scene {
    views: Vec<View>,
    /// The revision each view is at.
    revisions: Vec<u64>,
    /// Device pixels, padding included; 0 for an empty view.
    widths: Vec<u32>,
    spans: Vec<Span>,
    /// The next layout, compared with `spans` before it replaces them.
    next: Vec<Span>,
    sections: Vec<Section>,
    layout: u64,
    /// The scale and bar width everything was measured at.
    measured: Option<(Scale, u32)>,
}

impl Scene {
    /// A scene for `placed`: its vectors are sized once, here.
    pub fn new(placed: &[Placed]) -> Self {
        let count = placed.len();
        Self {
            views: vec![View::default(); count],
            revisions: vec![NEVER; count],
            widths: vec![0; count],
            spans: vec![Span::default(); count],
            next: vec![Span::default(); count],
            sections: placed.iter().map(|p| p.section).collect(),
            layout: 0,
            measured: None,
        }
    }

    /// Whether some module changed since `shown` was drawn.
    pub fn stale(placed: &[Placed], shown: &Record) -> bool {
        placed
            .iter()
            .zip(&shown.revisions)
            .any(|(placed, &shown)| placed.revision != shown)
    }

    /// Brings the views, widths and spans up to date for a bar `width`
    /// device pixels wide at `scale`.
    pub fn update(
        &mut self,
        placed: &[Placed],
        output: &OutputView<'_>,
        text: Option<&Text>,
        style: &Style,
        scale: Scale,
        width: u32,
    ) {
        let all = self.measured != Some((scale, width));
        self.measured = Some((scale, width));
        let em = em(style.font_size, scale);
        let padding = device(style.padding, scale).saturating_mul(2);
        let entries = placed
            .iter()
            .zip(&mut self.views)
            .zip(&mut self.revisions)
            .zip(&mut self.widths);
        let mut resized = all;
        for (((placed, view), revision), measured) in entries {
            if !all && *revision == placed.revision {
                continue;
            }
            view.clear();
            placed.module.view(output, view);
            *revision = placed.revision;
            let width = match text {
                Some(text) if !view.is_empty() => text
                    .measure(view.icon(), view.text(), em)
                    .saturating_add(padding)
                    .max(1),
                // Nothing to show, or no font to show it with.
                _ => 0,
            };
            resized |= width != *measured;
            *measured = width;
        }
        // The layout is recomputed only when a width (or the bar) changed.
        if !resized {
            return;
        }
        layout::arrange(
            &self.sections,
            &self.widths,
            device(style.spacing, scale),
            width,
            &mut self.next,
        );
        if self.next != self.spans {
            std::mem::swap(&mut self.next, &mut self.spans);
            self.layout = self.layout.wrapping_add(1);
        }
    }

    /// Each module's span, for tests (and, later, hit tests and the agent
    /// interface's `layout`).
    #[cfg(test)]
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// Each module's view as last asked.
    #[cfg(test)]
    pub fn views(&self) -> &[View] {
        &self.views
    }
}

/// What a buffer, or the surface, shows: the frame and layout it was drawn
/// at, and each module's revision in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    at: Option<(Frame, u64)>,
    revisions: Vec<u64>,
}

impl Record {
    /// A record of nothing drawn, for `count` modules.
    pub fn new(count: usize) -> Self {
        Self {
            at: None,
            revisions: vec![NEVER; count],
        }
    }

    /// Forgets what is shown: the next draw is whole.
    pub fn reset(&mut self) {
        self.at = None;
        self.revisions.fill(NEVER);
    }

    /// Makes this record say what `scene` shows at `frame`.
    fn set(&mut self, scene: &Scene, frame: Frame) {
        self.at = Some((frame, scene.layout));
        self.revisions.copy_from_slice(&scene.revisions);
    }
}

/// Paints `scene` at `frame` into `canvas`, whose pixels `record` says it
/// shows, and updates `record`. Only what differs is painted.
pub fn paint(
    canvas: &mut Canvas<'_>,
    record: &mut Record,
    scene: &Scene,
    text: Option<&mut Text>,
    style: &Style,
    frame: Frame,
) {
    let whole = record.at != Some((frame, scene.layout));
    if whole {
        canvas.fill_span(
            Span {
                x: 0,
                width: canvas.width(),
            },
            style.theme.background,
        );
    }
    let Some(text) = text else {
        // No font: no module has any width, so nothing but the background.
        record.set(scene, frame);
        return;
    };
    let em = em(style.font_size, frame.scale);
    let padding = device(style.padding, frame.scale);
    let baseline = text.metrics(em).baseline(canvas.height());
    let entries = scene
        .views
        .iter()
        .zip(&scene.spans)
        .zip(&scene.revisions)
        .zip(&record.revisions);
    for (((view, &span), &revision), &painted) in entries {
        if !whole && revision == painted {
            continue;
        }
        if !whole {
            canvas.fill_span(span, style.theme.background);
        }
        if span.width == 0 {
            continue;
        }
        text.draw(
            canvas,
            view.icon(),
            view.text(),
            em,
            i64::from(span.x) + i64::from(padding),
            baseline,
            style.theme.class(view.class()),
            span,
        );
    }
    record.set(scene, frame);
}

/// The spans to damage on a surface showing `shown` so it shows `scene` at
/// `frame`, into `damage` (cleared first); `true` for the whole surface.
/// Updates `shown` to match.
pub fn damage(shown: &mut Record, scene: &Scene, frame: Frame, damage: &mut Vec<Span>) -> bool {
    damage.clear();
    let whole = shown.at != Some((frame, scene.layout));
    if !whole {
        for ((&span, &revision), &was) in scene
            .spans
            .iter()
            .zip(&scene.revisions)
            .zip(&shown.revisions)
        {
            if revision != was && span.width > 0 {
                damage.push(span);
            }
        }
    }
    shown.set(scene, frame);
    whole
}
