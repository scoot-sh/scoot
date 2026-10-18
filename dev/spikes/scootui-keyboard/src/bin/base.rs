#![forbid(unsafe_code)]
//! Baseline for the spike's RSS rows: no keyboard stack at all, just idle.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    println!("ready baseline");
    std::thread::sleep(std::time::Duration::from_secs(secs));
}
