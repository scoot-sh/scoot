//! The seat's pointer on the daemon's side: `wl_pointer` events turned into
//! calls on the pure state machine (`crate::pointer`), and what it says to
//! do carried out on the bar (`crate::action`).
//!
//! **A pointer only when something needs one.** The bar binds the seat
//! when the compositor has one, but asks it for a `wl_pointer` only while
//! a placed module answers pointer input (a binding in the config, or a
//! default of the module's own: the workspaces module's click), or, with
//! tooltips on, a module that can show one (`Module::tooltips`: the battery,
//! the network, the window title, a `push` or `exec` one). A clock-only
//! bar with no bindings never takes the pointer, so it costs what it did
//! before there was any input, and a reload that adds or removes bindings
//! takes or drops it then ([`State::sync_pointer`]). The bar never takes
//! touch, and the keyboard only for a popup that grabbed, for as long as it
//! is open (`daemon::popup`), so it never disturbs focus otherwise.
//!
//! **Events cost almost nothing.** A motion stores two numbers; hover is
//! worked out once per turn of the loop, when the output's scene is asked
//! what to draw (`daemon::draw`), so a thousand motions a second are a
//! thousand stores and one hit test (and, with a pointer on the bar, one more
//! for the tooltip's hover: `daemon::popup::tooltip`). A press or release resolves the module
//! under the pointer with the layout the last draw committed (the pixels
//! the user saw), so a click during a redraw lands on what was on screen.
//! Nothing here allocates.

use std::time::{Duration, Instant};

use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};

use super::Content;
use super::wayland::State;
use crate::action::{self, Action, Effects, ScootAction, Trigger};
use crate::density::Scale;
use crate::modules::{ClickCtx, Input, OutputView, Placed};
use crate::outputs::Entry;
use crate::pointer::{Focus, Target, wants_pointer};
use crate::print::warn;
use crate::render;

/// The shortest time between two failure lines on stderr: a binding that
/// cannot run (a missing program) under a scroll would otherwise write a
/// line per action.
const WARN_EVERY: Duration = Duration::from_secs(1);

/// Rate-limits what a failing action says.
#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<Instant>,
    /// Failures not said since the last line.
    held: u32,
}

impl Throttle {
    /// Whether a failure at `now` may be said; a failure that may not is
    /// counted and reported with the next line that may.
    pub(crate) fn allow(&mut self, now: Instant) -> Option<u32> {
        if let Some(last) = self.last {
            if now.saturating_duration_since(last) < WARN_EVERY {
                self.held = self.held.saturating_add(1);
                return None;
            }
        }
        self.last = Some(now);
        Some(std::mem::take(&mut self.held))
    }
}

/// Surface-local logical pixels to device pixels at `scale`: pointer
/// coordinates arrive in the surface's (viewport destination) space.
fn to_device(x: f64, scale: Scale) -> u32 {
    if !x.is_finite() || x <= 0.0 {
        return 0;
    }
    match scale {
        Scale::Integer(factor) => (x * f64::from(factor.max(1))) as u32,
        Scale::Fractional(v120) => (x * f64::from(v120.max(1)) / 120.0) as u32,
    }
}

/// The pointer's x on `entry`'s bar in device pixels, if it is over this
/// output's bar (inside the surface as it is drawn).
pub(super) fn pointer_x(
    focus: Option<Focus>,
    entry: &Entry<super::surfaces::Objects>,
) -> Option<u32> {
    let focus = focus.filter(|focus| focus.output == entry.output.id())?;
    let size = entry.output.surface_size(&entry.objects.bar)?;
    let inside = (0.0..f64::from(size.height)).contains(&focus.y)
        && (0.0..f64::from(size.width)).contains(&focus.x);
    inside.then(|| to_device(focus.x, entry.output.scale()))
}

impl State {
    /// The module under the pointer, on the layout the last draw committed.
    pub(super) fn target(&self) -> Option<Target> {
        let focus = self.input.focus()?;
        let entry = self
            .outputs
            .iter()
            .find(|entry| entry.output.id() == focus.output)?;
        let x = pointer_x(Some(focus), entry)?;
        let member = entry.objects.scene.member_at(x)?;
        Some(Target {
            output: focus.output,
            member,
        })
    }

    /// Whether any placed module answers pointer input.
    pub fn needs_pointer(&self) -> bool {
        self.content.modules.iter().any(Placed::interactive) || self.wants_tooltips()
    }

    /// Whether tooltips are on and a placed module can have one: the bar
    /// then takes a pointer (and `xdg_wm_base`) to show it. A bar of modules
    /// that never have one (a clock) takes neither.
    pub fn wants_tooltips(&self) -> bool {
        #[cfg(feature = "popup")]
        {
            self.popup.tooltips_on()
                && self
                    .content
                    .modules
                    .iter()
                    .any(|placed| placed.module.tooltips())
        }
        #[cfg(not(feature = "popup"))]
        false
    }

    /// Takes a `wl_pointer` when a module needs one and the seat has one;
    /// lets go of it when none does. Called when the seat's capabilities
    /// change and after a reload.
    pub fn sync_pointer(&mut self, qh: &QueueHandle<Self>) {
        let want = self.seat_pointer && self.needs_pointer();
        match (&self.pointer, want, &self.globals.seat) {
            (None, true, Some(seat)) => self.pointer = Some(seat.get_pointer(qh, ())),
            (Some(pointer), false, _) => {
                // `release` exists from version 3; an older pointer stays
                // bound and unused (its events are ignored below).
                if pointer.version() >= 3 {
                    pointer.release();
                }
                self.pointer = None;
                self.input.leave();
            }
            _ => {}
        }
    }

    /// Carries out the scroll a frame's worth of events earned, at most
    /// one per [`crate::pointer::SCROLL_FRAME`]. Called once per turn of the
    /// loop, after the dispatch; reads the clock only while steps wait.
    pub fn pump_scroll(&mut self, now: &mut Option<Instant>) {
        if !self.input.scroll_waiting() {
            return;
        }
        let now = *now.get_or_insert_with(Instant::now);
        let Some((trigger, steps)) = self.input.take_scroll(now) else {
            return;
        };
        #[cfg(feature = "popup")]
        self.dismiss_tooltip();
        // What is under the pointer now; nothing there drops the steps.
        if let Some(target) = self.target() {
            self.fire(trigger, target, Some(steps), now);
        }
    }

    /// How long the loop may sleep for a scroll held back to its frame.
    pub fn scroll_timeout(&self, now: &mut Option<Instant>) -> Option<Duration> {
        if !self.input.scroll_waiting() {
            return None;
        }
        let now = *now.get_or_insert_with(Instant::now);
        self.input.scroll_wait(now)
    }

    /// How long the loop may sleep for a tooltip that is due (none, and no
    /// timeout at all, unless the pointer rests on a module that has one).
    pub fn tooltip_wait(&self, now: &mut Option<Instant>) -> Option<Duration> {
        #[cfg(feature = "popup")]
        {
            self.tooltip_timeout(now)
        }
        #[cfg(not(feature = "popup"))]
        {
            let _ = now;
            None
        }
    }

    /// Runs the action `trigger` means on `target`'s module: the config's
    /// binding, else the module's own default. `steps` is a scroll's.
    /// Says whether the module asks for its popup surface now (see
    /// [`crate::modules::Module::wants_popup`]): the caller opens it, a
    /// press's serial in hand for the grab.
    fn fire(&mut self, trigger: Trigger, target: Target, steps: Option<u32>, now: Instant) -> bool {
        let Some(entry) = self.outputs.get_mut(target.output) else {
            return false;
        };
        let scene = &entry.objects.scene;
        let (Some(span), Some(view), Some(module)) = (
            scene.spans().get(target.member).copied(),
            scene.view(target.member),
            scene.module(target.member),
        ) else {
            return false;
        };
        let Content {
            modules,
            text,
            style,
            ..
        } = &mut self.content;
        let (Some(text), Some(placed), Some(focus)) =
            (text.as_ref(), modules.get_mut(module), self.input.focus())
        else {
            return false;
        };
        let scale = entry.output.scale();
        let x = to_device(focus.x, scale);
        // The surface's device height, as the last draw sized it.
        let height = entry
            .output
            .surface_size(&entry.objects.bar)
            .and_then(|size| scale.buffer(size))
            .map_or(0, |(_, height)| height);
        let output = OutputView {
            name: entry.output.info().name.as_deref(),
        };
        // The click is measured in the output's own em, as its bar was
        // drawn: a press lands on what that output shows.
        let font_size = entry.objects.font_size;
        let at = ClickCtx {
            output,
            x: x.saturating_sub(span.x),
            view,
            text,
            em: render::em(font_size, scale),
            padding: render::device(style.padding, scale),
            span_width: span.width,
            height,
            scale,
        };
        let input = Input { trigger, at: &at };
        let Placed {
            id,
            module,
            bindings,
            revision,
        } = placed;
        let id: &'static str = id;
        let offered;
        let action: &Action = match bindings.get(trigger) {
            Some(bound) => bound,
            None => match module.on_input(&input) {
                Some(default) => {
                    offered = default;
                    &offered
                }
                None => return false,
            },
        };
        let mut effects = Launch::new(&mut self.spawner);
        let performed = action::perform(
            &mut **module,
            revision,
            &output,
            action,
            steps,
            &mut effects,
        );
        // A popup bound to a click opens on the press (`State::button`),
        // never here: what reaches this point asking for one is a scroll,
        // which carries no input serial a popup grab could use.
        let performed = match performed {
            Ok(()) if effects.popup => Err(action::Failed::Effect(
                "a popup opens from a click, not from a scroll".to_owned(),
            )),
            other => other,
        };
        if let Err(failure) = performed {
            if let Some(held) = self.warned.allow(now) {
                let more = if held > 0 {
                    format!(" ({held} more since)")
                } else {
                    String::new()
                };
                warn(format_args!(
                    "scootbar: {} on the {id} module: {failure}{more}",
                    trigger.key()
                ));
            }
            return false;
        }
        #[cfg(feature = "popup")]
        {
            module.wants_popup()
        }
        #[cfg(not(feature = "popup"))]
        {
            false
        }
    }

    /// A press or release at the pointer's place, `serial` the event's.
    fn button(&mut self, code: u32, pressed: bool, serial: u32, qh: &QueueHandle<Self>) {
        let target = self.target();
        if pressed {
            // A tooltip is not a popup: a press closes it and goes on to
            // act (the click is not spent on dismissing it).
            #[cfg(feature = "popup")]
            self.dismiss_tooltip();
            let armed = self.input.press(code, target);
            #[cfg(feature = "popup")]
            self.press_for_popups(armed, target, serial, qh);
            #[cfg(not(feature = "popup"))]
            let _ = (armed, serial, qh);
        } else if let Some((trigger, target)) = self.input.release(code, target) {
            #[cfg(feature = "popup")]
            let member = target.member;
            #[cfg(feature = "popup")]
            let output = target.output;
            let wants = self.fire(trigger, target, None, Instant::now());
            // A module action that names what to show (the tray's
            // `menu N`) opens its popup on the release, with the
            // release's serial for the grab. A press with a binding
            // already opened or closed above; a scroll never asks.
            #[cfg(feature = "popup")]
            if wants {
                if let Err(why) = self.open_popup(qh, output, member, Some(serial)) {
                    if let Some(held) = self.warned.allow(Instant::now()) {
                        let more = if held > 0 {
                            format!(" ({held} more since)")
                        } else {
                            String::new()
                        };
                        warn(format_args!("scootbar: popup: {why}{more}"));
                    }
                }
            }
            #[cfg(not(feature = "popup"))]
            let _ = wants;
        }
    }

    /// What a press means to popups, before the release can fire anything:
    ///
    /// - **A press on the bar while a popup is open dismisses it**, and the
    ///   click is spent doing so (disarmed, so the release fires nothing): a
    ///   second click on the module that opened the popup closes it rather
    ///   than opening it again, and a click on another module acts through
    ///   no dismissal.
    /// - **A popup bound to the click opens on the press, not the release**,
    ///   the one exception to "clicks fire on release" (`crate::pointer`):
    ///   a compositor may refuse a popup grab whose serial is not that of a
    ///   button *still held* (the protocol allows it; Smithay's reference
    ///   compositor checks it, and KDE and GNOME are known to), and a refused
    ///   grab is a popup that never takes a click-outside or Escape. Opened
    ///   here it has the press's serial while the button is down; the release
    ///   that follows fires nothing. Measured: scoot takes any recent input
    ///   serial and wlroots (sway) takes any at all (a bogus serial was
    ///   accepted), so no compositor available here tells press from release,
    ///   and this is the choice that cannot be the one refused.
    #[cfg(feature = "popup")]
    fn press_for_popups(
        &mut self,
        armed: Option<Trigger>,
        target: Option<Target>,
        serial: u32,
        qh: &QueueHandle<Self>,
    ) {
        if self.popup.is_open() {
            self.popup.close();
            self.input.disarm();
            return;
        }
        let (Some(trigger), Some(target)) = (armed, target) else {
            return;
        };
        if !self.binds_popup(trigger, target) {
            return;
        }
        self.input.disarm();
        if let Err(why) = self.open_popup(qh, target.output, target.member, Some(serial)) {
            if let Some(held) = self.warned.allow(Instant::now()) {
                let more = if held > 0 {
                    format!(" ({held} more since)")
                } else {
                    String::new()
                };
                warn(format_args!(
                    "scootbar: {} opening a popup: {why}{more}",
                    trigger.key()
                ));
            }
        }
    }

    /// Whether the config binds `trigger` on the module at `target` to its
    /// popup.
    #[cfg(feature = "popup")]
    fn binds_popup(&self, trigger: Trigger, target: Target) -> bool {
        let Some(entry) = self
            .outputs
            .iter()
            .find(|entry| entry.output.id() == target.output)
        else {
            return false;
        };
        let Some(placed) = entry
            .objects
            .scene
            .module(target.member)
            .and_then(|module| self.content.modules.get(module))
        else {
            return false;
        };
        matches!(
            placed.bindings.get(trigger),
            Some(Action::Module(named)) if named.name == action::POPUP
        )
    }
}

/// The two ways out of the process, for [`action::perform`] (a pointer press,
/// and `invoke`).
pub(super) struct Launch<'a> {
    spawner: &'a mut crate::spawn::Spawner,
    /// The action asked for the module's popup: the caller (which knows the
    /// output, the module's place and the input serial) opens it once
    /// [`action::perform`] returns.
    pub(super) popup: bool,
}

impl<'a> Launch<'a> {
    pub(super) fn new(spawner: &'a mut crate::spawn::Spawner) -> Self {
        Self {
            spawner,
            popup: false,
        }
    }
}

impl Effects for Launch<'_> {
    fn popup(&mut self) -> Result<(), String> {
        self.popup = true;
        Ok(())
    }

    fn exec(&mut self, argv: &[String]) -> Result<(), String> {
        self.spawner.spawn(argv)
    }

    fn scoot(&mut self, action: ScootAction) -> Result<(), String> {
        crate::scoot::send(action)
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities { capabilities } = event else {
            return;
        };
        // Touch and the keyboard are never taken.
        state.seat_pointer = matches!(capabilities, WEnum::Value(caps) if wants_pointer(caps));
        // The keyboard is not taken here: a popup that grabbed takes it for
        // its own lifetime, for Escape (`popup`).
        #[cfg(feature = "popup")]
        {
            state.seat_keyboard = matches!(
                capabilities,
                WEnum::Value(caps) if caps.contains(wl_seat::Capability::Keyboard)
            );
        }
        state.sync_pointer(qh);
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        pointer: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if state.pointer.as_ref() != Some(pointer) {
            return;
        }
        // A popup under the pointer takes its events (and the bar loses
        // its own focus on the enter).
        #[cfg(feature = "popup")]
        if state.popup_pointer(&event) {
            return;
        }
        // Frames group events from version 5; before it there are none, and
        // each axis event is its own frame.
        let framed = pointer.version() >= 5;
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                // The bar surface entered, if it is still a live one.
                let id = state
                    .outputs
                    .iter()
                    .find(|entry| {
                        entry.objects.layer.as_ref().map(|l| &l.surface) == Some(&surface)
                    })
                    .map(|entry| entry.output.id());
                match id {
                    Some(id) => state.input.enter(id, surface_x, surface_y),
                    None => state.input.leave(),
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => state.input.motion(surface_x, surface_y),
            wl_pointer::Event::Leave { .. } => state.input.leave(),
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(pressed),
                serial,
                ..
            } => state.button(
                button,
                pressed == wl_pointer::ButtonState::Pressed,
                serial,
                qh,
            ),
            wl_pointer::Event::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } => {
                state.input.axis(value);
                if !framed {
                    state.input.frame();
                }
            }
            wl_pointer::Event::AxisDiscrete {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                discrete,
            } => state.input.axis_discrete(discrete),
            wl_pointer::Event::AxisValue120 {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value120,
            } => state.input.axis_value120(value120),
            wl_pointer::Event::AxisStop {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                ..
            } => state.input.axis_stop(),
            wl_pointer::Event::Frame => state.input.frame(),
            // Horizontal scroll, the axis source and direction: ignored.
            _ => {}
        }
    }
}
