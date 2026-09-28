//! A live `set` whose image decodes but cannot be drawn: the reply is an
//! error, `query` reports `draw_failed` with the reason (`draw_error`),
//! the output keeps what it showed, the daemon lives, and the next `set`
//! draws.
//!
//! What fails is the one allocation a draw makes that is not the image's:
//! the `wl_shm` buffer, a sealed memfd mapped the size of the output. The
//! daemon's address-space limit (`RLIMIT_AS`, its soft value only, so it
//! can be raised back without privilege) is lowered, once it is idle, to
//! what it maps plus [`MARGIN`]: room for the decoding thread and a 1×1
//! PNG, but not for a 3840×2160 buffer (33 MB). `center` scales nothing,
//! so the buffer is the draw's only large allocation, and it is fallible
//! (`mmap` failing is an error return, `ShmError::Io`), where a scaler's
//! or decoder's `Vec` refused by the allocator would abort the daemon.
//! This is the out-of-memory case the ticket names; a buffer too large for
//! `wl_shm` itself needs an output of over 536 million pixels, which no
//! headless compositor here can be asked for without allocating it too.
#![cfg(target_os = "linux")]

mod common;

use std::path::Path;

use common::Session;
use rustix::process::{Pid, Resource, Rlimit, prlimit};
use serde_json::{Value, json};

/// What the daemon may map beyond what it maps when idle: its decoding
/// thread's stack (2 MiB) and a 1×1 PNG's decode, with room to spare, but
/// half the 3840×2160 buffer.
const MARGIN: u64 = 16 << 20;
const WIDTH: u32 = 3840;
const HEIGHT: u32 = 2160;

fn write_png(path: &Path) {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 0, 255]).unwrap();
        writer.finish().unwrap();
    }
    std::fs::write(path, out).unwrap();
}

/// A number from `/proc/PID/status` (`VmSize:  1234 kB`), in bytes.
fn vm_size(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("VmSize:"))
        .unwrap_or_else(|| panic!("no VmSize in {status}"));
    let kib: u64 = line.trim().trim_end_matches("kB").trim().parse().unwrap();
    kib * 1024
}

/// Sets the soft address-space limit of `pid` (`None`: unlimited),
/// leaving the hard one as it was. Returns the soft limit it had.
fn limit_address_space(pid: u32, soft: Option<u64>) -> Option<u64> {
    let hard = std::fs::read_to_string(format!("/proc/{pid}/limits"))
        .unwrap()
        .lines()
        .find(|line| line.starts_with("Max address space"))
        .and_then(|line| line.split_whitespace().nth(4).map(str::to_owned))
        .unwrap();
    let maximum = (hard != "unlimited").then(|| hard.parse().unwrap());
    let old = prlimit(
        Pid::from_raw(i32::try_from(pid).unwrap()),
        Resource::As,
        Rlimit {
            current: soft,
            maximum,
        },
    )
    .unwrap();
    old.current
}

fn output(session: &Session) -> Value {
    session.query()["outputs"][0].clone()
}

#[test]
fn a_live_set_that_cannot_be_drawn_says_why_in_query() {
    let Some(session) = Session::start_sized("drawfail", WIDTH, HEIGHT) else {
        return;
    };
    let picture = session.scratch.0.join("blue.png");
    write_png(&picture);
    let mut daemon = session.daemon_logged(&[]);
    session.query_until("configured", |o| {
        o.len() == 1 && o[0]["surface"]["pixels"] == json!({"width": WIDTH, "height": HEIGHT})
    });
    let set = |args: &[&str]| session.run(&[&["set"], args].concat());
    let done = set(&["#102030"]);
    assert!(done.status.success(), "{done:?}");
    let before = output(&session);
    assert_eq!(before["shows"], json!({"color": "#102030"}));
    assert_eq!(before["draw_failed"], false);
    assert_eq!(before["draw_error"], Value::Null);

    let pid = daemon.id();
    let limit = vm_size(pid) + MARGIN;
    let old = limit_address_space(pid, Some(limit));
    let failed = set(&[picture.to_str().unwrap(), "--mode", "center"]);
    // Lifted before anything is asserted, so a failure below leaves a
    // daemon that can still be stopped cleanly.
    limit_address_space(pid, old);
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert_eq!(failed.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("could not be drawn"), "{stderr}");
    assert!(stderr.contains("scootbg query"), "{stderr}");

    let after = output(&session);
    assert_eq!(after["draw_failed"], true, "{after}");
    let why = after["draw_error"].as_str().unwrap_or_default();
    assert!(
        why.starts_with("shared memory:") && why.contains("os error 12"),
        "draw_error {why:?} is not the buffer's ENOMEM (limit {limit} bytes)"
    );
    // What it showed before stays, and is not what was asked.
    assert_eq!(after["shows"], json!({"color": "#102030"}));
    assert!(daemon.try_wait().unwrap().is_none(), "the daemon died");
    let log = std::fs::read_to_string(session.daemon_log()).unwrap();
    assert!(
        log.contains("cannot draw") && log.contains("blue.png") && log.contains(why),
        "stderr and query disagree: {log}"
    );

    // Not retried by itself (no loop), and the next request draws.
    assert_eq!(output(&session)["draw_failed"], true);
    let drawn = set(&[picture.to_str().unwrap(), "--mode", "center"]);
    assert!(drawn.status.success(), "{drawn:?}");
    let now = output(&session);
    assert_eq!(now["draw_failed"], false, "{now}");
    assert_eq!(now["draw_error"], Value::Null);
    assert_eq!(now["shows"]["mode"], "center");

    let killed = session.run(&["kill"]);
    assert!(killed.status.success(), "{killed:?}");
    assert!(common::wait_exit(&mut daemon).success());
}
