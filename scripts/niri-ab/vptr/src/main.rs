//! Pointer motion for `scripts/niri-ab-bench.sh`: one `zwlr_virtual_pointer_v1`
//! device, kept for the whole run, alternating absolute motion between two
//! fixed points at a fixed rate.
//!
//! Why not `wlrctl pointer move`: each invocation creates a device, sends one
//! motion and destroys it, so a wlroots host (cage) toggles its seat's pointer
//! capability on and off per event. A nested compositor then never holds a
//! `wl_pointer` long enough to be sent any motion -- measured: 60 `wlrctl`
//! moves delivered zero `wl_pointer.motion` events. One persistent device
//! avoids that.
//!
//! Usage: nab-vptr X1 Y1 X2 Y2 EXTENT_W EXTENT_H RATE_HZ SECS [SETTLE_MS]
//!
//! Coordinates are absolute within an EXTENT_W x EXTENT_H extent that the
//! host maps onto its output layout. Prints `events=N elapsed_s=T` on stdout.
//! Paced against a deadline that never bursts to catch up, so a stall costs
//! events rather than delivering them faster than RATE_HZ.

use std::process::ExitCode;
use std::thread::sleep;
use std::time::{Duration, Instant};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

macro_rules! ignore_events {
    ($($ty:ty),*) => {$(
        impl Dispatch<$ty, ()> for State {
            fn event(
                _: &mut Self,
                _: &$ty,
                _: <$ty as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}
ignore_events!(
    wl_seat::WlSeat,
    ZwlrVirtualPointerManagerV1,
    ZwlrVirtualPointerV1
);

fn parse<T: std::str::FromStr>(args: &[String], i: usize, name: &str) -> Result<T, String> {
    args.get(i)
        .ok_or_else(|| format!("missing {name}"))?
        .parse()
        .map_err(|_| format!("bad {name}: {}", args[i]))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let x1: u32 = parse(&args, 0, "X1")?;
    let y1: u32 = parse(&args, 1, "Y1")?;
    let x2: u32 = parse(&args, 2, "X2")?;
    let y2: u32 = parse(&args, 3, "Y2")?;
    let ext_w: u32 = parse(&args, 4, "EXTENT_W")?;
    let ext_h: u32 = parse(&args, 5, "EXTENT_H")?;
    let rate: f64 = parse(&args, 6, "RATE_HZ")?;
    let secs: f64 = parse(&args, 7, "SECS")?;
    let settle_ms: u64 = if args.len() > 8 {
        parse(&args, 8, "SETTLE_MS")?
    } else {
        500
    };
    if ext_w == 0 || ext_h == 0 || x1 >= ext_w || x2 >= ext_w || y1 >= ext_h || y2 >= ext_h {
        return Err("points must lie inside a non-empty extent".into());
    }
    if !(rate.is_finite() && rate > 0.0 && secs.is_finite() && secs >= 0.0) {
        return Err("RATE_HZ must be > 0 and SECS >= 0".into());
    }

    let conn = Connection::connect_to_env().map_err(|e| format!("connect: {e}"))?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&conn).map_err(|e| format!("registry: {e}"))?;
    let qh = queue.handle();
    let manager: ZwlrVirtualPointerManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .map_err(|e| format!("zwlr_virtual_pointer_manager_v1: {e}"))?;
    let seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("wl_seat: {e}"))?;
    let pointer = manager.create_virtual_pointer(Some(&seat), &qh, ());
    queue
        .roundtrip(&mut State)
        .map_err(|e| format!("roundtrip: {e}"))?;
    // Park on the first point, then give the host time to announce the new
    // pointer capability and the nested compositor time to bind a wl_pointer
    // before the timed run starts.
    pointer.motion_absolute(0, x1, y1, ext_w, ext_h);
    pointer.frame();
    conn.flush().map_err(|e| format!("flush: {e}"))?;
    sleep(Duration::from_millis(settle_ms));

    let period = Duration::from_secs_f64(1.0 / rate);
    let duration = Duration::from_secs_f64(secs);
    let start = Instant::now();
    let mut next = start;
    let mut events: u64 = 0;
    while start.elapsed() < duration {
        let (x, y) = if events.is_multiple_of(2) {
            (x2, y2)
        } else {
            (x1, y1)
        };
        // The protocol's time is a u32 of milliseconds; it wraps by design.
        let time = start.elapsed().as_millis() as u32;
        pointer.motion_absolute(time, x, y, ext_w, ext_h);
        pointer.frame();
        conn.flush().map_err(|e| format!("flush: {e}"))?;
        queue
            .dispatch_pending(&mut State)
            .map_err(|e| format!("dispatch: {e}"))?;
        events += 1;
        next += period;
        let now = Instant::now();
        if next > now {
            sleep(next - now);
        } else {
            next = now;
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    pointer.destroy();
    queue
        .roundtrip(&mut State)
        .map_err(|e| format!("roundtrip: {e}"))?;
    println!("events={events} elapsed_s={elapsed:.3}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nab-vptr: {e}");
            ExitCode::FAILURE
        }
    }
}
