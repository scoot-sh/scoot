//! The seat's pointer on the daemon's side: `wl_pointer` events turned into
//! calls on the pure state machine (`crate::pointer`), and what it says to
//! do carried out on the bar (`crate::action`).
//!
//! **A pointer only when something needs one.** The bar binds the seat
//! when the compositor has one, but asks it for a `wl_pointer` only while
//! a placed module answers pointer input (a binding in the config, or a
//! default of the module's own: the workspaces module's click). A clock-only
//! bar with no bindings never takes the pointer, so it costs what it did
//! before there was any input, and a reload that adds or removes bindings
//! takes or drops it then ([`State::sync_pointer`]). The bar never takes
//! the keyboard or touch, so it never disturbs focus and ignores touch.
//!
//! **Events cost almost nothing.** A motion stores two numbers; hover is
//! worked out once per turn of the loop, when the output's scene is asked
//! what to draw (`daemon::draw`), so a thousand motions a second are a
//! thousand stores and one hit test. A press or release resolves the module
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
    fn target(&self) -> Option<Target> {
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
        self.content.modules.iter().any(Placed::interactive)
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

    /// Runs the action `trigger` means on `target`'s module: the config's
    /// binding, else the module's own default. `steps` is a scroll's.
    fn fire(&mut self, trigger: Trigger, target: Target, steps: Option<u32>, now: Instant) {
        let Some(entry) = self.outputs.get_mut(target.output) else {
            return;
        };
        let scene = &entry.objects.scene;
        let (Some(span), Some(view), Some(module)) = (
            scene.spans().get(target.member).copied(),
            scene.view(target.member),
            scene.module(target.member),
        ) else {
            return;
        };
        let Content {
            modules,
            text,
            style,
        } = &mut self.content;
        let (Some(text), Some(placed), Some(focus)) =
            (text.as_ref(), modules.get_mut(module), self.input.focus())
        else {
            return;
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
        let at = ClickCtx {
            output,
            x: x.saturating_sub(span.x),
            view,
            text,
            em: render::em(style.font_size, scale),
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
        let offered;
        let action: &Action = match bindings.get(trigger) {
            Some(bound) => bound,
            None => match module.on_input(&input) {
                Some(default) => {
                    offered = default;
                    &offered
                }
                None => return,
            },
        };
        let mut effects = Launch {
            spawner: &mut self.spawner,
        };
        if let Err(failure) = action::perform(
            &mut **module,
            revision,
            &output,
            action,
            steps,
            &mut effects,
        ) {
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
        }
    }

    /// A press or release at the pointer's place.
    fn button(&mut self, code: u32, pressed: bool) {
        let target = self.target();
        if pressed {
            self.input.press(code, target);
        } else if let Some((trigger, target)) = self.input.release(code, target) {
            self.fire(trigger, target, None, Instant::now());
        }
    }
}

/// The two ways out of the process, for [`action::perform`] (a pointer press,
/// and `invoke`).
pub(super) struct Launch<'a> {
    pub(super) spawner: &'a mut crate::spawn::Spawner,
}

impl Effects for Launch<'_> {
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
        _: &QueueHandle<Self>,
    ) {
        if state.pointer.as_ref() != Some(pointer) {
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
                ..
            } => state.button(button, pressed == wl_pointer::ButtonState::Pressed),
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
