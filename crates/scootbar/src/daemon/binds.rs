//! Which optional globals the bar holds, decided by what it shows.
//!
//! A bound `ext_workspace_manager_v1` makes the compositor send the bar
//! every workspace change, and the daemon wake for and parse each one, so
//! it is bound only while a workspaces module is placed. The same holds
//! for the `zwlr_foreign_toplevel_manager_v1` and the window-title module:
//! every title change would wake the bar otherwise. `xdg_wm_base` is bound
//! only while some binding in the config names `popup` (`on-click =
//! "popup"`), and let go when none does and no popup is open; a bar that
//! never opens one has no popup global at all, and `scootbar msg invoke ID
//! popup` on one binds it, and that bind is kept until the next
//! [`State::sync_binds`] (a reload or a registry event), not released when its
//! popup closes ([`State::bind_xdg`]). The seat is there
//! for pointer input, so it is bound only while a placed module takes any
//! (a binding in the config, or the workspaces and window-title clicks:
//! [`State::needs_pointer`]); the `wl_pointer` itself is `input`'s.
//!
//! The decision is remade at every point that can change it, by
//! [`State::sync_binds`]: once at connect (from the globals the registry
//! listed), when a global appears later (a compositor that starts the
//! protocol after the bar), and after a `reload` changed what is placed.
//! Going from placed to not placed lets the protocol go (stops the
//! manager, destroys its handles, releases the seat and its pointer) so
//! the compositor stops sending; going the other way binds it then.

use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Proxy, QueueHandle};
#[cfg(feature = "workspaces")]
use wayland_protocols::ext::workspace::v1::client::ext_workspace_manager_v1::ExtWorkspaceManagerV1;
#[cfg(feature = "window-title")]
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1;

#[cfg(feature = "popup")]
use wayland_protocols::xdg::shell::client::xdg_wm_base::XdgWmBase;

use super::wayland::State;

/// A global the registry offered: its name and the highest version offered.
#[derive(Debug, Clone, Copy)]
struct Offer {
    name: u32,
    version: u32,
}

/// What the registry offers of the globals bound on demand, and the
/// registry to bind them from.
#[derive(Debug)]
pub struct Binds {
    registry: WlRegistry,
    #[cfg(feature = "workspaces")]
    manager: Option<Offer>,
    #[cfg(feature = "window-title")]
    toplevel: Option<Offer>,
    seat: Option<Offer>,
    #[cfg(feature = "popup")]
    xdg: Option<Offer>,
}

impl Binds {
    pub fn new(registry: WlRegistry) -> Self {
        Self {
            registry,
            #[cfg(feature = "workspaces")]
            manager: None,
            #[cfg(feature = "window-title")]
            toplevel: None,
            seat: None,
            #[cfg(feature = "popup")]
            xdg: None,
        }
    }
}

impl State {
    /// Whether the workspaces module is placed, so it needs the manager.
    #[cfg(feature = "workspaces")]
    fn wants_workspaces(&self) -> bool {
        self.content
            .modules
            .iter()
            .any(|placed| placed.id == crate::modules::workspaces::ID)
    }

    /// Whether the window-title module is placed, so it needs the
    /// toplevel manager.
    #[cfg(feature = "window-title")]
    fn wants_title(&self) -> bool {
        self.content
            .modules
            .iter()
            .any(|placed| placed.id == crate::modules::window_title::ID)
    }

    /// Whether the config binds a click to a popup, so it needs
    /// `xdg_wm_base`.
    #[cfg(feature = "popup")]
    fn wants_xdg(&self) -> bool {
        self.content
            .modules
            .iter()
            .any(|placed| placed.bindings.binds_module_action(crate::action::POPUP))
    }

    /// `xdg_wm_base`, bound now if it is not, from what the registry offered
    /// (no round trip: the registry listed it): what a popup opened with no
    /// binding (an agent's `invoke`) needs. `None` where the compositor has
    /// none. Held until a [`State::sync_binds`] finds no binding and no open
    /// popup.
    #[cfg(feature = "popup")]
    pub(super) fn bind_xdg(&mut self, qh: &QueueHandle<Self>) -> Option<XdgWmBase> {
        if self.globals.xdg.is_none() {
            let offer = self.binds.xdg?;
            // Version 1 has everything a popup needs.
            self.globals.xdg = Some(self.binds.registry.bind::<XdgWmBase, _, _>(
                offer.name,
                offer.version.min(1),
                qh,
                (),
            ));
        }
        self.globals.xdg.clone()
    }

    /// Whether anything placed takes pointer input, so it needs the seat.
    fn wants_seat(&self) -> bool {
        self.needs_pointer()
    }

    /// Notes a global the registry listed or announced. Whether it is one
    /// of the on-demand ones: bound now if something placed wants it.
    pub fn offer(
        &mut self,
        interface: &str,
        name: u32,
        version: u32,
        qh: &QueueHandle<Self>,
    ) -> bool {
        let offer = Some(Offer { name, version });
        #[cfg(feature = "workspaces")]
        if interface == ExtWorkspaceManagerV1::interface().name {
            self.binds.manager = offer;
            self.sync_binds(qh);
            return true;
        }
        #[cfg(feature = "window-title")]
        if interface == ZwlrForeignToplevelManagerV1::interface().name {
            self.binds.toplevel = offer;
            self.sync_binds(qh);
            return true;
        }
        #[cfg(feature = "popup")]
        if interface == XdgWmBase::interface().name {
            self.binds.xdg = offer;
            self.sync_binds(qh);
            return true;
        }
        if interface == WlSeat::interface().name {
            self.binds.seat = offer;
        } else {
            return false;
        }
        self.sync_binds(qh);
        true
    }

    /// Notes a global gone. A proxy already bound stays until the
    /// compositor says it is finished with it.
    pub fn withdraw(&mut self, name: u32) -> bool {
        let mut found = false;
        #[cfg(feature = "workspaces")]
        if self.binds.manager.is_some_and(|offer| offer.name == name) {
            self.binds.manager = None;
            found = true;
        }
        #[cfg(feature = "window-title")]
        if self.binds.toplevel.is_some_and(|offer| offer.name == name) {
            self.binds.toplevel = None;
            found = true;
        }
        #[cfg(feature = "popup")]
        if self.binds.xdg.is_some_and(|offer| offer.name == name) {
            self.binds.xdg = None;
            found = true;
        }
        if self.binds.seat.is_some_and(|offer| offer.name == name) {
            self.binds.seat = None;
            found = true;
        }
        found
    }

    /// Makes the binds as what is placed says, and only that: a global
    /// already held, or not offered, is left alone. Cheap and idempotent.
    pub fn sync_binds(&mut self, qh: &QueueHandle<Self>) {
        #[cfg(feature = "workspaces")]
        {
            let mut shared = self.workspaces.0.borrow_mut();
            if !self.wants_workspaces() {
                if shared.has_manager() {
                    shared.release();
                }
            } else if !shared.has_manager() {
                if let Some(offer) = self.binds.manager {
                    // Version 1 is the only one.
                    let manager = self.binds.registry.bind::<ExtWorkspaceManagerV1, _, _>(
                        offer.name,
                        offer.version.min(1),
                        qh,
                        (),
                    );
                    shared.set_manager(manager);
                }
            }
        }
        #[cfg(feature = "window-title")]
        {
            let mut shared = self.title.0.borrow_mut();
            if !self.wants_title() {
                if shared.has_manager() {
                    shared.release();
                }
            } else if !shared.has_manager() {
                if let Some(offer) = self.binds.toplevel {
                    // Version 3 is the newest: `fullscreen` state from 2,
                    // `parent` from 3.
                    let manager = self
                        .binds
                        .registry
                        .bind::<ZwlrForeignToplevelManagerV1, _, _>(
                            offer.name,
                            offer.version.min(3),
                            qh,
                            (),
                        );
                    shared.set_manager(manager);
                }
            }
        }
        #[cfg(feature = "popup")]
        {
            if self.wants_xdg() {
                let _ = self.bind_xdg(qh);
            } else if !self.popup.is_open() {
                // No binding names a popup and none is open: let the global go
                // (`destroy` from the first version). One an `invoke` bound
                // goes here too, at the next sync after its popup closed.
                if let Some(base) = self.globals.xdg.take() {
                    base.destroy();
                }
            }
        }
        if self.wants_seat() {
            if self.globals.seat.is_none() {
                if let Some(offer) = self.binds.seat {
                    self.globals.seat = Some(self.binds.registry.bind::<WlSeat, _, _>(
                        offer.name,
                        offer.version.min(9),
                        qh,
                        (),
                    ));
                }
            }
            // Whatever `activate` rides on: the title module keeps it while
            // one is bound.
            #[cfg(feature = "window-title")]
            if let Some(seat) = &self.globals.seat {
                self.title.0.borrow_mut().set_seat(seat);
            }
        } else if self.globals.seat.as_ref().is_some_and(|s| s.version() >= 5) {
            // Below version 5 a seat cannot be released: it stays, with its
            // pointer, rather than leave a live seat nothing holds.
            if let Some(pointer) = self.pointer.take() {
                if pointer.version() >= 3 {
                    pointer.release();
                }
            }
            self.input.leave();
            self.seat_pointer = false;
            #[cfg(feature = "popup")]
            {
                self.seat_keyboard = false;
            }
            if let Some(seat) = self.globals.seat.take() {
                seat.release();
            }
            // No seat is bound, so none is kept for `activate` either.
            #[cfg(feature = "window-title")]
            self.title.0.borrow_mut().clear_seat();
        }
    }
}
