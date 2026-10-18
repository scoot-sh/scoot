//! Temporary debug: a 5 MiB Notify sender with timing. Deleted before the PR.
use std::time::Instant;

use scoot_dbus::conn::{self, Event};
use scoot_dbus::proto::Writer;

fn main() {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap();
    let path = address.strip_prefix("unix:path=").unwrap().to_owned();
    let mut conn = conn::connect(std::path::Path::new(&path)).unwrap();
    conn.set_max_message(6 * 1024 * 1024);
    let big = "b".repeat(5 * 1024 * 1024);
    let mut out = Writer::with_cap(6 * 1024 * 1024);
    out.begin_call(
        500,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "Notify",
        "susssasa{sv}i",
        0,
    );
    out.str("Huge");
    out.u32(0);
    out.str("");
    out.str("big");
    out.str(&big);
    let actions = out.open_array(4).unwrap();
    out.close_array(actions);
    let hints = out.open_array(8).unwrap();
    out.close_array(hints);
    out.i32(-1);
    let frame = out.finish().unwrap();
    eprintln!("frame: {} bytes", frame.len());
    conn.queue_raw(&frame);
    let start = Instant::now();
    while conn.want_write() {
        let _ = conn.pump();
        if conn.dead() {
            eprintln!("CLIENT DEAD while flushing after {:?}", start.elapsed());
            std::process::exit(2);
        }
    }
    eprintln!("flushed after {:?}", start.elapsed());
    loop {
        if start.elapsed() > std::time::Duration::from_secs(10) {
            eprintln!("TIMEOUT waiting for the answer");
            std::process::exit(3);
        }
        if conn.dead() {
            eprintln!("CLIENT DEAD while waiting");
            std::process::exit(4);
        }
        let (events, _) = conn.pump();
        for event in events {
            match event {
                Event::CallError { name, .. } => {
                    eprintln!("error {name} after {:?}", start.elapsed());
                    return;
                }
                Event::Reply { .. } => {
                    eprintln!("reply after {:?}", start.elapsed());
                    return;
                }
                Event::Dropped { token } => {
                    eprintln!("dropped {token} after {:?}", start.elapsed());
                    return;
                }
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
