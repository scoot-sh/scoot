//! A module's popup on the Wayland side: an `xdg_popup` parented to the
//! bar's layer surface (`zwlr_layer_surface_v1.get_popup`), drawn from the
//! module's declarative content (`crate::popup`), alive only while open.
//!
//! ## The life of a popup
//!
//! 1. **Open** ([`State::open_popup`]) on the press of a click bound to
//!    `popup`, which carries the serial the grab is asked with (on the
//!    press, not the release: a compositor may refuse a grab whose serial is
//!    not that of a button still held, as some do), or from
//!    `scootbar msg invoke ID popup`, which has none. The content is asked
//!    of the module, laid out at the output's
//!    scale, and the popup made: a positioner anchored to the module's rect
//!    on the bar, gravity away from the bar's edge, sliding along the bar
//!    and flipping across it where the output's edge would cut it (the
//!    compositor does the fitting: `constraint_adjustment`); `get_popup`,
//!    then `grab` where there is a serial, then a first commit with no
//!    buffer. Nothing else exists before: no timer, no buffer, no keyboard.
//! 2. **Configure**: the compositor places it and sends the size and an
//!    `xdg_surface.configure`; the bar acks it and only then draws and
//!    commits the first buffer. A size that differs from the one asked is
//!    adopted. A later refill whose computed size differs reopens the
//!    popup at the new size with the stored grab serial (a same-size
//!    refill keeps the surface).
//! 3. **Redraw**: once per turn of the loop, after the module events are
//!    in, when something changed: the module's revision moved (a level
//!    changed elsewhere), the pointer hovered or dragged. The module's
//!    content is refilled into a spare, compared, and drawn only if it
//!    differs. Steady state allocates nothing.
//! 4. **Close**, from any of: `popup_done` (a click outside it, the session
//!    locking, the compositor's own decision), Escape, a press on the bar
//!    (which is what makes a second click on the module a toggle, and a
//!    click on another one a dismissal), the module having nothing to show
//!    or leaving the bar, the output going away or the bar being hidden or
//!    made again, a scale change, a full reload. A style-only reload
//!    instead restyles it in place where the new style fits its surface,
//!    remakes it where it does not, and only closes it where even that
//!    fails ([`State::restyle_popups`]). [`Popups::close`] is
//!    idempotent and destroys the popup before the surface it hangs off,
//!    so every site that destroys a bar surface calls it first.
//!
//! ## Tooltips
//!
//! A module's tooltip is the same `xdg_popup` with no grab, no keyboard and no
//! input region, in a slot of its own, shown after the pointer rests on the
//! module for `bar.tooltip-delay` (`tooltip`).
//!
//! ## The keyboard
//!
//! The bar's layer surface asks for no keyboard (`KeyboardInteractivity::
//! None`) and still does. A `wl_keyboard` is taken from the seat **only
//! while a popup that grabbed is open**, to hear its keys (the popup grab is
//! what gives the popup the keyboard: without a grab the compositor keeps
//! the keyboard where it was, so scoot never moves focus onto a popup that
//! did not grab — a tooltip is an ordinary `xdg_popup` too, and handing one
//! the keyboard would steal it from the window being typed into — and an
//! `invoke`-opened popup has no keyboard at all), and released with it.
//! Without a grab (an agent's `invoke`) there is no keyboard and no Escape:
//! the popup is closed by invoking `popup` again, or by anything in the
//! list above. Escape closes the popup; Up/Down move the hover among its
//! button rows (clamped at the ends) with the scroll following, and Enter
//! activates the hovered row. Keybindings of the compositor still win.
//!
//! ## Pointer routing
//!
//! A popup is its own pointer focus. `wl_pointer.enter` on its surface
//! makes it the target of motion, buttons and scroll until the `leave`, and
//! clears the bar's own focus, so a scroll over the popup is never a scroll
//! on a module and hover on the bar does not stick. The popup's events never
//! reach `crate::pointer`.

mod buffers;
mod events;
#[cfg(test)]
mod tests;
mod tooltip;

use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Proxy, QueueHandle};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::xdg::shell::client::xdg_popup::XdgPopup;
use wayland_protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity,
};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface;

use buffers::Pool;

use super::input::Launch;
use super::wayland::State;
use crate::action::{self, Action, ModuleAction};
use crate::bar::Edge;
use crate::density::Scale;
use crate::modules::OutputView;
use crate::outputs::{OutputId, Size};
use crate::popup::{self, Activate, Content, Interaction, Key, Layout, Round, Wheel};
use crate::print::warn;
use crate::render;

pub use events::PopupId;

/// A popup's content bound on either axis, in logical pixels: what a
/// compositor is asked for, far above any real popup and far below
/// anything that overflows a buffer.
const MAX_SIDE: u32 = 8192;

/// Whether a popup is open, and what it keeps between turns: the one open
/// popup (at most one, on one output) and the spare the module's content is
/// refilled into.
#[derive(Debug, Default)]
pub struct Popups {
    /// The click popup (or an `invoke`'s): the one that may hold a grab.
    open: Option<Open>,
    /// The tooltip, alive only while shown, never with a grab, a keyboard
    /// or an input region. Never both it and `open` (`tooltip`).
    tip: Option<Open>,
    /// When the tooltip shows and goes (`crate::popup::Hover`).
    hover: popup::Hover,
    next: u64,
    spare: Content,
}

/// The open popup.
#[derive(Debug)]
struct Open {
    id: PopupId,
    output: OutputId,
    /// The module's index among the placed ones, and its registry id.
    module: usize,
    /// The module's span on the bar, logical, as the positioner was told
    /// (`start`, `end`): a tooltip follows the module when it moves.
    anchor: (u32, u32),
    name: &'static str,
    surface: WlSurface,
    xdg: XdgSurface,
    popup: XdgPopup,
    viewport: Option<WpViewport>,
    keyboard: Option<wayland_client::protocol::wl_keyboard::WlKeyboard>,
    /// Acked the first `configure`, so a buffer may be attached.
    mapped: bool,
    /// What the positioner asked, and what the `xdg_popup.configure` said.
    requested: Size,
    configured: Option<Size>,
    /// The scale it is drawn at: a change closes it.
    scale: Scale,
    /// The buffer's size in device pixels.
    dims: (u32, u32),
    /// The grab serial it opened with, if any: a reopen after a resize
    /// reuses it, so the popup keeps whatever grab the open earned.
    serial: Option<u32>,
    em: f32,
    content: Content,
    layout: Layout,
    /// The frame's shape at this size and scale, built once: the paint
    /// reads its corner tables every frame and allocates nothing.
    round: Round,
    /// The popup's drop shadow tables (device pixels), if it has one.
    /// Built once on open via [`Shadow::prepare`] and read every redraw;
    /// `None` for `shadow = "none"` (nothing built or painted). Rebuilt
    /// only when the popup reopens at a new size or scale, never per
    /// frame.
    shadow: Option<scootui::paint::Shadow>,
    /// The shadow's tint (black, or the accent for a glow).
    shadow_tint: scootui::color::Color,
    /// The popup's top-left at rest in the surface (device pixels): the
    /// shadow's pad plus the entrance's room, `(0, 0)` without either. The
    /// paint offsets all of the frame and widgets by it, so clicks in the
    /// shadow fall through to the windows below. Mid-entrance the popup
    /// draws above or below this (the slide), settling here.
    origin: (u32, u32),
    /// Where the popup last drew (device pixels in the surface): at rest,
    /// or a slide away mid-entrance. Pointer hits subtract this to reach
    /// popup-local coordinates, so clicks during the entrance hit where
    /// the popup is drawn, never where it was a frame ago.
    drawn: (u32, u32),
    /// The popup fill's top and bottom colors and its alpha: the gradient
    /// stops over the background, solid when equal.
    fill: (scootui::color::Color, scootui::color::Color),
    opacity: u8,
    /// The room on each side (device pixels): the shadow's pad plus the
    /// entrance's room when motion moves. The surface is the content plus
    /// this pad, the content rests at `(pad.left, pad.top)`. Zero for
    /// tooltips and for still popups without a shadow.
    pad: scootui::paint::Pad,
    /// The entrance timing (`bar.motion` + duration + bounce):
    /// [`Timing::NONE`](scootui::motion::Timing::NONE) opens instantly,
    /// exactly as before. Tooltips never move.
    timing: scootui::motion::Timing,
    /// When the entrance began (the first mapped draw), while it runs:
    /// `None` before it starts and for `motion = "none"`. A look switch
    /// that changes the timing restarts it (see `restyle_open`).
    entered: Option<std::time::Instant>,
    /// The bar's edge this popup hangs off: the slide runs away from it.
    edge: Edge,
    /// Whether a `wl_surface.frame` callback is outstanding: one at most,
    /// none at rest and none for `motion = "none"`.
    frame_pending: bool,
    /// Pooled input-region rects: `scratch` is rebuilt every draw, `sent`
    /// is what the compositor was last told. Comparing them sends a new
    /// region only when it differs (at rest every draw but the first after
    /// a change sends nothing; in motion every frame moves and sends).
    /// No allocation once warm.
    region_scratch: Vec<scootui::region::Rect>,
    region_sent: Vec<scootui::region::Rect>,
    interaction: Interaction,
    /// The wheel movement of the frame being read, grouped into rows.
    wheel: Wheel,
    /// Whether pointer frames group events (version 5+): older ones flush
    /// each axis event as its own frame.
    framed: bool,
    /// The module revision `content` was filled at.
    revision: u64,
    /// A redraw is owed.
    dirty: bool,
    /// The pointer is over the popup, at `pointer` (logical, surface-local).
    focused: bool,
    pointer: (f64, f64),
    pool: Pool,
    /// The viewport's destination and the buffer scale last sent.
    sent_destination: Option<Size>,
    sent_scale: u32,
}

impl Open {
    /// Everything, children first: the keyboard, the popup, its role
    /// object, the viewport, the surface, then the buffers.
    fn destroy(mut self) {
        if let Some(keyboard) = self.keyboard.take()
            && keyboard.version() >= 3
        {
            keyboard.release();
        }
        self.popup.destroy();
        self.xdg.destroy();
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
        self.surface.destroy();
        self.pool.clear();
    }

    /// The frame's wheel movement, if any, as rows: moves the scroll,
    /// marking a redraw where it moved.
    fn flush_wheel(&mut self) {
        let steps = self.wheel.frame();
        if steps != 0 && self.interaction.scroll_by(&self.layout, steps) {
            self.dirty = true;
        }
    }
}

/// What [`State::build_popup`] makes.
#[derive(Debug, Clone, Copy)]
enum Flavor {
    /// The module's popup, with the grab `serial` asks for (none: no grab).
    Popup { serial: Option<u32> },
    /// The module's tooltip, drawn from its view: no grab, no keyboard, and
    /// no input region, so the pointer and every click go to the bar below.
    Tooltip,
}

impl Popups {
    /// With `delay` before a tooltip shows (zero: no tooltips).
    pub fn new(delay: std::time::Duration) -> Self {
        Self {
            hover: popup::Hover::new(delay),
            ..Self::default()
        }
    }

    /// Whether a popup or a tooltip is up: what keeps `xdg_wm_base` bound.
    pub fn is_up(&self) -> bool {
        self.open.is_some() || self.tip.is_some()
    }

    /// Whether tooltips are on (`bar.tooltip-delay` is not 0).
    pub fn tooltips_on(&self) -> bool {
        self.hover.enabled()
    }

    /// A reload: whatever was hovered is for modules that are gone, and the
    /// delay may be new. The shown tooltip goes with the hover state:
    /// `reset` forgets `Shown`, and an `Idle` hover never emits the `Hide`
    /// a kept tip would need when the pointer then leaves — so the tip
    /// would sit on screen past a delay-changing reload that kept it.
    pub fn reset_hover(&mut self, delay: std::time::Duration) {
        self.close_tip();
        self.hover.reset(delay);
    }

    /// Whether a click popup is open. A tooltip is not one: it never blocks
    /// a click, a second click on its module, or an `invoke`.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The open popup or tooltip that is `id`'s.
    fn find_mut(&mut self, id: PopupId) -> Option<&mut Open> {
        self.open
            .as_mut()
            .into_iter()
            .chain(self.tip.as_mut())
            .find(|open| open.id == id)
    }

    /// Closes the tooltip, if shown. Idempotent.
    pub fn close_tip(&mut self) {
        if let Some(tip) = self.tip.take() {
            tip.destroy();
        }
    }

    /// Whether the open popup belongs to this module on this output.
    pub fn is_for(&self, output: OutputId, module: usize) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.output == output && open.module == module)
    }

    /// Closes the popup and the tooltip, if open. Idempotent.
    pub fn close(&mut self) {
        if let Some(open) = self.open.take() {
            open.destroy();
        }
        self.close_tip();
    }

    /// Closes what hangs off `output`'s bar: called wherever that surface is
    /// destroyed, before it is.
    pub fn close_for(&mut self, output: OutputId) {
        if self.open.as_ref().is_some_and(|open| open.output == output) {
            self.close();
        }
        if self.tip.as_ref().is_some_and(|tip| tip.output == output) {
            self.close_tip();
        }
    }

    /// The scene member showing `module` on `scene`, with room on the bar.
    pub fn member_of(scene: &render::Scene, module: usize) -> Option<usize> {
        scene
            .spans()
            .iter()
            .enumerate()
            .find(|(member, span)| span.width > 0 && scene.module(*member) == Some(module))
            .map(|(member, _)| member)
    }
}

/// A module's span on the bar as the positioner is anchored to it: logical
/// pixels in the layer surface's own coordinates, at least one wide, held to
/// the bar.
fn anchor_span(scale: Scale, span: crate::paint::Span, bar_width: u32) -> (u32, u32) {
    let start = scale.logical_floor(span.x).min(bar_width);
    let end = scale
        .logical_ceil(span.end())
        .min(bar_width)
        .max(start.saturating_add(1));
    (start, end)
}

/// Whether a refilled popup must reopen at a new size: its content's
/// computed size no longer matches the surface it opened at (an
/// `xdg_popup` cannot change size after configure). A same-size refill
/// keeps the surface it has. The caller reopens with the stored grab
/// serial, and the reopened popup reads the same content and revision,
/// so this fires once per size change, never in a loop. The surface is
/// the content plus the shadow's pad on every side.
fn resized(layout: &Layout, pad: scootui::paint::Pad, dims: (u32, u32)) -> bool {
    let surf = (
        layout
            .width
            .saturating_add(pad.left)
            .saturating_add(pad.right),
        layout
            .height
            .saturating_add(pad.top)
            .saturating_add(pad.bottom),
    );
    surf != dims
}

/// Whether a restyled popup or tooltip still fits the surface it opened
/// at: the new layout's device size, back through the logical size the
/// compositor was asked for, buffers to no more than the buffer has — the
/// same check `keep_tooltip` uses for a refilled tooltip. A smaller layout
/// keeps the surface it has (no flicker); only one that no longer fits is
/// made again. Comparing device sizes directly would remake at every
/// reload on scales where the buffer rounds up (scale 2 with an odd width,
/// any fractional scale), reusing the old grab serial each time — which a
/// compositor may refuse, closing the popup.
fn fits(scale: Scale, layout: &Layout, pad: scootui::paint::Pad, dims: (u32, u32)) -> bool {
    let surf_w = layout
        .width
        .saturating_add(pad.left)
        .saturating_add(pad.right);
    let surf_h = layout
        .height
        .saturating_add(pad.top)
        .saturating_add(pad.bottom);
    scale
        .buffer(Size {
            width: scale.logical_ceil(surf_w),
            height: scale.logical_ceil(surf_h),
        })
        .is_some_and(|(width, height)| width <= dims.0 && height <= dims.1)
}

/// A protocol `int` for a size or a coordinate: saturated.
fn int(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The popup's input rects at `origin`: its own rounded rect, never the
/// shadow or the entrance room. Into `out` (cleared first, capacity kept):
/// no allocation once warm.
fn popup_rects(
    width: u32,
    height: u32,
    radius: u32,
    origin: (u32, u32),
    out: &mut Vec<scootui::region::Rect>,
) {
    use crate::region;
    let (ox, oy) = origin;
    out.clear();
    if radius == 0 {
        out.push(scootui::region::Rect {
            x: ox,
            y: oy,
            width,
            height,
        });
        return;
    }
    region::input_rects(width, height, radius, out);
    for rect in out.iter_mut() {
        rect.x = rect.x.saturating_add(ox);
        rect.y = rect.y.saturating_add(oy);
    }
}

/// The popup's rest origin in the surface (device pixels): the shadow's pad
/// plus the entrance's room when it moves. A shadowless moving popup still
/// keeps the motion room above it: the slide starts away from rest and needs
/// somewhere to start (`(0, 0)` pins every upward frame to the surface edge
/// via the `clamp` in `draw_open`, so the entrance never moves). Tooltips
/// keep `(0, 0)` (their pad is 0).
fn rest_origin(is_popup: bool, pad_left: u32, pad_top: u32) -> (u32, u32) {
    if is_popup {
        (pad_left, pad_top)
    } else {
        (0, 0)
    }
}

/// Sets the input region for a click popup's own rect at `origin` (device
/// pixels offset by the shadow's pad) of `width` by `height` with `radius`:
/// the rounded shape, never the shadow, so clicks in the shadow fall
/// through to the windows below. A square popup gets its own rect
/// explicitly (to exclude the shadow); without a shadow a square popup
/// would keep the default whole surface, but every caller here has a
/// shadow or a radius (the `needs_alpha` branch), so an explicit region is
/// always right.
fn set_popup_input_region(
    globals: &super::wayland::Globals,
    qh: &QueueHandle<State>,
    surface: &WlSurface,
    origin: (u32, u32),
    width: u32,
    height: u32,
    radius: u32,
) {
    let mut rects = Vec::with_capacity(2 * radius as usize + 1);
    popup_rects(width, height, radius, origin, &mut rects);
    let region = globals.compositor.create_region(qh, ());
    for rect in rects {
        region.add(int(rect.x), int(rect.y), int(rect.width), int(rect.height));
    }
    surface.set_input_region(Some(&region));
    region.destroy();
}

impl State {
    /// Opens the popup of the module shown at scene `member` of `output`'s
    /// bar, closing any other (and the tooltip). `serial` is the input event
    /// that asked (grabbing the pointer and keyboard for the popup); `None`
    /// opens it with no grab. `Err` says why not, for stderr or for the
    /// agent that asked.
    pub fn open_popup(
        &mut self,
        qh: &QueueHandle<State>,
        output: OutputId,
        member: usize,
        serial: Option<u32>,
    ) -> Result<(), String> {
        self.popup.close();
        let open = self.build_popup(qh, output, member, Flavor::Popup { serial })?;
        self.popup.open = Some(open);
        Ok(())
    }

    /// Makes the popup or tooltip of the module at scene `member` of
    /// `output`'s bar, not yet stored: the protocol objects, the first
    /// commit with no buffer.
    fn build_popup(
        &mut self,
        qh: &QueueHandle<State>,
        output: OutputId,
        member: usize,
        flavor: Flavor,
    ) -> Result<Open, String> {
        let Some(wm) = self.bind_xdg(qh) else {
            return Err("the compositor has no xdg_wm_base, so no popups".to_owned());
        };
        let entry = self.outputs.get_mut(output).ok_or("that output is gone")?;
        let Some(layer) = entry.objects.layer.as_ref() else {
            return Err("the bar is not on screen".to_owned());
        };
        let bar_size = entry
            .output
            .surface_size(&entry.objects.bar)
            .ok_or("the bar is not configured yet")?;
        let (Some(span), Some(module)) = (
            entry.objects.scene.spans().get(member).copied(),
            entry.objects.scene.module(member),
        ) else {
            return Err("that module is not on the bar".to_owned());
        };
        if span.width == 0 {
            return Err("the module has nothing on the bar to open from".to_owned());
        }
        let drawn = entry.output.scale();
        let scale = if layer.viewport.is_some() {
            drawn
        } else {
            Scale::Integer(drawn.integer())
        };
        let name = entry.output.info().name.as_deref();
        let (Some(placed), Some(text)) = (
            self.content.modules.get_mut(module),
            self.content.text.as_ref(),
        ) else {
            return Err("that module is not placed".to_owned());
        };
        let style = self.content.style;
        let mut content = Content::default();
        // The popup hangs off this output's bar, so it measures in its em.
        let em = render::em(entry.objects.font_size, scale);
        let mut layout = Layout::default();
        let pad = render::device(style.padding, scale);
        let frame = render::device(1, scale).max(1);
        match flavor {
            Flavor::Popup { .. } => {
                if !placed.module.popup(&OutputView { name }, &mut content) || content.is_empty() {
                    return Err(format!("`{}` has no popup to show now", placed.id));
                }
                layout.compute(&content, text, em, pad, frame);
            }
            Flavor::Tooltip => {
                let tip = entry
                    .objects
                    .scene
                    .view(member)
                    .map_or("", |view| view.tooltip());
                // As wide as the em allows, and never past the bar.
                let room = render::device(bar_size.width, scale)
                    .saturating_sub(pad.saturating_add(frame).saturating_mul(2));
                let wide = ((em * popup::TIP_MAX_EM) as u32).min(room).max(1);
                content.wrap_tooltip(tip, wide, |line| text.measure(None, line, em));
                if content.is_empty() {
                    return Err(format!("`{}` has no tooltip to show now", placed.id));
                }
                layout.compute_tooltip(&content, text, em, pad, frame);
            }
        }
        let is_popup = matches!(flavor, Flavor::Popup { .. });
        let edge = entry.objects.bar.edge;
        let timing = if is_popup {
            style.timing
        } else {
            scootui::motion::Timing::NONE
        };
        // The popup's shadow (click popups only; tooltips stay as they
        // were): the surface reserves the shadow's room on every side, and
        // the popup sits at the pad offset. The positioner is told the
        // whole surface, so the compositor places it with its shadow.
        // The entrance keeps its own room on top when it moves (a slide
        // away from the bar needs somewhere to start): `motion = "none"`
        // keeps the surface it always had.
        let (pad_left, pad_top, pad_right, pad_bottom, shadow_tables, shadow_tint) = if is_popup {
            let (shadow_left, shadow_top, shadow_right, shadow_bottom, tables, tint) =
                if let Some(logical) = style.shadow_spec(edge) {
                    let dev_spec = scootui::paint::ShadowSpec {
                        offset_x: i32::try_from(scale.to_device(i64::from(logical.offset_x)))
                            .unwrap_or(logical.offset_x),
                        offset_y: i32::try_from(scale.to_device(i64::from(logical.offset_y)))
                            .unwrap_or(logical.offset_y),
                        blur: render::device(logical.blur, scale),
                        alpha: logical.alpha,
                    };
                    let pad = scootui::paint::Shadow::new(dev_spec).pad();
                    let tint = match style.shadow {
                        scootui::effects::ShadowStyle::Glow => style.theme.accent,
                        _ => scootui::color::Color { r: 0, g: 0, b: 0 },
                    };
                    (
                        pad.left,
                        pad.top,
                        pad.right,
                        pad.bottom,
                        Some((dev_spec, tint)),
                        tint,
                    )
                } else {
                    (0, 0, 0, 0, None, scootui::color::Color { r: 0, g: 0, b: 0 })
                };
            let (motion_left, motion_top, motion_right, motion_bottom) =
                crate::popup_motion::motion_pad(&timing);
            let motion_left = render::device(motion_left, scale);
            let motion_top = render::device(motion_top, scale);
            let motion_right = render::device(motion_right, scale);
            let motion_bottom = render::device(motion_bottom, scale);
            (
                shadow_left.saturating_add(motion_left),
                shadow_top.saturating_add(motion_top),
                shadow_right.saturating_add(motion_right),
                shadow_bottom.saturating_add(motion_bottom),
                tables,
                tint,
            )
        } else {
            (0, 0, 0, 0, None, scootui::color::Color { r: 0, g: 0, b: 0 })
        };
        let _ = shadow_tint;
        let content_w = layout.width;
        let content_h = layout.height;
        let surf_w = content_w.saturating_add(pad_left).saturating_add(pad_right);
        let surf_h = content_h.saturating_add(pad_top).saturating_add(pad_bottom);
        let requested = Size {
            width: scale.logical_ceil(surf_w).clamp(1, MAX_SIDE),
            height: scale.logical_ceil(surf_h).clamp(1, MAX_SIDE),
        };
        let dims = scale
            .buffer(requested)
            .ok_or("the popup is too large for a buffer")?;
        // The buffer rounds up from what was asked: the content grows to
        // fill it less the shadow's pad, and the frame is drawn at the
        // content's edge (the shadow owns the rest).
        let new_content_w = dims.0.saturating_sub(pad_left).saturating_sub(pad_right);
        let new_content_h = dims.1.saturating_sub(pad_top).saturating_sub(pad_bottom);
        layout.width = new_content_w.max(1);
        layout.height = new_content_h.max(1);
        // The frame's shape at this size and scale: the radius cut back to
        // what the content holds, the corner tables built once and read
        // every frame it is drawn.
        let round = Round::new(
            render::device(style.popup_radius, scale),
            frame,
            layout.width,
            layout.height,
        );
        // The shadow tables for the content size, built once on open and
        // read every redraw: no per-frame allocation once warm.
        // `shadow = "none"` builds nothing.
        let (shadow, origin) = if is_popup {
            if let Some((dev_spec, tint)) = shadow_tables {
                let radius = round.outer().radius();
                let mut tables = scootui::paint::Shadow::new(dev_spec);
                tables.prepare(radius, [(layout.width, layout.height)]);
                let _ = tint;
                (Some(tables), rest_origin(true, pad_left, pad_top))
            } else {
                (None, rest_origin(true, pad_left, pad_top))
            }
        } else {
            (None, rest_origin(false, pad_left, pad_top))
        };
        let shadow_tint = if is_popup {
            match style.shadow {
                scootui::effects::ShadowStyle::Glow => style.theme.accent,
                _ => scootui::color::Color { r: 0, g: 0, b: 0 },
            }
        } else {
            scootui::color::Color { r: 0, g: 0, b: 0 }
        };
        let (fill_top, fill_bottom) = if is_popup {
            style.fill_stops()
        } else {
            (style.theme.background, style.theme.background)
        };
        let popup_opacity = if is_popup { style.opacity } else { 255 };

        self.popup.next = self.popup.next.wrapping_add(1);
        let id = PopupId(self.popup.next);
        let surface = self.globals.compositor.create_surface(qh, id);
        let viewport = self
            .globals
            .viewporter
            .as_ref()
            .filter(|_| layer.viewport.is_some())
            .map(|viewporter| viewporter.get_viewport(&surface, qh, ()));
        // Anchored to the module's rect on the bar (logical, in the layer
        // surface's own coordinates), opening away from the bar's edge.
        let (start, end) = anchor_span(scale, span, bar_size.width);
        let positioner = wm.create_positioner(qh, ());
        positioner.set_size(int(requested.width), int(requested.height));
        positioner.set_anchor_rect(int(start), 0, int(end - start), int(bar_size.height.max(1)));
        let (anchor, gravity) = match entry.objects.bar.edge {
            Edge::Top => (Anchor::Bottom, Gravity::Bottom),
            Edge::Bottom => (Anchor::Top, Gravity::Top),
        };
        positioner.set_anchor(anchor);
        positioner.set_gravity(gravity);
        positioner
            .set_constraint_adjustment(ConstraintAdjustment::SlideX | ConstraintAdjustment::FlipY);
        let xdg = wm.get_xdg_surface(&surface, qh, id);
        let popup = xdg.get_popup(None, &positioner, qh, id);
        layer.adopt(&popup);
        let mut keyboard = None;
        match flavor {
            Flavor::Popup {
                serial: Some(serial),
            } => {
                if let Some(seat) = self.globals.seat.as_ref() {
                    popup.grab(seat, serial);
                    if self.seat_keyboard {
                        keyboard = Some(seat.get_keyboard(qh, id));
                    }
                }
            }
            Flavor::Popup { serial: None } => {}
            Flavor::Tooltip => {
                // An empty input region: the pointer never enters it and a
                // click under it lands on the bar. (The region is copied by
                // the request, so it is destroyed at once.)
                let region = self.globals.compositor.create_region(qh, ());
                surface.set_input_region(Some(&region));
                region.destroy();
            }
        }
        if round.rounded() || shadow.is_some() || popup_opacity < 255 || timing.active() {
            // The corners are transparent, the shadow falls outside the
            // popup, the fill itself is translucent, or the entrance keeps
            // room around the popup: the compositor blends the whole surface
            // (the default whole-opaque would show cleared pixels as black).
            // A square opaque still popup keeps the default.
            super::canvas::set_opaque_region(&self.globals, qh, &surface, (requested, None));
            // A click popup's input shape is the popup's own rounded rect
            // at its offset, never its shadow, so a click in the shadow or
            // in a cut corner does not land on the popup. A tooltip keeps
            // its empty input region instead (it never takes a click).
            if matches!(flavor, Flavor::Popup { .. }) {
                // Rounded or square: the popup's own rect at its origin,
                // excluding the shadow on every side.
                let radius = crate::region::effective_radius(
                    style.popup_radius,
                    layout.width,
                    layout.height,
                );
                set_popup_input_region(
                    &self.globals,
                    qh,
                    &surface,
                    origin,
                    layout.width,
                    layout.height,
                    radius,
                );
            }
        }
        // The pooled input-region lists start warm: `sent` holds what the
        // open just told the compositor (or nothing when it told nothing),
        // so the first draw sends only when the entrance moved it. `none`
        // keeps the single send it always did.
        let mut region_sent = Vec::new();
        if matches!(flavor, Flavor::Popup { .. })
            && (round.rounded() || shadow.is_some() || popup_opacity < 255 || timing.active())
        {
            let radius =
                crate::region::effective_radius(style.popup_radius, layout.width, layout.height);
            popup_rects(
                layout.width,
                layout.height,
                radius,
                origin,
                &mut region_sent,
            );
        }
        // `round.rounded()` already takes the branch above, so no `else if`
        // remains: a square opaque unshadowed popup needs no regions at all.
        positioner.destroy();
        // No buffer yet: the compositor answers with the configure.
        surface.commit();
        // Frames group a wheel's events from pointer version 5; before it
        // each axis event is its own frame (as the bar's own scroll does).
        let framed = self
            .pointer
            .as_ref()
            .is_none_or(|pointer| pointer.version() >= 5);
        let serial = match flavor {
            Flavor::Popup { serial } => serial,
            Flavor::Tooltip => None,
        };
        Ok(Open {
            id,
            output,
            module,
            anchor: (start, end),
            name: placed.id,
            surface,
            xdg,
            popup,
            viewport,
            keyboard,
            mapped: false,
            requested,
            configured: None,
            scale,
            dims,
            serial,
            em,
            content,
            layout,
            round,
            shadow,
            shadow_tint,
            origin,
            drawn: origin,
            fill: (fill_top, fill_bottom),
            opacity: popup_opacity,
            pad: scootui::paint::Pad {
                top: pad_top,
                right: pad_right,
                bottom: pad_bottom,
                left: pad_left,
            },
            timing,
            entered: None,
            edge,
            frame_pending: false,
            region_scratch: Vec::new(),
            region_sent,
            interaction: Interaction::default(),
            wheel: Wheel::default(),
            framed,
            revision: placed.revision,
            dirty: false,
            focused: false,
            pointer: (0.0, 0.0),
            pool: Pool::default(),
            sent_destination: None,
            sent_scale: 1,
        })
    }

    /// A style-only reload's popup half ([`State::restyle`], after the
    /// outputs are placed again and the new font and style are swapped
    /// in): an open popup or tooltip is brought onto the new style, never
    /// left painting old glyphs at old metrics. A tooltip with tooltips
    /// now off closes, as a full reload closes everything. What stays open
    /// is remeasured at the new em, laid out again and marked dirty, so
    /// its next draw paints the new style throughout. Where the new layout
    /// no longer fits the surface it opened at (an `xdg_popup` cannot
    /// change size after configure) or the frame changed shape (square
    /// wants no alpha buffer or regions, rounded wants both), it is made
    /// again at once where it belongs — the tooltip with no new delay,
    /// the popup with the grab serial the open earned — and only closed
    /// where even that fails. The modules are the same ones in the same
    /// order, so their indices still name the same modules, and the grab,
    /// the keyboard and the pointer focus stay where they were.
    pub fn restyle_popups(&mut self, qh: &QueueHandle<State>) {
        // Tooltips now off close: a shown tooltip with no delay to govern
        // it is a mixed state. (`reset_hover` already ran with the new
        // delay, so this reads new.)
        if !self.popup.tooltips_on() {
            self.popup.close_tip();
        }
        if self.popup.open.is_some() {
            self.restyle_open(qh);
        }
        if self.popup.tip.is_some() {
            self.restyle_tip(qh);
        }
    }

    /// Brings the open click popup onto the new style: remeasured at the
    /// new em and laid out again from its content, then repainted once.
    /// Remade where the new layout does not fit the surface it opened at
    /// or the frame changed shape (see [`Self::restyle_popups`]).
    fn restyle_open(&mut self, qh: &QueueHandle<State>) {
        let Some(mut open) = self.popup.open.take() else {
            return;
        };
        let style = self.content.style;
        let font_size = match self.outputs.get_mut(open.output) {
            Some(entry) if entry.objects.layer.is_some() => entry.objects.font_size,
            _ => {
                open.destroy();
                return;
            }
        };
        // The output's scale is not a reload's business: outputs do not
        // re-scale, so the open keeps the scale it drew at.
        let em = render::em(font_size, open.scale);
        let pad = render::device(style.padding, open.scale);
        let frame = render::device(1, open.scale).max(1);
        let Some(text) = self.content.text.as_mut() else {
            open.destroy();
            return;
        };
        open.layout.compute(&open.content, text, em, pad, frame);
        // A shadow or motion-room change resizes the surface: remake it, as
        // at the open. The bar's edge decides the shadow's mirror (top bars
        // shadow below, bottom bars above) and the entrance's direction.
        let bar_edge = self
            .outputs
            .iter()
            .find(|entry| entry.output.id() == open.output)
            .map(|entry| entry.objects.bar.edge)
            .unwrap_or(crate::bar::Edge::Top);
        let shadow_pad = if let Some(logical) = style.shadow_spec(bar_edge) {
            let dev_spec = scootui::paint::ShadowSpec {
                offset_x: i32::try_from(open.scale.to_device(i64::from(logical.offset_x)))
                    .unwrap_or(logical.offset_x),
                offset_y: i32::try_from(open.scale.to_device(i64::from(logical.offset_y)))
                    .unwrap_or(logical.offset_y),
                blur: render::device(logical.blur, open.scale),
                alpha: logical.alpha,
            };
            scootui::paint::Shadow::new(dev_spec).pad()
        } else {
            scootui::paint::Pad::ZERO
        };
        let (motion_left, motion_top, motion_right, motion_bottom) =
            crate::popup_motion::motion_pad(&style.timing);
        let new_pad = scootui::paint::Pad {
            left: shadow_pad
                .left
                .saturating_add(render::device(motion_left, open.scale)),
            top: shadow_pad
                .top
                .saturating_add(render::device(motion_top, open.scale)),
            right: shadow_pad
                .right
                .saturating_add(render::device(motion_right, open.scale)),
            bottom: shadow_pad
                .bottom
                .saturating_add(render::device(motion_bottom, open.scale)),
        };
        if new_pad != open.pad {
            let (output, module, serial) = (open.output, open.module, open.serial);
            open.destroy();
            let member = self
                .outputs
                .iter()
                .find(|entry| entry.output.id() == output)
                .and_then(|entry| Popups::member_of(&entry.objects.scene, module));
            if let Some(member) = member
                && let Ok(fresh) = self.build_popup(qh, output, member, Flavor::Popup { serial })
            {
                self.popup.open = Some(fresh);
            }
            return;
        }
        let round = Round::new(
            render::device(style.popup_radius, open.scale),
            frame,
            open.layout.width,
            open.layout.height,
        );
        if !fits(open.scale, &open.layout, open.pad, open.dims)
            || round.rounded() != open.round.rounded()
        {
            let (output, module, serial) = (open.output, open.module, open.serial);
            open.destroy();
            let member = self
                .outputs
                .iter()
                .find(|entry| entry.output.id() == output)
                .and_then(|entry| Popups::member_of(&entry.objects.scene, module));
            if let Some(member) = member
                && let Ok(fresh) = self.build_popup(qh, output, member, Flavor::Popup { serial })
            {
                self.popup.open = Some(fresh);
            }
            return;
        }
        open.em = em;
        // The popup keeps the surface it opened at: the content grows to
        // fill it less the shadow's pad (no flicker on a smaller refill).
        open.layout.width = open
            .dims
            .0
            .saturating_sub(open.pad.left)
            .saturating_sub(open.pad.right)
            .max(1);
        open.layout.height = open
            .dims
            .1
            .saturating_sub(open.pad.top)
            .saturating_sub(open.pad.bottom)
            .max(1);
        // The frame's new shape at the kept size, and the shadow's new
        // tables for it (same size, new style): no per-frame allocation
        // once warm.
        open.round = Round::new(
            render::device(style.popup_radius, open.scale),
            frame,
            open.layout.width,
            open.layout.height,
        );
        if let Some(logical) = style.shadow_spec(bar_edge) {
            let dev_spec = scootui::paint::ShadowSpec {
                offset_x: i32::try_from(open.scale.to_device(i64::from(logical.offset_x)))
                    .unwrap_or(logical.offset_x),
                offset_y: i32::try_from(open.scale.to_device(i64::from(logical.offset_y)))
                    .unwrap_or(logical.offset_y),
                blur: render::device(logical.blur, open.scale),
                alpha: logical.alpha,
            };
            let mut tables = scootui::paint::Shadow::new(dev_spec);
            tables.prepare(
                open.round.outer().radius(),
                [(open.layout.width, open.layout.height)],
            );
            open.shadow = Some(tables);
        } else {
            open.shadow = None;
        }
        open.shadow_tint = match style.shadow {
            scootui::effects::ShadowStyle::Glow => style.theme.accent,
            _ => scootui::color::Color { r: 0, g: 0, b: 0 },
        };
        open.fill = style.fill_stops();
        open.opacity = style.opacity;
        open.origin = rest_origin(true, open.pad.left, open.pad.top);
        // The entrance timing is part of the cache key: a runtime look
        // switch that changes it restarts the entrance (a colors-only reload
        // keeps the settled state, so it never re-animates). A switch that
        // flips active/inactive already remade the surface above (the total
        // pad changed), so this only runs when the room is the same.
        if style.timing != open.timing {
            open.timing = style.timing;
            open.entered = None;
            open.frame_pending = false;
        }
        // The edge needs no restart here: it is bar geometry, and a geometry
        // change takes the full reload (which closes the popup), never this
        // style-only path (`respond::style_only`, pinned by
        // `style_tests::bar_geometry_is_not_style_only`). At rest `dy = 0`
        // either way; this assignment only keeps the invariant.
        open.edge = bar_edge;
        open.interaction.retain(&open.content, &open.layout);
        open.dirty = true;
        // The new style on the same surface: the regions are re-sent, as at
        // the open. A shadowed, rounded, translucent, gradient or moving
        // popup blends; otherwise the defaults stand.
        let needs_alpha = open.round.rounded()
            || open.shadow.is_some()
            || open.opacity < 255
            || open.fill.0 != open.fill.1
            || open.timing.active();
        if needs_alpha {
            super::canvas::set_opaque_region(
                &self.globals,
                qh,
                &open.surface,
                (open.requested, None),
            );
            let radius = crate::region::effective_radius(
                style.popup_radius,
                open.layout.width,
                open.layout.height,
            );
            set_popup_input_region(
                &self.globals,
                qh,
                &open.surface,
                open.origin,
                open.layout.width,
                open.layout.height,
                radius,
            );
        }
        self.popup.open = Some(open);
    }

    /// Brings the shown tooltip onto the new style: its text re-wrapped at
    /// the new em and laid out again, then repainted once. Remade where
    /// the new layout does not fit the surface it opened at or the frame
    /// changed shape (see [`Self::restyle_popups`]).
    fn restyle_tip(&mut self, qh: &QueueHandle<State>) {
        let Some(mut tip) = self.popup.tip.take() else {
            return;
        };
        let style = self.content.style;
        let found = match self.outputs.get_mut(tip.output) {
            Some(entry) => {
                let member = Popups::member_of(&entry.objects.scene, tip.module);
                let line = member
                    .and_then(|member| entry.objects.scene.view(member))
                    .map_or("", |view| view.tooltip())
                    .to_owned();
                let bar_width = entry
                    .output
                    .surface_size(&entry.objects.bar)
                    .map_or(0, |size| size.width);
                member.map(|member| (member, line, entry.objects.font_size, bar_width))
            }
            None => None,
        };
        let (member, line, font_size, bar_width) = match found {
            Some(found) => found,
            None => {
                tip.destroy();
                return;
            }
        };
        if line.is_empty() {
            tip.destroy();
            return;
        }
        let em = render::em(font_size, tip.scale);
        let pad = render::device(style.padding, tip.scale);
        let frame = render::device(1, tip.scale).max(1);
        let room = render::device(bar_width, tip.scale)
            .saturating_sub(pad.saturating_add(frame).saturating_mul(2));
        let wide = ((em * popup::TIP_MAX_EM) as u32).min(room).max(1);
        let Some(text) = self.content.text.as_mut() else {
            tip.destroy();
            return;
        };
        let mut content = Content::default();
        content.wrap_tooltip(&line, wide, |line| text.measure(None, line, em));
        if content.is_empty() {
            tip.destroy();
            return;
        }
        let mut layout = Layout::default();
        layout.compute_tooltip(&content, text, em, pad, frame);
        let round = Round::new(
            render::device(style.popup_radius, tip.scale),
            frame,
            tip.dims.0,
            tip.dims.1,
        );
        if !fits(tip.scale, &layout, tip.pad, tip.dims) || round.rounded() != tip.round.rounded() {
            let (output, module) = (tip.output, tip.module);
            tip.destroy();
            let key = Key { output, module };
            if !self.open_tooltip(qh, key, member) {
                self.popup.hover.dismiss(None);
            }
            return;
        }
        tip.content = content;
        tip.em = em;
        // The tooltip keeps the size it opened at.
        layout.width = tip.dims.0;
        layout.height = tip.dims.1;
        tip.layout = layout;
        tip.round = round;
        // The new style's background (a colors-only reload repaints the
        // tooltip in it); tooltips stay solid and opaque, without a shadow.
        tip.fill = (style.theme.background, style.theme.background);
        tip.dirty = true;
        if tip.round.rounded() {
            let radius = crate::region::effective_radius(
                style.popup_radius,
                tip.requested.width,
                tip.requested.height,
            );
            super::canvas::set_opaque_region(
                &self.globals,
                qh,
                &tip.surface,
                (tip.requested, Some(radius.saturating_add(1))),
            );
            // A tooltip keeps its empty input region: it never takes a
            // click, at any radius.
        }
        self.popup.tip = Some(tip);
    }

    /// Carries out what a popup interaction asks of its module, through the
    /// one way an action is carried out ([`action::perform`]). A button
    /// that closes (`closes`) closes the popup after the action runs: the
    /// action runs first, while the popup still names its module.
    fn activate_popup(&mut self, activate: Activate) {
        let Some(open) = self.popup.open.as_ref() else {
            return;
        };
        let (module, output) = (open.module, open.output);
        let name = self
            .outputs
            .iter()
            .find(|entry| entry.output.id() == output)
            .and_then(|entry| entry.output.info().name.as_deref());
        let Some(placed) = self.content.modules.get_mut(module) else {
            return;
        };
        let id = placed.id;
        let action = Action::Module(ModuleAction::new(activate.action, activate.arg));
        let mut effects = Launch::new(&mut self.spawner);
        if let Err(failure) = action::perform(
            &mut *placed.module,
            &mut placed.revision,
            &OutputView { name },
            &action,
            None,
            &mut effects,
        ) && let Some(held) = self.warned.allow(std::time::Instant::now())
        {
            let more = if held > 0 {
                format!(" ({held} more since)")
            } else {
                String::new()
            };
            warn(format_args!(
                "scootbar: {} on the {id} module's popup: {failure}{more}",
                activate.action
            ));
        }
        if activate.closes {
            self.popup.close();
        }
    }

    /// Once a turn, after the module events and before the bars draw: closes
    /// a popup whose reason to exist is gone, refreshes its content when its
    /// module changed, and draws it when a redraw is owed. Nothing at all
    /// (one `Option` check) while no popup is open.
    pub fn pump_popup(&mut self, qh: &QueueHandle<State>) {
        let Some(open) = self.popup.open.as_mut() else {
            return;
        };
        let Some(entry) = self.outputs.get_mut(open.output) else {
            self.popup.close();
            return;
        };
        let live = entry.objects.layer.is_some()
            && entry.objects.scene.section_of(open.module).is_some()
            && Popups::member_of(&entry.objects.scene, open.module).is_some();
        let drawn = entry.output.scale();
        let scale = if entry
            .objects
            .layer
            .as_ref()
            .is_some_and(|layer| layer.viewport.is_some())
        {
            drawn
        } else {
            Scale::Integer(drawn.integer())
        };
        if !live || scale != open.scale {
            self.popup.close();
            return;
        }
        let name = entry.output.info().name.as_deref();
        let (Some(placed), Some(text)) = (
            self.content.modules.get_mut(open.module),
            self.content.text.as_mut(),
        ) else {
            self.popup.close();
            return;
        };
        if placed.revision != open.revision {
            let spare = &mut self.popup.spare;
            spare.clear();
            if !placed.module.popup(&OutputView { name }, spare) || spare.is_empty() {
                self.popup.close();
                return;
            }
            open.revision = placed.revision;
            if *spare != open.content {
                std::mem::swap(spare, &mut open.content);
                let style = &self.content.style;
                open.layout.compute(
                    &open.content,
                    text,
                    open.em,
                    render::device(style.padding, scale),
                    render::device(1, scale).max(1),
                );
                // The content outgrew (or shrank past) the surface it
                // opened at: a menu opens on its `...` line and fills a
                // turn later, and an xdg_popup cannot change size after
                // configure. Reopen it at the new size, with the grab
                // serial the open earned (hover and the scroll position
                // do not survive: the rows moved anyway). Same size
                // keeps the surface it has; a reopened popup reads the
                // same content and revision, so this fires once per size
                // change, never in a loop. The surface is the content
                // plus the shadow's pad.
                if resized(&open.layout, open.pad, open.dims) {
                    let (output, module, serial) = match self.popup.open.as_ref() {
                        Some(open) => (open.output, open.module, open.serial),
                        None => return,
                    };
                    self.popup.close();
                    let member = self
                        .outputs
                        .iter()
                        .find(|entry| entry.output.id() == output)
                        .and_then(|entry| Popups::member_of(&entry.objects.scene, module));
                    if let Some(member) = member {
                        let _ = self.open_popup(qh, output, member, serial);
                    }
                    return;
                }
                // The popup keeps the surface it opened at: the content
                // grows to fill it less the shadow's pad.
                open.layout.width = open
                    .dims
                    .0
                    .saturating_sub(open.pad.left)
                    .saturating_sub(open.pad.right)
                    .max(1);
                open.layout.height = open
                    .dims
                    .1
                    .saturating_sub(open.pad.top)
                    .saturating_sub(open.pad.bottom)
                    .max(1);
                open.interaction.retain(&open.content, &open.layout);
                open.dirty = true;
            }
        }
        if !draw_open(open, &self.globals, qh, text, &self.content.style.theme) {
            self.popup.close();
        }
    }
}

/// Draws `open` into a free buffer and commits it, if a redraw is owed and
/// it is mapped. `false` when it cannot be drawn at all (no buffer to be had,
/// a canvas that cannot be made): the caller closes it.
///
/// A popup with motion moves only on frame callbacks: the first mapped draw
/// starts its entrance, each frame draws where the timeline says it is and
/// asks for the next callback while it still moves, and the input region
/// moves with it every frame (sent only when it differs, so at rest every
/// draw but the first after a change sends nothing). `motion = "none"`
/// never starts, never asks, and paints exactly what it always did.
fn draw_open(
    open: &mut Open,
    globals: &super::wayland::Globals,
    qh: &QueueHandle<State>,
    text: &mut crate::text::Text,
    theme: &crate::theme::Theme,
) -> bool {
    if !open.dirty || !open.mapped {
        return true;
    }
    let now = std::time::Instant::now();
    // The entrance starts on the first mapped draw, once.
    if open.timing.active() && open.entered.is_none() {
        open.entered = Some(now);
    }
    let settled = match open.entered {
        Some(start) => crate::popup_motion::settled(&open.timing, start, now),
        None => true,
    };
    // Where the popup draws this frame: at rest, or a slide away mid-entrance.
    let dy_device: i32 = match open.entered {
        Some(start) if !settled => {
            let dy_logical = crate::popup_motion::entrance_dy(&open.timing, start, now, open.edge);
            (f64::from(dy_logical) * open.scale.factor()).round() as i32
        }
        _ => 0,
    };
    let (rest_ox, rest_oy) = open.origin;
    let cur_oy =
        (i64::from(rest_oy) + i64::from(dy_device)).clamp(0, i64::from(open.dims.1)) as u32;
    let cur_origin = (rest_ox, cur_oy);
    // Where this frame draws, for the hit test: clicks hit where the popup
    // is drawn, never where it was a frame ago.
    open.drawn = cur_origin;
    // A moving popup keeps transparent room around it, so it blends even
    // when it otherwise would not. `none` keeps the buffer it always had.
    let needs_alpha = open.round.rounded()
        || open.shadow.is_some()
        || open.opacity < 255
        || open.fill.0 != open.fill.1
        || open.timing.active();
    let slot = match open.pool.take(globals, qh, open.id, open.dims, needs_alpha) {
        // Every buffer is held: the release wakes the loop.
        Ok(None) => return true,
        Ok(Some(slot)) => slot,
        Err(error) => {
            warn(format_args!(
                "scootbar: cannot draw the {} popup: {error}",
                open.name
            ));
            return false;
        }
    };
    let Some(mut canvas) = crate::paint::Canvas::new(slot.pixels_mut(), open.dims.0, open.dims.1)
    else {
        return false;
    };
    // Tooltips (no shadow, solid, opaque, at the origin) paint exactly as
    // before, so their goldens hold. Everything else goes through the
    // depth-aware path (shadow, gradient, translucency, offset), which is
    // identical for the defaults. A moving popup never takes the default
    // path: its origin carries the entrance room.
    let is_default = open.shadow.is_none()
        && open.fill.0 == open.fill.1
        && open.fill.0 == theme.background
        && open.opacity == 255
        && open.origin == (0, 0)
        && !open.timing.active();
    if is_default {
        popup::paint(
            &mut canvas,
            text,
            theme,
            &open.content,
            &open.layout,
            &open.interaction,
            open.em,
            &open.round,
        );
    } else {
        let shadow = open
            .shadow
            .as_ref()
            .map(|tables| (tables, open.shadow_tint, open.round.outer().radius()));
        popup::paint_depth(
            &mut canvas,
            text,
            theme,
            &open.content,
            &open.layout,
            &open.interaction,
            open.em,
            &open.round,
            cur_origin,
            shadow,
            open.fill,
            open.opacity,
        );
    }
    let surface = &open.surface;
    surface.attach(Some(&slot.buffer), 0, 0);
    match &open.viewport {
        Some(viewport) => {
            let size = open.configured.unwrap_or(open.requested);
            if open.sent_destination != Some(size) {
                viewport.set_destination(int(size.width), int(size.height));
                open.sent_destination = Some(size);
            }
            if open.sent_scale != 1 {
                surface.set_buffer_scale(1);
                open.sent_scale = 1;
            }
        }
        None => {
            let factor = open.scale.integer();
            if open.sent_scale != factor {
                surface.set_buffer_scale(int(factor));
                open.sent_scale = factor;
            }
        }
    }
    surface.damage_buffer(0, 0, int(open.dims.0), int(open.dims.1));
    // The input region follows the popup every frame in motion (where it
    // is drawn, not where it rests), and is sent only when it differs from
    // what the compositor was last told: at rest every draw but the first
    // after a change sends nothing, in motion every frame sends its own.
    // No allocation once the pooled lists are warm. A square opaque still
    // popup keeps the default (no region), as before.
    if needs_alpha {
        // Disjoint field borrows: `slot` holds `open.pool`, the region
        // lists are separate.
        let (scratch, sent) = (&mut open.region_scratch, &mut open.region_sent);
        update_popup_input_region(
            scratch,
            sent,
            &open.layout,
            &open.round,
            &open.surface,
            globals,
            qh,
            cur_origin,
        );
    }
    // While something moves, ask for the next frame callback (one at most):
    // the callback marks the next redraw. At rest and for `none`, ask for
    // none, so an idle popup costs nothing.
    if open.timing.active() && !settled && !open.frame_pending {
        open.surface.frame(qh, open.id);
        open.frame_pending = true;
    }
    if settled {
        open.frame_pending = false;
    }
    surface.commit();
    slot.held = true;
    open.dirty = false;
    true
}

/// Rebuilds the popup's input region at `origin` (where it draws this
/// frame) into the pooled scratch list, sending it only when it differs
/// from what was last sent. A square popup gets its own rect explicitly
/// (to exclude the shadow and the entrance room); a rounded one gets its
/// outline rows. No allocation once warm.
#[allow(clippy::too_many_arguments)]
fn update_popup_input_region(
    scratch: &mut Vec<scootui::region::Rect>,
    sent: &mut Vec<scootui::region::Rect>,
    layout: &Layout,
    round: &Round,
    surface: &WlSurface,
    globals: &super::wayland::Globals,
    qh: &QueueHandle<State>,
    origin: (u32, u32),
) {
    popup_rects(
        layout.width,
        layout.height,
        round.outer().radius(),
        origin,
        scratch,
    );
    if scratch == sent {
        return;
    }
    let region = globals.compositor.create_region(qh, ());
    for rect in scratch.iter() {
        region.add(int(rect.x), int(rect.y), int(rect.width), int(rect.height));
    }
    surface.set_input_region(Some(&region));
    region.destroy();
    std::mem::swap(scratch, sent);
}
