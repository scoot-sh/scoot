//! Moving and resizing floating windows with the pointer.
//!
//! Two ways in, one grab:
//!
//! - **The modifier.** `[floating] modifier` (Super unless configured)
//!   held with the left button anywhere on a floating window moves it; with
//!   the right button it resizes it from the nearest edge or corner (the
//!   window split in thirds each way, the middle third going to the nearest
//!   corner). The press focuses the window as any click does, then the grab
//!   takes the pointer and the press is swallowed: the window's client sees
//!   a pointer `leave`, no button, and an `enter` when it ends. On a tiled
//!   window the press is an ordinary click -- tiled windows are the strip's
//!   to place.
//! - **The client's own request** (`xdg_toplevel.move` / `.resize`, what a
//!   CSD titlebar or border sends while its button is held). Honoured only
//!   while that button is still held: the request's serial must be the
//!   press serial of the pointer's live implicit grab (`has_grab`), and the
//!   surface that press went to must be the requesting client's -- so a
//!   client can drag only with its own press, never borrow another's, and a
//!   stale serial finds no grab. A request from a tiled window (or a
//!   fullscreen one) is ignored: nothing in the strip is placed by
//!   dragging, and a toolkit whose request goes unanswered simply stays in
//!   its own drag. A request while locked is ignored too.
//!
//! What the grab does per motion is the whole hot path, and it allocates
//! nothing: it asks the core to move or resize the window
//! ([`Action::MoveFloating`] / [`Action::ResizeFloating`] -- the core keeps
//! the geometry, so the window stays where it was dragged), reads back where
//! that put it ([`World::floating_geometry`](scoot_core::World::floating_geometry),
//! the same arithmetic `arrange` uses), and moves the window's element in
//! the space to match -- no `apply()`, which arranges everything and would
//! allocate per motion. A resize also sends the client a configure with the
//! `resizing` state -- only when the size asked for changed and the client
//! has acked every configure before it, so configures go out at the rate
//! the client answers them, not the mouse's (a configure is Smithay's
//! allocation, about eleven of them, measured); the window's position
//! follows when it draws that size (`observe_frame`), the edge the user is
//! not dragging held by the core.
//!
//! **Every callback runs inside Smithay's pointer lock** (`PointerHandle`
//! holds its mutex around the grab), so nothing here may reach the pointer:
//! no `apply()` (it can re-derive pointer focus), no `refresh_pointer_focus`,
//! no `PointerHandle` call. What needs a full `apply()` -- the grab ending,
//! or the window crossing onto another output (workspaces, output stamps,
//! focus) -- sets [`State::floating_grab_resync`] instead, and the input
//! path that ran the grab settles it once the pointer call has returned
//! ([`State::settle_floating_grab`]).
//!
//! **Ending**, so no grab outlives what it was for: the button that started
//! it is released; any button is pressed (a second button, or the same one
//! again after a release that never arrived); the window closes, stops
//! floating, goes fullscreen or stops being shown (a workspace switch) --
//! checked at every motion, and at once on close; the session locks (the
//! lock transition drops every grab); the session is paused for a VT
//! switch (its button release will never arrive); and an output changes
//! size under it (the geometry it started from no longer holds).

use scoot_core::{Action, Edges, OutputId, Rect, Size, WindowId};
use scoot_ipc::{Modifier, PointerButton};
use smithay::backend::input::{ButtonState, InputTime};
use smithay::desktop::Window;
use smithay::input::Seat;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, CursorIcon, CursorImageStatus, Focus, GestureHoldBeginEvent,
    GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
    GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData,
    MotionEvent, PointerGrab, PointerHandle, PointerInnerHandle, RelativeMotionEvent,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::utils::{Logical, Point, SERIAL_COUNTER, Serial};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData};

use crate::compositor::State;
use crate::compositor::input::button_code;
use crate::compositor::layer_shell;

#[cfg(test)]
mod tests;

/// What a floating window's pointer grab does with the motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::compositor) enum Drag {
    Move,
    /// Moving these edges; the others stay put.
    Resize(Edges),
}

/// A floating window being moved or resized by the pointer. See the module
/// doc.
pub(in crate::compositor) struct FloatingGrab {
    /// No focus (the window's client gets no pointer events while it is
    /// dragged), the button that ends the grab, and where the pointer was
    /// when the drag began: motion is measured from there.
    start_data: GrabStartData<State>,
    id: WindowId,
    window: Window,
    drag: Drag,
    /// Where the window was placed when the drag began.
    start: Rect,
    /// The output it is on, as of the last motion: a change means it
    /// crossed onto another output, which needs a full `apply()`.
    output: OutputId,
    /// The size the last configure of a resize asked for, so a motion that
    /// changes nothing about it sends nothing.
    requested: Option<Size>,
    /// Whether the next press is the one that started the grab (a modifier
    /// drag installs the grab just before its own press is delivered), to be
    /// swallowed rather than treated as a second button.
    swallow_press: bool,
    /// `unset` has run: it can be reached twice (an explicit end, then
    /// Smithay's own unset on replacement), and must act once.
    ended: bool,
}

impl FloatingGrab {
    /// Applies one pointer position to the window. Answers whether the grab
    /// still has a window to drag.
    fn follow(&mut self, state: &mut State, location: Point<f64, Logical>) -> bool {
        // Rounded, and `as` saturates: a pointer position is bounded by the
        // outputs anyway, and the core saturates everything it adds.
        let dx = (location.x - self.start_data.location.x).round() as i32;
        let dy = (location.y - self.start_data.location.y).round() as i32;
        let action = match self.drag {
            Drag::Move => Action::MoveFloating {
                id: self.id,
                x: self.start.x.saturating_add(dx),
                y: self.start.y.saturating_add(dy),
            },
            Drag::Resize(edges) => Action::ResizeFloating {
                id: self.id,
                size: resized(self.start, edges, dx, dy),
                edges,
            },
        };
        // No effects come back from either (an empty `Vec`, unallocated).
        state.world.handle_action(action);
        let Some(geometry) = state.world.floating_geometry(self.id) else {
            // Closed, tiled again, or fullscreen: nothing left to drag.
            return false;
        };
        if state.space.element_location(&self.window).is_none() {
            // No longer shown -- its workspace was switched away.
            return false;
        }
        if geometry.output != self.output {
            self.output = geometry.output;
            state.floating_grab_resync = true;
        } else {
            state
                .space
                .relocate_element(&self.window, (geometry.rect.x, geometry.rect.y));
        }
        // A new size goes out once the client has answered every configure
        // it was already sent: a mouse reports at up to 1000Hz and a client
        // draws at its frame rate, so a configure per motion would only queue
        // sizes it has to skip (and cost Smithay's allocations per
        // configure). The size the core asks for is current either way; the
        // next motion after the ack sends it, and so does the `apply()` the
        // client's resized frame provokes (`observe_frame`).
        if matches!(self.drag, Drag::Resize(_))
            && geometry.requested != self.requested
            && !awaiting_ack(&self.window)
        {
            self.requested = geometry.requested;
            configure_resizing(&self.window, Some(geometry.requested), true);
        }
        state.request_render();
        true
    }
}

impl PointerGrab<State> for FloatingGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(
            <State as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &MotionEvent,
    ) {
        // No client gets pointer events while a window is dragged.
        handle.motion(data, None, event);
        if !self.follow(data, event.location) {
            handle.unset_grab(self, data, event.serial, event.time, false);
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(
            <State as smithay::input::SeatHandler>::PointerFocus,
            Point<f64, Logical>,
        )>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, None, event);
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        // Nothing is delivered: the dragged window's client never saw the
        // press of a modifier drag, and a client that asked to be dragged
        // gave its press to the compositor.
        let ends = match event.state {
            ButtonState::Pressed
                if self.swallow_press && event.button == self.start_data.button =>
            {
                self.swallow_press = false;
                false
            }
            ButtonState::Pressed => true,
            ButtonState::Released => event.button == self.start_data.button,
        };
        if ends {
            handle.unset_grab(self, data, event.serial, event.time, false);
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<State> {
        &self.start_data
    }

    /// The grab is over, however it ended. Inside the pointer lock like
    /// every callback, so the full `apply()` it wants is left to
    /// [`State::settle_floating_grab`] -- which also gives pointer focus
    /// back: the grab's own ends unset without Smithay's focus restore, so
    /// the surface under the pointer is entered the ordinary way.
    fn unset(&mut self, data: &mut State) {
        if std::mem::replace(&mut self.ended, true) {
            return;
        }
        if matches!(self.drag, Drag::Resize(_)) {
            // The size stays what the core asks for (`apply()` re-sends it);
            // only the `resizing` state goes.
            configure_resizing(&self.window, None, false);
        }
        data.cursor.set_status(CursorImageStatus::default_named());
        data.cursor_changed();
        data.floating_grab_resync = true;
        data.request_render();
    }
}

/// The size a resize drag asks for when the pointer has moved `dx`, `dy`
/// since it began on a window placed at `start`: the dragged edges follow
/// the pointer. The core clamps the rest (limits, room, at least 1).
fn resized(start: Rect, edges: Edges, dx: i32, dy: i32) -> Size {
    let w = if edges.right {
        start.w.saturating_add(dx)
    } else if edges.left {
        start.w.saturating_sub(dx)
    } else {
        start.w
    };
    let h = if edges.bottom {
        start.h.saturating_add(dy)
    } else if edges.top {
        start.h.saturating_sub(dy)
    } else {
        start.h
    };
    Size::new(w, h)
}

/// Sends a toplevel the `resizing` state (or takes it away), with a new
/// size when `size` is `Some` (the inner `None` is "choose your own"). A
/// configure goes out only if that changed what the client was last told,
/// and nothing goes to a toplevel that is already gone.
fn configure_resizing(window: &Window, size: Option<Option<Size>>, resizing: bool) {
    let Some(toplevel) = window.toplevel().filter(|toplevel| toplevel.alive()) else {
        return;
    };
    toplevel.with_pending_state(|state| {
        if let Some(size) = size {
            state.size = size.map(|size| (size.w, size.h).into());
        }
        if resizing {
            state.states.set(xdg_toplevel::State::Resizing);
        } else {
            state.states.unset(xdg_toplevel::State::Resizing);
        }
    });
    toplevel.send_pending_configure();
}

/// Whether the toplevel has configures it has not acked yet. One lock of
/// its role data; nothing allocated. A toplevel that is already gone is
/// awaiting nothing (and `configure_resizing` sends it nothing either).
fn awaiting_ack(window: &Window) -> bool {
    let Some(toplevel) = window.toplevel().filter(|toplevel| toplevel.alive()) else {
        return false;
    };
    with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| {
                data.lock()
                    .ok()
                    .map(|data| !data.pending_configures().is_empty())
            })
            .unwrap_or(false)
    })
}

/// Which edges a modifier resize moves when it grabs `rect` at `at`: the
/// thirds nearest the pointer on each axis, and for the middle third on both
/// axes, the nearest corner -- a resize always moves something.
pub(in crate::compositor) fn nearest_edges(rect: Rect, at: Point<f64, Logical>) -> Edges {
    let fx = (at.x - f64::from(rect.x)) / f64::from(rect.w.max(1));
    let fy = (at.y - f64::from(rect.y)) / f64::from(rect.h.max(1));
    let third = 1.0 / 3.0;
    let edges = Edges {
        left: fx < third,
        right: fx > 1.0 - third,
        top: fy < third,
        bottom: fy > 1.0 - third,
    };
    if edges.left || edges.right || edges.top || edges.bottom {
        return edges;
    }
    Edges {
        left: fx < 0.5,
        right: fx >= 0.5,
        top: fy < 0.5,
        bottom: fy >= 0.5,
    }
}

/// The edges an `xdg_toplevel.resize` names, or `None` for `none` (and any
/// value a newer protocol adds): nothing to resize.
pub(in crate::compositor) fn requested_edges(edge: xdg_toplevel::ResizeEdge) -> Option<Edges> {
    use xdg_toplevel::ResizeEdge;
    let (left, right, top, bottom) = match edge {
        ResizeEdge::Top => (false, false, true, false),
        ResizeEdge::Bottom => (false, false, false, true),
        ResizeEdge::Left => (true, false, false, false),
        ResizeEdge::Right => (false, true, false, false),
        ResizeEdge::TopLeft => (true, false, true, false),
        ResizeEdge::TopRight => (false, true, true, false),
        ResizeEdge::BottomLeft => (true, false, false, true),
        ResizeEdge::BottomRight => (false, true, false, true),
        _ => return None,
    };
    Some(Edges {
        left,
        right,
        top,
        bottom,
    })
}

/// The cursor shown while a drag lasts.
fn drag_cursor(drag: Drag) -> CursorIcon {
    let Drag::Resize(edges) = drag else {
        return CursorIcon::Grabbing;
    };
    match (edges.left, edges.right, edges.top, edges.bottom) {
        (true, _, true, _) => CursorIcon::NwResize,
        (_, true, true, _) => CursorIcon::NeResize,
        (true, _, _, true) => CursorIcon::SwResize,
        (_, true, _, true) => CursorIcon::SeResize,
        (true, ..) => CursorIcon::WResize,
        (_, true, ..) => CursorIcon::EResize,
        (_, _, true, _) => CursorIcon::NResize,
        _ => CursorIcon::SResize,
    }
}

impl State {
    /// A press of `button` with `[floating] modifier` held, over a floating
    /// window: starts moving (left) or resizing (right) it, and answers
    /// whether it did -- the grab then swallows this press. Called after the
    /// press has focused (and so raised) the window, and before the press is
    /// delivered, with the press's own serial.
    ///
    /// Nothing starts while locked, while the pointer is already grabbed (a
    /// menu's popup grab, a drag-and-drop, another button held), over a
    /// layer surface drawn above windows, or over a tiled or fullscreen
    /// window.
    pub(in crate::compositor) fn begin_modifier_drag(
        &mut self,
        pointer: &PointerHandle<Self>,
        button: PointerButton,
        serial: Serial,
    ) -> bool {
        if matches!(button, PointerButton::Middle)
            || self.session_lock.is_locked()
            || !self.drag_modifier_held()
            || pointer.is_grabbed()
        {
            return false;
        }
        let location = pointer.current_location();
        if self
            .layer_under(&layer_shell::ABOVE_WINDOWS, location)
            .is_some()
        {
            return false;
        }
        let Some(window) = self
            .window_element_under(location)
            .map(|(window, _)| window.clone())
        else {
            return false;
        };
        let Some(id) = self
            .windows
            .iter()
            .find(|(_, candidate)| **candidate == window)
            .map(|(&id, _)| id)
        else {
            return false;
        };
        let Some(geometry) = self.world.floating_geometry(id) else {
            return false;
        };
        let drag = match button {
            PointerButton::Left => Drag::Move,
            _ => Drag::Resize(nearest_edges(geometry.rect, location)),
        };
        self.start_floating_grab(
            pointer,
            FloatingGrabStart {
                id,
                window,
                drag,
                button: button_code(button),
                location,
                serial,
                swallow_press: true,
            },
        );
        true
    }

    /// `xdg_toplevel.move` (`edges` `None`) or `.resize` from `surface`'s
    /// client. See the module doc for what is honoured.
    pub(in crate::compositor) fn client_floating_drag(
        &mut self,
        surface: &ToplevelSurface,
        seat: &WlSeat,
        serial: Serial,
        edges: Option<Edges>,
    ) {
        // `None` only for a `wl_seat` that is already dead: a client that
        // disconnected mid-request.
        let Some(seat) = Seat::<Self>::from_resource(seat) else {
            return;
        };
        if seat != self.seat || self.session_lock.is_locked() {
            return;
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // The press this request rides on must still be held, and be the one
        // with this serial.
        if !pointer.has_grab(serial) {
            tracing::debug!(
                ?serial,
                "refusing an interactive move/resize: no button press held with that serial"
            );
            return;
        }
        let Some(start) = pointer.grab_start_data() else {
            return;
        };
        // ...and it must have gone to this client (a modifier drag's own
        // grab has no focus, so no client can take it over either).
        let pressed_here = start
            .focus
            .as_ref()
            .is_some_and(|(pressed, _)| pressed.id().same_client_as(&surface.wl_surface().id()));
        if !pressed_here {
            tracing::debug!(
                "refusing an interactive move/resize: the held press went to another client"
            );
            return;
        }
        let Some(id) = self.id_of(surface.wl_surface()) else {
            return;
        };
        let Some(window) = self.windows.get(&id).cloned() else {
            return;
        };
        if self.world.floating_geometry(id).is_none()
            || self.space.element_location(&window).is_none()
        {
            tracing::debug!(
                ?id,
                "ignoring an interactive move/resize: the window is not a floating window on screen"
            );
            return;
        }
        let drag = edges.map_or(Drag::Move, Drag::Resize);
        self.start_floating_grab(
            &pointer,
            FloatingGrabStart {
                id,
                window,
                drag,
                button: start.button,
                // The press, not where the pointer is now: a toolkit asks
                // once the pointer has moved past its drag threshold, and
                // measuring from the press keeps the point the user pressed
                // under the pointer.
                location: start.location,
                serial,
                swallow_press: false,
            },
        );
    }

    fn start_floating_grab(&mut self, pointer: &PointerHandle<Self>, start: FloatingGrabStart) {
        if matches!(start.drag, Drag::Resize(_)) {
            // Once per grab: the limits the resize clamps to, as the window
            // has them now (the core's copy is only refreshed on a title,
            // app id or parent change).
            self.sync_size_hints(start.id);
        }
        let Some(geometry) = self.world.floating_geometry(start.id) else {
            return;
        };
        let icon = drag_cursor(start.drag);
        let grab = FloatingGrab {
            start_data: GrabStartData {
                focus: None,
                button: start.button,
                location: start.location,
            },
            id: start.id,
            window: start.window,
            drag: start.drag,
            start: geometry.rect,
            output: geometry.output,
            requested: geometry.requested,
            swallow_press: start.swallow_press,
            ended: false,
        };
        tracing::debug!(id = ?start.id, drag = ?start.drag, "a floating window's pointer drag began");
        pointer.set_grab(self, grab, start.serial, Focus::Clear);
        // After the grab is set, not before: clearing focus sends the
        // client its `leave`, and Smithay resets the cursor to the default
        // with it. Nothing resets it again until the grab ends (its motion
        // keeps focus empty).
        self.cursor.set_status(CursorImageStatus::Named(icon));
        self.cursor_changed();
    }

    /// Whether `[floating] modifier` is held right now.
    fn drag_modifier_held(&self) -> bool {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        let held = keyboard.modifier_state();
        match self.floating_modifier {
            Modifier::Super => held.logo,
            Modifier::Alt => held.alt,
            Modifier::Ctrl => held.ctrl,
            Modifier::Shift => held.shift,
        }
    }

    /// The window a floating pointer grab is dragging, if one is.
    pub(in crate::compositor) fn floating_grab_window(&self) -> Option<WindowId> {
        let pointer = self.seat.get_pointer()?;
        pointer
            .with_grab(|_, grab| grab.downcast_ref::<FloatingGrab>().map(|grab| grab.id))
            .flatten()
    }

    /// Ends a floating window's pointer grab, if one is running; any other
    /// grab is left alone. Leaves the `apply()` it asks for to the caller
    /// (every caller applies, or calls [`State::settle_floating_grab`]).
    /// Must not be called from inside a pointer callback.
    pub(in crate::compositor) fn end_floating_grab(&mut self) {
        if self.floating_grab_window().is_none() {
            return;
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let time = InputTime::from_millis(self.millis());
        pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
    }

    /// Runs the `apply()` a floating grab asked for from inside the pointer
    /// lock, if it asked. One `bool` test otherwise, which is what the
    /// per-motion path pays.
    pub(in crate::compositor) fn settle_floating_grab(&mut self) {
        if !self.floating_grab_resync {
            return;
        }
        self.apply();
        // A drag the grab ended itself left pointer focus empty (it unsets
        // without Smithay's focus restore): re-derived here through the
        // ordinary arrival path, so the window under the pointer gets its
        // `enter` -- and a pointer lock or confinement waiting on it
        // engages, as on any arrival (`move_absolute`). Idempotent where
        // `apply()` already re-derived it.
        if self.floating_grab_window().is_none() {
            self.refresh_pointer_focus();
        }
    }

    /// Re-reads a window's size limits into the core when they changed: the
    /// core's copy is taken when the window is created and on a title, app
    /// id or parent change, and `set_min_size`/`set_max_size` land at a
    /// commit, usually after the first of those.
    fn sync_size_hints(&mut self, id: WindowId) {
        let info = self.info_of(id);
        if self
            .world
            .window_info(id)
            .is_some_and(|current| current.hints != info.hints)
        {
            self.world
                .handle_event(scoot_core::Event::WindowChanged { id, info });
        }
    }
}

/// What starting a floating grab needs, gathered by its two callers.
struct FloatingGrabStart {
    id: WindowId,
    window: Window,
    drag: Drag,
    /// The `BTN_*` code whose release ends the grab.
    button: u32,
    /// Where the pointer was when the drag began.
    location: Point<f64, Logical>,
    serial: Serial,
    swallow_press: bool,
}
