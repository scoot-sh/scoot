//! Animation canvases fail cleanly under an address-space limit: the
//! RGBA canvas behind a first frame (`gif_first_frame`, and the same
//! seam in the full `gif`/`apng` decodes) is allocated through the
//! checked `buffer()` (`scootbg_mem::zeroed_bytes`), so a file that
//! claims a huge size is a `set` refusal naming out-of-memory, the
//! daemon lives, and the next `set` draws. Before the fix the canvas
//! was an infallible `vec!`, which aborted the daemon (`memory
//! allocation of ... bytes failed`, SIGABRT) under the same limit.
//!
//! The limit mechanics (soft `RLIMIT_AS` only, the `Lowered` guard, the
//! arena premise) are `draw_failed.rs`'s, via `scaler_probe.rs`: lowering
//! needs no privilege, and by the time it happens the color `set` has
//! already run the state saver on a thread, so the decoding thread
//! reuses its arena and needs no new address space of its own. The
//! fixture is a GIF with a 16384×16384 logical screen (the pixel budget's
//! maximum, so the header passes) and tiny 16×16 frames: the decoder's
//! own allocations stay small while the canvas is 1 GiB, so only the
//! canvas can fail.

mod common;

use std::path::Path;

use common::Session;
use rustix::process::{Pid, Resource, Rlimit, prlimit};
use serde_json::json;

/// Room past what the daemon maps when idle: plenty for the decoding
/// thread and the GIF decoder's small allocations, but far less than
/// the 1 GiB RGBA canvas behind a 16384×16384 first frame.
const MARGIN: u64 = 32 << 20;

/// The pixel budget's maximum side: the header passes, the canvas does not.
const SCREEN: u16 = 16384;

fn write_big_screen_gif(path: &Path, frames: usize) {
    let palette = [0u8, 0, 0, 255, 255, 255];
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = gif::Encoder::new(file, SCREEN, SCREEN, &palette).unwrap();
    for _ in 0..frames {
        let frame = gif::Frame {
            width: 16,
            height: 16,
            delay: 10,
            buffer: std::borrow::Cow::Owned(vec![0u8; 16 * 16]),
            ..Default::default()
        };
        encoder.write_frame(&frame).unwrap();
    }
}

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

/// The soft and hard address-space limits of `pid`, `None` for unlimited.
fn address_limits(pid: u32) -> (Option<u64>, Option<u64>) {
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).unwrap();
    let line = limits
        .lines()
        .find(|line| line.starts_with("Max address space"))
        .unwrap_or_else(|| panic!("no address-space line in {limits}"));
    // "Max address space  SOFT  HARD  bytes": the name is three words.
    let mut words = line.split_whitespace().skip(3);
    let mut limit = || {
        let word = words.next().unwrap_or_else(|| panic!("{line}"));
        (word != "unlimited").then(|| word.parse().unwrap())
    };
    let soft = limit();
    (soft, limit())
}

/// Sets the soft address-space limit of `pid` (`None`: unlimited), the
/// hard one kept at `hard`. Returns the soft limit it had.
fn set_soft_limit(pid: u32, soft: Option<u64>, hard: Option<u64>) -> std::io::Result<Option<u64>> {
    let old = prlimit(
        Pid::from_raw(i32::try_from(pid).unwrap()),
        Resource::As,
        Rlimit {
            current: soft,
            maximum: hard,
        },
    )?;
    Ok(old.current)
}

/// The daemon's soft address-space limit, lowered while this lives and
/// put back on drop, so a failed assertion (a panic) still leaves a daemon
/// that can be stopped and a limit that does not outlive the test.
struct Lowered {
    pid: u32,
    old: Option<u64>,
    hard: Option<u64>,
}

impl Lowered {
    fn new(pid: u32, soft: u64, hard: Option<u64>) -> Self {
        let old = set_soft_limit(pid, Some(soft), hard).unwrap();
        Self { pid, old, hard }
    }
}

impl Drop for Lowered {
    fn drop(&mut self) {
        // The daemon may be gone (the failure being tested killed it):
        // nothing to restore then.
        let _ = set_soft_limit(self.pid, self.old, self.hard);
    }
}

/// Before the fix this aborted the daemon (`memory allocation of
/// 1073741824 bytes failed`, SIGABRT): a first frame behind a huge
/// logical screen is now an out-of-memory `set` refusal, with and
/// without `--no-animate`, the daemon lives, and a later `set` with the
/// limit lifted draws.
#[test]
fn animation_canvases_refuse_under_a_tight_limit() {
    let Some(session) = Session::start_sized("animcanvas", 1920, 1080) else {
        return;
    };
    let big_single = session.scratch.0.join("big-single.gif");
    write_big_screen_gif(&big_single, 1);
    let big_single = big_single.to_str().unwrap().to_owned();
    let small = session.scratch.0.join("blue.png");
    write_png(&small);
    let small = small.to_str().unwrap().to_owned();
    let mut daemon = session.daemon_logged(&[]);
    session.query_until("configured", |o| {
        o.len() == 1 && o[0]["surface"]["pixels"].is_object()
    });
    let set = |args: &[&str]| session.run(&[&["set"], args].concat());
    let done = set(&["#102030"]);
    assert!(done.status.success(), "{done:?}");

    let pid = daemon.id();
    let limit = vm_size(pid) + MARGIN;
    let (soft, hard) = address_limits(pid);
    // Without room between the limit and what the daemon maps plus the
    // margin, the assertions below say nothing, so the test cannot run:
    // skipped, loudly.
    let needed = limit + MARGIN;
    eprintln!("address space: limit {limit}, needed {needed}, soft {soft:?}, hard {hard:?}");
    if [soft, hard].into_iter().flatten().any(|cap| cap < needed) {
        eprintln!(
            "skipped -- the address-space limit (soft {soft:?}, hard {hard:?} bytes) is below \
             what the daemon maps and twice the margin ({needed} bytes)"
        );
        let _ = session.run(&["kill"]);
        let _ = common::wait_exit(&mut daemon);
        return;
    }
    let lowered = Lowered::new(pid, limit, hard);
    // With checks on (the default) and with `--no-animate` alike, the
    // single-frame fixture reaches the same canvas: a refusal, not an
    // abort.
    for args in [
        vec![big_single.as_str()],
        vec![big_single.as_str(), "--no-animate"],
    ] {
        let refused = set(&args);
        let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
        assert_eq!(refused.status.code(), Some(1), "{args:?}: {stderr}");
        assert!(
            stderr.contains("out of memory") && stderr.contains("cannot allocate"),
            "{args:?}: {stderr}"
        );
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "{args:?}: the daemon died"
        );
    }
    // Lifted before anything is asserted (and on a panic, by the guard),
    // so a failure below leaves a daemon that can still be stopped.
    drop(lowered);

    // What it showed before stays, and the next request draws.
    let after = session.query()["outputs"][0].clone();
    assert_eq!(after["shows"], json!({"color": "#102030"}), "{after}");
    assert_eq!(after["draw_failed"], false, "{after}");
    assert!(daemon.try_wait().unwrap().is_none(), "the daemon died");
    let drawn = set(&[small.as_str()]);
    assert!(drawn.status.success(), "{drawn:?}");
    let now = session.query()["outputs"][0].clone();
    assert_eq!(now["draw_failed"], false, "{now}");
    assert_eq!(now["shows"]["mode"], "fill", "{now}");

    let killed = session.run(&["kill"]);
    assert!(killed.status.success(), "{killed:?}");
    assert!(common::wait_exit(&mut daemon).success());
}
