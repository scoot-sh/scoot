//! Pointer cursor sessions: `ext_image_copy_capture_manager_v1
//! .create_pointer_cursor_session`.
//!
//! A capture session carries the screen. A *cursor* session carries the
//! pointer alone -- its image in a buffer of its own, plus enter/leave,
//! position and hotspot events -- which is how a remote-desktop server such
//! as wayvnc learns the shape to send as a VNC cursor, instead of painting
//! the pointer into the screen's pixels (`paint_cursors`) and re-sending
//! screen damage on every move.
//!
//! # What a session is told
//!
//! - **Constraints**: the cursor image's current size in the output's
//!   physical pixels, `Argb8888` only (a cursor needs its alpha). A new image
//!   of another size is announced with a fresh constraint batch and the
//!   client's frame failed with `buffer_constraints`, which is the protocol's
//!   own way to say "allocate again" -- never a buffer written past its end.
//!   A cursor that hides keeps the last size, so hiding and showing does not
//!   churn the client's buffers.
//! - **Frames**: one parked frame at most, answered when the cursor's *image*
//!   changed (a new shape, a client's cursor surface committing, hiding and
//!   reshowing, a scale change) -- never on plain motion, which is
//!   [`State::cursor_serial`]'s alone and arrives up to 1000 times a second.
//!   The first frame is always answered. A hidden cursor, or one on another
//!   output, is answered with a fully transparent image, so the remote
//!   viewer's pointer goes away instead of going stale; a frame is never
//!   failed for it, because wayvnc treats any failure but
//!   `buffer_constraints` as the end of cursor capture.
//! - **Events**: `enter`, `hotspot` and `position` as the frame that carries
//!   the image is answered (the hotspot takes effect at `ready`), `position`
//!   again on later ticks while the pointer moves, `leave` when the pointer
//!   leaves the output or the cursor hides. All of it is coalesced to the
//!   frame tick: a motion event only arms the tick.
//!
//! # Cost
//!
//! With no cursor session the cost is an empty-`Vec` check per motion event
//! (`State::cursor_changed`). With one, motion arms the frame tick (at most
//! once per frame interval) and a tick does a pointer-location lookup and a
//! deduplicated position event; the image is re-rendered -- one cursor-sized
//! offscreen render, shared by every session on the output, into a buffer
//! kept for the next one (`render::capture_cursor::image`) -- only when the
//! image changed. A parked frame nothing is changing keeps no timer alive.
//!
//! # Lifecycle
//!
//! The owned [`CursorSession`] lives here and nowhere else; an entry leaving
//! the list ends the session. Entries leave in
//! [`ImageCopyCaptureHandler::cursor_session_destroyed`] (the client's
//! destroy or disconnect -- Smithay never sweeps its own list on that, so
//! this also calls `cleanup`) and in [`State::stop_captures_on`] (the output
//! went away). The `wl_pointer` a session was created with is not kept:
//! wayvnc releases it right after creating the session.

use std::collections::HashMap;
use std::time::Duration;

use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::utils::{Buffer as BufferCoords, IsAlive, Physical, Point, Rectangle, Transform};
use smithay::wayland::image_capture_source::ImageCaptureSource;
use smithay::wayland::image_copy_capture::{
    BufferConstraints, CaptureFailureReason, CursorSession, CursorSessionRef, Frame, FrameRef,
};

use scoot_core::OutputId;

use super::{BYTES_PER_PIXEL, WeakOutput, write_capture};
use crate::compositor::State;
use crate::compositor::cursor::physical_size;
use crate::compositor::render::{CursorImage, MAX_CURSOR_IMAGE_EDGE};

/// Everything this compositor keeps for pointer cursor sessions.
#[derive(Default)]
pub(super) struct CursorSessions {
    sessions: Vec<CursorCapture>,
    /// The rendered cursor image of each output that has a session, kept so
    /// every session on the output is answered from one render and a
    /// re-render reuses the same pixel buffer. An entry exists only while a
    /// session on its output does.
    images: HashMap<OutputId, CachedImage>,
}

impl CursorSessions {
    /// Whether some session has a frame parked: the one case where a cursor
    /// change owes the frame tick a wake-up (see `State::cursor_changed`).
    pub(super) fn frame_parked(&self) -> bool {
        // The empty check first: this runs per pointer-motion event, and with
        // no cursor session (nearly always) it is all that runs.
        !self.sessions.is_empty() && self.sessions.iter().any(|cursor| cursor.pending.is_some())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Clears the parked frame `frame` names, if some session holds it.
    pub(super) fn abort_frame(&mut self, frame: &FrameRef) {
        for cursor in &mut self.sessions {
            if cursor
                .pending
                .as_ref()
                .is_some_and(|parked| parked == frame)
            {
                cursor.pending = None;
                return;
            }
        }
    }

    /// How many sessions this compositor is tracking.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Drops the cached image of every output no session is left on.
    fn sweep_images(&mut self) {
        let sessions = &self.sessions;
        self.images
            .retain(|id, _| sessions.iter().any(|cursor| cursor.output == *id));
    }
}

/// One pointer cursor session, and the single frame a client may have
/// outstanding on it.
struct CursorCapture {
    session: CursorSession,
    /// The output this session follows, resolved once in
    /// [`State::add_cursor_session`] from its source. A removed output ends
    /// its sessions first ([`State::stop_captures_on`]), so it cannot go
    /// stale.
    output: OutputId,
    /// The frame whose `capture` request has arrived and has not been
    /// answered. At most one (a second is failed, as for capture sessions).
    pending: Option<Frame>,
    /// The [`ImageKey`] of the image last delivered, or `None` before the
    /// first: what makes a session's first frame exempt from waiting for a
    /// change.
    delivered: Option<ImageKey>,
    /// The size the client was last told to allocate.
    told: (i32, i32),
}

/// What decides whether the image a session holds is still the right one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImageKey {
    /// [`State::cursor_image_serial`].
    serial: u64,
    /// Whether the cursor is shown on the session's output at all. Hidden and
    /// elsewhere both answer a transparent image, and are told apart from a
    /// shown cursor so a cursor that reappears is delivered.
    shown: bool,
    /// The output scale's bits: the image is drawn at it.
    scale: u64,
}

struct CachedImage {
    key: ImageKey,
    image: CursorImage,
    /// Tightly packed `Argb8888`, `image.size` of them.
    pixels: Vec<u8>,
}

/// What a client is told to allocate for a `size` image.
fn constraints(size: (i32, i32)) -> BufferConstraints {
    BufferConstraints {
        size: size.into(),
        shm: vec![wl_shm::Format::Argb8888],
        // Written into the client's shm buffer only, like capture sessions
        // (see this module's parent).
        dma: None,
    }
}

impl State {
    /// The output a source names, resolved the way capture sessions do.
    fn cursor_source_output(&self, source: &ImageCaptureSource) -> Option<OutputId> {
        let output = source.user_data().get::<WeakOutput>()?.upgrade()?;
        let id = self.outputs.id_of(&output)?;
        self.backends.contains_key(&id).then_some(id)
    }

    /// Where the pointer's hotspot sits on output `id`, in that output's
    /// physical pixels -- or `None` while the cursor is not shown there: it
    /// is hidden (by the client or after idle), or the pointer is over
    /// another output, or none.
    fn cursor_hotspot_on(&self, id: OutputId) -> Option<Point<i32, Physical>> {
        if self.cursor_idle_hidden || self.cursor.is_hidden() {
            return None;
        }
        let pointer = self.seat.get_pointer()?.current_location();
        let (output, origin) = self.output_under(pointer)?;
        if self.outputs.id_of(&output) != Some(id) {
            return None;
        }
        let scale = output.current_scale().fractional_scale();
        let local = pointer - origin.to_f64();
        Some(Point::from((
            (local.x * scale).round() as i32,
            (local.y * scale).round() as i32,
        )))
    }

    fn cursor_image_key(&self, id: OutputId, shown: bool) -> Option<ImageKey> {
        let output = self.outputs.get(id)?;
        Some(ImageKey {
            serial: self.cursor_image_serial,
            shown,
            scale: output.current_scale().fractional_scale().to_bits(),
        })
    }

    /// Brings output `id`'s cached cursor image up to `key`, rendering only
    /// when it is stale. `at` is the hotspot position the image is drawn for
    /// (`None`: not shown, answered transparent without touching a renderer).
    fn refresh_cursor_image(
        &mut self,
        id: OutputId,
        key: ImageKey,
        at: Option<Point<i32, Physical>>,
    ) {
        if self
            .screencopy
            .cursors
            .images
            .get(&id)
            .is_some_and(|cached| cached.key == key)
        {
            return;
        }
        let mut cached = self
            .screencopy
            .cursors
            .images
            .remove(&id)
            .unwrap_or_else(|| {
                let scale = f64::from_bits(key.scale);
                let edge = physical_size(self.cursor.logical_size(), scale)
                    .clamp(1, MAX_CURSOR_IMAGE_EDGE);
                CachedImage {
                    key,
                    image: CursorImage {
                        size: (edge, edge),
                        hotspot: (0, 0),
                    },
                    pixels: Vec::new(),
                }
            });
        // The size clients were last told, so hiding and reshowing does not
        // re-announce one.
        let fallback = cached.image.size;
        // The backend is taken only for a render that will put it back: a
        // tuple scrutinee would take it for the transparent case too, and drop
        // the output's render target on the floor.
        let rendered = match at {
            Some(at) => self.take_backend(id).map(|mut backend| {
                let image =
                    self.render_cursor_image(&mut backend, id, at, fallback, &mut cached.pixels);
                self.put_backend(id, backend);
                image
            }),
            None => None,
        };
        let image =
            rendered.unwrap_or_else(|| CursorImage::transparent(fallback, &mut cached.pixels));
        cached.key = key;
        cached.image = image;
        self.screencopy.cursors.images.insert(id, cached);
    }

    /// `cursor_capture_constraints`: what a new pointer cursor session on
    /// `source` must allocate, or `None` to refuse a source that names no
    /// live output.
    ///
    /// The size is the image's real size, rendered now: a session told a
    /// guess would have its first frame failed with `buffer_constraints`,
    /// which wayvnc answers by recreating the session -- and would be told
    /// the same guess again.
    pub(super) fn cursor_session_constraints(
        &mut self,
        source: &ImageCaptureSource,
    ) -> Option<BufferConstraints> {
        let id = self.cursor_source_output(source)?;
        let at = self.cursor_hotspot_on(id);
        let key = self.cursor_image_key(id, at.is_some())?;
        self.refresh_cursor_image(id, key, at);
        let cached = self.screencopy.cursors.images.get(&id)?;
        Some(constraints(cached.image.size))
    }

    /// `new_cursor_session`: starts tracking a session Smithay accepted.
    pub(super) fn add_cursor_session(&mut self, session: CursorSession) {
        let Some(output) = self.cursor_source_output(&session.source()) else {
            // Unreachable -- the constraints above resolved the same source
            // a moment ago. Dropping the session ends it with `stopped`.
            return;
        };
        let Some(cached) = self.screencopy.cursors.images.get(&output) else {
            return;
        };
        let told = cached.image.size;
        self.screencopy.cursors.sessions.push(CursorCapture {
            session,
            output,
            pending: None,
            delivered: None,
            told,
        });
    }

    /// `cursor_frame`: parks a capture request until the next frame tick.
    pub(super) fn park_cursor_frame(&mut self, session: &CursorSessionRef, frame: Frame) {
        let Some(cursor) = self
            .screencopy
            .cursors
            .sessions
            .iter_mut()
            .find(|cursor| cursor.session == *session)
        else {
            // Unreachable, as for capture sessions: but a frame with
            // nowhere to be parked must still be answered.
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };
        if cursor.pending.is_some() {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        }
        cursor.pending = Some(frame);
        self.ensure_ticking();
    }

    /// `cursor_session_destroyed`: the client destroyed the session, or went
    /// away with it. Dropping the entry fails its parked frame; Smithay's own
    /// list is swept too, since nothing upstream does.
    pub(super) fn forget_cursor_session(&mut self, session: &CursorSessionRef) {
        self.screencopy
            .cursors
            .sessions
            .retain(|cursor| cursor.session != *session);
        self.screencopy.cursors.sweep_images();
        self.screencopy.capture.cleanup();
    }

    /// Ends every cursor session on output `id`, which is going away: a
    /// parked frame fails with `stopped`, and dropping the session sends its
    /// own `stopped`.
    pub(super) fn stop_cursor_sessions_on(&mut self, id: OutputId) {
        self.screencopy.cursors.sessions.retain_mut(|cursor| {
            if cursor.output != id {
                return true;
            }
            if let Some(frame) = cursor.pending.take() {
                frame.fail(CaptureFailureReason::Stopped);
            }
            false
        });
        self.screencopy.cursors.sweep_images();
    }

    /// Serves the cursor sessions for this frame tick: leave/position events
    /// for the pointer's motion, and the image to every session whose parked
    /// frame is due.
    ///
    /// Costs an `is_empty` check when no client has a cursor session, which
    /// is the normal case.
    pub(super) fn service_cursor_sessions(&mut self) {
        if self.screencopy.cursors.is_empty() {
            return;
        }
        // Moved out for the tick so each session can be served with `self`
        // free to render; nothing in between creates or drops a session.
        let mut sessions = std::mem::take(&mut self.screencopy.cursors.sessions);
        for cursor in &mut sessions {
            self.serve_cursor_session(cursor);
        }
        self.screencopy.cursors.sessions = sessions;
        // `render_cursor_image` and the writes above queue events; the tick
        // flushes at its end (`service_captures`), not here.
    }

    fn serve_cursor_session(&mut self, cursor: &mut CursorCapture) {
        let at = self.cursor_hotspot_on(cursor.output);
        let Some(key) = self.cursor_image_key(cursor.output, at.is_some()) else {
            return;
        };
        let position = at.map(|at| Point::<i32, BufferCoords>::from((at.x, at.y)));
        if position.is_none() {
            // Deduplicated by Smithay: silent when already outside.
            cursor.session.set_cursor_pos(None);
        }
        let entered = cursor.session.has_cursor();
        // A pointer that left and came back with the same image still owes an
        // `enter`, and an `enter` travels with the hotspot, so it is a reason
        // to answer a parked frame again.
        let owes_enter = position.is_some() && !entered;
        if cursor.pending.is_none() || (cursor.delivered == Some(key) && !owes_enter) {
            // Nothing to deliver: motion only moves `position`, and only for
            // a session that has entered.
            if let Some(position) = position
                && entered
            {
                cursor.session.set_cursor_pos(Some(position));
            }
            return;
        }

        self.refresh_cursor_image(cursor.output, key, at);
        let Some(frame) = cursor.pending.take() else {
            return;
        };
        let Some(cached) = self.screencopy.cursors.images.get(&cursor.output) else {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };
        if !cursor.session.alive() {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        }
        let (width, height) = cached.image.size;
        if cached.image.size != cursor.told {
            // The image is another size than the client allocated for:
            // announce the new one, and fail this frame the way the
            // protocol asks. `delivered` stays, so the retry is due at once.
            cursor.told = cached.image.size;
            cursor
                .session
                .update_constraints(constraints(cached.image.size));
            frame.fail(CaptureFailureReason::BufferConstraints);
            return;
        }
        match write_capture(
            &frame.buffer(),
            &cached.pixels,
            width,
            height,
            width as usize * BYTES_PER_PIXEL as usize,
            false,
            None,
        ) {
            Ok(()) => {
                // The hotspot first: Smithay sends it with `enter`, and it
                // takes effect at the `ready` below.
                cursor
                    .session
                    .set_cursor_hotspot(Point::<i32, BufferCoords>::from(cached.image.hotspot));
                cursor.session.set_cursor_pos(position);
                cursor.delivered = Some(key);
                let presented = Duration::from(self.screencopy.clock.now());
                frame.success(
                    Transform::Normal,
                    vec![Rectangle::from_size((width, height).into())],
                    presented,
                );
            }
            Err(reason) => frame.fail(reason),
        }
    }
}
