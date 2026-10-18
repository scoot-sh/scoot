#![forbid(unsafe_code)]
//! Baseline 2: the Wayland glue every client pays, without a keyboard stack.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    // Touch wayland-client so it is really linked, then idle.
    let _ = wayland_client::Connection::connect_to_env().is_ok();
    println!("ready wayland-only");
    std::thread::sleep(std::time::Duration::from_secs(secs));
}
