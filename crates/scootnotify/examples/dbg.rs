//! Temporary debug: 500 rapid Notifies. Deleted before the PR.
use scoot_dbus::conn::{self, Event};
use scoot_dbus::proto::Writer;

fn main() {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap();
    let path = address.strip_prefix("unix:path=").unwrap().to_owned();
    let mut conn = conn::connect(std::path::Path::new(&path)).unwrap();
    let mut ids = 0;
    for index in 0..500u64 {
        let mut out = Writer::new();
        out.str("Flood");
        out.u32(0);
        out.str("");
        out.str("s");
        out.str("b");
        let cookie = out.open_array(4).unwrap();
        out.close_array(cookie);
        let hints = out.open_array(8).unwrap();
        out.close_array(hints);
        out.i32(60000);
        let body = out.take_body().unwrap();
        let token = 7000 + index;
        conn.call(
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
            "susssasa{sv}i",
            &body,
            0,
            token,
        )
        .unwrap();
        loop {
            let (events, _) = conn.pump();
            let mut done = false;
            for event in events {
                if let Event::Reply { token: got, .. } = event {
                    if got == token {
                        ids += 1;
                        done = true;
                    }
                }
            }
            if done {
                break;
            }
        }
    }
    eprintln!("flood: {ids}/500 answered");
}
