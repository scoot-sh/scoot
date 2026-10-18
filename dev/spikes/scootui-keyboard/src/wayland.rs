//! The ticket's live leg: bind `wl_seat` v9+, read the keymap fd scoot
//! sends, decode key events through it. Backend-agnostic: each bin plugs
//! its own [`ClientBackend`] in and the Wayland mechanics stay identical.

use std::io::Read;
use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};

use crate::escape;

/// What a backend must do with scoot's keymap: compile it once, then turn
/// key events into (keysym name, text) with the effective mods/group the
/// compositor sent.
pub trait ClientBackend: Sized + 'static {
    fn compile(keymap_bytes: &[u8]) -> Result<Self, String>;
    fn decode(
        &self,
        evdev: u32,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> (String, String);
}

struct State<B> {
    backend: Option<B>,
    keymap_bytes: usize,
    depressed: u32,
    latched: u32,
    locked: u32,
    group: u32,
    keys_seen: u64,
    seat_version: u32,
}

impl<B: ClientBackend> Dispatch<wl_registry::WlRegistry, ()> for State<B> {
    fn event(
        state: &mut State<B>,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<State<B>>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == wl_seat::WlSeat::interface().name
        {
            // The ticket asks for v9+: bind 9 wherever the compositor
            // offers at least that.
            if version < 9 {
                eprintln!("seat_version_offered={version} too old, need >= 9");
                return;
            }
            state.seat_version = 9;
            registry.bind::<wl_seat::WlSeat, _, _>(name, 9, qh, ());
            println!("seat_bind_version=9 offered={version}");
        }
    }
}

impl<B: ClientBackend> Dispatch<wl_seat::WlSeat, ()> for State<B> {
    fn event(
        _: &mut State<B>,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<State<B>>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            eprintln!("seat caps={capabilities:?}");
            if let WEnum::Value(caps) = capabilities
                && caps.contains(wl_seat::Capability::Keyboard)
            {
                eprintln!("keyboard cap present, getting keyboard");
                seat.get_keyboard(qh, ());
            }
        }
    }
}

impl<B: ClientBackend> Dispatch<wl_keyboard::WlKeyboard, ()> for State<B> {
    fn event(
        state: &mut State<B>,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State<B>>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(format),
                fd,
                ..
            } => {
                if format != wl_keyboard::KeymapFormat::XkbV1 {
                    println!("keymap_format=unsupported");
                    return;
                }
                let mut file = std::fs::File::from(fd);
                let mut bytes = Vec::new();
                match file.read_to_end(&mut bytes) {
                    Ok(_) => {
                        state.keymap_bytes = bytes.len();
                        // Test hook only: persist the exact bytes scoot sent
                        // so `keymap-file` can replay them through both stacks.
                        if let Ok(path) = std::env::var("SPIKE_SAVE_KEYMAP") {
                            let _ = std::fs::write(&path, &bytes);
                        }
                        let t = Instant::now();
                        match B::compile(&bytes) {
                            Ok(b) => {
                                let ms = t.elapsed().as_secs_f64() * 1000.0;
                                println!("keymap_bytes={} compile_ms={:.3}", bytes.len(), ms);
                                state.backend = Some(b);
                            }
                            Err(e) => println!("keymap_compile=failed {e}"),
                        }
                    }
                    Err(e) => println!("keymap_read=failed {e}"),
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                state.depressed = mods_depressed;
                state.latched = mods_latched;
                state.locked = mods_locked;
                state.group = group;
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                if key_state == wl_keyboard::KeyState::Pressed {
                    state.keys_seen += 1;
                    match &state.backend {
                        Some(b) => {
                            let (sym, text) = b.decode(
                                key,
                                state.depressed,
                                state.latched,
                                state.locked,
                                state.group,
                            );
                            println!(
                                "key evdev={key} group={} keysym={sym} text={}",
                                state.group,
                                escape(&text),
                            );
                        }
                        None => println!("key evdev={key} no_keymap_yet"),
                    }
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                println!("repeat rate={rate} delay={delay}");
            }
            _ => {}
        }
    }
}

/// Connect, bind the seat, decode for `secs` seconds, print what arrived.
/// `secs == 0` means one short burst (keymap + repeat info only).
pub fn run<B: ClientBackend>(secs: u64) {
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY connects");
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let display = conn.display();
    display.get_registry(&qh, ());
    let mut state = State::<B> {
        backend: None,
        keymap_bytes: 0,
        depressed: 0,
        latched: 0,
        locked: 0,
        group: 0,
        keys_seen: 0,
        seat_version: 0,
    };
    queue.roundtrip(&mut state).expect("registry roundtrip");
    queue.roundtrip(&mut state).expect("seat roundtrip");
    let deadline = Instant::now() + Duration::from_secs(if secs == 0 { 2 } else { secs });
    while Instant::now() < deadline {
        queue.dispatch_pending(&mut state).expect("dispatch");
        std::thread::sleep(Duration::from_millis(10));
    }
    println!(
        "summary keys_seen={} keymap_bytes={} seat_version={}",
        state.keys_seen, state.keymap_bytes, state.seat_version
    );
}
