//! Managed X11 windows in the core: the XWayland half of `shell.rs`.
//!
//! An X window enters the core when it asks to be mapped (`MapRequest`,
//! after the reparenting XWM has framed it) and leaves when it unmaps or is
//! destroyed -- X's own boundary, unlike an `xdg_toplevel`, which is in the
//! layout from creation to destruction. A re-mapped X window is a new window
//! with a new id, which is also what a taskbar sees (a withdrawn X window
//! is gone from every list, as under any X window manager).
//!
//! # Policy
//!
//! - **Columns by default.** Every managed X window is a column entry,
//!   and its own position is ignored: the layout decides.
//! - **Floating, from the same signals and rules as an xdg window**
//!   (`window_rules.rs`), read once at the map request: a transient
//!   (`WM_TRANSIENT_FOR`, parent or not -- a group transient, or one naming
//!   a window scoot does not manage, still floats, with no parent to centre
//!   on), a window typed anything but `_NET_WM_WINDOW_TYPE_NORMAL`
//!   (dialog, utility, splash, and the menu/tooltip/notification types a
//!   client maps as managed windows), `_NET_WM_STATE_MODAL`, or a fixed size
//!   (`WM_NORMAL_HINTS` minimum equal to maximum, both non-zero). A
//!   `[[window_rule]]`'s `match_app_id` matches an X window's `WM_CLASS`
//!   class (see [`x11_app_id`]), `match_title` its `_NET_WM_NAME` (falling
//!   back to `WM_NAME`), exactly like an xdg window's app id and title.
//! - **A floating X window keeps the position it asked for** when it asked
//!   for one -- `WM_NORMAL_HINTS` carries `USPosition` or `PPosition` -- and
//!   the whole window fits inside one output's usable area there. Otherwise
//!   it is centred, on its parent when it has one on screen, exactly like an
//!   xdg dialog. X clients that position their own dialogs compute the
//!   position from their parent's root coordinates, which scoot keeps true
//!   by configuring every window at its placement; a hint pointing off
//!   screen (or under a bar) is the case centring exists for. After the map
//!   an X window's position requests are ignored, like an xdg window's.
//! - **Size.** A floating X window is asked for the size it mapped at (X
//!   has no "choose your own" -- the window *is* a size), or a rule's
//!   `size`; a later `ConfigureRequest` for a new size is honoured as a
//!   floating resize, clamped the way the core clamps any. A tiled or
//!   fullscreen window's requests are refused: Smithay answers every one
//!   with a synthetic `ConfigureNotify` naming the geometry the layout gave.
//!   A window that has not mapped yet is granted what it asks (clamped to
//!   the X wire limits): it is not laid out, and clients size themselves
//!   before mapping.
//! - **Fullscreen** is `_NET_WM_STATE_FULLSCREEN` both ways: the client's
//!   request (and a state set before mapping) reaches the core as its own
//!   request, the same event an `xdg_toplevel.set_fullscreen` is, and the
//!   property follows the core's answer on every `apply()`.
//! - **No X "tiled" state.** X has no equivalent of xdg's `tiled_*` states;
//!   `_NET_WM_STATE_MAXIMIZED_*` is the nearest, and setting it would lie to
//!   clients that change their chrome for maximized windows (a restore
//!   button, a dropped shadow). Nothing is set.
//! - **Decorations.** An X window gets scoot's focus ring and rounded clip
//!   like any window: both key off the window's place in the layout and its
//!   drawn geometry (`drawn.rs`), not its protocol. Motif hints
//!   (`_MOTIF_WM_HINTS`) are read by Smithay and ignored here: scoot draws
//!   no titlebar for a client to opt out of, and a client drawing its own
//!   is in the same position as an xdg client drawing client-side
//!   decorations.
//! - **Minimums.** A tiled X window's frames never teach the core a
//!   minimum: the X server resizes the window synchronously, so a frame at
//!   the old size is an in-flight commit, not a refusal, and learning from
//!   it would pin the column wide (the race `shell.rs`'s `observe_frame`
//!   documents for xdg). `WM_NORMAL_HINTS` carries the minimum instead. A
//!   floating X window's frames are reported, so the core places it at the
//!   size it drew.
//! - **Scale.** X clients draw at scale 1; at a fractional `[output]
//!   scale` they are upscaled like any scale-unaware client.
//!
//! What is not honoured yet: `_NET_WM_MOVERESIZE` (a client-side titlebar
//! drag -- the modifier drag works), `_NET_WM_ICON`, and the X halves of the
//! clipboard, drag-and-drop and XIM (Phase 4).

use scoot_core::{Action, Edges, Event, Rect, Size, SizeHints, WindowId, WindowInfo};
use smithay::desktop::Window;
use smithay::utils::{Logical, Rectangle};
use smithay::xwayland::X11Surface;
use smithay::xwayland::xwm::{WmWindowType, X11Window};

use super::super::State;
use super::super::shell::{clamp_hint, hint_limit};
use super::super::window_rules::{Decision, MapSignals};

/// The largest window dimension an X server accepts. The wire field is a
/// `CARD16`, but servers refuse anything past `SHRT_MAX` (`BadValue`).
const X_MAX_SIZE: i32 = i16::MAX as i32;

/// The app id an X window is known by: its `WM_CLASS` *class* (the second
/// string -- `XTerm` for `xterm`), falling back to the instance (the first)
/// when a client sets only that. The class is what `.desktop` files'
/// `StartupWMClass` names, so it is the id a taskbar can find an icon by,
/// and the one a `[[window_rule]]` `match_app_id` matches.
pub(in crate::compositor) fn x11_app_id(window: &X11Surface) -> String {
    let class = window.class();
    x11_text(if class.is_empty() {
        window.instance()
    } else {
        class
    })
}

/// An X window's title, cut like its app id (see [`x11_text`]).
fn x11_title(window: &X11Surface) -> String {
    x11_text(window.title())
}

/// A string an X client set, made safe to pass on: cut at its first NUL.
///
/// X properties are byte arrays, so `WM_NAME`, `_NET_WM_NAME` and `WM_CLASS`
/// may carry a NUL -- and every Wayland string argument is a C string:
/// wayland-scanner's generated senders build it with
/// `CString::new(..).unwrap()`, so a title with a NUL in it, sent to a
/// taskbar's foreign-toplevel handle, panics the compositor. (An xdg title
/// cannot contain one: it arrived as a C string.) Cutting at the first NUL
/// is what any C consumer of the property reads anyway. Allocation-free when
/// there is no NUL, which is always, bar a hostile client.
pub(in crate::compositor) fn x11_text(mut text: String) -> String {
    if let Some(nul) = text.find('\0') {
        text.truncate(nul);
    }
    text
}

/// A rectangle an X server will accept: position in `INT16`, size in
/// `1..=`[`X_MAX_SIZE`]. Every configure scoot sends goes through this, so
/// no layout rect -- a column scrolled far off screen, a client- or
/// config-derived size -- can wrap on the wire or draw a `BadValue`.
fn x_rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
    let coordinate = |value: i32| value.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
    Rectangle::new(
        (coordinate(x), coordinate(y)).into(),
        (w.clamp(1, X_MAX_SIZE), h.clamp(1, X_MAX_SIZE)).into(),
    )
}

/// A client-supplied dimension, as the core and the wire both need it.
fn x_dimension(value: u32) -> i32 {
    i32::try_from(value)
        .unwrap_or(X_MAX_SIZE)
        .clamp(1, X_MAX_SIZE)
}

/// The size an X window is right now: what it was last configured to, which
/// for an X window is what it is (the server resizes synchronously).
fn x11_size(window: &X11Surface) -> Size {
    let size = window.last_configure().size;
    Size::new(size.w.clamp(1, X_MAX_SIZE), size.h.clamp(1, X_MAX_SIZE))
}

/// Whether `inner` lies wholly inside `outer`. In `i64`, so a rect near the
/// `i32` edge cannot overflow its own far edge.
fn contains(outer: Rect, inner: Rect) -> bool {
    let far = |origin: i32, extent: i32| i64::from(origin) + i64::from(extent);
    outer.x <= inner.x
        && outer.y <= inner.y
        && far(inner.x, inner.w) <= far(outer.x, outer.w)
        && far(inner.y, inner.h) <= far(outer.y, outer.h)
}

impl State {
    /// The core id of a managed X window, if it is one.
    ///
    /// By X window id and window manager, not `X11Surface`'s `==`: that
    /// compares liveness too, and Smithay marks a surface dead *before*
    /// `destroyed_window` runs, so an equality lookup would never find the
    /// window it is being told to forget. One walk of the window map and no
    /// lock (both ids are plain fields).
    pub(in crate::compositor) fn id_of_x11(&self, window: &X11Surface) -> Option<WindowId> {
        let (xwm, xid) = (window.xwm_id(), window.window_id());
        self.windows
            .iter()
            .find(|(_, candidate)| {
                candidate
                    .x11_surface()
                    .is_some_and(|x11| x11.window_id() == xid && x11.xwm_id() == xwm)
            })
            .map(|(&id, _)| id)
    }

    /// The core id of the managed window with X id `xid` -- what a
    /// `WM_TRANSIENT_FOR` names. There is one window manager per session, so
    /// the X id alone is unambiguous.
    fn id_of_x11_window(&self, xid: X11Window) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, candidate)| {
                candidate
                    .x11_surface()
                    .is_some_and(|x11| x11.window_id() == xid)
            })
            .map(|(&id, _)| id)
    }

    /// A normal X window asked to be mapped: grant it, and put it in the
    /// core. See the module doc for the policy, and `focus.rs` for whether
    /// it takes focus.
    pub(super) fn map_x11_window(&mut self, window: X11Surface) {
        if window.is_override_redirect() {
            // Unreachable through Smithay (an override-redirect window maps
            // itself, with no request), and if it ever were reachable the
            // unmanaged path is the right one: the XWM cannot configure it.
            self.map_x11_unmanaged(window);
            return;
        }
        if self.id_of_x11(&window).is_some() {
            // A repeated request for a window already managed: re-grant, and
            // leave its place in the layout alone.
            if let Err(error) = window.set_mapped(true) {
                tracing::debug!(id = window.window_id(), %error, "could not re-map an X11 window");
            }
            return;
        }
        if let Err(error) = window.set_mapped(true) {
            // The connection is gone or the window already died: nothing to
            // manage. The death paths clean up whatever the server left.
            tracing::debug!(id = window.window_id(), %error, "could not map an X11 window");
            return;
        }
        let focus = self.x11_focus_on_map(&window);
        self.next_id += 1;
        let id = WindowId(self.next_id);
        self.windows
            .insert(id, Window::new_x11_window(window.clone()));
        self.open_window(id, focus);
        let decision = self.x11_map_decision(&window);
        if decision.float {
            let size = decision.size.unwrap_or_else(|| x11_size(&window));
            self.world.handle_event(Event::FloatingRequested {
                id,
                floating: true,
                size: Some(size),
            });
            // The client's own placement, applied the way a pointer drag
            // would place it -- directly on the core, not through `act`: it
            // is the window's own doing at its own map, like the
            // `FloatingRequested` above, and is honoured while locked for the
            // same reason (nothing behind the lock is drawn, and the session
            // is as the client left it at unlock). `MoveFloating` spawns and
            // closes nothing, so there are no effects to run.
            if let Some((x, y)) = self.x11_requested_position(&window, size) {
                let _ = self.world.handle_action(Action::MoveFloating { id, x, y });
            }
        }
        if window.is_fullscreen() {
            // `_NET_WM_STATE_FULLSCREEN` set before mapping, which EWMH says
            // the window manager must respect: the client's own request.
            self.world.handle_event(Event::FullscreenRequested {
                id,
                fullscreen: true,
            });
        }
        tracing::debug!(
            ?id,
            xid = window.window_id(),
            focus,
            float = decision.float,
            reason = ?decision.reason,
            "X11 window mapped"
        );
        self.apply();
    }

    /// What `window_rules.rs` answers for an X window, from what it has set
    /// by its map request (the module doc lists the signals).
    fn x11_map_decision(&self, window: &X11Surface) -> Decision {
        let app_id = x11_app_id(window);
        let title = x11_title(window);
        let dialog =
            !matches!(window.window_type(), None | Some(WmWindowType::Normal)) || window.is_modal();
        let fixed_size = matches!(
            (window.min_size(), window.max_size()),
            (Some(min), Some(max)) if min.w > 0 && min.h > 0 && min == max
        );
        self.floating_rules.decide(&MapSignals {
            app_id: &app_id,
            title: &title,
            dialog,
            parent: window.is_transient_for().is_some(),
            fixed_size,
        })
    }

    /// Where a floating X window asked to be, if it asked (`USPosition` or
    /// `PPosition`) and a window of `size` there fits inside one output's
    /// usable area. The position itself is the window's own geometry -- the
    /// `WM_NORMAL_HINTS` position fields are obsolete (ICCCM 4.1.2.3), only
    /// the flags still mean anything.
    fn x11_requested_position(&self, window: &X11Surface, size: Size) -> Option<(i32, i32)> {
        window.size_hints()?.position?;
        let at = window.last_configure().loc;
        let rect = Rect::new(at.x, at.y, size.w, size.h);
        self.world
            .usable_areas()
            .into_iter()
            .any(|area| contains(area, rect))
            .then_some((at.x, at.y))
    }

    /// An X window's `WindowInfo`: `shell.rs`'s `info_of` for the X half.
    /// The hints go through the same clamp an xdg window's do, since
    /// `WM_NORMAL_HINTS` is as unvalidated as `set_min_size`.
    pub(in crate::compositor) fn x11_info(&self, window: &X11Surface) -> WindowInfo {
        let limit = hint_limit(
            self.world.usable_areas().into_iter(),
            self.world.config().gap,
        );
        let min = window
            .min_size()
            .map(|size| Size::new(size.w, size.h))
            .unwrap_or_default();
        let max = window
            .max_size()
            .map(|size| Size::new(size.w.max(0), size.h.max(0)))
            .unwrap_or_default();
        WindowInfo {
            app_id: x11_app_id(window),
            title: x11_title(window),
            hints: SizeHints {
                min: clamp_hint(min, limit),
                max,
            },
            // A window naming itself is no parent (the core would take it
            // as its own ancestor).
            parent: window
                .is_transient_for()
                .filter(|&xid| xid != window.window_id())
                .and_then(|xid| self.id_of_x11_window(xid)),
        }
    }

    /// An X window unmapped or was destroyed: out of the core if it was
    /// managed, off the unmanaged list if it was not. Idempotent -- the
    /// destroy that follows an unmap finds nothing left to do.
    pub(super) fn forget_x11_window(&mut self, window: &X11Surface, why: &str) {
        if let Some(id) = self.id_of_x11(window) {
            tracing::debug!(
                ?id,
                xid = window.window_id(),
                why,
                "X11 window left the layout"
            );
            self.remove_window(id);
            return;
        }
        self.unmap_x11_unmanaged(window);
    }

    /// A `ConfigureRequest` (see the module doc's size policy).
    pub(super) fn x11_configure_request(
        &mut self,
        window: &X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
    ) {
        if let Some(id) = self.id_of_x11(window) {
            if (w.is_some() || h.is_some())
                && self.world.is_floating(id)
                && !self.world.is_fullscreen(id)
            {
                let current = x11_size(window);
                let size = Size::new(
                    w.map_or(current.w, x_dimension),
                    h.map_or(current.h, x_dimension),
                );
                // Directly on the core, like the map-time placement: the
                // window's own doing. Clamped there to its hints and its
                // output's usable area; `apply()` configures the result, and
                // Smithay's synthetic notify after this reports it.
                let _ = self.world.handle_action(Action::ResizeFloating {
                    id,
                    size,
                    edges: Edges::BOTTOM_RIGHT,
                });
                self.apply();
            }
            return;
        }
        if window.is_override_redirect() {
            return;
        }
        // Not mapped yet: grant it (clamped).
        let current = window.last_configure();
        let rect = x_rect(
            x.unwrap_or(current.loc.x),
            y.unwrap_or(current.loc.y),
            w.map_or(current.size.w, x_dimension),
            h.map_or(current.size.h, x_dimension),
        );
        if let Err(error) = window.configure(rect) {
            tracing::debug!(id = window.window_id(), %error, "could not configure an unmapped X11 window");
        }
    }

    /// `_NET_WM_STATE_FULLSCREEN` added or removed by the client: its own
    /// request, like `xdg_toplevel.set_fullscreen` (see `fullscreen.rs` for
    /// why that is honoured while locked). Only a change re-applies; the
    /// property follows the core's answer from there.
    pub(super) fn x11_fullscreen_request(&mut self, window: &X11Surface, fullscreen: bool) {
        let Some(id) = self.id_of_x11(window) else {
            return;
        };
        let before = self.world.is_fullscreen(id);
        self.world
            .handle_event(Event::FullscreenRequested { id, fullscreen });
        if self.world.is_fullscreen(id) != before {
            self.apply();
        } else if let Err(error) = window.set_fullscreen(before) {
            // Refused or already so: the property states the answer.
            tracing::debug!(?id, %error, "could not restate an X11 window's fullscreen state");
        }
    }

    /// A title, class, size-hint or transient change: re-read, the way an
    /// xdg window's `title_changed` and friends do.
    pub(super) fn x11_properties_changed(&mut self, window: &X11Surface) {
        if let Some(id) = self.id_of_x11(window) {
            self.refresh_window(id);
        }
    }

    /// XWayland paired an X window with its `wl_surface`. A focused window
    /// could not take the keyboard until now (it had no surface -- see
    /// `State::window_keyboard_focus`), so the focus is re-derived; on the
    /// loop's next idle rather than here, because this can run inside the
    /// surface's own pre-commit hook.
    pub(super) fn x11_surface_associated(&mut self, window: &X11Surface) {
        if !window.is_override_redirect()
            && let Some(id) = self.id_of_x11(window)
            && self.focus == Some(id)
        {
            self.loop_handle
                .insert_idle(|state| state.refresh_keyboard_focus());
        }
        self.request_render();
    }

    /// The XWM lost its server: every X window goes, managed and unmanaged.
    /// A death event, once per server -- the id list is the one allocation.
    pub(super) fn sweep_x11_windows(&mut self) {
        let managed: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, window)| window.x11_surface().is_some())
            .map(|(&id, _)| id)
            .collect();
        for id in managed {
            self.remove_window(id);
        }
        self.clear_x11_unmanaged();
    }

    /// A frame an X window committed: reported only for a floating one (see
    /// the module doc on minimums), as a window that chooses its own size --
    /// the core places it at the size it drew.
    pub(in crate::compositor) fn observe_x11_frame(&mut self, id: WindowId) {
        let Some(floating) = self.world.floating_size(id) else {
            return;
        };
        let Some(size) = self.window(id).map(|window| window.geometry().size) else {
            return;
        };
        self.world.handle_event(Event::FrameObserved {
            id,
            requested: Size::default(),
            actual: Size::new(size.w, size.h),
        });
        if self.world.floating_size(id) != Some(floating) {
            self.apply();
        }
    }

    /// Gives the core a size for every X window floating with none: one the
    /// user just floated with the toggle, which the core asks to choose its
    /// own size and places invisible until it draws. An X window has a size
    /// already and may not commit again for a long time (an idle terminal),
    /// so its current size is reported as its first frame. Runs at the top
    /// of `apply()`, before the arrangement is read; a map lookup per X
    /// window, and a core event only in that rare case.
    pub(in crate::compositor) fn size_undrawn_x11_floats(&mut self) {
        for (&id, window) in &self.windows {
            let Some(x11) = window.x11_surface() else {
                continue;
            };
            if let Some((drawn, None)) = self.world.floating_size(id)
                && drawn == Size::default()
            {
                let size = x11_size(x11);
                self.world.handle_event(Event::FrameObserved {
                    id,
                    requested: Size::default(),
                    actual: size,
                });
            }
        }
    }
}

/// `apply()`'s X half for one placement: the fullscreen property always,
/// and -- for a visible placement -- the configure, only when it differs
/// from the one the window last had (so an `apply()` that moved nothing
/// costs no X traffic).
pub(in crate::compositor) fn configure_x11(window: &X11Surface, placement: &scoot_core::Placement) {
    if let Err(error) = window.set_fullscreen(placement.fullscreen) {
        tracing::debug!(id = window.window_id(), %error, "could not set an X11 window's fullscreen state");
    }
    if !placement.visible {
        return;
    }
    let size = placement
        .requested
        .unwrap_or(Size::new(placement.rect.w, placement.rect.h));
    let rect = x_rect(placement.rect.x, placement.rect.y, size.w, size.h);
    if window.last_configure() == rect {
        return;
    }
    if let Err(error) = window.configure(rect) {
        tracing::debug!(id = window.window_id(), %error, "could not configure an X11 window");
    }
}
