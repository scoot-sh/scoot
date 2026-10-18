#![forbid(unsafe_code)]
//! Spike backend 2: `xkbcommon` 0.9.0 (the crate over libxkbcommon, the
//! reference implementation every compositor and toolkit uses) as a
//! Wayland keyboard client.
//!
//! Client pattern, the same one scoot's own tests use
//! (`crates/scoot/src/compositor/input/tests.rs`): `Context` +
//! `Keymap::new_from_string` (the `wl_keyboard.keymap` fd) or
//! `Keymap::new_from_names` (RMLVO) -> `State::new`; the compositor's
//! `modifiers` event into `update_mask`; `key_get_one_sym` for the
//! keysym and `key_get_utf8` for text. Compose through `compose::Table`
//! plus `compose::State`, fed per press exactly the way a toolkit does.

use std::time::Instant;

use spike::{LAYOUTS, escape, mean_ms};
use xkbcommon::xkb;
use xkbcommon::xkb::compose::{self, Status};

fn compile_names(layout: &str, variant: &str, options: Option<&str>) -> Result<xkb::Keymap, String> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        layout,
        variant,
        options.map(str::to_owned),
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .ok_or_else(|| format!("keymap_from_names failed for {layout}/{variant}"))
}

fn compile_string(text: &str) -> Result<xkb::Keymap, String> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_string(
        &context,
        text.to_owned(),
        xkb::KEYMAP_FORMAT_TEXT_V1,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .ok_or_else(|| "keymap_from_string failed".to_owned())
}

/// The Shift bit of a depressed-mods mask, resolved through the keymap
/// itself rather than assumed.
fn shift_bit(keymap: &xkb::Keymap) -> u32 {
    let idx = keymap.mod_get_index("Shift");
    if idx == xkb::MOD_INVALID { 1 } else { 1 << idx }
}

fn decode(
    keymap: &xkb::Keymap,
    state: &xkb::State,
    evdev: u32,
    shift: bool,
    _group: u32,
) -> (String, String) {
    let _ = keymap;
    let code = xkb::Keycode::new(evdev + 8);
    let sym = state.key_get_one_sym(code);
    let text = state.key_get_utf8(code);
    let _ = shift;
    (format!("{sym:?}"), text)
}

fn state_for(keymap: &xkb::Keymap, shift: bool, group: u32) -> xkb::State {
    let mut state = xkb::State::new(keymap);
    let depressed = if shift { shift_bit(keymap) } else { 0 };
    state.update_mask(depressed, 0, 0, 0, 0, group);
    state
}

fn cmd_vectors() {
    for case in LAYOUTS {
        let keymap = compile_names(case.layout, case.variant, case.options)
            .unwrap_or_else(|e| panic!("{}: {e}", case.id));
        for p in case.probes {
            let state = state_for(&keymap, p.shift, p.group);
            let (sym, text) = decode(&keymap, &state, p.evdev, p.shift, p.group);
            println!(
                "layout={} evdev={} shift={} group={} keysym={} text={}",
                case.id,
                p.evdev,
                p.shift as u8,
                p.group,
                sym,
                escape(&text),
            );
        }
    }
}

fn cmd_compile(iters: usize) {
    for case in LAYOUTS {
        let mut samples = Vec::with_capacity(iters);
        for _ in 0..iters {
            let t = Instant::now();
            let _ = compile_names(case.layout, case.variant, case.options)
                .unwrap_or_else(|e| panic!("{}: {e}", case.id));
            samples.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "compile backend=xkbcommon layout={} iters={} mean_ms={:.3}",
            case.id,
            iters,
            mean_ms(&samples),
        );
    }
}

fn cmd_keymap_file(path: &str) {
    let bytes = std::fs::read(path).expect("keymap file reads");
    let text = String::from_utf8(bytes.clone()).expect("keymap is text");
    let t = Instant::now();
    let keymap = compile_string(&text).expect("scoot keymap compiles under xkbcommon");
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("keymap_bytes={} compile_ms={:.3}", bytes.len(), ms);
    for (evdev, shift, group) in [(30u32, false, 0u32), (30, true, 0), (2, true, 0)] {
        let state = state_for(&keymap, shift, group);
        let (sym, text) = decode(&keymap, &state, evdev, shift, group);
        println!("live evdev={evdev} shift={} group={group} keysym={sym} text={}", shift as u8, escape(&text));
    }
}

/// Feed one keysym through a compose state and report the status, the way
/// a toolkit does per press.
fn feed(state: &mut compose::State, keymap_state: &xkb::State, label: &str, sym: xkb::Keysym) {
    let _ = keymap_state;
    state.feed(sym);
    match state.status() {
        Status::Composed => println!(
            "feed {label}: composed string={}",
            escape(&state.utf8().unwrap_or_default()),
        ),
        Status::Composing => println!("feed {label}: composing"),
        Status::Cancelled => println!("feed {label}: cancelled"),
        Status::Nothing => println!("feed {label}: nothing"),
    }
}

fn cmd_compose() {
    let locale = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_CTYPE"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    println!("locale={locale}");
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    // The session-locale path: what a launcher/greeter would actually use.
    match compose::Table::new_from_locale(
        &context,
        std::ffi::OsStr::new(&locale),
        compose::COMPILE_NO_FLAGS,
    ) {
        Ok(table) => {
            println!("compose_table=ok");
            let keymap = compile_names("us", "intl", None).expect("us(intl) compiles");
            let probe = |evdev: u32, label: &str, st: &mut compose::State| {
                let ks = state_for(&keymap, false, 0).key_get_one_sym(xkb::Keycode::new(evdev + 8));
                let s = state_for(&keymap, false, 0);
                feed(st, &s, label, ks);
            };
            let mut st = compose::State::new(&table, compose::STATE_NO_FLAGS);
            probe(40, "dead_acute", &mut st);
            probe(18, "e", &mut st);
            let mut st = compose::State::new(&table, compose::STATE_NO_FLAGS);
            probe(40, "dead_acute", &mut st);
            probe(40, "dead_acute", &mut st);
        }
        Err(()) => println!("compose_table=missing"),
    }
}

fn cmd_sleep(secs: u64) {
    let keymap = compile_names("us", "", None).expect("us compiles");
    let state = state_for(&keymap, false, 0);
    let (sym, text) = decode(&keymap, &state, 30, false, 0);
    println!("ready keysym={sym} text={}", escape(&text));
    std::thread::sleep(std::time::Duration::from_secs(secs));
}

struct XkbClient {
    keymap: xkb::Keymap,
    state: xkb::State,
}

impl spike::wayland::ClientBackend for XkbClient {
    fn compile(keymap_bytes: &[u8]) -> Result<Self, String> {
        let text =
            std::str::from_utf8(keymap_bytes).map_err(|e| format!("keymap not utf8: {e}"))?;
        let keymap = compile_string(text)?;
        let state = xkb::State::new(&keymap);
        Ok(XkbClient { keymap, state })
    }

    fn decode(
        &self,
        evdev: u32,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> (String, String) {
        let mut state = self.state.clone();
        state.update_mask(depressed, latched, locked, 0, 0, group);
        let _ = &self.keymap;
        decode(&self.keymap, &state, evdev, false, group)
    }
}

fn usage() -> ! {
    eprintln!("usage: spike-xkb <vectors|compile [N]|keymap-file PATH|compose|sleep SECS|wayland [SECS]|focused SECS]>");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }
    match args[1].as_str() {
        "vectors" => cmd_vectors(),
        "compile" => cmd_compile(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20)),
        "keymap-file" => {
            if args.len() < 3 {
                usage();
            }
            cmd_keymap_file(&args[2]);
        }
        "compose" => cmd_compose(),
        "sleep" => cmd_sleep(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60)),
        "focused" => spike::wayland::run_focused::<XkbClient>(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30)),
        "wayland" => {
            spike::wayland::run::<XkbClient>(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0))
        }
        _ => usage(),
    }
}
