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
use crate::modules::{CustomDraw, OutputView, Placed, View};
use crate::outputs::{Frame, Size};
use crate::paint::{Canvas, Corners, Span};
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
    /// The width of the line drawn between neighbouring modules in a
    /// section, logical pixels; 0 draws none.
    pub separator: u32,
    /// The bar's corner radius, in logical pixels; 0 is square.
    pub radius: u32,
    /// The background's alpha, 255 (opaque) down to 0.
    pub opacity: u8,
}

impl Style {
    /// Whether the buffer needs an alpha channel: a translucent
    /// background, or corners with nothing behind them. Otherwise the bar
    /// is `XRGB8888`, as it was before either option existed.
    pub fn translucent(&self) -> bool {
        self.opacity < u8::MAX || self.radius > 0
    }

    /// How far in from each edge the surface is opaque, in logical pixels,
    /// for the opaque region: `Some(0)` all of it, `Some(n)` all but the
    /// corner squares of side `n`, `None` none (a translucent background
    /// is blended by the compositor). One more than the radius: the
    /// corner's antialiased edge, at a fractional scale, can round half a
    /// device pixel past it.
    pub fn opaque_inset(&self) -> Option<u32> {
        (self.opacity == u8::MAX).then(|| match self.radius {
            0 => 0,
            radius => radius.saturating_add(1),
        })
    }

    /// The corner radius in device pixels at `scale`, for a bar `extent`
    /// device pixels: cut back to what the bar can hold.
    fn corners(&self, scale: Scale, extent: Size) -> u32 {
        device(self.radius, scale)
            .min(extent.width / 2)
            .min(extent.height / 2)
    }
}

/// A logical length at `scale`, in device pixels.
/// 0 stays 0 (`scaled_length` makes every other length at least 1).
pub(crate) fn device(length: u32, scale: Scale) -> u32 {
    if length == 0 {
        return 0;
    }
    match scale {
        Scale::Integer(factor) => length.saturating_mul(factor.max(1)),
        Scale::Fractional(v120) => scaled_length(length, v120).unwrap_or(u32::MAX),
    }
}

/// The em at `scale`, in device pixels.
pub(crate) fn em(font_size: u32, scale: Scale) -> f32 {
    let factor = match scale {
        Scale::Integer(factor) => factor.max(1) as f32,
        Scale::Fractional(v120) => v120.max(1) as f32 / DENOMINATOR as f32,
    };
    font_size as f32 * factor
}

/// A module an output shows, and the section it goes in there: outputs
/// place the one set of started modules differently (`crate::policy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    /// Index into the started modules.
    pub module: usize,
    pub section: Section,
    /// Logical pixels of room on each side of it, from the layout.
    pub margin: u32,
    /// Whether it is tinted under the pointer (`Placed::hoverable`).
    pub hover: bool,
}

/// The modules `layout` places, as members, in its order. A module that
/// did not start (unavailable) is skipped, as it takes no space.
pub fn members(layout: &layout::Layout, placed: &[Placed]) -> Vec<Member> {
    layout
        .placed()
        .filter_map(|(section, id)| {
            let module = placed.iter().position(|p| p.id == id)?;
            Some(Member {
                module,
                section,
                margin: layout.margin_of(id),
                hover: placed[module].hoverable(),
            })
        })
        .collect()
}

/// One output's modules as last measured and laid out. Everything here is
/// indexed by member (its place on this output), not by started module.
#[derive(Debug, Default)]
pub struct Scene {
    /// Each member's index into the started modules.
    modules: Vec<usize>,
    views: Vec<View>,
    /// The revision each view is at.
    revisions: Vec<u64>,
    /// Device pixels, padding included; 0 for an empty view.
    widths: Vec<u32>,
    /// Each member's margin, logical pixels (the layout's).
    margins: Vec<u32>,
    /// Each member's margin as last laid out, device pixels.
    margins_device: Vec<u32>,
    spans: Vec<Span>,
    /// The next layout, compared with `spans` before it replaces them.
    next: Vec<Span>,
    /// Which members are tinted under the pointer.
    hoverable: Vec<bool>,
    /// The pointer's x on this output's bar, device pixels, while it is
    /// over it.
    pointer: Option<u32>,
    /// The member the pointer is over and tints, if any.
    hover: Option<usize>,
    sections: Vec<Section>,
    layout: u64,
    /// The scale and bar width everything was measured at.
    measured: Option<(Scale, u32)>,
    /// The corners' coverage, rebuilt only when their radius changes.
    corners: Corners,
}

impl Scene {
    /// A scene showing the first `sections.len()` started modules, each in
    /// its section.
    #[cfg(test)]
    pub fn all(sections: &[Section]) -> Self {
        let members: Vec<Member> = sections
            .iter()
            .enumerate()
            .map(|(module, &section)| Member {
                module,
                section,
                margin: 0,
                hover: false,
            })
            .collect();
        Self::with_members(&members)
    }

    /// A scene showing the first `sections.len()` started modules, each in
    /// its section, with the members `hover` lists tinted under the pointer.
    #[cfg(test)]
    pub fn all_hoverable(sections: &[Section], hover: &[bool]) -> Self {
        let members: Vec<Member> = sections
            .iter()
            .zip(hover)
            .enumerate()
            .map(|(module, (&section, &hover))| Member {
                module,
                section,
                margin: 0,
                hover,
            })
            .collect();
        Self::with_members(&members)
    }

    /// A scene for `members`: its vectors are sized once, here.
    pub fn with_members(members: &[Member]) -> Self {
        let count = members.len();
        Self {
            modules: members.iter().map(|m| m.module).collect(),
            views: vec![View::default(); count],
            revisions: vec![NEVER; count],
            widths: vec![0; count],
            margins: members.iter().map(|m| m.margin).collect(),
            margins_device: vec![0; count],
            spans: vec![Span::default(); count],
            next: vec![Span::default(); count],
            hoverable: members.iter().map(|m| m.hover).collect(),
            pointer: None,
            hover: None,
            sections: members.iter().map(|m| m.section).collect(),
            layout: 0,
            measured: None,
            corners: Corners::NONE,
        }
    }

    /// Whether some module this scene shows changed since `shown` was
    /// drawn.
    pub fn stale(&self, placed: &[Placed], shown: &Record) -> bool {
        self.hover != shown.hover
            || self
                .modules
                .iter()
                .zip(&shown.revisions)
                .any(|(&module, &shown)| placed.get(module).is_some_and(|p| p.revision != shown))
    }

    /// The pointer is at `x` device pixels on this output's bar, or (`None`)
    /// not over it. The hovered member follows, and is drawn at the next
    /// draw ([`Scene::stale`] says so).
    pub fn set_pointer(&mut self, x: Option<u32>) {
        self.pointer = x;
        self.resolve_hover();
    }

    /// The member whose span holds device pixel `x` (an empty span holds
    /// none): where a press goes.
    pub fn member_at(&self, x: u32) -> Option<usize> {
        self.spans
            .iter()
            .position(|span| span.width > 0 && x >= span.x && x < span.end())
    }

    fn resolve_hover(&mut self) {
        self.hover = self
            .pointer
            .and_then(|x| self.member_at(x))
            .filter(|&member| self.hoverable.get(member).copied().unwrap_or(false));
    }

    /// Brings the views, widths and spans up to date for a bar `extent`
    /// device pixels big at `scale`.
    pub fn update(
        &mut self,
        placed: &[Placed],
        output: &OutputView<'_>,
        text: Option<&Text>,
        style: &Style,
        scale: Scale,
        extent: Size,
    ) {
        let width = extent.width;
        let radius = style.corners(scale, extent);
        if radius != self.corners.radius() {
            self.corners = Corners::new(radius);
        }
        let all = self.measured != Some((scale, width));
        self.measured = Some((scale, width));
        let em = em(style.font_size, scale);
        let padding = device(style.padding, scale).saturating_mul(2);
        let entries = self
            .modules
            .iter()
            .zip(&mut self.views)
            .zip(&mut self.revisions)
            .zip(&mut self.widths);
        let mut resized = all;
        for (((&module, view), revision), measured) in entries {
            let Some(placed) = placed.get(module) else {
                continue;
            };
            if !all && *revision == placed.revision {
                continue;
            }
            view.clear();
            placed.module.view(output, view);
            *revision = placed.revision;
            let width = match text {
                Some(text) if !view.is_empty() => {
                    let content = content_width(text, view, em);
                    // A module with a maximum width (the window title's
                    // `max-width`) never measures past it, so it yields
                    // the bar to the other modules; longer text is cut to
                    // the span, which the module draws (an ellipsis).
                    let capped = match placed.module.max_width() {
                        Some(max) => content.min(device(max, scale)),
                        None => content,
                    };
                    capped.saturating_add(padding).max(1)
                }
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
        // Clear the corners: the first module's padding starts at the
        // radius (its pill, which reaches half a padding out, stays past
        // it), so nothing is drawn in a corner square.
        let edge = radius.saturating_sub(padding / 4);
        for (device_margin, &margin) in self.margins_device.iter_mut().zip(&self.margins) {
            *device_margin = device(margin, scale);
        }
        layout::arrange(
            &self.sections,
            &self.widths,
            &self.margins_device,
            device(style.spacing, scale),
            edge,
            width,
            &mut self.next,
        );
        if self.next != self.spans {
            std::mem::swap(&mut self.next, &mut self.spans);
            self.layout = self.layout.wrapping_add(1);
            // What is under the pointer moved with the layout.
            self.resolve_hover();
        }
    }

    /// Each module's span, in the order the modules were placed: the hit
    /// test routes a pointer press by it.
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// Each module's view as last asked.
    #[cfg(test)]
    pub fn views(&self) -> &[View] {
        &self.views
    }

    /// The section started module `module` is in on this output, if it
    /// shows it.
    pub fn section_of(&self, module: usize) -> Option<Section> {
        let member = self.modules.iter().position(|&m| m == module)?;
        self.sections.get(member).copied()
    }

    /// The started module that member `index` is, for the click routing.
    pub fn module(&self, index: usize) -> Option<usize> {
        self.modules.get(index).copied()
    }

    /// One member's view as last asked, for the click routing.
    pub fn view(&self, index: usize) -> Option<&View> {
        self.views.get(index)
    }
}

/// What a buffer, or the surface, shows: the frame and layout it was drawn
/// at, and each module's revision in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    at: Option<(Frame, u64)>,
    revisions: Vec<u64>,
    /// The member drawn tinted under the pointer.
    hover: Option<usize>,
}

impl Record {
    /// A record of nothing drawn, for `count` modules.
    pub fn new(count: usize) -> Self {
        Self {
            at: None,
            revisions: vec![NEVER; count],
            hover: None,
        }
    }

    /// Forgets what is shown: the next draw is whole.
    pub fn reset(&mut self) {
        self.at = None;
        self.revisions.fill(NEVER);
        self.hover = None;
    }

    /// Makes this record say what `scene` shows at `frame`.
    fn set(&mut self, scene: &Scene, frame: Frame) {
        self.at = Some((frame, scene.layout));
        self.revisions.copy_from_slice(&scene.revisions);
        self.hover = scene.hover;
    }

    /// Whether member `index` must be painted again: its view changed, or
    /// it entered or left the pointer since this record's pixels were drawn.
    fn differs(&self, scene: &Scene, index: usize) -> bool {
        scene.revisions.get(index) != self.revisions.get(index)
            || (scene.hover == Some(index)) != (self.hover == Some(index))
    }
}

/// Paints `scene` at `frame` into `canvas`, whose pixels `record` says it
/// shows, and updates `record`. Only what differs is painted.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    canvas: &mut Canvas<'_>,
    record: &mut Record,
    scene: &Scene,
    placed: &[Placed],
    text: Option<&mut Text>,
    style: &Style,
    frame: Frame,
    output: &OutputView<'_>,
) {
    let whole = record.at != Some((frame, scene.layout));
    if whole {
        canvas.fill_shaped(
            Span {
                x: 0,
                width: canvas.width(),
            },
            style.theme.background,
            style.opacity,
            &scene.corners,
        );
    }
    if whole {
        separators(canvas, scene, style, frame.scale);
    }
    let Some(text) = text else {
        // No font: no module has any width, so nothing but the background.
        record.set(scene, frame);
        return;
    };
    let em = em(style.font_size, frame.scale);
    let padding = device(style.padding, frame.scale);
    let baseline = text.metrics(em).baseline(canvas.height());
    for (index, view) in scene.views.iter().enumerate() {
        let span = scene.spans[index];
        if !whole && !record.differs(scene, index) {
            continue;
        }
        if !whole {
            canvas.fill_shaped(span, style.theme.background, style.opacity, &scene.corners);
        }
        if span.width == 0 {
            continue;
        }
        // The modules with their own look draw themselves (the
        // workspaces pill, a window title longer than its span); the rest
        // draw as plain text.
        let mut custom = false;
        if let Some(placed) = scene.modules.get(index).and_then(|&m| placed.get(m)) {
            custom = placed.module.custom_draw(&mut CustomDraw {
                output: *output,
                view,
                canvas: &mut *canvas,
                text: &mut *text,
                span,
                em,
                baseline,
                padding,
                hovered: scene.hover == Some(index),
                scale: frame.scale,
                theme: &style.theme,
            });
        }
        if !custom {
            let color = if scene.hover == Some(index) {
                style.theme.accent
            } else {
                style.theme.class(view.class())
            };
            let mut x = i64::from(span.x) + i64::from(padding);
            if let Some(art) = view.art() {
                text.draw_art(canvas, art, em, x, color, span);
                x += i64::from(art_extent(text, view, em));
            }
            text.draw(
                canvas,
                view.icon(),
                view.text(),
                em,
                x,
                baseline,
                color,
                span,
            );
        }
    }
    record.set(scene, frame);
}

/// What a path or image icon takes before the text: its square, and the
/// gap after it when text follows; 0 for a view with none.
fn art_extent(text: &Text, view: &View, em: f32) -> u32 {
    // An icon past `icon::MAX_SIDE` is not drawn, so it takes no room
    // either (a blank gap of that size would be worse than none).
    if view.art().is_none() || Text::art_side(em) > crate::icon::MAX_SIDE {
        return 0;
    }
    let gap = if view.text().is_empty() {
        0
    } else {
        text.art_gap(em)
    };
    Text::art_side(em).saturating_add(gap)
}

/// A view's width in device pixels, padding not included: the icon (a
/// glyph, or a path or image square), a gap, then the text.
fn content_width(text: &Text, view: &View, em: f32) -> u32 {
    text.measure(view.icon(), view.text(), em)
        .saturating_add(art_extent(text, view, em))
}

/// The lines between neighbouring modules in a section, in the theme's
/// `dim` token: one centered in each gap, `style.separator` wide (cut back
/// to the gap, so a line never touches a module's span, which the
/// module's own repaint would overwrite), from a quarter of the bar's
/// height down to three quarters. Painted only with the whole bar: no
/// module's repaint reaches a gap.
fn separators(canvas: &mut Canvas<'_>, scene: &Scene, style: &Style, scale: Scale) {
    let width = device(style.separator, scale);
    if width == 0 {
        return;
    }
    let height = canvas.height();
    let (top, bottom) = (height / 4, height - height / 4);
    let mut previous: Option<(Section, u32)> = None;
    for (&span, &section) in scene.spans.iter().zip(&scene.sections) {
        if span.width == 0 {
            continue;
        }
        if let Some((was, end)) = previous {
            let gap = span.x.saturating_sub(end);
            if was == section && gap > 0 {
                let line = width.min(gap);
                let x = end + (gap - line) / 2;
                canvas.fill_rect(Span { x, width: line }, top, bottom, style.theme.dim);
            }
        }
        previous = Some((section, span.end()));
    }
}

/// The spans to damage on a surface showing `shown` so it shows `scene` at
/// `frame`, into `damage` (cleared first); `true` for the whole surface.
/// Updates `shown` to match.
pub fn damage(shown: &mut Record, scene: &Scene, frame: Frame, damage: &mut Vec<Span>) -> bool {
    damage.clear();
    let whole = shown.at != Some((frame, scene.layout));
    if !whole {
        for (index, &span) in scene.spans.iter().enumerate() {
            if shown.differs(scene, index) && span.width > 0 {
                damage.push(span);
            }
        }
    }
    shown.set(scene, frame);
    whole
}
