//! The prototype panel: a layer-shell overlay with exclusive keyboard,
//! drawn with scootbar's own `Canvas` + `Text`, decoding keys through the
//! keyboard spike's pick (`xkbcommon`, dynamically linked, no wrapper).

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::protocol::{
    wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_keyboard::WlKeyboard,
    wl_output::WlOutput, wl_pointer::WlPointer, wl_registry::WlRegistry,
    wl_seat::{Capability as SeatCapability, WlSeat}, wl_shm::{Format as ShmFormat, WlShm},
    wl_shm_pool::WlShmPool, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::activation::v1::client::{
    xdg_activation_token_v1::{Event as TokenEvent, XdgActivationTokenV1},
    xdg_activation_v1::XdgActivationV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer as ShellLayer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{
        Anchor, Event as LayerEvent, KeyboardInteractivity, ZwlrLayerSurfaceV1,
    },
};
use xkbcommon::xkb::{self, Keycode};

use crate::draw;
use crate::index::Entry;
use crate::text::Text;
use crate::shm::Shm;

const NS: &str = "w9-launch";
const KEY_ESC: u32 = 0xff1b;
const KEY_RETURN: u32 = 0xff0d;
const KEY_BS: u32 = 0xff08;
const KEY_UP: u32 = 0xff52;
const KEY_DOWN: u32 = 0xff54;
const BTN_LEFT: u32 = 0x110;

pub struct Opts {
    pub font: std::path::PathBuf,
    pub trace: Option<std::path::PathBuf>,
    pub stub_sock: Option<std::path::PathBuf>,
    pub spawn_stub: bool,
    pub quit_after_ms: Option<u64>,
    pub use_focus_serial: bool,
}

fn trace(log: &mut Option<std::fs::File>, t0: Instant, msg: &str) {
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let line = format!("{ms:10.3} {msg}\n");
    if let Some(f) = log {
        let _ = f.write_all(line.as_bytes());
    }
}

/// One wl_shm buffer over a sealed memfd (see `shm.rs`).
struct Slot {
    shm: Option<Shm>,
    pool: Option<WlShmPool>,
    buffer: Option<WlBuffer>,
    busy: bool,
    w: u32,
    h: u32,
}

struct Globals {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    layer_shell: Option<ZwlrLayerShellV1>,
    seat: Option<WlSeat>,
    activation: Option<XdgActivationV1>,
    output_scale: u32,
}

struct Kb {
    state: Option<xkb::State>,
    compose: Option<xkb::compose::State>,
    last_serial: u32,
    focus_serial: u32,
}

pub struct Ui {
    t0: Instant,
    log: Option<std::fs::File>,
    text: Text,
    entries: Vec<Entry>,
    query: String,
    order: Vec<usize>,
    selected: usize,
    opts: Opts,
    globals: Globals,
    surface: Option<WlSurface>,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
    pointer_y: Option<f64>,
    kb: Kb,
    slots: [Slot; 2],
    first_commit: bool,
    presented: bool,
    dirty: bool,
    scale: u32,
    token: Option<String>,
    token_obj: Option<XdgActivationTokenV1>,
    token_deadline: Option<Instant>,
    launch_done: bool,
    exit_code: i32,
    should_exit: bool,
}

impl Ui {
    fn retrace(&mut self, msg: &str) {
        trace(&mut self.log, self.t0, msg);
    }

    fn clocks(&mut self, tag: &str) {
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        self.retrace(&format!("CLOCKS {tag} mono_us={} wall_ns={wall}", self.t0.elapsed().as_micros()));
    }

    fn rescore(&mut self) {
        let t = Instant::now();
        self.order = crate::index::rank(&self.entries, &self.query).order;
        self.selected = 0;
        self.retrace(&format!(
            "RESCORE query={:?} hits={} us={}",
            self.query,
            self.order.len(),
            t.elapsed().as_micros()
        ));
    }

    fn redraw(&mut self, qh: &QueueHandle<Ui>) {
        let t = Instant::now();
        self.retrace("REDRAW_ENTER");
        let scale = self.scale.max(1);
        let w = draw::W * scale;
        let h = draw::H * scale;
        let slot_idx = self.slots.iter().position(|s| !s.busy);
        let Some(slot_idx) = slot_idx else {
            // No free buffer: the compositor still holds both. Mark dirty;
            // the next Release repaints.
            if !self.dirty {
                self.dirty = true;
                self.retrace("REDRAW_STALLED dirty");
            }
            return;
        };
        self.dirty = false;
        let need_new = self.slots[slot_idx].shm.is_none()
            || self.slots[slot_idx].w != w
            || self.slots[slot_idx].h != h;
        if need_new {
            let (Some(shm_global), Some(_)) =
                (self.globals.shm.clone(), self.surface.clone())
            else {
                return;
            };
            let slot = &mut self.slots[slot_idx];
            slot.pool.take().map(|p| p.destroy());
            slot.buffer.take().map(|b| b.destroy());
            let Some(shm) = Shm::new(w, h) else { return };
            let pool =
                shm_global.create_pool(shm.memfd.as_fd(), (w * h * 4) as i32, qh, ());
            let buffer = pool.create_buffer(
                0,
                w as i32,
                h as i32,
                (w * 4) as i32,
                ShmFormat::Argb8888,
                qh,
                (),
            );
            slot.shm = Some(shm);
            slot.pool = Some(pool);
            slot.buffer = Some(buffer);
            slot.w = w;
            slot.h = h;
            self.retrace("REDRAW_SLOT");
        }
        let slot = &mut self.slots[slot_idx];
        let Some(shm) = slot.shm.as_mut() else { return };
        // Scale-1 draw, then nearest-neighbor upscale when scale is 2.
        if scale == 1 {
            draw::draw_panel(
                shm.pixels(),
                &mut self.text,
                &self.query,
                &self.order,
                &self.entries,
                self.selected,
            );
        } else {
            let mut tmp = vec![0u8; (draw::W * draw::H * 4) as usize];
            draw::draw_panel(
                &mut tmp,
                &mut self.text,
                &self.query,
                &self.order,
                &self.entries,
                self.selected,
            );
            let dst = shm.pixels();
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let s = ((y / 2) * draw::W as usize + x / 2) * 4;
                    let d = (y * w as usize + x) * 4;
                    dst[d..d + 4].copy_from_slice(&tmp[s..s + 4]);
                }
            }
        }
        let (Some(surface), Some(buffer)) =
            (self.surface.clone(), slot.buffer.clone())
        else {
            return;
        };
        surface.attach(Some(&buffer), 0, 0);
        if scale == 2 {
            surface.set_buffer_scale(2);
        }
        surface.damage_buffer(0, 0, w as i32, h as i32);
        surface.commit();
        slot.busy = true;
        if !self.first_commit {
            self.first_commit = true;
            let wall = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            eprintln!("WALL_FIRST_COMMIT {wall}");
            self.retrace("FIRST_COMMIT");
            self.clocks("first_commit");
        }
        self.retrace(&format!("REDRAW us={} scale={scale}", t.elapsed().as_micros()));
    }

    fn pick(&mut self, serial: u32, qh: &QueueHandle<Ui>) {
        let Some(&idx) = self.order.get(self.selected) else { return };
        let id = self.entries[idx].id.clone();
        // The negative control for the fuzzel failure the compositor
        // documents: minting from the bare focus serial must be refused
        // (no `done`), while a real key/button serial is accepted.
        let serial = if self.opts.use_focus_serial {
            self.retrace(&format!("PICK id={id} focus-serial={} (expect refusal)", self.kb.focus_serial));
            self.kb.focus_serial
        } else {
            self.retrace(&format!("PICK id={id} serial={serial}"));
            serial
        };
        let (Some(act), Some(surface), Some(seat)) = (
            self.globals.activation.clone(),
            self.surface.clone(),
            self.globals.seat.clone(),
        ) else {
            self.retrace("PICK no-activation-global");
            self.finish_launch(None);
            return;
        };
        let token_obj = act.get_activation_token(qh, ());
        token_obj.set_serial(serial, &seat);
        token_obj.set_app_id(NS.to_string());
        token_obj.set_surface(&surface);
        token_obj.commit();
        self.token_obj = Some(token_obj);
        self.token_deadline = Some(Instant::now() + Duration::from_millis(200));
        self.token = None;
        self.retrace("TOKEN_REQ");
    }

    fn finish_launch(&mut self, token: Option<String>) {
        if self.launch_done {
            return;
        }
        self.launch_done = true;
        match token {
            Some(t) => {
                self.retrace(&format!("TOKEN_DONE len={}", t.len()));
                if let Some(sock) = self.opts.stub_sock.clone() {
                    match UnixStream::connect(&sock) {
                        Ok(mut s) => {
                            let _ = s.write_all(format!("ACTIVATE {t}\n").as_bytes());
                            self.retrace("ACTIVATE_SENT");
                        }
                        Err(e) => self.retrace(&format!("ACTIVATE_CONNECT_FAIL {e}")),
                    }
                    self.exit_code = 0;
                    self.should_exit = true;
                } else if self.opts.spawn_stub {
                    let exe = std::env::current_exe().unwrap();
                    let mut cmd = Command::new(&exe);
                    cmd.arg("--stub")
                        .arg("--app-id")
                        .arg("w9.launch.stub")
                        .env("XDG_ACTIVATION_TOKEN", &t)
                        .env("WAYLAND_DISPLAY", std::env::var("WAYLAND_DISPLAY").unwrap_or_default())
                        .env(
                            "XDG_RUNTIME_DIR",
                            std::env::var("XDG_RUNTIME_DIR").unwrap_or_default(),
                        );
                    // The child reuses the panel's trace path with a suffix,
                    // so its redeem is on the record.
                    if let Some(tp) = &self.opts.trace {
                        let mut s = tp.as_os_str().to_owned();
                        s.push("-spawn");
                        cmd.arg("--trace").arg(s);
                    }
                    match cmd.spawn() {
                        Ok(child) => self.retrace(&format!("STUB_SPAWNED pid={}", child.id())),
                        Err(e) => self.retrace(&format!("STUB_SPAWN_FAIL {e}")),
                    }
                    self.exit_code = 0;
                    self.should_exit = true;
                } else {
                    println!("TOKEN {t}");
                    self.exit_code = 0;
                    self.should_exit = true;
                }
            }
            None => {
                self.retrace("TOKEN_TIMEOUT launch-without-token");
                self.exit_code = 0;
                self.should_exit = true;
            }
        }
    }

    fn push_char(&mut self, c: char, qh: &QueueHandle<Ui>) {
        if c.is_control() {
            return;
        }
        self.query.push(c);
        self.rescore();
        self.redraw(qh);
    }

    fn backspace(&mut self, qh: &QueueHandle<Ui>) {
        // Char-boundary pop: UTF-8 aware caret is a later ticket's widget.
        while !self.query.is_empty() {
            self.query.pop();
            if self.query.is_char_boundary(self.query.len()) {
                break;
            }
        }
        self.rescore();
        self.redraw(qh);
    }

    fn move_sel(&mut self, delta: i32, qh: &QueueHandle<Ui>) {
        if self.order.is_empty() {
            return;
        }
        let n = self.order.len();
        let cur = self.selected.min(n - 1);
        self.selected = (cur as i32 + delta).clamp(0, n as i32 - 1) as usize;
        self.redraw(qh);
    }
}

// --- Dispatch impls ---

impl Dispatch<WlRegistry, ()> for Ui {
    fn event(
        ui: &mut Ui,
        registry: &WlRegistry,
        event: <WlRegistry as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Ui>,
    ) {
        use wayland_client::protocol::wl_registry::Event as RegEvent;
        if let RegEvent::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    ui.globals.compositor =
                        Some(registry.bind::<WlCompositor, _, _>(name, version.min(6), qh, ()));
                }
                "wl_shm" => {
                    ui.globals.shm = Some(registry.bind::<WlShm, _, _>(name, 1, qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    ui.globals.layer_shell =
                        Some(registry.bind::<ZwlrLayerShellV1, _, _>(name, version.min(4), qh, ()));
                }
                "wl_seat" => {
                    ui.globals.seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(9), qh, ()));
                }
                "wl_output" => {
                    registry.bind::<WlOutput, _, _>(name, version.min(4), qh, ());
                }
                "xdg_activation_v1" => {
                    ui.globals.activation =
                        Some(registry.bind::<XdgActivationV1, _, _>(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(Ui: ignore WlCompositor);
delegate_noop!(Ui: ignore WlShm);
delegate_noop!(Ui: ignore WlShmPool);
delegate_noop!(Ui: ignore ZwlrLayerShellV1);
delegate_noop!(Ui: ignore XdgActivationV1);

impl Dispatch<WlBuffer, ()> for Ui {
    fn event(
        ui: &mut Ui,
        buffer: &WlBuffer,
        event: <WlBuffer as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Ui>,
    ) {
        use wayland_client::protocol::wl_buffer::Event as BufEvent;
        if matches!(event, BufEvent::Release) {
            let mut freed_idx = None;
            for (i, slot) in ui.slots.iter_mut().enumerate() {
                if slot.buffer.as_ref() == Some(buffer) {
                    slot.busy = false;
                    freed_idx = Some(i);
                }
            }
            if let Some(i) = freed_idx {
                ui.retrace(&format!("RELEASE slot={i}"));
            }
            if !ui.presented {
                ui.presented = true;
                ui.retrace("FIRST_RELEASE presented");
            }
            if freed_idx.is_some() && ui.dirty {
                ui.retrace("REDRAW_RETRY dirty");
                ui.redraw(qh);
            }
        }
    }
}

impl Dispatch<WlOutput, ()> for Ui {
    fn event(
        ui: &mut Ui,
        _: &WlOutput,
        event: <WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Ui>,
    ) {
        use wayland_client::protocol::wl_output::Event as OutEvent;
        if let OutEvent::Scale { factor } = event {
            ui.globals.output_scale = ui.globals.output_scale.max(factor as u32).max(1);
        }
    }
}

impl Dispatch<WlSurface, ()> for Ui {
    fn event(
        _: &mut Ui,
        _: &WlSurface,
        _: <WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Ui>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Ui {
    fn event(
        ui: &mut Ui,
        layer: &ZwlrLayerSurfaceV1,
        event: LayerEvent,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Ui>,
    ) {
        match event {
            LayerEvent::Configure { serial, .. } => {
                layer.ack_configure(serial);
                ui.retrace("CONFIGURE");
                ui.scale = ui.globals.output_scale.max(1);
                ui.redraw(qh);
            }
            LayerEvent::Closed => {
                ui.retrace("LAYER_CLOSED");
                ui.exit_code = 1;
                ui.should_exit = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for Ui {
    fn event(
        ui: &mut Ui,
        seat: &WlSeat,
        event: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Ui>,
    ) {
        use wayland_client::protocol::wl_seat::Event as SeatEvent;
        if let SeatEvent::Capabilities { capabilities } = event {
            if let wayland_client::WEnum::Value(caps) = capabilities {
                if caps.contains(SeatCapability::Keyboard) && ui.keyboard.is_none() {
                    ui.keyboard = Some(seat.get_keyboard(qh, ()));
                    ui.retrace("SEAT keyboard");
                }
                if caps.contains(SeatCapability::Pointer) && ui.pointer.is_none() {
                    ui.pointer = Some(seat.get_pointer(qh, ()));
                    ui.retrace("SEAT pointer");
                }
            }
        }
    }
}

impl Dispatch<WlKeyboard, ()> for Ui {
    fn event(
        ui: &mut Ui,
        _: &WlKeyboard,
        event: <WlKeyboard as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Ui>,
    ) {
        use wayland_client::protocol::wl_keyboard::Event as KbEvent;
        match event {
            KbEvent::Keymap { fd, size, .. } => {
                use rustix::fs::{SeekFrom, seek};
                // The compositor hands every client one shared description,
                // so the offset is wherever the last reader left it: seek
                // back first (keyboard-spike finding 1), or we read empty.
                let _ = seek(&fd, SeekFrom::Start(0));
                let mut buf = vec![0u8; size as usize];
                let mut got = 0;
                while got < buf.len() {
                    match rustix::io::read(&fd, &mut buf[got..]) {
                        Ok(0) => break,
                        Ok(n) => got += n,
                        Err(_) => break,
                    }
                }
                buf.truncate(got);
                let text = String::from_utf8_lossy(&buf).into_owned();
                let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                if let Some(keymap) = xkb::Keymap::new_from_string(
                    &ctx,
                    text,
                    xkb::KEYMAP_FORMAT_TEXT_V1,
                    xkb::KEYMAP_COMPILE_NO_FLAGS,
                ) {
                    ui.retrace(&format!("KEYMAP bytes={got} lang={:?}", std::env::var("LANG").ok()));
                    let lang = compose_locale();
                    let compose = xkb::compose::Table::new_from_locale(
                        &ctx,
                        std::ffi::OsStr::new(&lang),
                        xkb::compose::COMPILE_NO_FLAGS,
                    )
                    .ok()
                    .map(|t| xkb::compose::State::new(&t, xkb::compose::STATE_NO_FLAGS));
                    ui.kb.state = Some(xkb::State::new(&keymap));
                    ui.kb.compose = compose;
                } else {
                    ui.retrace("KEYMAP_COMPILE_FAIL");
                }
            }
            KbEvent::Enter { serial, .. } => {
                ui.kb.focus_serial = serial;
                ui.retrace(&format!("KBD_ENTER serial={serial}"));
            }
            KbEvent::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
                if let Some(st) = ui.kb.state.as_mut() {
                    st.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
            }
            KbEvent::RepeatInfo { rate, delay } => {
                ui.retrace(&format!("REPEAT rate={rate} delay={delay}"));
            }
            KbEvent::Key { serial, key, state: key_state, .. } => {
                use wayland_client::protocol::wl_keyboard::KeyState;
                let code: Keycode = (key + 8).into();
                let down = matches!(key_state, wayland_client::WEnum::Value(KeyState::Pressed));
                if let Some(st) = ui.kb.state.as_mut() {
                    st.update_key(
                        code,
                        if down { xkb::KeyDirection::Down } else { xkb::KeyDirection::Up },
                    );
                }
                if !down {
                    return;
                }
                ui.kb.last_serial = serial;
                let raw: u32 = ui
                    .kb
                    .state
                    .as_ref()
                    .map(|st| st.key_get_one_sym(code))
                    .unwrap_or_else(|| xkb::Keysym::from(0u32))
                    .into();
                ui.retrace(&format!("KEY code={key} sym=0x{raw:x}"));
                let sym = xkb::Keysym::from(raw);
                // Compose first: dead keys resolve through the table.
                let (fed, status, composed) = match ui.kb.compose.as_mut() {
                    Some(cs) => {
                        use xkb::compose::FeedResult;
                        let fed = cs.feed(sym);
                        let status = cs.status();
                        let composed = cs.utf8();
                        (Some(fed), Some(status), composed)
                    }
                    None => (None, None, None),
                };
                use xkb::compose::{FeedResult, Status};
                if let (Some(fed), Some(status)) = (fed, status) {
                    ui.retrace(&format!("COMPOSE fed={fed:?} status={status:?}"));
                    if matches!(fed, FeedResult::Accepted) {
                        match status {
                            Status::Composed => {
                                if let Some(s) = composed {
                                    for c in s.chars() {
                                        ui.push_char(c, qh);
                                    }
                                }
                                return;
                            }
                            Status::Composing => return, // dead key: wait for more
                            _ => {}
                        }
                    }
                }
                match raw {
                    KEY_ESC => {
                        ui.retrace("EXIT escape");
                        ui.exit_code = 1;
                        ui.should_exit = true;
                    }
                    KEY_RETURN => ui.pick(serial, qh),
                    KEY_BS => ui.backspace(qh),
                    KEY_UP => ui.move_sel(-1, qh),
                    KEY_DOWN => ui.move_sel(1, qh),
                    _ => {
                        let n = ui.kb.state.as_ref().map(|st| st.key_get_utf32(code)).unwrap_or(0);
                        if let Some(c) = char::from_u32(n) {
                            ui.push_char(c, qh);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, ()> for Ui {
    fn event(
        ui: &mut Ui,
        _: &WlPointer,
        event: <WlPointer as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Ui>,
    ) {
        use wayland_client::protocol::wl_pointer::Event as PtrEvent;
        match event {
            PtrEvent::Enter { surface_y, .. } => {
                ui.pointer_y = Some(surface_y);
            }
            PtrEvent::Motion { surface_y, .. } => {
                ui.pointer_y = Some(surface_y);
            }
            PtrEvent::Button { serial, button, state, .. } => {
                use wayland_client::protocol::wl_pointer::ButtonState;
                if button == BTN_LEFT && matches!(state, wayland_client::WEnum::Value(ButtonState::Pressed)) {
                    if let Some(y) = ui.pointer_y {
                        let row =
                            (y as u32).saturating_sub(draw::QUERY_H + draw::PAD) / draw::ROW_H;
                        if (row as usize) < ui.order.len().min(draw::VISIBLE) {
                            ui.selected = row as usize;
                        }
                        ui.retrace(&format!("CLICK y={y} row={row}"));
                        ui.pick(serial, qh);
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgActivationTokenV1, ()> for Ui {
    fn event(
        ui: &mut Ui,
        _: &XdgActivationTokenV1,
        event: TokenEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Ui>,
    ) {
        if let TokenEvent::Done { token } = event {
            ui.token = Some(token);
        }
    }
}

/// Locale for the compose table. A `C`/`POSIX`/unset locale gives
/// libxkbcommon's minimal table, which swallows plain letters into a
/// sequence that never completes; a launcher must type under any locale,
/// so fall back to en_US.UTF-8 for the table only (never the keymap).
fn compose_locale() -> String {
    match std::env::var("LANG") {
        Ok(lang) => {
            let base = lang.split('.').next().unwrap_or("");
            if base.is_empty() || base == "C" || base == "POSIX" {
                "en_US.UTF-8".to_string()
            } else {
                lang
            }
        }
        Err(_) => "en_US.UTF-8".to_string(),
    }
}

/// No-compositor check of the exact compose glue the Key handler uses:
/// RMLVO keymap straight from xkeyboard-config, dead_acute + e in, é out.
pub fn selftest_compose() -> i32 {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &ctx,
        "",
        "",
        "us",
        "intl",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    );
    let Some(keymap) = keymap else {
        eprintln!("SELFTEST no keymap for us(intl)");
        return 1;
    };
    let lang = compose_locale();
    let table = xkb::compose::Table::new_from_locale(
        &ctx,
        std::ffi::OsStr::new(&lang),
        xkb::compose::COMPILE_NO_FLAGS,
    );
    let Ok(table) = table else {
        eprintln!("SELFTEST no compose table for lang={lang:?}");
        return 1;
    };
    let mut cs = xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS);
    use xkb::compose::{FeedResult, Status};
    // dead_acute then e, the keysyms a real us(intl) keypress pair produces.
    for sym in [0xfe51u32, 0x65] {
        let fed = cs.feed(xkb::Keysym::from(sym));
        let status = cs.status();
        eprintln!("SELFTEST feed=0x{sym:x} fed={fed:?} status={status:?} utf8={:?}", cs.utf8());
        if !matches!(fed, FeedResult::Accepted) {
            eprintln!("SELFTEST not accepted");
            return 1;
        }
        let _ = status;
    }
    match cs.utf8().as_deref() {
        Some("é") => {
            eprintln!("SELFTEST composed=é OK (lang={lang:?}, keymap-rules=us+intl)");
            0
        }
        other => {
            eprintln!("SELFTEST composed={other:?} FAIL");
            1
        }
    }
}

pub fn run(opts: Opts, entries: Vec<Entry>) -> i32 {
    let t0 = Instant::now();
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    eprintln!("WALL_START {wall}");
    let log = opts.trace.clone().and_then(|p| {
        std::fs::OpenOptions::new().create(true).append(true).open(p).ok()
    });
    // Font load is pure file I/O: do it before any Wayland round trip.
    let text = crate::font::text(Some(opts.font.as_path()), &[]).expect("font load");
    let conn = Connection::connect_to_env().expect("wayland connect");
    let mut queue: EventQueue<Ui> = conn.new_event_queue();
    let qh = queue.handle();
    let mut ui = Ui {
        t0,
        log,
        text,
        entries,
        query: String::new(),
        order: Vec::new(),
        selected: 0,
        opts,
        globals: Globals {
            compositor: None,
            shm: None,
            layer_shell: None,
            seat: None,
            activation: None,
            output_scale: 1,
        },
        surface: None,
        keyboard: None,
        pointer: None,
        pointer_y: None,
        kb: Kb { state: None, compose: None, last_serial: 0, focus_serial: 0 },
        slots: [
            Slot { shm: None, pool: None, buffer: None, busy: false, w: 0, h: 0 },
            Slot { shm: None, pool: None, buffer: None, busy: false, w: 0, h: 0 },
        ],
        first_commit: false,
        presented: false,
        dirty: false,
        scale: 1,
        token: None,
        token_obj: None,
        token_deadline: None,
        launch_done: false,
        exit_code: 0,
        should_exit: false,
    };
    let _registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut ui).expect("roundtrip");
    ui.retrace("START");
    ui.clocks("start");
    ui.rescore();

    let compositor = ui.globals.compositor.clone().expect("no wl_compositor");
    let layer_shell = ui.globals.layer_shell.clone().expect("no layer shell");
    let surface = compositor.create_surface(&qh, ());
    let layer =
        layer_shell.get_layer_surface(&surface, None, ShellLayer::Overlay, NS.to_string(), &qh, ());
    layer.set_anchor(Anchor::Top);
    layer.set_size(draw::W, draw::H);
    layer.set_exclusive_zone(0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    surface.commit();
    ui.surface = Some(surface);
    ui.retrace("SURFACE_COMMITTED");
    println!("READY");

    let backend = conn.backend();
    let wayland_fd = backend.poll_fd();
    let quit_at = ui.opts.quit_after_ms.map(|ms| t0 + Duration::from_millis(ms));
    loop {
        queue.dispatch_pending(&mut ui).expect("dispatch");
        // Token wait: the compositor answers `done` within the flow.
        if let Some(tok) = ui.token.take() {
            if let Some(o) = ui.token_obj.take() {
                o.destroy();
            }
            ui.token_deadline = None;
            ui.finish_launch(Some(tok));
        } else if ui.token_deadline.map(|d| Instant::now() >= d).unwrap_or(false) {
            ui.token_deadline = None;
            ui.token_obj.take().map(|o| o.destroy());
            ui.finish_launch(None);
        }
        if ui.should_exit {
            break;
        }
        if let Some(end) = quit_at {
            if Instant::now() >= end {
                ui.retrace("QUIT_TIMER");
                break;
            }
        }
        conn.flush().expect("flush");
        let timeout_ms: i32 = if ui.token_deadline.is_some() {
            20
        } else if let Some(end) = quit_at {
            end.saturating_duration_since(Instant::now()).as_millis().min(500) as i32
        } else {
            500
        };
        let timeout = Timespec { tv_sec: 0, tv_nsec: (timeout_ms as i64).saturating_mul(1_000_000) };
        let mut fds = [PollFd::from_borrowed_fd(wayland_fd.as_fd(), PollFlags::IN)];
        let _ = poll(&mut fds, (!ui.should_exit).then_some(&timeout));
        // Read whatever arrived; WouldBlock is fine (spurious wakeup).
        if let Some(guard) = queue.prepare_read() {
            let _ = guard.read();
        }
    }
    ui.retrace(&format!("EXIT code={}", ui.exit_code));
    ui.exit_code
}
