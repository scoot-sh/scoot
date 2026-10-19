//! The popup stack's Wayland half: one on-demand `overlay` layer
//! surface holding the whole stack.
//!
//! The surface exists only while there is a card to show: created on the
//! first (a null output — the compositor's choice — or the configured
//! name), destroyed with its buffers unmapped when the last one leaves,
//! so an idle daemon holds no surface and no pixels. The input region is
//! the cards' union, so the gaps click through.
//!
//! Pointer only, no keyboard (`keyboard_interactivity: none`): a left
//! click on the body invokes `default` (or dismisses without one), a
//! click on a button invokes that action, a right click dismisses, a
//! middle click dismisses all. Hover pauses that card's expiry. A click's
//! activation token is minted from the click's button serial and emitted
//! before `ActionInvoked`.
//!
//! A card's expiry clock starts on the `wl_surface.frame` callback for
//! the commit that first contains it (the spec's "since the display").
//! The compositor serves no frame callbacks for a hidden surface —
//! locked, output-less — so those cards wait with their clocks stopped,
//! by construction rather than by IPC. Cards already on screen keep
//! expiring through a manual lock: the timerfd keeps running.
//!
//! Input idle pauses every clock through `ext-idle-notify` v2
//! (`get_input_idle_notification`, which ignores inhibitors by design,
//! so a playing video does not count as presence).
//!
//! No `unwrap`/`expect`/`panic` on any runtime path: a missing global, a
//! refused bind, a failed draw or a gone compositor reads as a degraded
//! mode or a clean exit, never a crash.

use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use scoot_rt::print::warn;
use scootui::output::Size;
use scootui::outputs::Frame;
use scootui::paint::Canvas;
use scootui::surfaces::{Buffers, Demand, Effect, Globals, LayerLive, LayerSpec, Plan};
use scootui::text::Text;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_output, wl_pointer, wl_region, wl_registry, wl_seat,
    wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::ExtIdleNotificationV1;
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    self, XdgActivationTokenV1,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor,
};

use crate::config::{Anchor as StackAnchor, Config};
use crate::look::Look;
use crate::model::store::Notification;
use crate::bus::hints::Urgency;
use crate::view::layout::{self, Stack};
use crate::view::render::{self, Hover, IconBits, ICON_SIDE};

/// evdev button codes: wayland-client names no button codes.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
/// How long a click waits for the compositor's token `done` before the
/// invoke goes out without one (the panel's own budget).
const TOKEN_TIMEOUT: Duration = Duration::from_millis(200);
/// The layer-shell namespace: what the surface is for.
const NAMESPACE: &str = "scootnotify";

/// What the Wayland half asks the daemon to do: the daemon owns the bus
/// connection and the store, this side owns the surface.
#[derive(Debug)]
pub enum Event {
    /// The frame callback for a commit landed: these cards are on
    /// screen, and their clocks start now.
    Presented(Vec<u32>),
    /// A left click on a card body: invoke `default`, or dismiss without
    /// one. The token (if minted) goes out before the invoke.
    ClickBody { id: u32, token: Option<String> },
    /// A left click on an action button.
    ClickButton {
        id: u32,
        key: String,
        token: Option<String>,
    },
    /// A right click on a card.
    Dismiss(u32),
    /// A middle click: dismiss everything on screen.
    DismissAll,
    /// The hovered card changed (None left the stack): pause the new
    /// one, resume the old one.
    Hover(Option<u32>),
    /// Input idle began (true) or ended (false): pause or resume every
    /// clock.
    Idle(bool),
}

/// The Wayland state: bound globals, the on-demand surface and its
/// buffers, the seat and pointer, the current layout, and what the
/// compositor last said.
pub struct App {
    config: Config,
    look: Look,
    text: Option<Text>,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<ZwlrLayerShellV1>,
    activation: Option<XdgActivationV1>,
    idle_notifier: Option<ExtIdleNotifierV1>,
    idle: Option<ExtIdleNotificationV1>,
    idle_warned: bool,
    seat_proxy: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    outputs: Vec<(u32, wl_output::WlOutput, String)>,
    named_output_warned: bool,
    button_serial: Option<u32>,
    token_done: Option<String>,
    pointer_in: bool,
    pointer_x: u32,
    pointer_y: u32,
    hover: Hover,
    demand: Demand,
    buffers: Buffers,
    live: Option<LayerLive>,
    configured_size: Option<Size>,
    content_size: Option<Size>,
    /// The size last passed to `set_size`: the surface converges on the
    /// content without re-asking every draw (a clamped compositor would
    /// otherwise storm configures).
    sized_to: Option<Size>,
    /// A draw skipped for lack of a free buffer: the next release
    /// retries it. Releases otherwise never dirty the surface (every
    /// commit would release, which would redraw, which would commit:
    /// a busy loop at full CPU).
    draw_skipped: bool,
    stack: Stack,
    icons: HashMap<u32, IconBits>,
    urgencies: HashMap<u32, Urgency>,
    /// Cards in the last commit(s), awaiting their frame callback.
    unpresented: Vec<u32>,
    /// Cards presented on an earlier frame: a replace restarts their
    /// clocks in the store without a new presentation.
    presented: std::collections::HashSet<u32>,
    dirty: bool,
    pending: Vec<Event>,
    click: Option<Click>,
    gone: bool,
}

/// A click waiting for its token: resolved by the popup (which owns the
/// queue) before the daemon invokes.
#[derive(Debug, Clone)]
struct Click {
    id: u32,
    key: Option<String>,
    serial: u32,
}

impl App {
    fn new(config: Config, look: Look) -> Self {
        Self {
            config,
            look,
            text: None,
            compositor: None,
            shm: None,
            layer_shell: None,
            activation: None,
            idle_notifier: None,
            idle: None,
            idle_warned: false,
            seat_proxy: None,
            pointer: None,
            outputs: Vec::new(),
            named_output_warned: false,
            button_serial: None,
            token_done: None,
            pointer_in: false,
            pointer_x: 0,
            pointer_y: 0,
            hover: None,
            demand: Demand::new(),
            buffers: Buffers::new(c"scootnotify"),
            live: None,
            configured_size: None,
            content_size: None,
            sized_to: None,
            draw_skipped: false,
            stack: Stack {
                width: 0,
                height: 0,
                cards: Vec::new(),
                hidden: 0,
                pill_label: String::new(),
                input: Vec::new(),
            },
            icons: HashMap::new(),
            urgencies: HashMap::new(),
            unpresented: Vec::new(),
            presented: std::collections::HashSet::new(),
            dirty: true,
            pending: Vec::new(),
            click: None,
            gone: false,
        }
    }

    /// Enough to show: without a layer shell there is no surface, and
    /// the daemon stays headless (the bus still serves).
    fn ready(&self) -> bool {
        self.compositor.is_some() && self.shm.is_some() && self.layer_shell.is_some()
    }

    fn globals(&self) -> Option<Globals<'_>> {
        Some(Globals {
            compositor: self.compositor.as_ref()?,
            layer_shell: self.layer_shell.as_ref(),
            xdg_wm_base: None,
            // Unbound in v1 (scale-1 rendering, said in `draw_frame`):
            // the compositor upscales the buffer on HiDPI outputs.
            viewporter: None,
            fractional_scale: None,
        })
    }

    /// The output the stack shows on: the configured name, or none (the
    /// compositor's choice). A name with no output left falls back to
    /// the compositor's choice, warned once per run.
    fn output(&mut self) -> Option<wl_output::WlOutput> {
        let Some(want) = self.config.output.as_deref() else {
            return None;
        };
        if let Some((_, output, _)) = self.outputs.iter().find(|(_, _, name)| name == want) {
            return Some(output.clone());
        }
        if !self.named_output_warned {
            self.named_output_warned = true;
            warn(format_args!(
                "scootnotify: output \"{want}\" is gone; showing on the compositor's choice"
            ));
        }
        None
    }

    /// Creates the surface for the current demand, sized to the content.
    fn show(&mut self, qh: &QueueHandle<Self>) {
        if self.live.is_some() || self.outputs.is_empty() {
            return;
        }
        match self.demand.show() {
            Effect::Create => {}
            _ => return,
        }
        let content = self.content_size.unwrap_or(Size {
            width: self.config.width_px,
            height: 1,
        });
        let margin = self.config.margin_px;
        let spec = LayerSpec {
            namespace: NAMESPACE,
            exclusive_keyboard: false,
            anchor: anchor_of(self.config.anchor),
            width: content.width,
            height: content.height.max(1),
            margin: [margin, margin, margin, margin],
            exclusive_zone: 0,
        };
        let output = self.output();
        let Some(globals) = self.globals() else {
            return;
        };
        match LayerLive::create(&globals, output.as_ref(), &spec, qh, ()) {
            Ok(live) => {
                self.live = Some(live);
                self.configured_size = None;
                self.dirty = true;
            }
            Err(error) => {
                warn(format_args!("scootnotify: cannot show the stack: {error:?}"));
            }
        }
    }

    /// Destroys the surface and unmaps every buffer: empty holds
    /// nothing. The clocks of waiting cards stay stopped (they were
    /// never presented); shown cards keep theirs (expiry runs through
    /// anything but the surface).
    fn destroy_live(&mut self) {
        self.buffers.clear();
        if let Some(live) = self.live.take() {
            live.destroy();
        }
        self.configured_size = None;
        self.sized_to = None;
        self.draw_skipped = false;
    }

    /// Hides an emptied stack: no surface, no buffers, no input region.
    fn hide(&mut self) {
        match self.demand.hide() {
            Effect::Destroy
            | Effect::None
            | Effect::Create
            | Effect::Ack(_)
            | Effect::DestroyAndGiveUp
            | Effect::DestroyAndRetry => {}
        }
        self.destroy_live();
        self.unpresented.clear();
        self.presented.clear();
        // The hover died with the surface: the daemon resumes the card
        // (its pause was the pointer's, which is gone with the pixels).
        if self.hover.map(|(id, _)| id).is_some() {
            self.pending.push(Event::Hover(None));
        }
        self.hover = None;
    }

    /// The laid-out card under the pointer, and the button within it.
    fn hit(&self) -> Option<(u32, Option<usize>)> {
        if !self.pointer_in {
            return None;
        }
        for card in &self.stack.cards {
            let r = card.rect;
            if self.pointer_x >= r.x
                && self.pointer_x < r.x + r.width
                && self.pointer_y >= r.y
                && self.pointer_y < r.y + r.height
            {
                for (index, button) in card.buttons.iter().enumerate() {
                    let b = button.rect;
                    if self.pointer_x >= b.x
                        && self.pointer_x < b.x + b.width
                        && self.pointer_y >= b.y
                        && self.pointer_y < b.y + b.height
                    {
                        return Some((card.id, Some(index)));
                    }
                }
                return Some((card.id, None));
            }
        }
        None
    }

    /// Recomputes the hover from the pointer: pause/resume and redraw
    /// follow in the daemon.
    fn rehover(&mut self) {
        let hit = self.hit();
        if hit != self.hover {
            let old = self.hover.map(|(id, _)| id);
            let new = hit.map(|(id, _)| id);
            self.hover = hit;
            if old != new {
                self.pending.push(Event::Hover(new));
            }
            self.dirty = true;
        }
    }

    /// Draws the stack into a pooled buffer and commits it, requesting a
    /// frame callback for cards this commit first contains.
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        // No surface, nothing to draw on.
        if self.live.is_none() {
            return;
        }
        // No pixels before the first configure: attaching a buffer
        // before acking the initial configure is a protocol error, and
        // the compositor kills the client for it. The configure sets
        // `dirty`, so the very next turn draws.
        if self.configured_size.is_none() {
            return;
        }
        // The content sizes the surface (the compositor echoes explicit
        // sizes back, so resolving 0-axes against the configured size
        // would pin the first card's size forever as the stack grows).
        let size = self.content_size.or(self.configured_size);
        match self.demand.plan(size, self.dirty) {
            Plan::Draw(frame) => {
                if self.draw_frame(frame, qh) {
                    self.dirty = false;
                }
            }
            // Committed as-was (a bare ack): nothing left to draw.
            Plan::Commit => {
                if let Some(live) = self.live.as_ref() {
                    live.surface().commit();
                    self.demand.committed();
                }
                self.dirty = false;
            }
            Plan::Nothing => {
                self.dirty = false;
            }
        }
    }

    /// One frame: paints, attaches, damages, commits. Returns whether
    /// the frame went out (both buffers held retries on the next
    /// release, keeping `dirty`).
    fn draw_frame(&mut self, frame: Frame, qh: &QueueHandle<Self>) -> bool {
        let dims = match self.demand.buffer_dims(frame.size) {
            Some(dims) => dims,
            None => return false,
        };
        if dims.0 == 0 || dims.1 == 0 {
            return false;
        }
        let shm = self.shm.clone();
        if self.text.is_none() {
            return false;
        }
        // The input region follows the layout, not the pixels: set it
        // before the buffer is taken (the take borrows the pools, and
        // the text borrows the glyph cache).
        self.set_input_region(qh);
        let (Some(shm), Some(text)) = (shm, self.text.as_mut()) else {
            return false;
        };
        let slot = match self.buffers.take(&shm, qh, dims, wl_shm::Format::Argb8888, ()) {
            Ok(Some(slot)) => slot,
            // Both buffers held: the next release retries (and only
            // then — releases never dirty an up-to-date surface, or
            // every commit's release would redraw into the next commit
            // at full CPU).
            Ok(None) => {
                self.draw_skipped = true;
                return false;
            }
            Err(error) => {
                warn(format_args!("scootnotify: cannot draw the stack: {error}"));
                return false;
            }
        };
        {
            let Some(mut canvas) = Canvas::new(slot.pixels_mut(), dims.0, dims.1) else {
                return false;
            };
            // Scale 1 in v1: the layout is logical pixels, and the
            // buffer is the logical size. The fractional globals stay
            // unbound and surface scale events are dropped, so the
            // demand's scale is always 1; a HiDPI output upscales the
            // buffer (correct size, softer text). Sharp fractional
            // rendering is a follow-up, said so in the site docs.
            render::paint(
                &mut canvas,
                text,
                &self.stack,
                &self.config,
                &self.look,
                &self.urgencies,
                self.hover,
                &self.icons,
            );
        }
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        // The content sizes the surface: ask once per size (a clamped
        // compositor answers its own size, and re-asking every draw
        // would storm configures). The size rides the same commit as
        // the pixels, so they apply together.
        if self.content_size.is_some() && self.content_size != self.sized_to {
            if let Some(content) = self.content_size {
                live.set_size(content.width.max(1), content.height.max(1));
                self.sized_to = Some(content);
            }
        }
        live.surface().attach(Some(slot.buffer()), 0, 0);
        // Full damage, every draw: the bar's shape (exact buffer rect).
        // Without it the compositor keeps showing the last presented
        // frame: the mapped first frame shows regardless, which reads
        // as "opens fine, never updates".
        live.surface()
            .damage_buffer(0, 0, dims.0 as i32, dims.1 as i32);
        live.prepare(frame.size, frame.scale);
        // The frame callback starts the clocks of the cards this commit
        // first contains. Steady-state repaints (hover, expiry redraws)
        // carry nothing: no request, no allocation.
        if !self.unpresented.is_empty() {
            let fresh = std::mem::take(&mut self.unpresented);
            live.surface().frame(qh, fresh);
        }
        live.surface().commit();
        slot.mark_committed();
        self.demand.drew(frame);
        self.draw_skipped = false;
        true
    }

    /// Sets the input region to the cards' union: the gaps click
    /// through to whatever is behind.
    fn set_input_region(&mut self, qh: &QueueHandle<Self>) {
        let (Some(live), Some(compositor)) = (self.live.as_ref(), self.compositor.as_ref()) else {
            return;
        };
        // A layer surface without a region takes input everywhere; an
        // empty stack never reaches here (it hides instead).
        let region = compositor.create_region(qh, ());
        for card in &self.stack.cards {
            let r = card.rect;
            region.add(r.x as i32, r.y as i32, r.width as i32, r.height as i32);
        }
        live.surface().set_input_region(Some(&region));
        region.destroy();
    }

    /// Re-themes in place: the new look over the live cards, which keep
    /// their ids, clocks, hover and history.
    fn retheme(&mut self, look: Look) {
        self.look = look;
        // The layout's sizes depend on the text size: force a rebuild
        // on the next refresh (the stack comparison would call equal
        // sizes equal and skip the draw).
        self.content_size = None;
        self.dirty = true;
    }

    /// Arms the idle notification once the seat exists: `pause_when_idle
    /// = 0` disables, a notifier without v2 warns once and never pauses.
    fn arm_idle(&mut self, qh: &QueueHandle<Self>) {
        if self.config.pause_when_idle_secs == 0 || self.idle.is_some() {
            return;
        }
        let (Some(notifier), Some(seat)) = (self.idle_notifier.as_ref(), self.seat_proxy.as_ref())
        else {
            return;
        };
        let secs = self.config.pause_when_idle_secs;
        let note = notifier.get_input_idle_notification(secs * 1000, seat, qh, ());
        self.idle = Some(note);
    }
}

/// The layer-shell anchor for the configured corner.
fn anchor_of(anchor: StackAnchor) -> Anchor {
    match anchor {
        StackAnchor::TopLeft => Anchor::Top | Anchor::Left,
        StackAnchor::TopRight => Anchor::Top | Anchor::Right,
        StackAnchor::BottomLeft => Anchor::Bottom | Anchor::Left,
        StackAnchor::BottomRight => Anchor::Bottom | Anchor::Right,
    }
}

/// The popup: the Wayland connection, the queue, and the app. The daemon
/// drives it beside the bus: `refresh` on content change, `dispatch` on
/// Wayland traffic, `draw` when dirty.
pub struct Popup {
    conn: Connection,
    queue: EventQueue<App>,
    app: App,
}

impl Popup {
    /// Connects to the compositor and binds the globals the stack
    /// needs. `Err` (no display, no compositor, no layer shell, no
    /// font) means headless: the bus still serves, and cards wait with
    /// their clocks stopped.
    pub fn connect(config: &Config, look: &Look, font_path: Option<&PathBuf>) -> Result<Self, String> {
        let conn = Connection::connect_to_env()
            .map_err(|error| format!("no compositor ({error:?}): headless"))?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let _registry = conn.display().get_registry(&qh, ());
        let mut app = App::new(config.clone(), look.clone());
        queue
            .roundtrip(&mut app)
            .map_err(|error| format!("no compositor ({error:?}): headless"))?;
        queue
            .roundtrip(&mut app)
            .map_err(|error| format!("no compositor ({error:?}): headless"))?;
        if !app.ready() {
            return Err(String::from("no layer surface to draw on: headless"));
        }
        app.text = match scootui::font::text(font_path.map(|path| path.as_path()), &[]) {
            Ok(text) => Some(text),
            Err(error) => return Err(format!("no font ({error}): headless")),
        };
        app.arm_idle(&qh);
        if let Err(error) = queue.flush() {
            return Err(format!("no compositor ({error:?}): headless"));
        }
        Ok(Self { conn, queue, app })
    }

    /// The Wayland fd: the daemon polls this beside the bus and the
    /// timerfd, and sleeps at idle with nothing staged.
    pub fn fd(&self) -> BorrowedFd<'_> {
        use std::os::fd::AsFd as _;
        self.conn.as_fd()
    }

    /// Whether the surface is up.
    pub fn is_live(&self) -> bool {
        self.app.live.is_some()
    }

    /// Whether the compositor went away: the daemon exits 1 (the unit
    /// goes with the session), like the bus-gone path.
    pub fn gone(&self) -> bool {
        self.app.gone
    }

    /// The outputs known: zero means cards wait with their clocks
    /// stopped.
    pub fn outputs(&self) -> usize {
        self.app.outputs.len()
    }

    /// Rebuilds the layout from the live set: redraws on change, shows
    /// on the first card, hides on the last close. Decodes icons for
    /// new cards (once, at arrival scale). Pure cost on content change;
    /// the repaint stays allocation-free.
    pub fn refresh(&mut self, live: &[&Notification], now_ms: u64) {
        let app = &mut self.app;
        let em = app.look.font_size_px;
        let text = match app.text.as_ref() {
            Some(text) => text,
            None => return,
        };
        let stack = layout::layout(live, &app.config, text, em, now_ms);
        if stack != app.stack {
            app.stack = stack;
            app.content_size = if app.stack.cards.is_empty() {
                None
            } else {
                Some(Size {
                    width: app.stack.width,
                    height: app.stack.height.max(1),
                })
            };
            app.dirty = true;
        }
        // Urgencies follow the live set (the paint reads them per card).
        app.urgencies.clear();
        app.urgencies.extend(live.iter().map(|note| (note.id, note.urgency)));
        // Decode icons for cards without any (arrival-priced, once).
        for note in live {
            if matches!(note.icon, crate::model::store::Icon::None) || app.icons.contains_key(&note.id) {
                continue;
            }
            if let Some(bits) = decode_icon(note) {
                app.icons.insert(note.id, bits);
            }
        }
        // Forget what left (a close frees its pixels with its card,
        // and drops its presentation: a re-arrival presents afresh).
        app.icons.retain(|id, _| live.iter().any(|note| note.id == *id));
        app.unpresented.retain(|id| live.iter().any(|note| note.id == *id));
        app.presented.retain(|id| live.iter().any(|note| note.id == *id));
        let qh = self.queue.handle();
        if app.stack.cards.is_empty() {
            if app.live.is_some() {
                app.hide();
            }
            return;
        }
        if app.outputs.is_empty() {
            // Zero outputs: cards wait, clocks stopped, no surface.
            if app.live.is_some() {
                app.destroy_live();
            }
            return;
        }
        if app.live.is_none() {
            app.show(&qh);
            // Everything on screen needs its first presentation: the
            // commit below carries them all.
            app.unpresented.clear();
            app.unpresented.extend(
                app.stack
                    .cards
                    .iter()
                    .map(|card| card.id)
                    .filter(|id| !app.presented.contains(id)),
            );
        } else {
            // New cards join the unpresented set; shown cards (a replace
            // restarts their clocks in the store already) need no new
            // presentation.
            for card in &app.stack.cards {
                if !app.presented.contains(&card.id) && !app.unpresented.contains(&card.id) {
                    app.unpresented.push(card.id);
                }
            }
        }
    }

    /// Draws when dirty, then flushes: the daemon calls this after
    /// `refresh` and after Wayland traffic. Marks the compositor gone
    /// when the flush fails (the daemon exits 1).
    pub fn draw(&mut self) {
        let qh = self.queue.handle();
        self.app.draw(&qh);
        if self.queue.flush().is_err() {
            self.app.gone = true;
        }
    }

    /// Dispatches Wayland traffic into daemon events: pointer hits (with
    /// tokens minted before the daemon invokes), presentations, idle,
    /// hover pauses.
    pub fn dispatch(&mut self) -> Vec<Event> {
        if self.queue.dispatch_pending(&mut self.app).is_err() {
            self.app.gone = true;
        }
        let _ = self.queue.flush();
        // A click waits for its token here (the queue is ours to pump):
        // the token signal precedes the invoke because the daemon
        // invokes only after this returns.
        if let Some(click) = self.app.click.take() {
            let token = self.mint_token(click.serial);
            let event = match click.key {
                Some(key) => Event::ClickButton {
                    id: click.id,
                    key,
                    token,
                },
                None => Event::ClickBody {
                    id: click.id,
                    token,
                },
            };
            self.app.pending.push(event);
        }
        std::mem::take(&mut self.app.pending)
    }

    /// Mints one activation token from a click's button serial: the
    /// compositor answers `done` with the token, or nothing within the
    /// budget (then the invoke goes out without one).
    fn mint_token(&mut self, serial: u32) -> Option<String> {
        let (activation, seat, live) = (
            self.app.activation.as_ref()?,
            self.app.seat_proxy.as_ref()?,
            self.app.live.as_ref()?,
        );
        self.app.token_done = None;
        let token = activation.get_activation_token(&self.queue.handle(), ());
        token.set_serial(serial, seat);
        token.set_surface(live.surface());
        token.commit();
        if self.queue.flush().is_err() {
            token.destroy();
            return None;
        }
        let deadline = Instant::now() + TOKEN_TIMEOUT;
        loop {
            if self.queue.dispatch_pending(&mut self.app).is_err() {
                token.destroy();
                return None;
            }
            if let Some(done) = self.app.token_done.take() {
                token.destroy();
                return if done.is_empty() { None } else { Some(done) };
            }
            if Instant::now() >= deadline {
                token.destroy();
                return None;
            }
            if self.queue.flush().is_err() {
                token.destroy();
                return None;
            }
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            let wait = deadline.checked_duration_since(Instant::now()).unwrap_or(Duration::ZERO);
            let timespec = rustix::time::Timespec {
                tv_sec: wait.as_secs() as _,
                tv_nsec: wait.subsec_nanos() as _,
            };
            let wayland = guard.connection_fd();
            let mut polls = [rustix::event::PollFd::new(&wayland, rustix::event::PollFlags::IN)];
            let _ = rustix::event::poll(&mut polls, Some(&timespec));
            if !polls[0].revents().is_empty() {
                let _ = guard.read();
            }
        }
    }

    /// Prepares a Wayland read for the daemon's poll: `None` while
    /// dispatched-but-unread events wait (then the daemon skips the
    /// read and `dispatch` collects them). The daemon polls the fd from
    /// [`Popup::fd`] beside the bus and the timerfd, reads the guard
    /// when it speaks, and drops it unread otherwise (the documented
    /// read-after-poll shape: the next turn collects it).
    pub fn prepare_read(&self) -> Option<wayland_client::backend::ReadEventsGuard> {
        self.queue.prepare_read()
    }

    /// Flushes queued requests: false when the compositor went away.
    pub fn flush(&mut self) -> bool {
        if self.queue.flush().is_err() {
            self.app.gone = true;
            return false;
        }
        true
    }

    /// Re-themes in place: the new look over the live cards, which keep
    /// their ids, clocks, hover and history. The control ticket's `msg
    /// reload` (with `scope = "look"`) calls this; refused fragments
    /// never reach it (the caller keeps the last good).
    pub fn retheme(&mut self, look: Look) {
        self.app.retheme(look);
    }

    /// The current look, for tests.
    #[cfg(test)]
    pub fn look(&self) -> &Look {
        &self.app.look
    }
}

/// Decodes a card's icon once, at arrival scale: `image-data` is
/// premultiplied and fitted into the decoded square, a path reads
/// through the tray's icon discipline (regular files only, capped), a
/// theme name draws the letter tile (`None`). Never fails loudly: a
/// refused image is a tile, not an error.
fn decode_icon(note: &Notification) -> Option<IconBits> {
    match &note.icon {
        crate::model::store::Icon::None => None,
        crate::model::store::Icon::Name(_) => None,
        crate::model::store::Icon::Path(path) => {
            let image = scootui::icon::image::load(std::path::Path::new(path)).ok()?;
            let mut pixels = vec![0u8; ICON_SIDE as usize * ICON_SIDE as usize * 4];
            image.scale_into(ICON_SIDE, &mut pixels);
            Some(IconBits {
                side: ICON_SIDE,
                pixels,
            })
        }
        crate::model::store::Icon::Data(image) => {
            let side = ICON_SIDE as usize;
            let src_w = image.width as usize;
            let src_h = image.height as usize;
            if src_w == 0 || src_h == 0 || src_w > 4096 || src_h > 4096 {
                return None;
            }
            if image.pixels.len() < src_w * src_h * 4 {
                return None;
            }
            // Straight RGBA into premultiplied BGRA (the sample filter
            // reads premultiplied rows).
            let mut rgba = vec![0u8; src_w * src_h * 4];
            for (dst, src) in rgba.chunks_exact_mut(4).zip(image.pixels.chunks_exact(4)) {
                let a = u32::from(src[3]);
                dst[0] = ((u32::from(src[2]) * a + 127) / 255).min(255) as u8;
                dst[1] = ((u32::from(src[1]) * a + 127) / 255).min(255) as u8;
                dst[2] = ((u32::from(src[0]) * a + 127) / 255).min(255) as u8;
                dst[3] = a.min(255) as u8;
            }
            let mut pixels = vec![0u8; side * side * 4];
            scootui::icon::sample::scale_into(&rgba, src_w, src_h, ICON_SIDE, &mut pixels);
            Some(IconBits {
                side: ICON_SIDE,
                pixels,
            })
        }
    }
}

// --- Wayland dispatch ---------------------------------------------------

macro_rules! empty_dispatch {
    ($object:ty) => {
        impl Dispatch<$object, ()> for App {
            fn event(
                _: &mut App,
                _: &$object,
                _: <$object as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<App>,
            ) {
            }
        }
    };
}

empty_dispatch!(wl_compositor::WlCompositor);
empty_dispatch!(wl_surface::WlSurface);
empty_dispatch!(wl_region::WlRegion);
empty_dispatch!(wl_shm::WlShm);
empty_dispatch!(wl_shm_pool::WlShmPool);
empty_dispatch!(WpViewport);
empty_dispatch!(WpFractionalScaleV1);
empty_dispatch!(ZwlrLayerShellV1);
empty_dispatch!(XdgActivationV1);
empty_dispatch!(ExtIdleNotifierV1);

impl Dispatch<XdgActivationTokenV1, ()> for App {
    fn event(
        app: &mut App,
        _: &XdgActivationTokenV1,
        event: <XdgActivationTokenV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            app.token_done = Some(token);
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        app: &mut App,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<App>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" => {
                    app.compositor = Some(registry.bind(name, 6.min(version), qh, ()));
                }
                "wl_shm" => {
                    app.shm = Some(registry.bind(name, 1.min(version), qh, ()));
                }
                "wl_output" => {
                    let output: wl_output::WlOutput = registry.bind(name, 4.min(version), qh, ());
                    app.outputs.push((name, output, String::new()));
                }
                "wl_seat" => {
                    app.seat_proxy = Some(registry.bind(name, 7.min(version), qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    app.layer_shell = Some(registry.bind(name, 4.min(version), qh, ()));
                }
                "xdg_activation_v1" => {
                    app.activation = Some(registry.bind(name, 1.min(version), qh, ()));
                }
                // The fractional globals stay unbound in v1 (scale-1
                // rendering, said in `draw_frame`).
                "wp_viewporter" | "wp_fractional_scale_manager_v1" => {}
                "ext_idle_notifier_v1" => {
                    if version >= 2 {
                        app.idle_notifier = Some(registry.bind(name, 2, qh, ()));
                    } else if !app.idle_warned {
                        app.idle_warned = true;
                        warn(format_args!(
                            "scootnotify: the compositor's idle notifier is v1: expiry never pauses for idle"
                        ));
                    }
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                // An output going away: the stack moves (a null output
                // follows the compositor; a named one falls back), and
                // with zero outputs the cards wait.
                app.outputs.retain(|(global, _, _)| *global != name);
                if app.outputs.is_empty() && app.live.is_some() {
                    app.destroy_live();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for App {
    fn event(
        app: &mut App,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        // The v4 name: what `output = "NAME"` pins.
        if let wl_output::Event::Name { name } = event
            && let Some(slot) = app.outputs.iter_mut().find(|(_, proxy, _)| proxy == output)
        {
            slot.2 = name;
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        app: &mut App,
        _: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<App>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event
            && let WEnum::Value(caps) = capabilities
        {
            use wl_seat::Capability;
            if caps.contains(Capability::Pointer) && app.pointer.is_none() {
                let Some(seat) = app.seat_proxy.as_ref() else {
                    return;
                };
                app.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        app: &mut App,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                app.pointer_in = app
                    .live
                    .as_ref()
                    .is_some_and(|live| live.surface() == &surface);
                (app.pointer_x, app.pointer_y) = (surface_x as u32, surface_y as u32);
                app.rehover();
            }
            wl_pointer::Event::Leave { .. } => {
                app.pointer_in = false;
                app.rehover();
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                (app.pointer_x, app.pointer_y) = (surface_x as u32, surface_y as u32);
                app.rehover();
            }
            wl_pointer::Event::Button {
                button,
                state,
                serial,
                ..
            } => {
                if !matches!(state, WEnum::Value(wl_pointer::ButtonState::Pressed)) {
                    return;
                }
                if button == BTN_MIDDLE {
                    app.pending.push(Event::DismissAll);
                    return;
                }
                // A click in a gap never arrives (the gaps click
                // through); a click on a card dismisses or invokes.
                if button == BTN_RIGHT {
                    if let Some((id, _)) = app.hit() {
                        app.pending.push(Event::Dismiss(id));
                    }
                    return;
                }
                if button != BTN_LEFT {
                    return;
                }
                let Some((id, hit_button)) = app.hit() else {
                    return;
                };
                app.button_serial = Some(serial);
                match hit_button {
                    Some(index) => {
                        let key = app.stack.cards.iter().find(|card| card.id == id).and_then(
                            |card| card.buttons.get(index).map(|button| button.key.clone()),
                        );
                        let Some(key) = key else {
                            return;
                        };
                        app.click = Some(Click { id, key: Some(key), serial });
                    }
                    None => {
                        app.click = Some(Click { id, key: None, serial });
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(
        app: &mut App,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let wl_buffer::Event::Release = event {
            app.buffers.released(buffer);
            // A skipped draw retries on the next release. Any other
            // release frees its slot and nothing more: dirtying here
            // would redraw every commit's release into the next commit
            // at full CPU.
            if app.draw_skipped {
                app.dirty = true;
            }
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for App {
    fn event(
        app: &mut App,
        role: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<App>,
    ) {
        // Stale configures for a destroyed surface are dropped by the
        // role comparison, as the surfaces contract asks.
        let live = app.live.as_ref().is_some_and(|live| live.role() == role);
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                if !live {
                    return;
                }
                // Every configure is acked at receipt, exactly once: the
                // compositor treats a second ack as a protocol error.
                if let Some(live) = app.live.as_ref() {
                    live.ack(serial);
                }
                match app.demand.configure(serial, width, height) {
                    Effect::Ack(_) | Effect::None | Effect::Create => {}
                    Effect::Destroy | Effect::DestroyAndGiveUp | Effect::DestroyAndRetry => {
                        app.destroy_live();
                    }
                }
                app.configured_size = Some(Size { width, height });
                app.dirty = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                if !live {
                    return;
                }
                // A fresh start, not a retry: `hide` forgets the closed
                // surface, so the next refresh shows again while cards
                // remain (a compositor that closes in a loop paces this
                // at its own event rate, like every other client it
                // closes). The panel exits here instead; the daemon
                // runs until killed, so it re-shows.
                match app.demand.closed() {
                    Effect::DestroyAndRetry | Effect::DestroyAndGiveUp | Effect::Destroy => {
                        app.hide();
                    }
                    Effect::Ack(_) | Effect::None | Effect::Create => {}
                }
                let _ = qh;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, Vec<u32>> for App {
    fn event(
        app: &mut App,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        batch: &Vec<u32>,
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        // The commit's cards are on screen: their clocks start now (a
        // card that left meanwhile presents to nothing — the daemon's
        // call is a no-op for unknown ids).
        if let wl_callback::Event::Done { .. } = event {
            let shown: Vec<u32> = batch.to_vec();
            app.unpresented.retain(|id| !shown.contains(id));
            app.presented.extend(shown.iter().copied());
            if !shown.is_empty() {
                app.pending.push(Event::Presented(shown));
            }
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for App {
    fn event(
        app: &mut App,
        _: &ExtIdleNotificationV1,
        event: <ExtIdleNotificationV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            <ExtIdleNotificationV1 as Proxy>::Event::Idled => {
                app.pending.push(Event::Idle(true));
            }
            <ExtIdleNotificationV1 as Proxy>::Event::Resumed => {
                app.pending.push(Event::Idle(false));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
