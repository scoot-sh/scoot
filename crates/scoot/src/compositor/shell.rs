//! Keeping Wayland and the core in step: events in, arrangement out.

use scoot_core::{Action, Effect, Event, Rect, Size, SizeHints, WindowId, WindowInfo};
use smithay::desktop::Window;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::SurfaceCachedState;
use smithay::wayland::shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData};

use super::State;

#[cfg(test)]
mod tests;

impl State {
    /// Registers a new toplevel with the core, which decides where it goes.
    pub fn add_window(&mut self, surface: ToplevelSurface) {
        self.next_id += 1;
        let id = WindowId(self.next_id);
        self.windows.insert(id, Window::new_wayland_window(surface));
        let info = self.info_of(id);
        // Before the core hears about it, though nothing depends on the order:
        // these are the other two lists of windows scoot publishes (see
        // `foreign_toplevel.rs` for `ext-foreign-toplevel-list-v1` and
        // `foreign_toplevel_management.rs` for the wlr protocol every
        // Quickshell-based shell actually binds), and both cover the same
        // lifetime as this one -- from here to `remove_window`, not from first
        // buffer to last. Announced together so the two can never disagree
        // about when a window came into existence.
        self.open_foreign_toplevel(id, &info);
        self.open_wlr_toplevel(id, &info);
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
        // Paired with `add_window`'s announcements: these send `closed` to
        // every client watching either list, so a taskbar drops the entry.
        // Before the focus is cleared below, so nothing tries to publish an
        // activation change for a window that has just been closed.
        self.close_foreign_toplevel(id);
        self.close_wlr_toplevel(id);
        self.requested.remove(&id);
        if self.focus == Some(id) {
            self.focus = None;
        }
        self.world.handle_event(Event::WindowClosed { id });
        self.apply();
    }

    /// Re-reads a window's app id, title and minimum size.
    ///
    /// Called only from `XdgShellHandler`'s `title_changed`/`app_id_changed`,
    /// which the pinned Smithay rev raises only when the value really changed
    /// -- and each foreign-toplevel publish compares once more against what
    /// its own handles were last sent, so only the field that moved goes out
    /// with the `done` that closes it.
    pub fn refresh_window(&mut self, id: WindowId) {
        let info = self.info_of(id);
        // Before `info` moves into the event below.
        self.publish_foreign_toplevel(id, &info);
        self.publish_wlr_toplevel(id, &info);
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

    /// Runs a window-management action: the one path every *requested*
    /// action goes through, whichever of the three asked for it (a
    /// keybinding, an IPC `action` request, or an `ext-workspace-v1` client
    /// activating a workspace).
    ///
    /// Which is why the session-lock gate is here as well as at each of
    /// those: `Spawn` would put a new client's window on a locked screen,
    /// `Quit` would tear the session down, `Close` would reach a window the
    /// user cannot see, and every layout action would rearrange a session
    /// behind the lock screen. Each caller still has its own check where it
    /// needs to *report* the refusal (`ipc.rs` answers an error;
    /// `input.rs::key` forwards the keystroke to the lock client instead of
    /// swallowing it), and this is the backstop that makes a caller added
    /// later safe by default rather than by remembering.
    ///
    /// Deliberately *not* a gate on the compositor's own window lifecycle:
    /// `add_window`/`remove_window`/`refresh_window` drive the core directly
    /// through `handle_event`, never through here, so a client mapping or
    /// closing a window while locked is still tracked (it is simply not
    /// drawn) and the session is intact when it unlocks.
    pub fn act(&mut self, action: Action) {
        if self.session_lock.is_locked() {
            tracing::debug!(?action, "ignoring an action: the session is locked");
            return;
        }
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
        // The one place workspace changes reach `ext-workspace-v1` clients:
        // every event and action that can add, drop or switch a workspace
        // ends here (see `ext_workspace.rs`). Costs one snapshot compare per
        // output when nothing about the workspaces changed, which is the
        // common case, and no allocation either way.
        self.refresh_workspaces();
        self.request_render();
    }

    /// Moves *window* focus -- activation, and with it the focus ring -- and
    /// then re-derives who actually gets the keys.
    ///
    /// The two are no longer the same question, and the second half runs
    /// unconditionally for a reason worth spelling out: clicking away from an
    /// `on_demand` layer surface onto the window that was already focused
    /// arrives here with `focus` unchanged, and that unconditional refresh is
    /// the only thing that takes the keyboard back off the layer surface.
    fn set_focus(&mut self, focus: Option<WindowId>) {
        if self.focus != focus {
            self.focus = focus;
            for (id, window) in &self.windows {
                window.set_activated(Some(*id) == focus);
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_pending_configure();
                }
            }
            // The third reader of the same `focus`, alongside `set_activated`
            // above and the focus ring the render path draws from
            // `State::focus`: `wlr-foreign-toplevel-management-v1`'s
            // `activated` state bit (see
            // `foreign_toplevel_management.rs`). Deliberately re-derived from
            // `self.focus` there rather than handed the two ids that moved --
            // `remove_window` writes that field directly without coming
            // through here, so the field is the only thing always current.
            // Sends nothing for the windows whose bit did not move, which is
            // every window but at most two.
            self.refresh_wlr_activation();
        }
        self.refresh_keyboard_focus();
    }

    /// Hands the keyboard to whatever should have it: a lock surface if the
    /// session is locked, otherwise a layer surface if the layer-shell policy
    /// says so (see `layer_shell.rs`), otherwise the focused window's
    /// toplevel, otherwise nobody.
    ///
    /// The lock branch is a replacement for the rest, not a first entry in
    /// it: while the session is locked, "nobody" is a correct answer and a
    /// window or a layer surface never is (see `session_lock.rs`).
    ///
    /// An active `xdg_popup.grab` is the one thing that can override the
    /// answer below, because Smithay's `PopupKeyboardGrab` ignores a
    /// `set_focus` while it is live. That is intended for a window and for a
    /// click-focused layer surface -- a menu is meant to hold the keyboard
    /// -- and unacceptable for the two cases handled here explicitly: a lock
    /// screen would never receive the password, and a launcher would never
    /// receive a keystroke. Both dismiss the grab outright rather than
    /// quietly losing to it; see `popup.rs` for the whole precedence order
    /// and why dismissal (not just unsetting the seat grab) is the right
    /// verb.
    ///
    /// Safe to call as often as anything might have changed. Smithay's own
    /// `set_focus` compares against the current focus and does nothing when
    /// it matches, so a redundant call costs a serial and two locks rather
    /// than a spurious `leave`/`enter` pair to two clients.
    ///
    /// Callers must not be holding a layer-map guard: this takes one (via
    /// `layer_keyboard_focus`) and then re-enters Smithay through
    /// `set_focus`. See `layer_shell.rs`'s guard-discipline note.
    pub(super) fn refresh_keyboard_focus(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        self.forget_dead_clicked_layer();
        let surface = if self.session_lock.is_locked() {
            // Not a layer surface, whatever the layer map says -- so the
            // gate `commit_layer_surface` reads must say so too, or a bar
            // committing while locked would re-derive focus for nothing.
            self.keyboard_on_layer = false;
            self.dismiss_popup_grab();
            self.lock_keyboard_focus()
        } else {
            // Owned, so the layer-map guard is already gone by the time
            // `dismiss_popup_grab` re-enters Smithay -- see this module's
            // guard-discipline note in `layer_shell.rs`.
            let layer = self.layer_keyboard_focus();
            self.keyboard_on_layer = layer.is_some();
            // An `exclusive` surface pre-empts a grab -- unless the grab is
            // its own menu (see `popup.rs`): the launcher that opened the
            // dropdown does not dismiss it. The root comes from the held
            // grab's own start data, since no grab request is in hand here;
            // a grab rooted anywhere else is still dismissed.
            let pre_empted = match &layer {
                Some(found) if found.exclusive => !self.popup_grab_rooted_on(&found.surface),
                _ => false,
            };
            if pre_empted {
                self.dismiss_popup_grab();
            }
            layer.map(|found| found.surface).or_else(|| {
                self.focus
                    .and_then(|id| self.windows.get(&id))
                    .and_then(Window::toplevel)
                    .map(|toplevel| toplevel.wl_surface().clone())
            })
        };
        let serial = SERIAL_COUNTER.next_serial();
        // The keyboard half of the popup-grab history (`popup.rs`): when
        // this `set_focus` below actually delivers an `enter`, that serial
        // is what a toolkit passes to `xdg_popup.grab` if the menu was
        // opened without a newer button or key event. Recorded under the
        // entered client, like everything else in `interaction_serials`.
        //
        // Two guards, each load-bearing:
        // - the focus must really move. A redundant derivation mints a
        //   serial but sends nothing, and filing it would put a serial the
        //   client never saw under its name -- guessable from the one it
        //   did see, which is exactly the hole the client half of each
        //   entry exists to close.
        // - no popup grab may be live. Smithay's `PopupKeyboardGrab`
        //   swallows a `set_focus` while it holds the seat, so nothing is
        //   delivered -- and the install path in `popup.rs` deliberately
        //   sets focus to the popup with the *grab's own* serial, which must
        //   never be filed as fresh evidence for the grabbing client.
        //   (`has_ended` without a preceding `cleanup` can over-report a
        //   grab as live; that only skips recording, the safe direction.)
        //   An input-method grab, by contrast, forwards `set_focus`, so an
        //   enter delivered through one is recorded normally.
        //
        // Costs one seat lock and a surface-handle clone per focus
        // derivation -- not per event or per frame, so no hot-path concern.
        if self.popup_grab.as_ref().is_none_or(|grab| grab.has_ended())
            && keyboard.current_focus().as_ref() != surface.as_ref()
            && let Some(ref entered) = surface
            && let Some(client) = self.client_of(entered)
        {
            self.interaction_serials.record_focus(serial, client);
        }
        keyboard.set_focus(self, surface, serial);
    }

    fn info_of(&self, id: WindowId) -> WindowInfo {
        let Some(toplevel) = self.window(id).and_then(Window::toplevel) else {
            return WindowInfo::default();
        };
        // Computed before the `with_states` closure rather than inside it:
        // that guard is a plain non-reentrant mutex (see `cursor.rs`'s
        // hotspot lookup for the same rule), and there is no reason to hold
        // it while walking the core's outputs.
        //
        // The gap read here is already `Config::validated`'s (`World::new`
        // runs it), so it is in `0..=Config::MAX_GAP` and `Rect::inset`'s
        // `2 * gap` cannot overflow.
        let limit = hint_limit(
            self.world.usable_areas().into_iter(),
            self.world.config().gap,
        );
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
                    min: clamp_hint(Size::new(min.w, min.h), limit),
                },
            }
        })
    }
}

/// The largest minimum size a client's own `min_size` may declare, per axis:
/// the usable area (an output's area, minus anything a layer-shell surface
/// reserved, inset by the layout gap) of the largest output the core knows
/// about.
///
/// `xdg_toplevel.set_min_size` takes two raw, unvalidated `i32`s -- the
/// pinned Smithay rev stores them verbatim in `SurfaceCachedState`
/// (`handlers/surface/toplevel.rs`: `toplevel_data.min_size = (width,
/// height).into()`, no range check) -- so without this, a client can put
/// `i32::MAX` into `WindowState::min()` and from there into
/// `layout::column_width`'s `width.max(min)`, i.e. into a column that wide.
/// Two unchecked `i32` adds downstream of that overflow on it, both proven by
/// deliberately disabling this clamp and watching the tests below fail
/// (debug builds panic; release wraps):
///
/// - `scoot_core`'s own `World::place_workspace`, `x + width` in the
///   on-screen test -- reached first, during `arrange`, before any rendering;
/// - `decorations::ring_rects`'s `rect.w + 2 * width`, whose wrapped result
///   `clip` happens to discard in release.
///
/// This is the same bound `scoot_core`'s `learn_from_frame` already applies
/// to a *learned* minimum, with one deliberate difference: that one caps to
/// the usable area of the output the window is on, and this takes the largest
/// of every output instead. Nothing in `World`'s public surface says which
/// output a given window is on, and `info_of` runs for a brand-new toplevel
/// before it has been placed on one at all -- so a bound no tighter than any
/// single output's is the honest version: it can never wrongly shrink a hint
/// a window could legitimately fill its own screen with. With the one output
/// this compositor creates today the two bounds are identical.
///
/// The other end of the chain is the CLI: `--width`/`--height` are refused
/// past 65535 (`cli::MAX_OUTPUT_DIMENSION`, the most DRM itself can report
/// for a mode axis), so the largest area that can reach this limit is
/// 65535x65535 -- 131070 a side at the `[output] scale` floor of 0.5 -- and
/// the limit is at most that per axis. That is what keeps this clamp a real
/// bound rather than the ~2x10^9 no-op a flag-spelled output used to make it.
///
/// With no outputs at all the limit is zero, which drops the hint entirely.
/// That is unreachable today (`headless::init` adds the output before the
/// event loop starts, and nothing removes one), and if output removal ever
/// lands the consequence is a hint lost until the client's next
/// `app_id`/`title` change re-reads it -- not a bad size: a window the core
/// has no output for is `unplaced`, so it is neither arranged nor rendered.
fn hint_limit(areas: impl Iterator<Item = Rect>, gap: i32) -> Size {
    areas.fold(Size::new(0, 0), |limit, area| {
        let usable = area.inset(gap);
        Size::new(limit.w.max(usable.w), limit.h.max(usable.h))
    })
}

/// Caps a client-declared minimum size to `limit`, and to zero from below
/// (`set_min_size` accepts negatives too, and a negative minimum is not a
/// minimum).
fn clamp_hint(min: Size, limit: Size) -> Size {
    // Deliberately not `i32::clamp`, which panics when its own `min > max`:
    // `hint_limit` cannot return a negative limit today, but a clamp with a
    // panic in it is not worth the symmetry inside a compositor.
    Size::new(min.w.max(0).min(limit.w), min.h.max(0).min(limit.h))
}
