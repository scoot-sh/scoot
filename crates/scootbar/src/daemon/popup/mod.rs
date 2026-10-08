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
//!    made again, a scale change, a reload. [`Popups::close`] is
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
use crate::popup::{self, Activate, Content, Interaction, Layout, Round, Wheel};
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
        if let Some(keyboard) = self.keyboard.take() {
            if keyboard.version() >= 3 {
                keyboard.release();
            }
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
    /// delay may be new.
    pub fn reset_hover(&mut self, delay: std::time::Duration) {
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
/// so this fires once per size change, never in a loop.
fn resized(layout: &Layout, dims: (u32, u32)) -> bool {
    (layout.width, layout.height) != dims
}

/// A protocol `int` for a size or a coordinate: saturated.
fn int(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
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
        let em = render::em(style.font_size, scale);
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
        let requested = Size {
            width: scale.logical_ceil(layout.width).clamp(1, MAX_SIDE),
            height: scale.logical_ceil(layout.height).clamp(1, MAX_SIDE),
        };
        let dims = scale
            .buffer(requested)
            .ok_or("the popup is too large for a buffer")?;
        // The buffer is the logical size at the scale, which rounds up from
        // what the layout needed: the frame is drawn at the buffer's edge.
        layout.width = dims.0;
        layout.height = dims.1;
        // The frame's shape at this size and scale: the radius cut back to
        // what the buffer holds, the corner tables built once and read
        // every frame it is drawn.
        let round = Round::new(
            render::device(style.popup_radius, scale),
            frame,
            dims.0,
            dims.1,
        );

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
        if round.rounded() {
            // The corners are transparent: the compositor blends only
            // them and skips blending under the rest (all but the corner
            // squares), in logical pixels like the bar's own region. The
            // default (the whole surface opaque) would show the cleared
            // corners as black, so a rounded tooltip needs this too.
            let radius = crate::region::effective_radius(
                style.popup_radius,
                requested.width,
                requested.height,
            );
            super::canvas::set_opaque_region(
                &self.globals,
                qh,
                &surface,
                (requested, Some(radius.saturating_add(1))),
            );
            // A click popup's input shape is the rounded one, as the bar's
            // own is, so a click in a cut corner does not land on the
            // popup. A tooltip keeps its empty input region instead (it
            // never takes a click).
            if matches!(flavor, Flavor::Popup { .. }) {
                super::canvas::set_input_region(&self.globals, qh, &surface, (requested, radius));
            }
        }
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
        ) {
            if let Some(held) = self.warned.allow(std::time::Instant::now()) {
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
                // change, never in a loop.
                if resized(&open.layout, open.dims) {
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
                // The popup keeps the size it opened at.
                open.layout.width = open.dims.0;
                open.layout.height = open.dims.1;
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
    let rounded = open.round.rounded();
    let slot = match open.pool.take(globals, qh, open.id, open.dims, rounded) {
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
    surface.commit();
    slot.held = true;
    open.dirty = false;
    true
}
