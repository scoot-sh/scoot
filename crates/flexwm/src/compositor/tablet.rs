//! `zwp_tablet_manager_v2` (version 1): drawing-tablet input.
//!
//! Smithay carries the whole protocol at the pinned rev
//! (`TabletManagerState` under `src/wayland/tablet_manager.rs`, the
//! `TabletSeat` under `src/input/tablet/` -- verified in source, not
//! assumed), so flexwm's side is the same shape as the other Smithay-carried
//! globals: one hold-alive field on [`State`] (see
//! [`State::new`](super::State::new)), the [`TabletSeatHandler`] impl in
//! `handlers.rs`, and the event plumbing below. No Smithay patch vendored;
//! everything stays in flexwm's handler layer.
//!
//! ## What a pen does
//!
//! A tablet tool is a cursor *and* a clicker, not a third focus system. Every
//! tool event runs through the same paths pointer events already use, so the
//! pen cannot disagree with the mouse about what is focused:
//!
//! - proximity and motion run [`State::pointer_move`]: the cursor follows the
//!   tool, pointer focus (and `enter` serials, constraints, relative motion)
//!   is derived exactly as for mouse motion, and the tool's own
//!   proximity/motion/axis events go to whatever `surface_under` found at
//!   the same location. The tool focus is re-read per event rather than
//!   cached: what is on screen can change between two tool events without
//!   anything moving (a window mapping under a hovering pen), and the hit
//!   test is what the pointer half already pays.
//! - tip down/up runs [`State::pointer_button`] with the left button: a pen
//!   tap focuses, activates and clicks like a mouse click -- including the
//!   interaction-serial record (so a tap mints activation tokens like a
//!   click) and the popup-grab settle (so a tap outside a menu dismisses
//!   it) -- and delivers the tool's own down/up alongside. Tip up always
//!   releases, so a tap can never wedge the pointer's implicit grab.
//! - barrel buttons deliver the tool's own button event only: a stylus
//!   button has no defined mapping onto pointer buttons, and inventing one
//!   (barrel-equals-right-click) would be bespoke semantics no protocol
//!   asks for.
//!
//! The tap is recorded once, as the pointer click it performs, not twice:
//! tool down/up serials are deliberately *not* added to
//! `interaction_serials` (unlike the button serials `pointer_button`
//! records). A client activating from the click's serial works; a serial
//! for the same physical tap under a second name would only widen what a
//! background client can guess.
//!
//! ## Session lock
//!
//! Nothing here special-cases the lock, because neither path it reuses
//! does: `surface_under` answers the lock surface while locked, and
//! `pointer_button`'s `focus_under_pointer` refuses to move window focus
//! behind it. A pen tap on the lock screen therefore reaches the locker
//! through both the tool and the pointer halves and moves nothing behind
//! it -- by construction on the two paths the session-lock suites already
//! pin, not by a third re-derivation that could disagree with them.
//!
//! ## What is deferred, and why
//!
//! - **Pads, strips and rings**: Smithay's input tablet module says it
//!   outright at the pinned rev ("Currently, pads are unsupported by this
//!   module"), and no pad object type exists anywhere in its tablet stack --
//!   there is nothing to drive, so this is deferred upstream, not omitted.
//! - **A per-client seat-object budget**: each `get_tablet_seat` mints a
//!   small seat object, unbounded per client -- the same standing exposure
//!   as `wl_seat.get_pointer`/`get_keyboard`, which this compositor has
//!   never budgeted either. No new policy is invented for the tablet twin.
//! - **Tool cursor images** (`tablet_tool_image`) land in the same
//!   [`Cursor`](super::cursor::Cursor) status a pointer cursor would: while a tool drives the
//!   pointer the tool's image *is* the cursor, so a second cursor store
//!   would only be a second notion of what is drawn at the pointer.
//!
//! ## Cost
//!
//! One `tablet_seat()` lookup (an `Arc` clone) plus one tool-map lookup per
//! tool event, beside the hit test and socket work the pointer half already
//! pays. `add_wp_tool`/`add_wp_tablet` run only for a tool/tablet the seat
//! has never seen, never per event. The axis frame is a stack struct of
//! `Option`s; nothing here allocates per event beyond what `pointer_move`
//! already does.

use smithay::backend::input::{ButtonState, InputTime, TabletToolDescriptor};
use smithay::input::pointer::CursorImageStatus;
use smithay::input::tablet::tool::{
    AxisFrame, ButtonEvent as ToolButtonEvent, DownEvent, MotionEvent as ToolMotionEvent,
    ProximityInEvent, ProximityOutEvent, UpEvent,
};
use smithay::input::tablet::{TabletDescriptor, TabletSeatTrait};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use super::State;
use flexwm_ipc::PointerButton;

#[cfg(test)]
mod tests;

impl State {
    /// Exposes a newly-seen tablet device to tablet clients, the anvil shape
    /// at the pinned rev: called from the libinput `DeviceAdded` arm for a
    /// device with the `TabletTool` capability. A tablet the seat already
    /// knows is re-created (Smithay's `add_tablet` removes first), so a
    /// device reappearing keeps no stale handle.
    pub(super) fn tablet_added(&mut self, tablet: &TabletDescriptor) {
        let dh = self.display_handle.clone();
        self.seat.tablet_seat().add_wp_tablet(&dh, tablet);
    }

    /// Forgets a removed tablet device, the anvil shape: called from the
    /// libinput `DeviceRemoved` arm. When no tablets remain the tools go
    /// too -- a tool with no tablet to be in proximity with can never
    /// produce another event, so keeping its handle would only keep dead
    /// client objects alive. A tool still in proximity keeps its `Tablet`
    /// alive through its own `Arc` until proximity-out, by Smithay's own
    /// construction, not by anything retained here.
    pub(super) fn tablet_removed(&mut self, tablet: &TabletDescriptor) {
        let tablet_seat = self.seat.tablet_seat();
        tablet_seat.remove_tablet(tablet);
        if tablet_seat.count_tablets() == 0 {
            tablet_seat.clear_tools();
        }
    }

    /// Routes one tool proximity event: the cursor moves first (so pointer
    /// focus lands where the tool is), then the tool's own proximity
    /// in/out, each framed.
    ///
    /// Coordinates are logical output pixels -- the libinput caller maps the
    /// device range onto the output first (the same
    /// `position_transformed` the absolute-pointer arm uses), so this stays
    /// backend-neutral like every other method the backends share.
    ///
    /// Activity is announced by the [`State::pointer_move`] below, not
    /// here: the entering half always moves the pointer, so announcing
    /// here too would reset the idle timers twice per arrival. (The
    /// departing half moves nothing and announces nothing -- a tool
    /// leaving is not a user at the machine.)
    pub fn tablet_proximity(
        &mut self,
        tablet: &TabletDescriptor,
        tool: &TabletToolDescriptor,
        entering: bool,
        x: f64,
        y: f64,
        axis: AxisFrame,
    ) {
        let location = Point::<f64, Logical>::from((x, y));
        let serial = SERIAL_COUNTER.next_serial();
        let time = InputTime::from_millis(self.millis());
        if entering {
            // Before the tool half, not after: `pointer_move` derives the
            // pointer focus the tool events are then addressed to, and a
            // pen appearing over a window must enter it on the pointer
            // half too -- otherwise the cursor would sit on a surface that
            // never saw `enter`.
            self.pointer_move(x, y);
            let under = self.surface_under(location);
            let handle = self.tool_or_add(tablet, tool);
            handle.proximity_in(
                self,
                under,
                self.tool_tablet(tablet),
                &ProximityInEvent {
                    location,
                    axis: Some(axis),
                    serial,
                    time,
                },
            );
            handle.frame(self, time);
        } else if let Some(handle) = self.tool(tool) {
            // No pointer move: the tool left, the cursor stays where it
            // was -- there is no surface under a departed tool to focus.
            // Unknown tools stay silent (Smithay warns and ignores a
            // proximity-out with no matching proximity-in); an out is not
            // a click, so unlike the tip path below there is no pointer
            // half to run regardless.
            handle.proximity_out(self, &ProximityOutEvent { serial, time });
            handle.frame(self, time);
        }
    }

    /// Routes one tool motion/axis event: the cursor moves (pointer focus,
    /// constraints and relative motion all follow), then the tool's own
    /// axis and motion, framed together.
    ///
    /// Motion for a tool with no proximity-in is pointer-only, the anvil
    /// shape: only proximity announces a tool to clients, so a motion the
    /// seat never saw enter has no tool object to address -- but the cursor
    /// still follows, because the device is physically there.
    ///
    /// Activity is announced by the [`State::pointer_move`] below, not
    /// here: this always moves the pointer, so announcing here too would
    /// reset the idle timers twice per event.
    pub fn tablet_motion(&mut self, tool: &TabletToolDescriptor, x: f64, y: f64, axis: AxisFrame) {
        let location = Point::<f64, Logical>::from((x, y));
        let serial = SERIAL_COUNTER.next_serial();
        let time = InputTime::from_millis(self.millis());
        self.pointer_move(x, y);
        if let Some(handle) = self.tool(tool) {
            let under = self.surface_under(location);
            handle.axis(self, axis);
            handle.motion(
                self,
                under,
                &ToolMotionEvent {
                    location,
                    serial,
                    time,
                },
            );
            handle.frame(self, time);
        }
    }

    /// Routes one tip event: the cursor moves to the tip, the tool's own
    /// down/up goes out framed, and the tip *clicks* -- a left press on the
    /// way down, its release on the way up, through the same
    /// [`State::pointer_button`] a mouse click uses.
    ///
    /// The click runs even for a tool the seat never saw (a tip with no
    /// proximity-in): the physical action is unambiguous -- the user
    /// pressed *there* -- and dropping it would lose input over a
    /// bookkeeping miss. The tool half stays silent in that case, the
    /// anvil shape.
    ///
    /// Activity is announced by the [`State::pointer_move`] and
    /// [`State::pointer_button`] below, not here: a tip always moves the
    /// pointer and always clicks, so announcing here too would reset the
    /// idle timers three times per tap.
    pub fn tablet_tip(&mut self, tool: &TabletToolDescriptor, down: bool, x: f64, y: f64) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = InputTime::from_millis(self.millis());
        self.pointer_move(x, y);
        if let Some(handle) = self.tool(tool) {
            if down {
                handle.down(self, &DownEvent { serial, time });
            } else {
                handle.up(self, &UpEvent { serial, time });
            }
            handle.frame(self, time);
        }
        // After the tool half, not before: `down` installs the tool's own
        // implicit grab on the down surface, and the click below installs
        // the pointer's -- each device keeps its own, and neither delivery
        // consults the other's grab.
        self.pointer_button(PointerButton::Left, down);
    }

    /// Routes one barrel-button event: the tool's own button, framed. No
    /// pointer synthesis (see the module doc): a stylus button is not a
    /// mouse button under another name.
    ///
    /// Silent for a tool with no proximity-in, the anvil shape -- a button
    /// is addressed to whoever holds the tool's focus, and with no
    /// proximity there is no focus to hold it.
    pub fn tablet_button(&mut self, tool: &TabletToolDescriptor, button: u32, pressed: bool) {
        self.announce_activity();
        let serial = SERIAL_COUNTER.next_serial();
        let time = InputTime::from_millis(self.millis());
        if let Some(handle) = self.tool(tool) {
            handle.button(
                self,
                &ToolButtonEvent {
                    serial,
                    button,
                    state: if pressed {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    },
                    time,
                },
            );
            handle.frame(self, time);
        }
    }

    /// The tool image a client named for one of its tools -- through
    /// `zwp_tablet_tool_v2.set_cursor` or `wp_cursor_shape_v1`'s tablet
    /// device -- lands in the same cursor status a pointer image would.
    /// Shared with `SeatHandler::cursor_image` in `handlers.rs`, which is
    /// the pointer half of the same rule: only `--tty` draws a cursor at
    /// all, so only it needs a redraw when the request changes.
    pub(super) fn set_tool_cursor_image(&mut self, image: CursorImageStatus) {
        self.cursor.set_status(image);
        if self.tty.is_some() {
            self.request_render();
        }
    }

    /// The known handle for `tool`, if the seat ever saw it enter
    /// proximity. Read-only: never announces a tool on its own (see
    /// [`State::tool_or_add`]).
    fn tool(
        &self,
        tool: &TabletToolDescriptor,
    ) -> Option<smithay::input::tablet::tool::TabletToolHandle<State>> {
        self.seat.tablet_seat().get_tool(tool)
    }

    /// The known handle for `tool`, announcing it (and its tablet) to
    /// clients first if this is its first proximity. Only the proximity
    /// path announces: it is the one event the protocol defines as a
    /// tool's arrival.
    fn tool_or_add(
        &mut self,
        tablet: &TabletDescriptor,
        tool: &TabletToolDescriptor,
    ) -> smithay::input::tablet::tool::TabletToolHandle<State> {
        let tablet_seat = self.seat.tablet_seat();
        let dh: DisplayHandle = self.display_handle.clone();
        // The tablet first: a tool arriving on an unannounced tablet would
        // otherwise leave clients with a tool and no tablet for it to be
        // in proximity with.
        tablet_seat.add_wp_tablet(&dh, tablet);
        if let Some(handle) = tablet_seat.get_tool(tool) {
            handle
        } else {
            tablet_seat.add_wp_tool(self, &dh, tool)
        }
    }

    /// The seat's handle for `tablet`, for the proximity-in that names it.
    /// Announced just above in [`State::tool_or_add`], so this lookup
    /// cannot miss -- and if it ever did, addressing the proximity at no
    /// tablet would be a worse lie than saying so loudly.
    fn tool_tablet(&self, tablet: &TabletDescriptor) -> smithay::input::tablet::Tablet {
        self.seat
            .tablet_seat()
            .get_tablet(tablet)
            .expect("a tablet announced just above")
    }
}
