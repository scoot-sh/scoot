//! Transitions running on outputs: starting one from a request, driving
//! its frames, and finishing at the new wallpaper.
//!
//! [`reconcile`] is the hook `daemon::change::reconcile` calls first: an
//! output with a running transition drives it (or drops it when its size
//! changed, falling through to the instant path), and an output with a
//! requested one starts it when both endpoints are ready. Everything else
//! — the choices, the worker, the replies — works as for a static change:
//! a transition holds its request's reply until its last frame, because
//! the output keeps reporting the old wallpaper until then.
//!
//! ## Frames
//!
//! One frame blends the two endpoints (`Endpoint`: a solid color, or
//! shared pixels that already exist) into a full-size frame buffer, which
//! is attached, damaged with only what changed
//! (`crate::transition::damage`) and committed. The first buffer is
//! allocated once per transition and rewritten in place; where the
//! compositor holds it until another attaches (release-on-replace), a
//! second is allocated lazily and the two alternate, so a held buffer
//! never stalls the animation. A mid-transition restart snapshots what
//! is on screen with one memcpy into a further buffer and starts the new
//! transition from there, so it never queues behind the old one. All of
//! them are freed when the transition finishes: an idle daemon costs
//! what a static wallpaper costs. At most two frame buffers run (plus the
//! snapshot across a restart), against the old and new endpoints they
//! blend: see `crate::transition`'s memory bound. An allocation that fails
//! ends the transition at once, showing the final wallpaper the normal
//! way.
//!
//! ## Pacing
//!
//! The eased progress comes from the clock, so a slow frame drops frames
//! rather than falling behind. Each commit asks for a frame callback (and
//! presentation feedback where the compositor offers `wp_presentation`),
//! either of which drives the next frame; a timer at [`FRAME_HZ`] covers
//! a compositor that sends neither, and forces progress past a callback
//! that never comes back. A frame that costs more than the budget skips
//! the next one (`crate::transition::skip_for`).
//!
//! ## No per-frame allocation
//!
//! Endpoints are shared references (colors fill one reused scanline per
//! transition, not per frame); the sweep is stack geometry; damage is at
//! most four stack rects. The frame callback and feedback objects are the
//! only per-frame protocol objects, freed at once.

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;

use scootbg_mem::ShmBuffer;

use super::canvas::{self, Content, Drew, Pixels};
use super::change::image_dims;
use super::surfaces::{Objects, RoundTrip};
use super::wayland::{Globals, State};
use crate::choices::Choices;
use crate::color::Color;
use crate::control::ConnId;
use crate::jobs::{Jobs, Target};
use crate::outputs::{Entry, OutputId};
use crate::paint::{self, Drawn, Plan};
use crate::print::warn;
use crate::transition::{self, Spec};
use crate::wallpaper::Wallpaper;

#[cfg(test)]
mod tests;

/// The timer cadence while a transition runs: about 60 Hz.
const FRAME_INTERVAL: Duration = Duration::from_millis(transition::FRAME_STEP_MS);

/// How long a frame callback or presentation feedback may take before the
/// timer drives past it: two missed cadences, then progress is forced
/// rather than wedged.
const AWAIT_GRACE: Duration = Duration::from_millis(transition::FRAME_STEP_MS * 2);

/// One endpoint of a transition: a solid color, or pixels that already
/// exist (an output's buffer, or the snapshot of what is on screen). A
/// shared reference in both cases: endpoints are never copied.
#[derive(Debug, Clone)]
enum Endpoint {
    Solid(Color),
    Pixels(Pixels),
}

/// A transition running on one output.
struct Active {
    from: Endpoint,
    to: Endpoint,
    /// The final draw, for finishing and for noticing a resize.
    target: Drawn,
    spec: Spec,
    dims: (u32, u32),
    started: Instant,
    last_step: u32,
    last_eased: f64,
    /// A frame went out: a restart snapshots from it.
    committed: bool,
    /// The frame buffers, rewritten in place. One is enough where the
    /// compositor releases a buffer soon after it is replaced; where it
    /// holds one until another attaches (release-on-replace), a second is
    /// allocated lazily, and the two alternate, so a held buffer skips
    /// nothing. At most two, freed with the transition.
    frames: [Option<canvas::ShmSlot>; 2],
    /// The frame on screen, for release matching and restart snapshots.
    current: Option<usize>,
    /// What is on screen at a restart, copied once from the last frame.
    /// `None` until the first restart needs it.
    snap: Option<Pixels>,
    /// Reused scanlines for solid endpoints, filled once per transition.
    from_row: Vec<u8>,
    to_row: Vec<u8>,
    /// The frame callback and feedback generation: stale callbacks (from
    /// before a restart) carry an older one and are ignored.
    seq: u64,
    /// A frame callback or presentation feedback is out: the next frame
    /// waits for it, up to [`AWAIT_GRACE`].
    awaiting: bool,
    await_since: Instant,
}

/// The transitions of the daemon: one [`Active`] per transitioning output
/// at most, and the timer that paces them, armed only while one runs.
#[derive(Default)]
pub struct Transitions {
    active: Vec<(OutputId, Active)>,
    timer: Option<OwnedFd>,
    next_seq: u64,
}

impl std::fmt::Debug for Transitions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transitions")
            .field("active", &self.active.len())
            .field("timer_armed", &self.timer.is_some())
            .finish()
    }
}

impl Transitions {
    /// Whether any output has a transition running.
    pub fn have_active(&self) -> bool {
        !self.active.is_empty()
    }

    /// Whether `id` has one.
    fn has(&self, id: OutputId) -> bool {
        self.active.iter().any(|(at, _)| *at == id)
    }

    /// The timer to poll while transitions run, if any.
    pub fn timer_fd(&self) -> Option<BorrowedFd<'_>> {
        self.timer.as_ref().map(AsFd::as_fd)
    }

    /// Clears the timer's expirations after it woke the loop, so it does
    /// not poll ready forever.
    pub fn drain_timer(&self) {
        if let Some(timer) = &self.timer {
            let mut expirations = [0u8; 8];
            // Best effort: whatever stays readable wakes the loop once
            // more, which drains it again.
            let _ = rustix::io::read(timer, &mut expirations);
        }
    }

    /// Drops the timer once nothing runs: no fd, no wakeups.
    fn idle_timer(&mut self) {
        if self.active.is_empty() {
            self.timer = None;
        }
    }

    /// Arms the timer, creating it on the first transition. Errors (a
    /// sandbox without `timerfd`) leave it disarmed: frame callbacks and
    /// the per-turn drive still pace the transition.
    fn arm_timer(&mut self) {
        let timer = match &self.timer {
            Some(timer) => timer,
            None => match timerfd_create(TimerfdClockId::Monotonic, TimerfdFlags::CLOEXEC) {
                Ok(timer) => self.timer.insert(timer),
                Err(error) => {
                    warn(format_args!(
                        "scootbg: cannot arm the transition timer ({error}); transitions \
                         still run, paced by frame callbacks"
                    ));
                    return;
                }
            },
        };
        let interval = Timespec {
            tv_sec: 0,
            tv_nsec: FRAME_INTERVAL.as_nanos() as _,
        };
        let spec = Itimerspec {
            it_interval: interval,
            it_value: interval,
        };
        if let Err(error) = timerfd_settime(timer, TimerfdTimerFlags::empty(), &spec) {
            warn(format_args!(
                "scootbg: cannot arm the transition timer ({error})"
            ));
        }
    }
}

/// The reconcile hook: drives a running transition, restarts one when a
/// newer request arrived mid-flight, and starts a requested one when both
/// endpoints are ready. Returns `Some(committed)` when it handled the
/// output, `None` when the caller should draw the normal way.
pub fn hook(
    globals: &Globals,
    choices: &Choices,
    jobs: &mut Jobs<ConnId>,
    transitions: &mut Transitions,
    entry: &mut Entry<Objects>,
    qh: &QueueHandle<State>,
    now: Instant,
) -> Option<bool> {
    let id = entry.output.id();
    let pending = entry.output.take_transition();
    match (transitions.has(id), pending) {
        (true, Some(spec)) => {
            // A newer request mid-transition aborts what runs: an instant
            // one lands at once through the normal path, anything else
            // restarts from what is on screen now, never queued behind
            // the old one.
            if spec.is_instant() {
                if let Some(position) = active_position(transitions, id) {
                    drop_active(transitions, entry, position);
                }
                None
            } else if restart(globals, transitions, choices, entry, qh, spec, now) {
                drive(globals, choices, jobs, transitions, entry, qh, now)
            } else {
                None
            }
        }
        (true, None) => drive(globals, choices, jobs, transitions, entry, qh, now),
        (false, Some(spec)) if !spec.is_instant() => {
            if start(globals, transitions, choices, entry, qh, spec, now) {
                drive(globals, choices, jobs, transitions, entry, qh, now)
            } else {
                None
            }
        }
        (false, _) => None,
    }
}

/// The active transition's index for `id`, if any.
fn active_position(transitions: &Transitions, id: OutputId) -> Option<usize> {
    transitions.active.iter().position(|(at, _)| *at == id)
}

/// Starts the requested transition when the surface can carry it and both
/// endpoints are ready: what is on screen now (a color, or an image whose
/// pixels are here), and the target (whose image is decoded). Otherwise
/// restores the request for an image still decoding and reports `false`,
/// so the caller draws the normal way; the pixels landing starts it then.
fn start(
    globals: &Globals,
    transitions: &mut Transitions,
    choices: &Choices,
    entry: &mut Entry<Objects>,
    qh: &QueueHandle<State>,
    spec: Spec,
    now: Instant,
) -> bool {
    let wanted = choices.for_output(entry.output.info().name.as_deref());
    let scale = paint::scale_for(globals.path, wanted, entry.output.scale());
    let Plan::Show(target) = entry.output.plan(wanted, scale) else {
        return false;
    };
    if !entry.output.surface_configured() {
        return false;
    }
    let Some(dims) = image_dims(&entry.output) else {
        return false;
    };
    if dims.0 == 0 || dims.1 == 0 {
        return false;
    }
    // Bounded before it is allocated: one extra full-size buffer per
    // output at most (`crate::transition`'s memory bound).
    if transition::frame_bytes(dims).is_none() {
        return false;
    }
    // What is on screen now.
    let from = match entry.output.shows() {
        Some(Wallpaper::Color(color)) => Endpoint::Solid(*color),
        Some(Wallpaper::Image(image)) => match entry.objects.canvas.image(image.serial, dims) {
            Some(pixels) => Endpoint::Pixels(Rc::clone(pixels)),
            None => return false,
        },
        None => return false,
    };
    // What it is becoming.
    let to = match &target.content {
        Wallpaper::Color(color) => Endpoint::Solid(*color),
        Wallpaper::Image(image) => match entry.objects.canvas.image(image.serial, dims) {
            Some(pixels) => Endpoint::Pixels(Rc::clone(pixels)),
            // Still decoding: ask again when the pixels land. The request
            // is restored at its generation, which still matches the
            // stamp.
            None => {
                entry.output.request_transition(spec, entry.output.stamp());
                return false;
            }
        },
    };
    let frame = match frame_buffer(globals, qh, entry.output.id(), dims) {
        Ok(frame) => frame,
        Err(error) => {
            warn(format_args!(
                "scootbg: cannot start a {} transition on {} ({error}); showing at once",
                spec.kind.name(),
                entry.output.label()
            ));
            return false;
        }
    };
    let row = |color: Color| {
        let pixel = color.xrgb8888();
        let mut row = vec![0u8; dims.0 as usize * 4];
        for chunk in row.chunks_exact_mut(4) {
            chunk.copy_from_slice(&pixel);
        }
        row
    };
    let from_row = match &from {
        Endpoint::Solid(color) => row(*color),
        Endpoint::Pixels(_) => Vec::new(),
    };
    let to_row = match &to {
        Endpoint::Solid(color) => row(*color),
        Endpoint::Pixels(_) => Vec::new(),
    };
    let step = transition::step_for_elapsed(spec.duration_ms, 0);
    transitions.active.push((
        entry.output.id(),
        Active {
            from,
            to,
            target,
            spec,
            dims,
            started: now,
            last_step: step,
            last_eased: transition::progress(&spec, 0),
            committed: false,
            frames: [Some(frame), None],
            current: None,
            snap: None,
            from_row,
            to_row,
            seq: transitions.next_seq,
            awaiting: false,
            await_since: now,
        },
    ));
    entry.output.set_running(Some(spec.kind));
    transitions.arm_timer();
    true
}

/// Restarts the running transition from what is on screen now: the last
/// committed frame, snapshotted once, or the original endpoints when no
/// frame went out yet. Returns whether the new transition runs (its target
/// may still be decoding: then the request is restored and the caller
/// draws the normal way, which starts it when the pixels land).
fn restart(
    globals: &Globals,
    transitions: &mut Transitions,
    choices: &Choices,
    entry: &mut Entry<Objects>,
    qh: &QueueHandle<State>,
    spec: Spec,
    now: Instant,
) -> bool {
    let id = entry.output.id();
    let Some(position) = active_position(transitions, id) else {
        return false;
    };
    let wanted = choices.for_output(entry.output.info().name.as_deref());
    let scale = paint::scale_for(globals.path, wanted, entry.output.scale());
    let Plan::Show(target) = entry.output.plan(wanted, scale) else {
        // Nothing to animate to (a `clear`, say): drop, draw normally.
        drop_active(transitions, entry, position);
        return false;
    };
    let Some(dims) = image_dims(&entry.output) else {
        drop_active(transitions, entry, position);
        return false;
    };
    if dims != transitions.active[position].1.dims {
        drop_active(transitions, entry, position);
        return false;
    }
    let to = match &target.content {
        Wallpaper::Color(color) => Endpoint::Solid(*color),
        Wallpaper::Image(image) => match entry.objects.canvas.image(image.serial, dims) {
            Some(pixels) => Endpoint::Pixels(Rc::clone(pixels)),
            None => {
                // Still decoding: drop the old animation, restore the
                // request, and draw the normal way until the pixels land.
                drop_active(transitions, entry, position);
                entry.output.request_transition(spec, entry.output.stamp());
                return false;
            }
        },
    };
    let committed = transitions.active[position].1.committed;
    if committed {
        // Snapshot what is on screen: one memcpy of the last frame into
        // the snapshot buffer.
        let current = transitions.active[position].1.current;
        let active = &mut transitions.active[position].1;
        let frame_bytes = match current.and_then(|index| active.frames[index].as_ref()) {
            Some(slot) => slot.memory().bytes(),
            None => {
                drop_active(transitions, entry, position);
                return false;
            }
        };
        // Reuse the snapshot only when nothing still reads it (a previous
        // restart's `from` may hold it); else a fresh buffer, so shared
        // pixels are never written.
        let mut reused = false;
        if let Some(memory) = active
            .snap
            .as_mut()
            .and_then(Rc::get_mut)
            .and_then(|shared| shared.memory_mut())
        {
            if memory.bytes_mut().len() == frame_bytes.len() {
                memory.bytes_mut().copy_from_slice(frame_bytes);
                reused = true;
            }
        }
        if !reused {
            match snap_from(frame_bytes, globals, qh, dims) {
                Some(fresh) => active.snap = Some(fresh),
                None => {
                    drop_active(transitions, entry, position);
                    return false;
                }
            }
        }
        let Some(snap) = active.snap.as_ref() else {
            drop_active(transitions, entry, position);
            return false;
        };
        active.from = Endpoint::Pixels(Rc::clone(snap));
    }
    retarget(
        transitions,
        entry,
        position,
        Retarget { to, target, spec },
        now,
    )
}

/// What a restart points a running transition at.
struct Retarget {
    to: Endpoint,
    target: Drawn,
    spec: Spec,
}

/// Points a running transition at a new target and spec, keeping its
/// frame buffer: the next frame blends from the snapshot (or the original
/// endpoints) to the new target.
fn retarget(
    transitions: &mut Transitions,
    entry: &mut Entry<Objects>,
    position: usize,
    retargeted: Retarget,
    now: Instant,
) -> bool {
    let dims = transitions.active[position].1.dims;
    let fill = |color: Color| {
        let pixel = color.xrgb8888();
        let mut row = vec![0u8; dims.0 as usize * 4];
        for chunk in row.chunks_exact_mut(4) {
            chunk.copy_from_slice(&pixel);
        }
        row
    };
    let active = &mut transitions.active[position].1;
    active.to = retargeted.to;
    active.target = retargeted.target;
    active.spec = retargeted.spec;
    active.started = now;
    active.last_step = 0;
    active.last_eased = 0.0;
    active.committed = false;
    active.awaiting = false;
    active.from_row = match &active.from {
        Endpoint::Solid(color) => fill(*color),
        Endpoint::Pixels(_) => Vec::new(),
    };
    active.to_row = match &active.to {
        Endpoint::Solid(color) => fill(*color),
        Endpoint::Pixels(_) => Vec::new(),
    };
    entry.output.set_running(Some(retargeted.spec.kind));
    transitions.arm_timer();
    true
}

/// A fresh snapshot buffer at `dims`, holding a copy of `frame_bytes`:
/// `None` when the allocation fails (the caller then shows at once).
fn snap_from(
    frame_bytes: &[u8],
    globals: &Globals,
    qh: &QueueHandle<State>,
    dims: (u32, u32),
) -> Option<Pixels> {
    let mut fresh = snapshot(globals, qh, dims).ok()?;
    let memory = Rc::get_mut(&mut fresh)?.memory_mut()?;
    if memory.bytes_mut().len() != frame_bytes.len() {
        return None;
    }
    memory.bytes_mut().copy_from_slice(frame_bytes);
    Some(fresh)
}

/// A fresh snapshot buffer at `dims`, holding nothing yet: no `wl_buffer`
/// over it, since only restarts read it.
fn snapshot(
    globals: &Globals,
    qh: &QueueHandle<State>,
    dims: (u32, u32),
) -> Result<Pixels, scootbg_mem::ShmError> {
    let shm = ShmBuffer::new(dims.0, dims.1)?;
    canvas::pixels(globals, qh, shm, Content::Frame)
}

/// A frame buffer: shared memory over a fresh memfd, with this output's
/// `wl_buffer` over it.
fn frame_buffer(
    globals: &Globals,
    qh: &QueueHandle<State>,
    id: OutputId,
    dims: (u32, u32),
) -> Result<canvas::ShmSlot, scootbg_mem::ShmError> {
    let shm = ShmBuffer::new(dims.0, dims.1)?;
    let pixels = canvas::pixels(globals, qh, shm, Content::Frame)?;
    Ok(canvas::buffer_over(pixels, qh, id))
}

/// Drives one output's running transition: renders the frame the clock
/// says, or finishes at the new wallpaper. Returns `Some(committed)` when
/// it handled the output, `None` when the transition was dropped and the
/// caller should draw the normal way.
fn drive(
    globals: &Globals,
    choices: &Choices,
    jobs: &mut Jobs<ConnId>,
    transitions: &mut Transitions,
    entry: &mut Entry<Objects>,
    qh: &QueueHandle<State>,
    now: Instant,
) -> Option<bool> {
    let id = entry.output.id();
    let position = transitions.active.iter().position(|(at, _)| *at == id)?;
    // A resize, a rescale or a replaced surface ends the animation at
    // once: the frame buffer is the wrong size, and resize storms must
    // not re-trigger animations.
    let live = entry.output.surface_configured() && entry.objects.layer.is_some();
    if !live || image_dims(&entry.output) != Some(transitions.active[position].1.dims) {
        drop_active(transitions, entry, position);
        return None;
    }
    let total = transition::total_steps(transitions.active[position].1.spec.duration_ms);
    let elapsed = now.saturating_duration_since(transitions.active[position].1.started);
    let elapsed_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
    let eased = transition::progress(&transitions.active[position].1.spec, elapsed_ms);
    let step =
        transition::step_for_elapsed(transitions.active[position].1.spec.duration_ms, elapsed_ms);
    if eased >= 1.0 || step >= total {
        return Some(finish(
            globals,
            choices,
            jobs,
            transitions,
            entry,
            qh,
            position,
        ));
    }
    let (last_step, awaiting, await_since) = {
        let active = &transitions.active[position].1;
        (active.last_step, active.awaiting, active.await_since)
    };
    if step == last_step {
        return Some(false);
    }
    if awaiting && now.saturating_duration_since(await_since) < AWAIT_GRACE {
        // A frame callback or presentation feedback is out: the compositor
        // sets the cadence. The timer forces past one that never comes.
        return Some(false);
    }
    Some(render_frame(
        globals,
        transitions,
        entry,
        qh,
        position,
        eased,
        step,
        now,
    ))
}

/// A writable frame buffer's index: the first free one, if any.
fn writable_frame(transitions: &Transitions, position: usize) -> Option<usize> {
    transitions.active[position]
        .1
        .frames
        .iter()
        .position(|slot| slot.as_ref().is_some_and(|slot| slot.is_writable()))
}

/// Allocates the second frame buffer, for a compositor holding the first.
/// At most one extra allocation per transition; an allocation that fails
/// skips the frame (the clock still advances).
fn alloc_frame(
    globals: &Globals,
    transitions: &mut Transitions,
    entry: &Entry<Objects>,
    qh: &QueueHandle<State>,
    position: usize,
    dims: (u32, u32),
) -> Option<usize> {
    let empty = transitions.active[position]
        .1
        .frames
        .iter()
        .position(|slot| slot.is_none())?;
    match frame_buffer(globals, qh, entry.output.id(), dims) {
        Ok(slot) => {
            transitions.active[position].1.frames[empty] = Some(slot);
            Some(empty)
        }
        Err(error) => {
            warn(format_args!(
                "scootbg: cannot allocate a second transition frame on {} ({error}); \
                 holding time with one",
                entry.output.label()
            ));
            None
        }
    }
}

/// Renders and commits the frame at `eased`, or skips it when every frame
/// buffer is still with the compositor (its time advances anyway: the
/// clock drops the frame rather than falling behind).
#[allow(clippy::too_many_arguments)]
fn render_frame(
    globals: &Globals,
    transitions: &mut Transitions,
    entry: &mut Entry<Objects>,
    qh: &QueueHandle<State>,
    position: usize,
    eased: f64,
    step: u32,
    now: Instant,
) -> bool {
    let id = entry.output.id();
    let dims = transitions.active[position].1.dims;
    let spec = transitions.active[position].1.spec;
    // A writable frame buffer: the first, else the second (allocated here
    // on first need), else this frame waits for a release. Either way the
    // clock advances: a held buffer never holds up time.
    let index = writable_frame(transitions, position);
    let index = match index {
        Some(index) => index,
        None => match alloc_frame(globals, transitions, entry, qh, position, dims) {
            Some(index) => index,
            None => {
                transitions.active[position].1.last_step = step;
                return false;
            }
        },
    };
    let started = Instant::now();
    let sweep = transition::Sweep::new(&spec, eased, dims);
    let row_len = dims.0 as usize * 4;
    // Borrowed once per side: the borrow checker cannot see through the
    // tuple, so the rows come out of a helper scope each row.
    {
        let active = &mut transitions.active[position].1;
        let Some(slot) = active.frames[index].as_mut() else {
            active.last_step = step;
            return false;
        };
        let Some(memory) = slot.memory_mut() else {
            active.last_step = step;
            return false;
        };
        let out = memory.bytes_mut();
        for y in 0..dims.1 {
            let o = y as usize * row_len;
            let from = endpoint_row(&active.from, &active.from_row, y, row_len);
            let to = endpoint_row(&active.to, &active.to_row, y, row_len);
            transition::blend_row(&sweep, y, from, to, &mut out[o..o + row_len]);
        }
    }
    let measured_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let skip = transition::skip_for(measured_ms);
    let damage = transition::damage(
        spec.kind,
        transitions.active[position].1.last_eased,
        eased,
        dims,
        spec.angle_deg,
        spec.pos,
    );
    if damage.is_empty() {
        // Identical to what is on screen: no commit, no callback. The
        // clock advances anyway; the timer drives the next frame.
        let active = &mut transitions.active[position].1;
        active.last_step = step + skip;
        active.last_eased = eased;
        return false;
    }
    let seq = transitions.next_seq;
    transitions.next_seq = transitions.next_seq.wrapping_add(1);
    // The buffer the frame goes through: the target's own size at its
    // scale, which is the frame's by construction. Unreachable in
    // practice (the same computation sized the frame); dropped rather
    // than drawn wrongly if it ever disagrees.
    let (buffer_scale, viewported, target_size) = {
        let active = &transitions.active[position].1;
        match active.target.buffer(globals.path) {
            Some(buffer) => (
                buffer.scale,
                !buffer.fits(active.target.size),
                active.target.size,
            ),
            None => {
                drop_active(transitions, entry, position);
                return false;
            }
        }
    };
    let active = &mut transitions.active[position].1;
    if active.frames[index].as_ref().is_none() {
        // Selected writable above but gone now: single-threaded, so this
        // is unreachable; skip the frame rather than panic.
        active.last_step = step;
        return false;
    }
    if entry.objects.layer.is_none() {
        drop_active(transitions, entry, position);
        return false;
    }
    let Some(layer) = entry.objects.layer.as_mut() else {
        drop_active(transitions, entry, position);
        return false;
    };
    let surface = layer.surface.clone();
    {
        let Some(slot) = active.frames[index].as_mut() else {
            active.last_step = step;
            return false;
        };
        surface.attach(Some(slot.buffer()), 0, 0);
        slot.attached();
    }
    active.current = Some(index);
    if canvas::sync_surface(
        globals,
        layer,
        &surface,
        qh,
        buffer_scale,
        target_size,
        viewported,
    )
    .is_err()
    {
        return false;
    }
    for rect in damage.iter() {
        surface.damage_buffer(clamp(rect.x), clamp(rect.y), clamp(rect.w), clamp(rect.h));
    }
    surface.commit();
    surface.frame(qh, RoundTrip::Frame(id, seq));
    if let Some(presentation) = &globals.presentation {
        presentation.feedback(&surface, qh, Feedback { id, seq });
    }
    active.seq = seq;
    active.awaiting = true;
    active.await_since = now;
    active.last_step = step + skip;
    active.last_eased = eased;
    active.committed = true;
    true
}

/// One row of an endpoint: the solid scanline (one row, reused for every
/// `y`) or the shared pixels' row.
fn endpoint_row<'a>(endpoint: &'a Endpoint, solid: &'a [u8], y: u32, row_len: usize) -> &'a [u8] {
    match endpoint {
        Endpoint::Solid(_) => {
            debug_assert_eq!(solid.len(), row_len);
            solid
        }
        Endpoint::Pixels(pixels) => {
            let bytes = pixels.memory().bytes();
            let o = y as usize * row_len;
            &bytes[o..o + row_len]
        }
    }
}

/// Finishes the transition: the final wallpaper goes on through the
/// normal draw, the extra buffers are freed, and the reply can go out
/// (the output finally reports what was asked).
fn finish(
    globals: &Globals,
    choices: &Choices,
    jobs: &mut Jobs<ConnId>,
    transitions: &mut Transitions,
    entry: &mut Entry<Objects>,
    qh: &QueueHandle<State>,
    position: usize,
) -> bool {
    let id = entry.output.id();
    let target = transitions.active[position].1.target.clone();
    // Still what is wanted: a choice made without a transition (an
    // unchanged reload putting an image back, say) drops the animation
    // instead of finishing stale, and the normal path draws it.
    let wanted = choices.for_output(entry.output.info().name.as_deref());
    if wanted != Some(&target.content) {
        drop_active(transitions, entry, position);
        return false;
    }
    let Some(layer) = entry.objects.layer.as_mut() else {
        drop_active(transitions, entry, position);
        return false;
    };
    let committed = match entry.objects.canvas.show(globals, qh, id, layer, &target) {
        Ok(Drew::Committed) => {
            entry.output.drew(target);
            true
        }
        Ok(Drew::Stalled) => false,
        Ok(Drew::NeedsRender(dims)) => {
            // The pixels were here when the transition started; if they
            // are gone, ask again and let the normal path draw them when
            // they land, rather than wedge.
            if let Wallpaper::Image(image) = &target.content {
                jobs.render(image, Target { output: id, dims });
            }
            false
        }
        Err(error) => {
            warn(format_args!(
                "scootbg: cannot draw on {}: {error}",
                entry.output.label()
            ));
            entry.output.draw_failed(error.to_string());
            false
        }
    };
    drop_active(transitions, entry, position);
    committed
}

/// Drops a running transition without drawing: its buffers go with it,
/// and the marker clears. The caller draws the normal way (or nothing, if
/// the surface is gone).
fn drop_active(transitions: &mut Transitions, entry: &mut Entry<Objects>, position: usize) {
    let (_, active) = transitions.active.remove(position);
    for slot in active.frames.into_iter().flatten() {
        canvas::destroy(slot);
    }
    entry.output.set_running(None);
    transitions.idle_timer();
}

/// Drops `id`'s transition, if any: an output gone, a surface closed, a
/// `clear`. Its buffers go with it.
pub fn drop_output(
    transitions: &mut Transitions,
    id: OutputId,
    output: &mut crate::outputs::Output,
) {
    if let Some(position) = transitions.active.iter().position(|(at, _)| *at == id) {
        let (_, active) = transitions.active.remove(position);
        for slot in active.frames.into_iter().flatten() {
            canvas::destroy(slot);
        }
        output.set_running(None);
        transitions.idle_timer();
    }
}

/// A `wl_buffer.release` for a transition frame: it may be rewritten.
/// Returns whether it was one (the caller then reconciles, which drives).
pub fn frame_released(transitions: &mut Transitions, id: OutputId, buffer: &WlBuffer) -> bool {
    let Some((_, active)) = transitions.active.iter_mut().find(|(at, _)| *at == id) else {
        return false;
    };
    let mut released = false;
    for slot in active.frames.iter_mut().flatten() {
        if slot.buffer() == buffer {
            slot.released();
            released = true;
        }
    }
    released
}

/// A frame callback came back for `(id, seq)`: the compositor sets the
/// cadence. Stale generations (from before a restart) are ignored.
pub fn frame_done(transitions: &mut Transitions, id: OutputId, seq: u64) -> bool {
    feedback_done(transitions, id, seq)
}

/// Presentation feedback came back for `(id, seq)`: presented or
/// discarded, the compositor answered. A discarded frame was replaced
/// before it showed: the clock already jumped past it, which is the drop.
pub fn feedback_done(transitions: &mut Transitions, id: OutputId, seq: u64) -> bool {
    match transitions.active.iter_mut().find(|(at, _)| *at == id) {
        Some((_, active)) if active.seq == seq && active.awaiting => {
            active.awaiting = false;
            true
        }
        _ => false,
    }
}

/// Drives every running transition whose frame is due: the timer's turn,
/// and every loop turn's (cheap: the step check skips the idle ones).
pub fn drive_due(
    globals: &Globals,
    choices: &Choices,
    jobs: &mut Jobs<ConnId>,
    outputs: &mut crate::outputs::Outputs<Objects>,
    transitions: &mut Transitions,
    qh: &QueueHandle<State>,
    now: Instant,
) {
    if !transitions.have_active() {
        return;
    }
    for entry in outputs.iter_mut() {
        // Removed meanwhile: the id lookup inside `drive` fails softly.
        let _ = drive(globals, choices, jobs, transitions, entry, qh, now);
    }
}

/// Restarts or drops bookkeeping for tests: the pure step decisions.
#[cfg(test)]
pub fn step_due(last_step: u32, duration_ms: u32, elapsed_ms: u64) -> bool {
    transition::step_for_elapsed(duration_ms, elapsed_ms) != last_step
}

/// What a frame callback or presentation feedback carries.
#[derive(Debug, Clone, Copy)]
pub struct Feedback {
    pub id: OutputId,
    pub seq: u64,
}

fn clamp(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
