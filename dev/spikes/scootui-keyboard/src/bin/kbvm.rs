#![forbid(unsafe_code)]
//! Spike backend 1: pure-Rust `kbvm` 0.2.0 as a Wayland keyboard client.
//!
//! Client pattern from kbvm's own docs: `Context` + `keymap_from_bytes`
//! (what arrives on the `wl_keyboard.keymap` fd) or `keymap_from_names`
//! (RMLVO, for the layout sweep) -> `to_builder` -> `build_lookup_table`;
//! effective mods/group straight from the compositor's `modifiers` event
//! into `LookupTable::lookup`; `KeysymProps::char` for text (no heap
//! string); `ComposeTable` for dead keys / XCompose.

use std::time::Instant;

use kbvm::lookup::LookupTable;
use kbvm::xkb::diagnostic::WriteToLog;
use kbvm::xkb::rmlvo::Group;
use kbvm::xkb::Context;
use kbvm::{GroupIndex, Keycode, ModifierMask};
use spike::{LAYOUTS, escape, mean_ms};

fn compile_names(
    ctx: &Context,
    layout: &str,
    variant: &str,
    options: Option<&str>,
) -> Result<LookupTable, String> {
    let groups: Vec<Group> = Group::from_layouts_and_variants(layout, variant).collect();
    let opt_refs: Option<Vec<&str>> = options.map(|o| vec![o]);
    let map = ctx.keymap_from_names(
        WriteToLog,
        None,
        None,
        Some(&groups),
        opt_refs.as_deref(),
    );
    let table = map.to_builder().build_lookup_table();
    Ok(table)
}

fn compile_bytes(ctx: &Context, bytes: &[u8]) -> Result<LookupTable, String> {
    let map = ctx
        .keymap_from_bytes(WriteToLog, None, bytes)
        .map_err(|e| format!("keymap_from_bytes: {e:?}"))?;
    let table = map.to_builder().build_lookup_table();
    Ok(table)
}

fn decode(table: &LookupTable, evdev: u32, shift: bool, group: u32) -> (String, String) {
    let mods = if shift { ModifierMask::SHIFT } else { ModifierMask::NONE };
    let lookup = table.lookup(GroupIndex(group), mods, Keycode::from_evdev(evdev));
    let mut sym = "NoSymbol".to_string();
    let mut text = String::new();
    for props in lookup.into_iter() {
        sym = format!("{:?}", props.keysym());
        if let Some(c) = props.char() {
            text.push(c);
        }
        break;
    }
    (sym, text)
}

fn cmd_vectors() {
    let ctx = Context::default();
    for case in LAYOUTS {
        let table = compile_names(&ctx, case.layout, case.variant, case.options)
            .unwrap_or_else(|e| panic!("{}: {e}", case.id));
        for p in case.probes {
            let (sym, text) = decode(&table, p.evdev, p.shift, p.group);
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
    let ctx = Context::default();
    for case in LAYOUTS {
        let mut samples = Vec::with_capacity(iters);
        for _ in 0..iters {
            let t = Instant::now();
            let _ = compile_names(&ctx, case.layout, case.variant, case.options)
                .unwrap_or_else(|e| panic!("{}: {e}", case.id));
            samples.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "compile backend=kbvm layout={} iters={} mean_ms={:.3}",
            case.id,
            iters,
            mean_ms(&samples),
        );
    }
}

fn cmd_keymap_file(path: &str) {
    let bytes = std::fs::read(path).expect("keymap file reads");
    let ctx = Context::default();
    let t = Instant::now();
    let table = compile_bytes(&ctx, &bytes).expect("scoot keymap compiles under kbvm");
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("keymap_bytes={} compile_ms={:.3}", bytes.len(), ms);
    // Decode a fixed us-anchored spot check through the live keymap.
    for (evdev, shift, group) in [(30u32, false, 0u32), (30, true, 0), (2, true, 0)] {
        let (sym, text) = decode(&table, evdev, shift, group);
        println!("live evdev={evdev} shift={} group={group} keysym={sym} text={}", shift as u8, escape(&text));
    }
}

fn cmd_compose() {
    use kbvm::syms;
    use kbvm::xkb::compose::FeedResult;

    let ctx = Context::default();
    println!("locale={}", ctx.locale());
    let table = ctx.compose_table_builder().build(WriteToLog);
    let table = match table {
        Some(t) => t,
        None => {
            println!("compose_table=missing");
            return;
        }
    };
    println!("compose_table=ok");
    // Dead acute then e: the us(intl) password-critical sequence.
    let mut st = table.create_state();
    for (label, sym) in [
        ("dead_acute", syms::dead_acute),
        ("e", syms::e),
    ] {
        match table.feed(&mut st, sym) {
            None => println!("feed {label}: none"),
            Some(FeedResult::Pending) => println!("feed {label}: pending"),
            Some(FeedResult::Aborted) => println!("feed {label}: aborted"),
            Some(FeedResult::Composed { string, keysym }) => println!(
                "feed {label}: composed string={} keysym={:?}",
                string.map(|s| escape(&s.to_string())).unwrap_or_default(),
                keysym,
            ),
        }
    }
    // Dead acute twice: the spacing acute on us(intl).
    let mut st = table.create_state();
    for (label, sym) in [("dead_acute", syms::dead_acute), ("dead_acute", syms::dead_acute)] {
        match table.feed(&mut st, sym) {
            None => println!("feed2 {label}: none"),
            Some(FeedResult::Pending) => println!("feed2 {label}: pending"),
            Some(FeedResult::Aborted) => println!("feed2 {label}: aborted"),
            Some(FeedResult::Composed { string, keysym }) => println!(
                "feed2 {label}: composed string={} keysym={:?}",
                string.map(|s| escape(&s.to_string())).unwrap_or_default(),
                keysym,
            ),
        }
    }
}

fn cmd_sleep(secs: u64) {
    // Steady state a launcher/greeter would hold: one compiled us keymap
    // plus its lookup table, then idle.
    let ctx = Context::default();
    let table = compile_names(&ctx, "us", "", None).expect("us compiles");
    let (sym, text) = decode(&table, 30, false, 0);
    println!("ready keysym={sym} text={}", escape(&text));
    std::thread::sleep(std::time::Duration::from_secs(secs));
}

struct KbvmClient(LookupTable);

impl spike::wayland::ClientBackend for KbvmClient {
    fn compile(keymap_bytes: &[u8]) -> Result<Self, String> {
        let ctx = Context::default();
        compile_bytes(&ctx, keymap_bytes).map(KbvmClient)
    }

    fn decode(
        &self,
        evdev: u32,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> (String, String) {
        let mods = ModifierMask(depressed | latched | locked);
        let lookup = self.0.lookup(GroupIndex(group), mods, Keycode::from_evdev(evdev));
        let mut sym = "NoSymbol".to_string();
        let mut text = String::new();
        for props in lookup.into_iter() {
            sym = format!("{:?}", props.keysym());
            if let Some(c) = props.char() {
                text.push(c);
            }
            break;
        }
        (sym, text)
    }
}

fn cmd_wayland(secs: u64) {
    spike::wayland::run::<KbvmClient>(secs);
}

fn usage() -> ! {
    eprintln!("usage: spike-kbvm <vectors|compile [N]|keymap-file PATH|compose|sleep SECS|wayland [SECS]|focused SECS]>");
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
        "focused" => spike::wayland::run_focused::<KbvmClient>(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30)),
        "wayland" => cmd_wayland(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0)),
        _ => usage(),
    }
}
