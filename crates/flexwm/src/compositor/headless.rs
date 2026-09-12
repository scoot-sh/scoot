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
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, Offscreen};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::Transform;

use super::State;

/// How often a changed screen is redrawn.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// The CPU renderer and the image it draws into.
pub struct Backend {
    pub renderer: PixmanRenderer,
    pub image: Image<'static, 'static>,
    pub damage: OutputDamageTracker,
    pub size: (i32, i32),
}

pub fn init(
    event_loop: &mut EventLoop<'static, State>,
    state: &mut State,
    width: i32,
    height: i32,
) -> Result<(), Box<dyn Error>> {
    let mode = Mode {
        size: (width, height).into(),
        refresh: 60_000,
    };
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
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    let mut renderer = PixmanRenderer::new()?;
    let image = renderer.create_buffer(Fourcc::Argb8888, (width, height).into())?;
    let damage = OutputDamageTracker::from_output(&output);

    state.backend = Some(Backend {
        renderer,
        image,
        damage,
        size: (width, height),
    });
    state.output = Some(output);
    state.world.handle_event(CoreEvent::OutputAdded {
        id: OutputId(1),
        area: Rect::new(0, 0, width, height),
    });
    state.apply();

    event_loop.handle().insert_source(
        Timer::from_duration(FRAME_INTERVAL),
        |_, _, state: &mut State| {
            state.render();
            state.settle_idle_waiters();
            TimeoutAction::ToDuration(FRAME_INTERVAL)
        },
    )?;
    Ok(())
}

impl State {
    /// Draws a frame, if anything changed since the last one.
    pub fn render(&mut self) {
        if !self.needs_render {
            return;
        }
        let (Some(output), Some(mut backend)) = (self.output.clone(), self.backend.take()) else {
            return;
        };
        {
            let Backend {
                renderer,
                image,
                damage,
                ..
            } = &mut backend;
            match renderer.bind(image) {
                Ok(mut framebuffer) => {
                    let result = smithay::desktop::space::render_output::<
                        _,
                        WaylandSurfaceRenderElement<PixmanRenderer>,
                        _,
                        _,
                    >(
                        &output,
                        renderer,
                        &mut framebuffer,
                        1.0,
                        0,
                        [&self.space],
                        &[],
                        damage,
                        [0.05, 0.05, 0.06, 1.0],
                    );
                    if let Err(error) = result {
                        tracing::warn!(%error, "could not render");
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
}
