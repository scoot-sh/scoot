//! Keeping Wayland and the core in step: events in, arrangement out.

use flexwm_core::{Action, Effect, Event, Size, SizeHints, WindowId, WindowInfo};
use smithay::desktop::Window;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::SurfaceCachedState;
use smithay::wayland::shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData};

use super::State;

impl State {
    /// Registers a new toplevel with the core, which decides where it goes.
    pub fn add_window(&mut self, surface: ToplevelSurface) {
        self.next_id += 1;
        let id = WindowId(self.next_id);
        self.windows.insert(id, Window::new_wayland_window(surface));
        let info = self.info_of(id);
        let output = self.world.outputs().first().map(|(id, _)| *id);
        self.world.handle_event(Event::WindowOpened {
            id,
            info,
            output,
            focus: true,
        });
        self.apply();
    }

    pub fn remove_window(&mut self, id: WindowId) {
        if let Some(window) = self.windows.remove(&id) {
            self.space.unmap_elem(&window);
        }
        self.requested.remove(&id);
        if self.focus == Some(id) {
            self.focus = None;
        }
        self.world.handle_event(Event::WindowClosed { id });
        self.apply();
    }

    /// Re-reads a window's app id, title and minimum size.
    pub fn refresh_window(&mut self, id: WindowId) {
        let info = self.info_of(id);
        self.world.handle_event(Event::WindowChanged { id, info });
        self.apply();
    }

    /// Tells the core what size a window actually took, so it can learn the
    /// minimums of windows that refuse to shrink.
    pub fn observe_frame(&mut self, id: WindowId) {
        let Some(window) = self.window(id) else {
            return;
        };
        let size = window.geometry().size;
        let actual = Size::new(size.w, size.h);
        let requested = self.requested.get(&id).copied().unwrap_or(actual);
        self.world.handle_event(Event::FrameObserved {
            id,
            requested,
            actual,
        });
    }

    pub fn act(&mut self, action: Action) {
        for effect in self.world.handle_action(action) {
            match effect {
                Effect::Close(id) => {
                    if let Some(toplevel) = self.window(id).and_then(Window::toplevel) {
                        toplevel.send_close();
                    }
                }
                Effect::Spawn(command) => self.spawn(&command),
                Effect::Quit => self.loop_signal.stop(),
            }
        }
        self.apply();
    }

    /// Pushes the core's arrangement onto the windows: position, size, focus.
    pub fn apply(&mut self) {
        let arrangement = self.world.arrange();
        for placement in &arrangement.placements {
            let Some(window) = self.windows.get(&placement.id).cloned() else {
                continue;
            };
            if !placement.visible {
                self.space.unmap_elem(&window);
                continue;
            }
            self.space
                .map_element(window.clone(), (placement.rect.x, placement.rect.y), false);
            if let Some(toplevel) = window.toplevel() {
                let size = Size::new(placement.rect.w, placement.rect.h);
                toplevel.with_pending_state(|state| state.size = Some((size.w, size.h).into()));
                toplevel.send_pending_configure();
                self.requested.insert(placement.id, size);
            }
        }
        self.set_focus(arrangement.focused);
        self.request_render();
    }

    fn set_focus(&mut self, focus: Option<WindowId>) {
        if self.focus == focus {
            return;
        }
        self.focus = focus;
        for (id, window) in &self.windows {
            window.set_activated(Some(*id) == focus);
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
        let surface = focus
            .and_then(|id| self.windows.get(&id))
            .and_then(Window::toplevel)
            .map(|toplevel| toplevel.wl_surface().clone());
        if let Some(keyboard) = self.seat.get_keyboard() {
            let serial = SERIAL_COUNTER.next_serial();
            keyboard.set_focus(self, surface, serial);
        }
    }

    fn info_of(&self, id: WindowId) -> WindowInfo {
        let Some(toplevel) = self.window(id).and_then(Window::toplevel) else {
            return WindowInfo::default();
        };
        with_states(toplevel.wl_surface(), |states| {
            let Some(data) = states.data_map.get::<XdgToplevelSurfaceData>() else {
                return WindowInfo::default();
            };
            let attributes = data.lock().expect("toplevel attributes");
            let min = states
                .cached_state
                .get::<SurfaceCachedState>()
                .current()
                .min_size;
            WindowInfo {
                app_id: attributes.app_id.clone().unwrap_or_default(),
                title: attributes.title.clone().unwrap_or_default(),
                hints: SizeHints {
                    min: Size::new(min.w, min.h),
                },
            }
        })
    }
}
