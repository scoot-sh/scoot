//! The headless backend: draw into memory on the CPU, with no display at all.
//!
//! This is what agents and tests drive. Nothing is shown anywhere; the way to
//! see the screen is a screenshot over IPC.

use std::error::Error;
use std::time::Duration;

use flexwm_core::{Event as CoreEvent, OutputId, Rect};
use pixman::Image;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::desktop::Window;
use smithay::desktop::space::{self, SpaceRenderElements};
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer, Physical, Rectangle, Transform};

use super::State;
use super::cursor::CursorElement;
use super::tty::Tty;

// Everything `render()` can draw, front-to-back as Smithay's damage tracker
// expects (see `render()`'s comment on that convention): the cursor, window
// surfaces, then this feature's focus-ring segments underneath them. The
// background isn't a variant here at all -- it's the `render_output` call's
// `clear_color`, always the bottom-most thing on screen by construction; see
// `decorations.rs`'s module doc for why that's simpler and safer than a
// full-output element.
//
// Fixed to `PixmanRenderer` (this backend's only renderer) rather than
// generic over `R`, which is why this lives here and not in
// `decorations.rs`/`cursor.rs` -- those modules stay renderer-agnostic,
// returning plain `SolidColorRenderElement`s and generic `CursorElement<R>`s
// this enum only wraps.
render_elements! {
    Elements<=PixmanRenderer>;
    Cursor = CursorElement<PixmanRenderer>,
    Space = SpaceRenderElements<PixmanRenderer, WaylandSurfaceRenderElement<PixmanRenderer>>,
    Decoration = SolidColorRenderElement,
}

/// How often a changed screen is redrawn, while there's something to redraw.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// The CPU renderer and the image it draws into.
pub struct Backend {
    pub renderer: PixmanRenderer,
    pub image: Image<'static, 'static>,
    pub damage: OutputDamageTracker,
    pub size: (i32, i32),
}

pub fn init(state: &mut State, width: i32, height: i32) -> Result<(), Box<dyn Error>> {
    let output = Output::new(
        "headless".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "flexwm".into(),
            model: "headless".into(),
            serial_number: "0".into(),
        },
    );
    output.create_global::<State>(&state.display_handle);
    set_mode(&output, width, height, Some((0, 0).into()));
    state.space.map_output(&output, (0, 0));

    state.backend = Some(create_backend(&output, width, height)?);
    state.output = Some(output);
    state.world.handle_event(CoreEvent::OutputAdded {
        id: OutputId(1),
        area: Rect::new(0, 0, width, height),
    });
    // apply() ends in request_render(), which arms the frame timer via
    // ensure_ticking() -- this is what puts the very first frame on it.
    state.apply();

    Ok(())
}

/// Updates an already-created output's mode. `location` is only meaningful
/// the first time (see `init`); later callers (`State::resize_output`) pass
/// `None` to leave it where it is.
fn set_mode(
    output: &Output,
    width: i32,
    height: i32,
    location: Option<smithay::utils::Point<i32, smithay::utils::Logical>>,
) {
    let mode = Mode {
        size: (width, height).into(),
        refresh: 60_000,
    };
    output.change_current_state(Some(mode), Some(Transform::Normal), None, location);
    output.set_preferred(mode);
}

/// Builds the CPU render target at a given size: a pixman renderer, an
/// offscreen image to draw into, and the damage tracker that pairs with it.
/// Shared by `init` and `State::resize_output` so the two can't drift apart.
fn create_backend(output: &Output, width: i32, height: i32) -> Result<Backend, Box<dyn Error>> {
    let mut renderer = PixmanRenderer::new()?;
    let image = renderer.create_buffer(Fourcc::Argb8888, (width, height).into())?;
    let damage = OutputDamageTracker::from_output(output);
    Ok(Backend {
        renderer,
        image,
        damage,
        size: (width, height),
    })
}

impl State {
    /// Draws a frame, if anything changed since the last one.
    pub fn render(&mut self) {
        if !self.needs_render {
            return;
        }
        // Order matters: `backend.take()` must not run unless `output` is
        // also present, or a None output would leave it taken and never put
        // back -- silently and permanently losing the backend on the next
        // render attempt.
        let Some(output) = self.output.clone() else {
            return;
        };
        let Some(mut backend) = self.backend.take() else {
            return;
        };
        // The client-supplied cursor surface this frame drew from, if any --
        // set only on the frames that actually went looking for one (`--tty`
        // with a pointer). See the `send_frames_surface_tree` call at the
        // end of this function.
        let mut cursor_surface: Option<WlSurface> = None;
        {
            let Backend {
                renderer,
                image,
                damage,
                size,
            } = &mut backend;
            // The core's arrangement, and this frame's ring segments built
            // from it -- computed once here rather than inside the match
            // below so a `space_render_elements` failure still logs without
            // having done this for nothing. See `decorations.rs`'s module
            // doc for why the background isn't part of this list.
            let arrangement = self.world.arrange();
            let (width, height) = *size;
            let bounds = Rect::new(0, 0, width, height);
            let ring_elements = self
                .decorations
                .elements(&arrangement, &self.appearance, bounds);
            match renderer.bind(image) {
                Ok(mut framebuffer) => {
                    // Only `--tty` ever draws a cursor -- see `cursor.rs`'s
                    // module doc; headless has no display and `--nested`
                    // already shows the host's own. The list is empty when
                    // there's nothing to draw (hidden, or a client cursor
                    // surface with no content yet) and can hold more than
                    // one element when a client's cursor surface has
                    // subsurfaces of its own.
                    let cursor_elements = if self.tty.is_some() {
                        match self.seat.get_pointer() {
                            Some(pointer) => {
                                cursor_surface = self.cursor.surface().cloned();
                                self.cursor.element(renderer, pointer.current_location())
                            }
                            None => Vec::new(),
                        }
                    } else {
                        Vec::new()
                    };
                    // Cursor first (topmost), then window surfaces, then the
                    // ring (bottom-most of the three): Smithay's damage
                    // tracker draws a `&[E]` back-to-front by walking it in
                    // reverse (confirmed in
                    // `OutputDamageTracker::render_output_internal`, which
                    // iterates `render_elements.iter().rev()`), so the
                    // *first* entry here ends up drawn *last*, i.e. on top.
                    // The cursor must win that ordering -- it's meaningless
                    // hidden behind a window -- and windows must still win
                    // over the ring: `shell.rs::apply()` positions a window
                    // from the layout's rect but sizes it from whatever the
                    // client actually committed, and a client that's slow to
                    // shrink (or a `--nested` resize still in flight) can
                    // briefly have a surface larger than its placement rect,
                    // reaching into the gap the ring is drawn in.
                    // Ring-on-top would paint over that live content every
                    // such frame; windows-on-top instead means the
                    // stale/oversized content can only ever cover the ring,
                    // never the reverse -- the same direction niri itself
                    // picks, and the only one of the two that can't corrupt
                    // what a client is showing.
                    match space::space_render_elements::<_, Window, _>(
                        renderer,
                        [&self.space],
                        &output,
                        1.0,
                    ) {
                        Ok(space_elements) => {
                            let mut elements = Vec::with_capacity(
                                cursor_elements.len() + space_elements.len() + ring_elements.len(),
                            );
                            elements.extend(cursor_elements.into_iter().map(Elements::Cursor));
                            elements.extend(space_elements.into_iter().map(Elements::Space));
                            elements.extend(ring_elements.into_iter().map(Elements::Decoration));

                            // `0` (always-full-redraw, unchanged from
                            // before this feature) for every presenter
                            // except `--tty`: see `buffers.rs`'s module doc
                            // on why that one specifically now needs a real
                            // buffer age -- cursor motion can trigger a
                            // render on every mouse-motion event, and a
                            // naive full-frame copy at that rate is exactly
                            // the multi-MB/s memcpy the roadmap calls out.
                            // Neither `--headless` nor `--nested` draws a
                            // cursor, so neither gained a new reason to
                            // render more often than before.
                            let age = self.tty.as_ref().map_or(0, Tty::next_buffer_age);
                            let result = damage.render_output(
                                renderer,
                                &mut framebuffer,
                                age,
                                &elements,
                                self.appearance.background_color,
                            );
                            match result {
                                Ok(render_result) => {
                                    // Must run unconditionally, even when
                                    // there turns out to be nothing to
                                    // present below -- see
                                    // `BufferPool::advance_generation`'s doc
                                    // for why this can't be skipped just
                                    // because this frame is.
                                    if let Some(tty) = &mut self.tty {
                                        tty.advance_generation();
                                    }
                                    // Both presenters read back the same
                                    // frame the same way; only what happens
                                    // with the pixels afterward differs, so
                                    // the read-back itself happens once for
                                    // whichever (or both) are set -- and not
                                    // at all with neither (plain
                                    // `--headless`, e.g. under IPC-only
                                    // control): that copy would be pure
                                    // waste on every render with nothing to
                                    // hand it to.
                                    if (self.host.is_some() || self.tty.is_some())
                                        && let Some(damaged) = render_result.damage
                                    {
                                        // Bounding box of every damaged
                                        // rect, not the rects themselves:
                                        // `copy_framebuffer` (and the
                                        // dumb-buffer write behind it) only
                                        // ever copies one contiguous region.
                                        // For `--headless`/`--nested` (`age`
                                        // always `0` above) this is always
                                        // the full frame regardless, so nothing
                                        // changes for them; for `--tty` a
                                        // small, cheap bbox is the common
                                        // case (see `buffers.rs`'s module
                                        // doc), and only a large pointer jump
                                        // or real content change grows it.
                                        let region = union_bbox(damaged);
                                        let buffer_region: Rectangle<i32, Buffer> = Rectangle::new(
                                            (region.loc.x, region.loc.y).into(),
                                            (region.size.w, region.size.h).into(),
                                        );
                                        // Argb8888 here is the same little-endian
                                        // BGRA layout wl_shm's own Argb8888 format
                                        // uses (see screenshot.rs's comment on the
                                        // same fact for the PNG path) -- unlike
                                        // there, this is a straight memcpy into the
                                        // presenter's own buffer, no channel
                                        // reordering.
                                        match renderer.copy_framebuffer(
                                            &framebuffer,
                                            buffer_region,
                                            Fourcc::Argb8888,
                                        ) {
                                            Ok(mapping) => match renderer.map_texture(&mapping) {
                                                Ok(pixels) => {
                                                    if let Some(host) = &mut self.host {
                                                        host.present(
                                                            pixels,
                                                            region.size.w,
                                                            region.size.h,
                                                        );
                                                    }
                                                    if let Some(tty) = &mut self.tty {
                                                        tty.present(
                                                            pixels,
                                                            region,
                                                            (width, height),
                                                        );
                                                    }
                                                }
                                                Err(error) => tracing::warn!(
                                                    %error,
                                                    "could not read back the frame for the presenter"
                                                ),
                                            },
                                            Err(error) => tracing::warn!(
                                                %error,
                                                "could not copy the framebuffer for the presenter"
                                            ),
                                        }
                                    }
                                    // else: no presenter is watching this
                                    // frame, or (only possible when `age >
                                    // 0`, i.e. only under `--tty`) nothing
                                    // actually changed -- e.g. a redundant
                                    // `request_render` with no real
                                    // difference -- so there's nothing to
                                    // read back or present either way.
                                }
                                Err(error) => tracing::warn!(%error, "could not render"),
                            }
                        }
                        Err(error) => tracing::warn!(
                            %error,
                            "could not gather window render elements"
                        ),
                    }
                }
                Err(error) => tracing::warn!(%error, "could not bind the framebuffer"),
            }
        }
        self.backend = Some(backend);
        self.needs_render = false;

        let time = self.start_time.elapsed();
        for window in self.space.elements() {
            window.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }
        // A client cursor surface is never in `self.space`, so the loop
        // above can't reach it -- and a well-behaved client with an animated
        // cursor (a spinner, a throbber) attaches one frame, requests a
        // callback, and waits for it before attaching the next. Without this
        // it waits forever and the animation freezes on its first frame.
        //
        // Scoped to the frames that presented this cursor (`--tty`, pointer
        // present, the status a live client surface) rather than sent
        // unconditionally: waking a client to draw cursor frames that
        // nothing on screen is showing is exactly the kind of pointless
        // work this compositor's own render loop avoids. It is deliberately
        // *not* narrowed further to "the surface produced at least one
        // element" -- a surface with no buffer yet produces none, and a
        // client that asks for a callback before its first attach (legal,
        // if unusual) would then stall on the very frame that should unstick
        // it.
        if let Some(surface) = &cursor_surface {
            send_frames_surface_tree(surface, &output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }
        self.space.refresh();
        self.popups.cleanup();
        let _ = self.display_handle.flush_clients();
    }

    /// Recreates the render target at a new size.
    ///
    /// Used only by `--nested`, when the host's first configure disagrees
    /// with the size flexwm started at (see `nested::init`). This function
    /// doesn't know `Host` exists -- it only touches the render target and
    /// the core's notion of output geometry; the caller is responsible for
    /// resizing `Host`'s own host-side buffers to match, separately.
    pub fn resize_output(&mut self, width: i32, height: i32) {
        let Some(output) = &self.output else {
            return;
        };
        set_mode(output, width, height, None);
        match create_backend(output, width, height) {
            Ok(backend) => self.backend = Some(backend),
            Err(error) => {
                tracing::warn!(%error, "could not resize the render target");
                return;
            }
        }
        self.world.handle_event(CoreEvent::OutputChanged {
            id: OutputId(1),
            area: Rect::new(0, 0, width, height),
        });
        self.request_render();
    }

    /// Marks the screen dirty and makes sure the frame ticker is running to
    /// actually redraw it.
    pub fn request_render(&mut self) {
        self.needs_render = true;
        self.ensure_ticking();
    }

    /// Arms the frame timer if it isn't already running.
    ///
    /// At idle -- nothing to redraw, no `wait-idle` outstanding -- the timer
    /// drops itself (see `frame_tick`) instead of polling 60-odd times a
    /// second forever, which is what this compositor did before: every tick
    /// woke the process just to find `needs_render` false and go back to
    /// sleep. This is the other half of that: whatever sets `needs_render` or
    /// registers a `PendingIdle` calls this to wake it back up.
    pub fn ensure_ticking(&mut self) {
        if self.timer_armed {
            return;
        }
        self.timer_armed = true;
        if let Err(error) = self
            .loop_handle
            .insert_source(Timer::from_duration(FRAME_INTERVAL), frame_tick)
        {
            // Rendering and wait-idle both depend on this timer; if it can't
            // be armed, both are now silently dead until something restarts
            // the process. That's worth a loud log even though there's no
            // Result to propagate up from here.
            tracing::error!(%error, "could not arm the frame timer");
            self.timer_armed = false;
        }
    }
}

fn frame_tick(_now: std::time::Instant, _metadata: &mut (), state: &mut State) -> TimeoutAction {
    state.render();
    state.settle_idle_waiters();
    if state.needs_render || !state.pending_idle.is_empty() {
        TimeoutAction::ToDuration(FRAME_INTERVAL)
    } else {
        state.timer_armed = false;
        TimeoutAction::Drop
    }
}

/// The smallest rectangle containing every rect in `rects`. Pulled out of
/// `render()` so it's testable without a live renderer, same rationale as
/// `input.rs`'s `clamp_to_extent`. `rects` must be non-empty --
/// `OutputDamageTracker::render_output` only ever returns `Some` damage
/// when it has at least one rectangle to report; an empty list would have
/// been `None` instead (confirmed against the pinned Smithay source).
fn union_bbox(rects: &[Rectangle<i32, Physical>]) -> Rectangle<i32, Physical> {
    let mut iter = rects.iter().copied();
    let first = iter
        .next()
        .expect("render_output never returns an empty damage list");
    iter.fold(first, |acc, rect| {
        let x0 = acc.loc.x.min(rect.loc.x);
        let y0 = acc.loc.y.min(rect.loc.y);
        let x1 = (acc.loc.x + acc.size.w).max(rect.loc.x + rect.size.w);
        let y1 = (acc.loc.y + acc.size.h).max(rect.loc.y + rect.size.h);
        Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_single_rect_is_its_own_bounding_box() {
        assert_eq!(union_bbox(&[rect(10, 20, 30, 40)]), rect(10, 20, 30, 40));
    }

    #[test]
    fn disjoint_rects_bound_the_gap_between_them() {
        // An old cursor position and a new one some distance away: the
        // bbox must cover both plus whatever's between them, since a
        // single `copy_framebuffer`/dumb-buffer write can only ever copy
        // one contiguous region.
        let old_position = rect(0, 0, 16, 16);
        let new_position = rect(100, 50, 16, 16);
        assert_eq!(
            union_bbox(&[old_position, new_position]),
            rect(0, 0, 116, 66)
        );
    }

    #[test]
    fn an_overlapping_rect_does_not_grow_the_box_past_the_union() {
        let a = rect(0, 0, 20, 20);
        let b = rect(10, 10, 20, 20);
        assert_eq!(union_bbox(&[a, b]), rect(0, 0, 30, 30));
    }

    #[test]
    fn a_rect_fully_containing_another_wins_alone() {
        let outer = rect(0, 0, 100, 100);
        let inner = rect(40, 40, 10, 10);
        assert_eq!(union_bbox(&[inner, outer]), outer);
    }
}
