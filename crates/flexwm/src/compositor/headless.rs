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
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Rectangle, Transform};

use super::State;

// Everything `render()` can draw, front-to-back as Smithay's damage tracker
// expects (see `render()`'s comment on that convention): window surfaces,
// then this feature's focus-ring segments underneath them. The background
// isn't a variant here at all -- it's the `render_output` call's
// `clear_color`, always the bottom-most thing on screen by construction; see
// `decorations.rs`'s module doc for why that's simpler and safer than a
// full-output element.
//
// Fixed to `PixmanRenderer` (this backend's only renderer) rather than
// generic over `R`, which is why this lives here and not in
// `decorations.rs` -- that module stays renderer-agnostic, returning plain
// `SolidColorRenderElement`s this enum only wraps.
render_elements! {
    Elements<=PixmanRenderer>;
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
                    // Window surfaces first (topmost) and the ring after
                    // (bottom-most of the two): Smithay's damage tracker
                    // draws a `&[E]` back-to-front by walking it in reverse
                    // (confirmed in `OutputDamageTracker::render_output_internal`,
                    // which iterates `render_elements.iter().rev()`), so the
                    // *first* entry here ends up drawn *last*, i.e. on top.
                    // Windows must win that ordering: `shell.rs::apply()`
                    // positions a window from the layout's rect but sizes it
                    // from whatever the client actually committed, and a
                    // client that's slow to shrink (or a `--nested` resize
                    // still in flight) can briefly have a surface larger
                    // than its placement rect, reaching into the gap the
                    // ring is drawn in. Ring-on-top would paint over that
                    // live content every such frame; windows-on-top instead
                    // means the stale/oversized content can only ever cover
                    // the ring, never the reverse -- the same direction niri
                    // itself picks, and the only one of the two that can't
                    // corrupt what a client is showing.
                    match space::space_render_elements::<_, Window, _>(
                        renderer,
                        [&self.space],
                        &output,
                        1.0,
                    ) {
                        Ok(space_elements) => {
                            let mut elements =
                                Vec::with_capacity(space_elements.len() + ring_elements.len());
                            elements.extend(space_elements.into_iter().map(Elements::Space));
                            elements.extend(ring_elements.into_iter().map(Elements::Decoration));
                            let result = damage.render_output(
                                renderer,
                                &mut framebuffer,
                                0,
                                &elements,
                                self.appearance.background_color,
                            );
                            match result {
                                Ok(_) => {
                                    // Both presenters read back the same frame the
                                    // same way; only what happens with the pixels
                                    // afterward differs, so the read-back itself
                                    // happens once for whichever (or both) are set.
                                    if self.host.is_some() || self.tty.is_some() {
                                        // Argb8888 here is the same little-endian BGRA
                                        // layout wl_shm's own Argb8888 format uses
                                        // (see screenshot.rs's comment on the same
                                        // fact for the PNG path) -- unlike there, this
                                        // is a straight memcpy into the presenter's
                                        // own buffer, no channel reordering.
                                        // (width, height) already bound above, from
                                        // this same *size, for `bounds`.
                                        let region = Rectangle::from_size((width, height).into());
                                        match renderer.copy_framebuffer(
                                            &framebuffer,
                                            region,
                                            Fourcc::Argb8888,
                                        ) {
                                            Ok(mapping) => match renderer.map_texture(&mapping) {
                                                Ok(pixels) => {
                                                    if let Some(host) = &mut self.host {
                                                        host.present(pixels, width, height);
                                                    }
                                                    if let Some(tty) = &mut self.tty {
                                                        tty.present(pixels, width, height);
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
