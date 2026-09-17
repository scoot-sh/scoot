//! `wp_presentation` (presentation-time): precise frame-timing feedback for
//! smooth video and animation clients.
//!
//! Smithay carries the whole protocol at the pinned rev (`PresentationState`
//! under `src/wayland/presentation/mod.rs` -- verified in source, not
//! assumed), so flexwm's side is the PR #88/#89 shape: one hold-alive field
//! on [`State`](super::State::new), no hand-rolled handler, no Smithay patch.
//! The blanket `Dispatch` in `dispatch.rs` forwards everything to Smithay's
//! own `Dispatch2` impls, so that file is untouched. What flexwm owns is the
//! per-frame half Smithay leaves to the compositor: taking each presented
//! surface's committed feedback and marking it presented with a timestamp
//! that means something on the backend that showed it.
//!
//! ## Timestamp semantics, per backend
//!
//! Every timestamp is `CLOCK_MONOTONIC` (the `clk_id` the bind handshake
//! reports), read once per presented frame and shared by that frame's events.
//! What the timestamp *is* differs by backend, honestly:
//!
//! - **`--headless` with no presenter: render-complete time.** There is no
//!   scanout; the framebuffer is the final image (it is what IPC screenshots
//!   read), so the moment the frame finished rendering is the moment the new
//!   image became current.
//! - **`--nested`: host-commit time.** The frame is committed to the host
//!   compositor here; when the host actually scans it out is the host's
//!   business and unknowable from inside, so this reports the commit, not a
//!   guess at the host's vblank.
//! - **`--tty`: flip-issue time.** The timestamp is read when the page flip
//!   (or modeset commit) is handed to DRM, so it leads the photons by up to
//!   one vblank -- the `VBlank` event itself carries no timestamp to report
//!   instead. The flip *is* vblank-synchronized, so the `vsync` flag is set;
//!   the other two backends report no flags, because there is no retrace to
//!   synchronize to and no zero-copy path behind a pixman copy.
//!
//! None of the three reports fiction as measurement: each timestamp is a
//! real reading of a real handoff, labelled by which handoff it was.
//!
//! ## Which surfaces get feedback
//!
//! Exactly the surfaces the frame drew -- the same set that got frame
//! callbacks on it: every mapped window and layer surface (popups included,
//! via Smithay's own `take_presentation_feedback`), the client cursor surface
//! when one was drawn, and -- while locked -- only the lock surfaces (plus
//! the cursor). A surface that was not displayed keeps its feedback queued:
//! the next commit supersedes it (`discarded`, Smithay's own `merge_into`),
//! which is what a video client pacing on feedback expects from frames that
//! never reached the screen. Feedback is taken if and only if the frame
//! actually went out: a `--tty` flip skipped for a busy CRTC, a `--nested`
//! commit dropped for lack of a free host buffer, and a render that drew
//! nothing at all all leave pending feedback queued for the next presented
//! frame rather than stamping it with a time nothing was shown at.
//!
//! `seq` is the compositor's frame serial (one per damaged frame -- see
//! `frame_serial`), not the DRM flip count: it orders distinct images,
//! which is what the protocol asks a sequence for. `refresh` is the output
//! mode's own refresh (60 Hz on every backend today), or zero when the mode
//! is unknown.
//!
//! ## Edge cases, stated rather than re-derived
//!
//! - **Take and mark happen atomically inside one render**, with no dispatch
//!   between them, so a client cannot destroy its feedback object in the
//!   middle: there is no window in which a `presented` could be addressed to
//!   a dead object. Pending feedback on a destroyed surface, or for a
//!   disconnected client, is Smithay's own `Drop`/`client()` path, which
//!   sends `discarded` or nothing -- pinned by tests, not re-argued here.
//! - **Unbounded `feedback` requests are accepted, like upstream.** Each one
//!   is a small queued callback drained by the next commit or presented
//!   frame; there is no amplification (no per-request event, no linear scan
//!   it feeds), so a client spamming them only spends its own socket budget.
//! - **Cost.** One monotonic read per presented frame, plus the per-feedback
//!   socket writes only for surfaces that asked. Nothing runs per
//!   frame-render beyond the take walk over mapped surfaces, and nothing at
//!   all on frames that present nothing -- so no benchmark: this lands on
//!   the per-present path, not the per-frame-render hot path.
//!
//! ## Trust model
//!
//! No client filter, the same deliberate consistency as every other
//! advertisement here: flexwm has no security-context support, so an
//! allow-list would be theatre (see `README.md`'s trust note). Feedback only
//! ever describes when the requesting client's own committed content was
//! shown -- it discloses no other client's pixels, input, or timing.

use std::time::Duration;

use smithay::desktop::layer_map_for_output;
use smithay::desktop::utils::{
    OutputPresentationFeedback, take_presentation_feedback_surface_tree,
};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Clock, Monotonic};
use smithay::wayland::presentation::Refresh;

use super::State;

#[cfg(test)]
mod tests;

impl State {
    /// Takes every displayed surface's committed presentation feedback and
    /// marks it presented with this frame's handoff timestamp.
    ///
    /// Call exactly when a drawn frame went out -- a `--tty` flip issued, a
    /// `--nested` commit handed to the host, or a presenter-less headless
    /// frame finished rendering -- and never otherwise (see the module doc
    /// for why a rendered-but-dropped frame must not stamp anything). Runs
    /// before the frame callbacks below it in `render()`, which is Smithay's
    /// own ordering: drain feedback first, then tell clients to draw next.
    ///
    /// `vsync` is whether the handoff was vblank-synchronized (`--tty`
    /// page flips are; the other two backends have no retrace), and becomes
    /// the `presented` flags verbatim. `cursor` is the client cursor surface
    /// the frame drew, if any -- the same surface `render()` sends frame
    /// callbacks to, so an animated cursor's pacing feedback and its draw
    /// pacing come from the same frame.
    pub(super) fn present_feedback(
        &mut self,
        output: &Output,
        vsync: bool,
        cursor: Option<&WlSurface>,
    ) {
        let flags = if vsync {
            wp_presentation_feedback::Kind::Vsync
        } else {
            wp_presentation_feedback::Kind::empty()
        };
        let mut feedback = OutputPresentationFeedback::new(output);
        if self.session_lock.is_locked() {
            // The blanked frame shows lock surfaces and nothing else (see
            // `render()`): windows behind it keep their feedback queued.
            self.session_lock
                .take_presentation_feedback(output, &mut feedback, flags);
        } else {
            for window in self.space.elements() {
                window.take_presentation_feedback(
                    &mut feedback,
                    |_, _| Some(output.clone()),
                    |_, _| flags,
                );
            }
            for layer in layer_map_for_output(output).layers() {
                layer.take_presentation_feedback(
                    &mut feedback,
                    |_, _| Some(output.clone()),
                    |_, _| flags,
                );
            }
        }
        if let Some(cursor) = cursor {
            take_presentation_feedback_surface_tree(
                cursor,
                &mut feedback,
                |_, _| Some(output.clone()),
                |_, _| flags,
            );
        }
        // The output mode's own refresh, like anvil's `winit` backend at the
        // pinned rev (`1_000 / refresh_mHz` seconds): the mode flexwm reports
        // is fixed at 60 Hz on every backend, so this is `Fixed`, never
        // `Variable` -- and `Unknown` (a zero refresh on the wire) when the
        // output has no mode at all rather than a number invented here.
        let refresh = output
            .current_mode()
            .filter(|mode| mode.refresh > 0)
            .map(|mode| Refresh::fixed(Duration::from_secs_f64(1_000f64 / f64::from(mode.refresh))))
            .unwrap_or(Refresh::Unknown);
        feedback.presented(
            Clock::<Monotonic>::new().now(),
            refresh,
            self.frame_serial,
            flags,
        );
    }
}
