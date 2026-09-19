//! The headless backend: draw into memory on the CPU, with no display at all.
//!
//! This is what agents and tests drive. Nothing is shown anywhere; the way to
//! see the screen is a screenshot over IPC.

use std::error::Error;
use std::time::{Duration, Instant};

use pixman::Image;
use scoot_core::{Event as CoreEvent, OutputId, Rect};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{AsRenderElements, render_elements};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::desktop::{LayerMap, layer_map_for_output};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Transform};
use smithay::wayland::shell::wlr_layer::Layer;

use super::State;
use super::cursor::CursorElement;
use super::layer_shell;
use super::output_scale::smithay_scale;
use super::session_lock::LOCK_VBLANK_TIMEOUT;
use super::tty::Tty;

// What kinds of thing `render()` can draw. Which one covers which is decided
// by the *order* they go into the list (see `render()`'s comment on Smithay's
// back-to-front convention), not by this enum. The background isn't a variant
// here at all -- it's the `render_output` call's `clear_color`, always the
// bottom-most thing on screen by construction; see `decorations.rs`'s module
// doc for why that's simpler and safer than a full-output element.
//
// Windows and layer-shell surfaces (bars, wallpapers) share the one `Surface`
// variant because they produce the same element type: a variant each would
// need two `From<WaylandSurfaceRenderElement<PixmanRenderer>>` impls on this
// enum, which cannot coexist. Nothing is lost by that -- what puts a bar in
// front of a window and a wallpaper behind one is where `render()` inserts
// it, and a variant could not have expressed that anyway.
//
// Fixed to `PixmanRenderer` (this backend's only renderer) rather than
// generic over `R`, which is why this lives here and not in
// `decorations.rs`/`cursor.rs` -- those modules stay renderer-agnostic,
// returning plain `SolidColorRenderElement`s and generic `CursorElement<R>`s
// this enum only wraps.
render_elements! {
    Elements<=PixmanRenderer>;
    Cursor = CursorElement<PixmanRenderer>,
    Surface = WaylandSurfaceRenderElement<PixmanRenderer>,
    Decoration = SolidColorRenderElement,
}

/// The core's id for the one output this compositor creates.
///
/// Every site that reports output geometry to [`scoot_core`] uses it, so the
/// "there is exactly one output" assumption lives in one named place rather
/// than as a bare `OutputId(1)` repeated at each of them. Multi-output
/// support replaces this with a real per-output id.
///
/// It is not the only thing multi-output has to touch, though, so this is not
/// a "change the constant and you're done" marker. `layer_shell.rs`'s
/// `new_layer_surface` already honours a client's requested `wl_output`
/// (`layer_map_for_output(requested.or(self.output))`), while six sites
/// unconditionally reach for `self.output` instead: `layer_destroyed`,
/// `commit_layer_surface`, `refresh_layer_zone`, `layer_hit`,
/// `layer_keyboard_focus` and `render()`'s frame-callback/cleanup pass.
/// Unreachable today -- exactly one `Output` exists, so "the one the client
/// asked for" and "the one we have" are the same object -- but every one of
/// them has to become per-output once there is more than one, and not all in
/// the same way: `layer_destroyed` and `commit_layer_surface` want the
/// surface's *own* output, `layer_hit` the one under the pointer,
/// `refresh_layer_zone` a zone per output rather than one, and
/// `layer_keyboard_focus` and `render()`'s pass have to walk more than one
/// map.
///
/// The same holds for `session_lock.rs`, whose four per-output sites are the
/// subject of `docs/backlog/resolved/session-lock-per-output-done.md` (resolved
/// as single-output pins, not as multi-output): `new_surface` honours the
/// client's named `wl_output` but falls back to the single output, and
/// refuses a second live surface for an already-covered output with the
/// protocol's `duplicate_output` error (see
/// `docs/backlog/resolved/session-lock-duplicate-output-done.md`);
/// `configure_all` sizes every surface to that one output together;
/// confirmation treats the first blanked frame as "presented on all outputs",
/// which is true if and only if there is one of them; and the locked render
/// path composites every current surface onto that output at its origin with
/// the keyboard on the first of them. Per-output, the fallback goes away,
/// each output gets its own size, `locked` waits for every output's blanked
/// frame, and the focus rule needs one surface per output rather than the
/// first of all of them.
pub(super) const OUTPUT_ID: OutputId = OutputId(1);

/// How often a changed screen is redrawn, while there's something to redraw.
///
/// Visible to the rest of the compositor because it is also the natural unit
/// for "how much work may one client ask for": `ipc.rs` spaces a connection's
/// screenshots by it, so a capture costs at most what a frame already does.
/// Not a claim about the pixels -- `render()` runs on demand from
/// `needs_render`, not on frame boundaries, so two captures a frame apart can
/// legitimately differ.
pub(super) const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// The CPU renderer and the image it draws into.
pub struct Backend {
    pub renderer: PixmanRenderer,
    pub image: Image<'static, 'static>,
    pub damage: OutputDamageTracker,
    pub size: (i32, i32),
}

/// The output's name -- and model -- when no backend has a better one:
/// `--headless` and `--nested`, and every test. `--tty` passes its
/// connector's name to [`init_named`] instead.
pub const OUTPUT_NAME: &str = "headless";

/// [`init_named`] under the default name -- what every test harness wants,
/// and the same output every test has always had. `compositor::run` goes
/// through `init_named` for all three backends (with [`OUTPUT_NAME`] for the
/// two connector-less ones), so this is test-only.
#[cfg(test)]
pub fn init(state: &mut State, width: i32, height: i32) -> Result<(), Box<dyn Error>> {
    init_named(state, OUTPUT_NAME, width, height)
}

/// Creates the one output and the CPU render target behind it. `name` is
/// what clients see as `wl_output.name` (and `model`): a connector name
/// such as `HDMI-A-1` or `Virtual-1` under `--tty`, so a bar or shell
/// labels the screen the way it would under any other compositor, or
/// [`OUTPUT_NAME`] where there is no connector.
pub fn init_named(
    state: &mut State,
    name: &str,
    width: i32,
    height: i32,
) -> Result<(), Box<dyn Error>> {
    let output = Output::new(
        name.to_owned(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "scoot".into(),
            model: name.to_owned(),
            serial_number: "0".into(),
        },
    );
    output.create_global::<State>(&state.display_handle);
    set_mode(
        &output,
        width,
        height,
        Some((0, 0).into()),
        state.output_scale,
    );
    state.space.map_output(&output, (0, 0));

    state.backend = Some(create_backend(&output, width, height)?);
    // What the core is told is the *logical* output rectangle, which is the
    // same rectangle Smithay's `Space` lays windows out in -- see
    // `output_scale.rs`'s `logical_size`. Handing the core the physical
    // framebuffer size instead (what this did before output scaling existed)
    // makes every window, gap and edge land at physical coordinates the
    // render path then scales a second time.
    let area = state
        .space
        .output_geometry(&output)
        .map(|geometry| {
            Rect::new(
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            )
        })
        .unwrap_or_else(|| Rect::new(0, 0, width, height));
    state.output = Some(output);
    // The cursor's startup position: centred, not at the origin Smithay
    // leaves it at. Here rather than per-backend, so all three backends
    // place it at the same init point -- the cursor draws only under `--tty`
    // today, but the position is backend-independent seat state. See
    // `place_pointer_at_output_centre` for the quiet-path and no-replace
    // reasoning.
    state.place_pointer_at_output_centre();
    // The head a display-configuration client sees. After `state.output` is
    // set, because that is where the advertised state is read from -- and
    // before `apply()`, so a client that bound the manager during `State::new`
    // (before there was an output) hears about the head in the same startup
    // pass everything else is announced in. See `output_management.rs`.
    state.refresh_output_heads();
    state.world.handle_event(CoreEvent::OutputAdded {
        id: OUTPUT_ID,
        area,
    });
    // apply() ends in request_render(), which arms the frame timer via
    // ensure_ticking() -- this is what puts the very first frame on it.
    state.apply();

    Ok(())
}

/// Updates an already-created output's mode and scale. `location` is only
/// meaningful the first time (see `init`); later callers
/// (`State::resize_output`) pass `None` to leave it where it is.
///
/// The third `change_current_state` argument is the scale slot the pre-output-
/// scaling code passed `None` for, which left every output at Smithay's
/// default `Scale::Integer(1)`. See `output_scale.rs` for why exactly 1.0 is
/// still spelled `Scale::Integer(1)`.
fn set_mode(
    output: &Output,
    width: i32,
    height: i32,
    location: Option<smithay::utils::Point<i32, smithay::utils::Logical>>,
    scale: f64,
) {
    let mode = Mode {
        size: (width, height).into(),
        refresh: 60_000,
    };
    // `set_preferred` first: `change_current_state` sends the new mode to
    // already-bound `wl_output` clients synchronously, with the `preferred`
    // bit exactly when the mode it is told about already is the preferred
    // one (see `wl_change_current_state` at the pinned Smithay rev -- no
    // batching, call order is what the wire sees). The other order told a
    // client bound before a resize about the new mode with no `preferred`
    // bit, and nothing ever resent it; see
    // `docs/backlog/resolved/wl-output-preferred-flag-on-late-mode-done.md`.
    output.set_preferred(mode);
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        Some(smithay_scale(scale)),
        location,
    );
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
        // While the `--tty` session holds no DRM master (`Tty::active` is
        // `false` -- VT-switched away, or a reactivation whose `drm.activate`
        // itself failed), `present()` drops every frame, so rendering one and
        // waking clients with frame callbacks is pure waste: compositor CPU
        // plus clients painting frames nobody shows. Skip both.
        //
        // `needs_render` is cleared, not left set: nothing can be shown until
        // a successful `reactivate()`, and that path unconditionally asks for
        // a fresh render (`session_event`'s `ActivateSession` arm maps even
        // `Reconfigured::Nothing` to `Render`), so no repaint is lost. With
        // the flag cleared the frame timer drops itself instead of ticking
        // 60-odd times a second to find the session still paused, and the
        // compositor idles until a client commit or the switch back re-arms
        // it. Damage needs no special handling: neither the damage tracker
        // (no `render_output` call advances its history) nor the dumb-buffer
        // ages (no `advance_generation`) move while renders are skipped, and
        // `reactivate()` invalidates the ages, so the first post-reactivate
        // render is a full repaint.
        //
        // Gated on `active` (no DRM master), deliberately *not*
        // `session_paused`: the failed-reactivation state (session active
        // again, master not reacquired) drops frames in `present()` exactly
        // like a paused one, so rendering there is the same waste. See
        // `Tty::active`'s doc for why the two differ.
        //
        // What this defers, all harmlessly: a session that locks while paused
        // stays bare-`pending` with no deadline armed -- `await_vblank`,
        // the sole site arming `blank_deadline`, and the fallback timer
        // beside it both live in the render tail past this gate, so neither
        // runs -- and confirms on the reactivation render, via vblank or
        // the fallback, both of which are intact there. No wedge: the wait
        // simply waits. Note the timing delta on this security-sensitive
        // path: pre-PR the locker got `locked` via the fallback about a
        // second after requesting it, with no blanked frame on scanout;
        // now `locked` waits for the switch-back, when one actually reaches
        // scanout -- arguably more protocol-correct, but a behaviour change
        // worth naming, not implying away; popup grabs, IME composition and keyboard
        // focus are seat state, untouched by withholding frame callbacks, and
        // only their *pacing* pauses (the same shape as a minimized window in
        // most compositors: no protocol promises a callback by any deadline);
        // dead-layer/popup cleanup and `flush_clients` wait for the next live
        // render. An IPC screenshot taken while paused reads back the last
        // pre-pause framebuffer -- the last thing anyone saw.
        if tty_blocks_render(self.tty.as_ref().map(Tty::is_active)) {
            self.needs_render = false;
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
        // Whether this frame actually reached the renderer. Only a frame that
        // did may confirm a pending session lock: the protocol forbids
        // sending `locked` before a blanked frame exists (see
        // `session_lock.rs`).
        let mut drew_a_frame = false;
        // The flip this frame went out on under `--tty`, if `present`
        // issued one. Only meaningful for a locked frame with a pending
        // lock (see the `confirm_lock`/`await_vblank` split at the end of
        // this function); every other frame leaves it for the tail to
        // ignore.
        let mut blank_seq: Option<u64> = None;
        // Whether this frame's `present` was refused after the pixels were
        // already written (a commit/page-flip the kernel rejected -- the
        // only present skip no completion event can retry, since nothing is
        // in flight). The tail re-arms the frame timer for it below, after
        // `needs_render` is cleared so the request isn't clobbered.
        let mut retry_render = false;
        // Whether this frame reached the host under `--nested`, if `present`
        // committed it. Like `blank_seq` above, this is what tells the tail
        // the frame actually went out rather than merely rendered: only a
        // presented frame may stamp presentation feedback (see
        // `presentation_time.rs`).
        let mut host_committed = false;
        // Read once, here, so every branch below -- elements, clear colour,
        // frame callbacks -- is answering the same question about the same
        // frame.
        let locked = self.session_lock.is_locked();
        {
            let Backend {
                renderer,
                image,
                damage,
                size,
            } = &mut backend;
            // The core's arrangement, and this frame's ring segments built
            // from it -- computed once here rather than inside the match
            // below so a failure to bind the framebuffer still logs without
            // having done this for nothing. See `decorations.rs`'s module
            // doc for why the background isn't part of this list.
            //
            // Not computed at all while locked: no window and no ring is
            // drawn then, so laying the windows out would be work for a frame
            // that cannot show it. `apply()` still runs the layout on every
            // change underneath, so nothing is lost by the time it unlocks.
            let (width, height) = *size;
            // What every element's own coordinates are built at, read from the
            // output rather than hardcoded so windows, layer surfaces, the
            // ring and the cursor can never disagree about it. 1.0 unless
            // `[output] scale` says otherwise (see `output_scale.rs`).
            let scale = output.current_scale().fractional_scale();
            // The output rectangle in *logical* coordinates -- the space the
            // core arranges in and the space every element is built in. The
            // decorations' clip bounds and the lock backdrop's physical origin
            // both come from here, which is the whole coordinate-space split
            // in one place: `bounds` is logical, `(width, height)` below is the
            // physical render target.
            let geometry = self.space.output_geometry(&output);
            let bounds = geometry
                .map(|geometry| {
                    Rect::new(
                        geometry.loc.x,
                        geometry.loc.y,
                        geometry.size.w,
                        geometry.size.h,
                    )
                })
                .unwrap_or_else(|| Rect::new(0, 0, width, height));
            let ring_elements = if locked {
                Vec::new()
            } else {
                let arrangement = self.world.arrange();
                self.decorations
                    .elements(&arrangement, &self.appearance, bounds, scale)
            };
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
                                self.cursor
                                    .element(renderer, pointer.current_location(), scale)
                            }
                            None => Vec::new(),
                        }
                    } else {
                        Vec::new()
                    };
                    // Smithay's damage tracker draws a `&[E]` back-to-front by
                    // walking it in reverse (confirmed in
                    // `OutputDamageTracker::render_output_internal`, which
                    // iterates `render_elements.iter().rev()`), so the *first*
                    // entry here ends up drawn *last*, i.e. on top. Read this
                    // list as "front to back":
                    //
                    // 1. the cursor -- meaningless hidden behind anything;
                    // 2. the overlay and top layer-shell layers, which the
                    //    protocol defines as being above ordinary windows (a
                    //    bar, a launcher, a notification);
                    // 3. windows, which must still win over the ring:
                    //    `shell.rs::apply()` positions a window from the
                    //    layout's rect but sizes it from whatever the client
                    //    actually committed, and a client that's slow to
                    //    shrink (or a `--nested` resize still in flight) can
                    //    briefly have a surface larger than its placement
                    //    rect, reaching into the gap the ring is drawn in.
                    //    Ring-on-top would paint over that live content every
                    //    such frame; windows-on-top instead means the
                    //    stale/oversized content can only ever cover the
                    //    ring, never the reverse -- the same direction niri
                    //    itself picks, and the only one of the two that can't
                    //    corrupt what a client is showing;
                    // 4. the focus ring, drawn in the layout's own gap;
                    // 5. the bottom and background layers (a wallpaper),
                    //    which the protocol defines as being below windows.
                    //
                    // The ring sits *between* windows and the background
                    // layer rather than below both, which is the one thing
                    // `space::space_render_elements` -- which gathers layer
                    // surfaces itself, in one fixed order -- cannot express:
                    // it would put a full-screen wallpaper on top of the
                    // ring, i.e. hide the ring completely for anyone running
                    // `swaybg`. So windows come from
                    // `Space::render_elements_for_region` (windows only, by
                    // construction -- see its own doc) and the layers are
                    // gathered here, around the ring.
                    //
                    // ...unless the session is locked, in which case this
                    // whole list is replaced -- not reordered -- by the lock
                    // screen's own (see `session_lock.rs`). Everything above
                    // is skipped outright rather than pushed behind an opaque
                    // backdrop, because "drawn behind something opaque" is a
                    // weaker guarantee than "never gathered": it would rest on
                    // element ordering, on no client surface ever being larger
                    // than the rect it was placed at, and on the damage
                    // tracker never surprising us. The cursor is the one thing
                    // still drawn in front, and it is this compositor's own
                    // shape (`SessionLockHandler::lock` resets it at lock
                    // time) -- a lock screen with a password field needs a
                    // pointer.
                    let elements = if locked {
                        let origin = geometry
                            .map(|geometry| geometry.loc.to_physical_precise_round(scale))
                            .unwrap_or_default();
                        let (lock_surfaces, backdrop) =
                            self.lock_elements(renderer, origin, scale, (width, height));
                        let mut elements =
                            Vec::with_capacity(cursor_elements.len() + lock_surfaces.len() + 1);
                        elements.extend(cursor_elements.into_iter().map(Elements::Cursor));
                        elements.extend(lock_surfaces.into_iter().map(Elements::Surface));
                        elements.push(Elements::Decoration(backdrop));
                        elements
                    } else {
                        let window_elements = match geometry {
                            Some(region) => self
                                .space
                                .render_elements_for_region(renderer, &region, scale, 1.0),
                            // Unreachable while `self.output` is the output
                            // `headless::init` mapped into the space; an output
                            // that isn't in the space has no region to render.
                            None => Vec::new(),
                        };
                        let layers = layer_map_for_output(&output);
                        let mut elements = Vec::with_capacity(
                            cursor_elements.len()
                                + window_elements.len()
                                + ring_elements.len()
                                + layers.len(),
                        );
                        elements.extend(cursor_elements.into_iter().map(Elements::Cursor));
                        layer_elements(
                            &layers,
                            &layer_shell::ABOVE_WINDOWS,
                            renderer,
                            scale,
                            &mut elements,
                        );
                        elements.extend(window_elements.into_iter().map(Elements::Surface));
                        elements.extend(ring_elements.into_iter().map(Elements::Decoration));
                        layer_elements(
                            &layers,
                            &layer_shell::BELOW_WINDOWS,
                            renderer,
                            scale,
                            &mut elements,
                        );
                        // Nothing below this point needs the layer map, and the
                        // frame-callback pass at the end of `render()` takes the
                        // same per-output lock again -- holding this one across
                        // the render would deadlock the compositor against
                        // itself.
                        drop(layers);
                        elements
                    };

                    // `0` (always-full-redraw) for every presenter except
                    // `--tty`: see `buffers.rs`'s module doc on why that one
                    // specifically needs a real buffer age -- cursor motion
                    // can trigger a render on every mouse-motion event, and a
                    // naive full-frame copy at that rate is exactly the
                    // multi-MB/s memcpy the roadmap calls out. Neither
                    // `--headless` nor `--nested` draws a cursor, so neither
                    // has a new reason to render more often than before.
                    let age = self.tty.as_ref().map_or(0, Tty::next_buffer_age);
                    // The backdrop element already covers the output opaquely
                    // while locked; this is the second line of defence behind
                    // it, so that even a frame whose elements somehow produced
                    // nothing clears to the lock colour rather than to the
                    // configured desktop background -- which a user may have
                    // given an alpha, and which is the colour the unlocked
                    // session is showing.
                    let clear_color = if locked {
                        self.lock_clear_color()
                    } else {
                        self.appearance.background_color.into()
                    };
                    let result = damage.render_output(
                        renderer,
                        &mut framebuffer,
                        age,
                        &elements,
                        clear_color,
                    );
                    match result {
                        Ok(render_result) => {
                            drew_a_frame = true;
                            // What `screencopy.rs` asks "have the pixels moved
                            // since this session's last capture?" with. Gated
                            // on the damage tracker having something to report
                            // rather than on reaching this arm at all: under
                            // `--tty` (the one backend passing a real buffer
                            // age) a redundant `request_render` legitimately
                            // draws nothing and leaves the previous frame on
                            // screen, and counting that as a change would make
                            // every capture session copy the same pixels
                            // again. See `State::frame_serial`'s doc for what
                            // this does and does not claim.
                            if render_result.damage.is_some() {
                                self.frame_serial = self.frame_serial.wrapping_add(1);
                            }
                            // Must run unconditionally, even when there
                            // turns out to be nothing to present below --
                            // see `BufferPool::advance_generation`'s doc for
                            // why this can't be skipped just because this
                            // frame is.
                            if let Some(tty) = &mut self.tty {
                                tty.advance_generation();
                            }
                            // Both presenters read back the same frame the
                            // same way; only what happens with the pixels
                            // afterward differs, so the read-back itself
                            // happens once for whichever (or both) are set --
                            // and not at all with neither (plain
                            // `--headless`, e.g. under IPC-only control):
                            // that copy would be pure waste on every render
                            // with nothing to hand it to.
                            if (self.host.is_some() || self.tty.is_some())
                                && let Some(damaged) = render_result.damage
                            {
                                // Bounding box of every damaged rect, not
                                // the rects themselves: `copy_framebuffer`
                                // (and the dumb-buffer write behind it) only
                                // ever copies one contiguous region. For
                                // `--headless`/`--nested` (`age` always `0`
                                // above) this is always the full frame
                                // regardless, so nothing changes for them;
                                // for `--tty` a small, cheap bbox is the
                                // common case (see `buffers.rs`'s module
                                // doc), and only a large pointer jump or a
                                // real content change grows it.
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
                                                host_committed = host.present(
                                                    pixels,
                                                    region.size.w,
                                                    region.size.h,
                                                );
                                            }
                                            if let Some(tty) = &mut self.tty {
                                                blank_seq =
                                                    tty.present(pixels, region, (width, height));
                                                retry_render = tty.take_retry_render();
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
                            // else: no presenter is watching this frame,
                            // or (only possible when `age > 0`, i.e. only
                            // under `--tty`) nothing actually changed -- e.g.
                            // a redundant `request_render` with no real
                            // difference -- so there's nothing to read back
                            // or present either way.
                        }
                        Err(error) => tracing::warn!(%error, "could not render"),
                    }
                }
                Err(error) => tracing::warn!(%error, "could not bind the framebuffer"),
            }
        }
        self.backend = Some(backend);
        self.needs_render = false;
        // A refused `--tty` flip (see `retry_render` above): nothing is in
        // flight, so no `VBlank` will ever arrive to retry it the way
        // `present_skipped` retries an in-flight skip -- re-arm the frame
        // timer directly so the re-presented frame goes out on the next
        // tick instead of waiting for unrelated damage. After the flag
        // clear above, so this request survives it; bounded at the source
        // (`present_retry.rs`), so a device that keeps refusing goes quiet
        // instead of pinning the loop.
        if retry_render {
            self.request_render();
        }

        // The frame a pending session lock has been waiting for. Only after
        // one has actually been drawn -- never after a failed bind or a
        // failed render, which would leave whatever was on screen before the
        // lock exactly where it was -- may the client be told `locked`. A
        // lock that could not be confirmed stays pending and stays *locked*;
        // the alternative, giving up and unlocking, would turn a renderer
        // failure into an unrequested unlock (see `session_lock.rs`).
        //
        // Where there is a scanout to wait for (`--tty`), confirmation
        // additionally waits for the vblank of the flip carrying the blanked
        // frame (see `SessionLock::await_vblank`): the previous, possibly
        // unlocked, frame can otherwise stay on scanout for up to one more
        // vblank after `locked` has gone out. Headless and nested have no
        // scanout, so the drawn frame *is* the shown one and confirms at
        // once, exactly as before.
        if drew_a_frame {
            if self.session_lock.awaiting_blank() && self.tty.is_some() {
                let now = Instant::now();
                if self.session_lock.await_vblank(blank_seq, now) {
                    // This frame needs a timer watching the fallback
                    // deadline: a newly armed wait has none yet, and a
                    // freshly issued flip restarted the bound out from
                    // under the previous one (see `await_vblank`). One
                    // shot each, dropped when they fire: the vblank path
                    // takes the wait first in the ordinary case, so a timer
                    // only ever fires for a vblank that never came -- and a
                    // stale one finds no wait and drops.
                    if let Err(error) = self
                        .loop_handle
                        .insert_source(Timer::from_duration(LOCK_VBLANK_TIMEOUT), blank_timeout)
                    {
                        // error!, not warn!: without this timer a lock whose
                        // vblank never arrives hangs its locker forever --
                        // the one failure mode this wait exists to prevent.
                        // The vblank path itself still works; only the
                        // fallback is gone.
                        tracing::error!(
                            %error,
                            "could not arm the session-lock vblank fallback timer; \
                             a lock whose vblank never arrives will hang its locker"
                        );
                    }
                }
            } else {
                self.confirm_lock();
            }
        }

        // Presentation feedback for the frame that just went out, before the
        // frame callbacks below (Smithay's ordering: drain feedback first,
        // then tell clients to draw next). Taken if and only if something
        // was actually shown: a `--tty` flip issued, a `--nested` commit
        // handed to the host, or -- with no presenter at all -- a frame
        // drawn into the framebuffer, which *is* the final image there. A
        // rendered-but-dropped frame (busy CRTC, no free host buffer, size
        // mismatch, failed bind or draw) leaves pending feedback queued for
        // the next presented frame rather than stamping a time nothing was
        // shown at; while locked only the lock surfaces are stamped (see
        // `presentation_time.rs` for the rule and the per-backend timestamp
        // semantics).
        //
        // `vsync` is the backend, not the flip: `--tty` page flips are
        // vblank-synchronized whenever one is issued, and this arm only runs
        // when one was.
        //
        // `seq` is per-backend per the protocol's `presented` contract (see
        // `presented_frame`): the issued-flip number on `--tty`, zero
        // everywhere else -- headless has no retrace to count and nested
        // output is self-refreshing with no queryable count.
        if let Some(seq) = super::presentation_time::presented_frame(
            self.host.is_some(),
            host_committed,
            self.tty.is_some(),
            blank_seq,
            drew_a_frame,
        ) {
            self.present_feedback(&output, self.tty.is_some(), cursor_surface.as_ref(), seq);
        }

        let time = self.start_time.elapsed();
        // A client cursor surface is never in `self.space`, so the window
        // loop below can't reach it -- and a well-behaved client with an
        // animated cursor (a spinner, a throbber) attaches one frame,
        // requests a callback, and waits for it before attaching the next.
        // Without this it waits forever and the animation freezes on its
        // first frame.
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
        //
        // Outside the lock branch below because the cursor is drawn either
        // way, and while locked that surface can only belong to the lock
        // client: `wl_pointer.set_cursor` is refused unless the asking client
        // holds pointer focus or a pointer grab (Smithay's
        // `allow_setting_cursor`), and locking resets the image, drops grabs
        // and moves pointer focus onto the lock surface.
        if let Some(surface) = &cursor_surface {
            send_frames_surface_tree(surface, &output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }
        if locked {
            // Lock surfaces are the only clients told to draw: "the
            // compositor must stop rendering and providing input to normal
            // clients". A client with no frame callback stops drawing by
            // itself, so this is both what the protocol asks for and what
            // keeps every window and bar in the session from burning CPU
            // behind a lock screen. They get one again on the first frame
            // after unlocking.
            if self.lock_post_frame(&output, time) {
                // A lock surface was dropped, and it may have been the one
                // holding the keyboard, the pointer or a grab -- and what is
                // drawn just changed. Pointer focus has to be re-derived
                // explicitly and not just hit-tested: `wl_pointer.button`
                // goes to whatever the pointer last entered, and a grab
                // outlives focus changes entirely (see `session_lock.rs`).
                self.lock_transition();
            }
        } else {
            for window in self.space.elements() {
                window.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                    Some(output.clone())
                });
            }
            // Layer surfaces aren't in `self.space` either, and a bar's clock
            // stops at whatever second it first drew without this -- the same
            // frame-callback starvation the cursor surface had. Sent to every
            // mapped layer surface rather than only the ones that produced an
            // element, matching both the window loop above and the cursor's own
            // reasoning: a client may legitimately ask for a callback before its
            // first attach, and withholding it would stall the very frame that
            // unsticks it.
            let dropped_dead_layers = {
                let mut layers = layer_map_for_output(&output);
                for layer in layers.layers() {
                    layer.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    });
                }
                // Second line of defence behind `layer_destroyed` (see
                // `layer_shell.rs`), for a client whose implicit teardown ran in
                // an order that left a dead surface mapped. `cleanup` only walks
                // the list -- no work at all with no layer surfaces -- and
                // re-arranges if it removed one, which is why the zone is
                // re-derived below when it did.
                let before = layers.len();
                layers.cleanup();
                before != layers.len()
            };
            if dropped_dead_layers {
                self.refresh_layer_zone();
                // One of those dead surfaces may have been holding the keyboard
                // (`layer_destroyed` is the usual path back, but this branch
                // exists precisely for the teardown orders it misses), and
                // `refresh_layer_zone` returns early when the zone didn't move.
                self.refresh_keyboard_focus();
            }
        }
        self.space.refresh();
        self.popups.cleanup();
        let _ = self.display_handle.flush_clients();
    }

    /// Recreates the render target at a new size. Returns whether it
    /// actually got there.
    ///
    /// Two callers: `--nested`, when the host's first configure disagrees
    /// with the size scoot started at (see `nested::init`), and `--tty`,
    /// when a DRM hotplug changes the connector's mode (see
    /// `tty/hotplug.rs`). This function doesn't know `Host` or `Tty` exists
    /// -- it only touches the render target and the core's notion of output
    /// geometry; the caller is responsible for resizing its own scanout
    /// buffers to match, separately.
    ///
    /// `false` means the render target could not be rebuilt and is still at
    /// the *old* size while the caller has (or is about to) size its scanout
    /// buffers to the new one. Both callers treat that as fatal-ish rather
    /// than something to limp on from, because the two sizes disagreeing is
    /// exactly what `present()`'s size guard silently drops every frame for:
    /// `--nested` stops the loop, `--tty` logs an error saying the screen
    /// stays as it is until the next hotplug or a restart. Nothing is
    /// reverted here -- a failure to build a pixman image at one size is not
    /// evidence that rebuilding it at the previous size would work.
    pub fn resize_output(&mut self, width: i32, height: i32) -> bool {
        let Some(output) = self.output.clone() else {
            // Logged, not a silent `false`: both callers' comments say
            // "`resize_output` has already logged what failed", and without
            // this that was only true of the `create_backend` path below.
            // Unreachable today -- `headless::init_named` runs before either
            // backend can ask for a resize -- but a `false` nobody can
            // explain is exactly the shape of failure this project treats as
            // seriously as a crash.
            tracing::warn!(width, height, "could not resize: there is no output yet");
            return false;
        };
        set_mode(&output, width, height, None, self.output_scale);
        match create_backend(&output, width, height) {
            Ok(backend) => self.backend = Some(backend),
            Err(error) => {
                tracing::warn!(%error, "could not resize the render target");
                return false;
            }
        }
        // The logical rectangle the core and the `Space` both work in -- see
        // `output_scale.rs`'s `logical_size`, and `init`'s comment on why the
        // core must never be handed the physical size.
        let logical = self
            .space
            .output_geometry(&output)
            .map(|geometry| (geometry.size.w, geometry.size.h))
            .unwrap_or((width, height));
        // A lock surface's configured size is an *exact* requirement -- the
        // next buffer that doesn't match it is a `dimensions_mismatch`
        // protocol error, i.e. a killed lock client on a locked session -- so
        // a resized output has to reconfigure them. A no-op when the session
        // isn't locked (there are none). Logical, not physical: a lock
        // surface's configure size is in surface-local logical coordinates.
        self.resize_lock_surfaces(logical);
        // Layer surfaces are anchored to the output's edges, so every one of
        // them has moved or resized -- `LayerMap::arrange` recomputes their
        // rectangles against the new mode and configures whoever needs a new
        // size. Before the core hears about the resize, so that
        // `refresh_layer_zone` below reports a zone measured against this
        // mode rather than the previous one.
        layer_map_for_output(&output).arrange();
        // The mode changed, so every output-management client is a mode, a
        // current-mode and a `done` behind. `set_mode` above is the write; this
        // is the only other site that reaches it (see `output_management.rs`).
        self.refresh_output_heads();
        // ...and every screen-capture client is holding a buffer sized for the
        // old framebuffer, which its next capture would be refused for with no
        // explanation. After the new backend is in place, because the size
        // re-advertised is read from it. See `screencopy.rs`.
        self.refresh_capture_constraints();
        self.world.handle_event(CoreEvent::OutputChanged {
            id: OUTPUT_ID,
            area: Rect::new(0, 0, logical.0, logical.1),
        });
        // The core re-clamps its old usable area into the new one on
        // `OutputChanged` (see `scoot_core`'s `Output::set_area`), which is
        // the right thing to do with a reservation nobody has re-reported
        // yet -- this is that re-report. It only does anything when the zone
        // actually moved (a bar is mapped, so the new mode leaves a
        // different usable rectangle); with nothing reserving anything it
        // returns early, because `OutputChanged`'s own re-clamp has already
        // left the core's usable area equal to the zone this recomputes.
        self.refresh_layer_zone();
        // `apply()`, not `request_render()`: the core now lays out against a
        // different rectangle, and nothing else pushes that arrangement onto
        // the windows -- `refresh_layer_zone` above ends in `apply()` only on
        // the paths where the zone moved, which is not the common case (no
        // bar mapped, or a bar whose exclusive zone is unchanged). Without
        // this, a window keeps the size it was configured at before the
        // resize and sits clipped or half off the new output until some
        // unrelated action happens to call `apply()`. That was harmless while
        // the only caller was `nested::apply_size` (one call per process, on
        // the host's first configure, before any window has mapped) and is
        // not once `--tty` resizes dynamically on hotplug with windows
        // already up. `apply()` ends in `request_render()`, so the frame is
        // still requested exactly once.
        self.apply();
        true
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
    // Immediately after the render, so a screen-capture client is handed the
    // frame that was just drawn rather than the one before it. Deliberately
    // *not* part of this tick's re-arm decision below: a capture session
    // waiting for the screen to change has nothing to do until something else
    // marks it dirty, and keeping the timer alive for it would undo the idle
    // behaviour `ensure_ticking` exists for. See `screencopy.rs`.
    state.service_captures();
    state.settle_idle_waiters();
    state.settle_shots();
    if state.needs_render || !state.pending_idle.is_empty() || state.shots_draining() {
        TimeoutAction::ToDuration(FRAME_INTERVAL)
    } else {
        state.timer_armed = false;
        TimeoutAction::Drop
    }
}

/// Fires [`LOCK_VBLANK_TIMEOUT`] after a blanked frame rendered under `--tty`
/// without its vblank confirming the lock: the fallback half of the vblank
/// wait (see `SessionLock::await_vblank`). One shot, always dropped -- a
/// re-presented flip arms its own timer against its own restarted bound, so
/// a timer only ever fires for a vblank that never came, and a stale one
/// finds no live deadline and drops. `Instant::now()` rather than the
/// timer's own timestamp, so the bound is measured against the same clock
/// the wait was armed with.
fn blank_timeout(_now: std::time::Instant, _metadata: &mut (), state: &mut State) -> TimeoutAction {
    state.note_blank_timeout(Instant::now());
    TimeoutAction::Drop
}

/// Appends the render elements of every mapped layer surface on `layers`,
/// front-most first, to `elements`.
///
/// Within one layer the most recently mapped surface wins, which is what
/// `layers_on(..).rev()` gives (the map keeps insertion order) and what
/// Smithay's own `space_render_elements` does with the same list. The
/// protocol itself leaves ordering *within* a layer undefined, so this is a
/// choice, not a rule -- but it is the same choice every wlroots-derived
/// compositor makes, and the one a client expects when it maps a second
/// surface on the same layer.
///
/// Allocates only when there is something to draw: `render_elements` returns
/// a `Vec` per surface (Smithay's own signature), so a session with no bars
/// or wallpaper -- the default -- adds no allocation to the frame at all.
fn layer_elements(
    layers: &LayerMap,
    which: &[Layer],
    renderer: &mut PixmanRenderer,
    scale: f64,
    elements: &mut Vec<Elements>,
) {
    for &layer in which {
        for surface in layers.layers_on(layer).rev() {
            // `layer_geometry` is `None` only for a surface this map never
            // mapped, which `layers_on` cannot produce.
            let Some(geometry) = layers.layer_geometry(surface) else {
                continue;
            };
            elements.extend(
                AsRenderElements::<PixmanRenderer>::render_elements::<
                    WaylandSurfaceRenderElement<PixmanRenderer>,
                >(
                    surface,
                    renderer,
                    geometry.loc.to_physical_precise_round(scale),
                    Scale::from(scale),
                    1.0,
                )
                .into_iter()
                .map(Elements::Surface),
            );
        }
    }
}

/// Whether `render()` must skip this frame because no presenter can show
/// it: `Some(false)` is a `--tty` session holding no DRM master (paused or a
/// failed reactivation -- see `Tty::active`), `Some(true)` is one holding it,
/// `None` is every other backend (headless, nested, and every test harness,
/// none of which has a `Tty` at all).
///
/// A free function over the flag rather than a method on `Tty` so the truth
/// table is unit-testable: no test harness can construct a `Tty` (it needs a
/// live DRM device), so a method could only ever be pinned live on `--tty`
/// hardware. The `map(Tty::is_active)` call at the `render()` gate is the
/// only caller.
fn tty_blocks_render(tty_active: Option<bool>) -> bool {
    tty_active.is_some_and(|active| !active)
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

    #[test]
    fn no_tty_never_blocks_a_render() {
        // Headless, nested, and every test harness: no `Tty` exists, so the
        // gate is a no-op and the whole existing suite pins the unblocked
        // path without knowing about this predicate.
        assert!(!tty_blocks_render(None));
    }

    #[test]
    fn an_active_tty_never_blocks_a_render() {
        assert!(!tty_blocks_render(Some(true)));
    }

    #[test]
    fn a_masterless_tty_blocks_a_render() {
        // Paused (VT-switched away) and failed-reactivation alike: both mean
        // `present()` drops every frame, so there is nothing a render could
        // show. This is the fail-first pin for the gate -- negate the
        // predicate body and this fails.
        assert!(tty_blocks_render(Some(false)));
    }

    /// The damage-tracker contract `Tty`'s failed-flip retry relies on,
    /// verified against the pinned Smithay source rather than assumed:
    /// `damage_output_internal` extends an unchanged frame's (empty) new
    /// damage with `old_damage.take(age - 1)`, so age 1 asks for nothing
    /// and reports `None`, while age 0 takes the full-redraw branch and
    /// reports the whole output. A retry that reads as age 1 therefore
    /// presents nothing on a quiet screen (the loss this fix closes); a
    /// retry at age 0 always re-presents.
    #[test]
    fn an_unchanged_frame_reports_no_damage_at_age_one_but_full_damage_at_age_zero() {
        use smithay::backend::renderer::element::Kind;
        use smithay::backend::renderer::element::solid::{
            SolidColorBuffer, SolidColorRenderElement,
        };

        let mut renderer = PixmanRenderer::new().expect("a cpu renderer");
        let mut image = renderer
            .create_buffer(Fourcc::Argb8888, (64, 64).into())
            .expect("an image");
        let mut tracker = OutputDamageTracker::new((64, 64), 1.0, Transform::Normal);
        let buffer = SolidColorBuffer::new((64, 64), [1.0, 0.0, 1.0, 1.0]);
        let element =
            SolidColorRenderElement::from_buffer(&buffer, (0, 0), 1.0, 1.0, Kind::Unspecified);
        let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
        let first = tracker
            .render_output(
                &mut renderer,
                &mut framebuffer,
                0,
                &[element],
                [0.0, 0.0, 0.0, 1.0],
            )
            .expect("a first render");
        assert!(first.damage.is_some());
        drop(first);
        // Unchanged, at the age a failed flip's retry would read without
        // the age clear: nothing new, and no history requested either.
        let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
        let element =
            SolidColorRenderElement::from_buffer(&buffer, (0, 0), 1.0, 1.0, Kind::Unspecified);
        let second = tracker
            .render_output(
                &mut renderer,
                &mut framebuffer,
                1,
                &[element],
                [0.0, 0.0, 0.0, 1.0],
            )
            .expect("a second render");
        assert!(second.damage.is_none());
        drop(second);
        // The same unchanged frame at age 0 -- what the cleared slot reads
        // as -- redraws the whole output.
        let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
        let element =
            SolidColorRenderElement::from_buffer(&buffer, (0, 0), 1.0, 1.0, Kind::Unspecified);
        let third = tracker
            .render_output(
                &mut renderer,
                &mut framebuffer,
                0,
                &[element],
                [0.0, 0.0, 0.0, 1.0],
            )
            .expect("a third render");
        assert!(third.damage.is_some());
    }
}
