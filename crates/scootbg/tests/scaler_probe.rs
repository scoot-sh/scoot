//! A live `set` that scales under an address-space limit: the scaler's own
//! allocation would abort the daemon (`memory allocation of 24883200 bytes
//! failed`, exit 134), so `scale` probes the whole budget first
//! (`probe_budget`: the output, both axes' weight tables and the row
//! scratch) and refuses the draw instead. `query` reports `draw_failed`
//! with the reason (`draw_error`), the output keeps what it showed, the
//! daemon lives, and the next `set` draws. Next to `draw_failed.rs`, which
//! covers the same shape for the `wl_shm` buffer (`--mode center`, which
//! scales nothing).
//!
//! The limit mechanics (soft `RLIMIT_AS` only, the `Lowered` guard, the
//! arena premise) are `draw_failed.rs`'s: lowering needs no privilege, and
//! by the time it happens the color `set` has already run the state saver
//! on a thread, so glibc's 128 MiB thread arena sits on its free list
//! (counted in `VmSize`) and the decoding thread reuses it, needing no new
//! address space of its own.
//!
//! Strict overcommit (`vm.overcommit_memory=2`) is deliberately not tested
//! here: it is a system setting, and the probe stays a heuristic there (a
//! process can take the commit charge between the probe and the scaler's
//! allocation). See `scale::probe_budget`'s docs.

mod common;

use std::path::Path;

use common::Session;
use rustix::process::{Pid, Resource, Rlimit, prlimit};
use serde_json::{Value, json};

/// Room past what the daemon maps when idle: plenty for the decoding
/// thread's stack (2 MiB) and a 1×1 PNG, but less than any scaling draw's
/// probe budget (the smallest, `nearest` at 3840×2160, is already
/// 24,883,200 + 65,536 = 24,948,736 bytes).
const MARGIN: u64 = 16 << 20;
const WIDTH: u32 = 3840;
const HEIGHT: u32 = 2160;

/// Every `--filter` name: the refusal must not depend on the filter, and
/// the margin draw must succeed with the hungriest one.
const FILTERS: [&str; 4] = ["lanczos3", "catmull-rom", "bilinear", "nearest"];

/// `scale::probe_budget` for 1×1 → 3840×2160 with `lanczos3` (the largest
/// budget), from `crates/scootbg/src/image/scale/tests.rs`:
/// 24,883,200 (output) + 284,300 (h weights) + 159,980 (v weights) + 12
/// (row scratch) + 65,536 (`PROBE_SLOP`).
const LANCZOS_BUDGET: u64 = 25_393_028;
/// The `wl_shm` buffer the draw then needs: 3840×2160×4.
const SHM_BYTES: u64 = 33_177_600;
/// Headroom past budget + buffer for the margin draw.
const HEADROOM: u64 = 8 << 20;

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

fn output(session: &Session) -> Value {
    session.query()["outputs"][0].clone()
}

/// Before the fix this aborted the daemon (`memory allocation of 24883200
/// bytes failed`, SIGABRT): for every filter, `fill` of a 1×1 PNG under a
/// limit of idle + 16 MiB is a `draw_error`, the daemon lives, and a later
/// `set` with the limit lifted draws.
#[test]
fn scaling_draws_refuse_under_a_tight_limit() {
    let Some(session) = Session::start_sized("scalprobe", WIDTH, HEIGHT) else {
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
    let (soft, hard) = address_limits(pid);
    // As in `draw_failed.rs`: without room between the limit, the 33 MB
    // buffer and the margin, the retry afterwards says nothing, so the
    // test cannot run: skipped, loudly.
    let needed = limit + u64::from(WIDTH) * u64::from(HEIGHT) * 4 + MARGIN;
    eprintln!("address space: limit {limit}, needed {needed}, soft {soft:?}, hard {hard:?}");
    if [soft, hard].into_iter().flatten().any(|cap| cap < needed) {
        eprintln!(
            "skipped -- the address-space limit (soft {soft:?}, hard {hard:?} bytes) is below \
             what the daemon maps, the buffer and twice the margin ({needed} bytes)"
        );
        let _ = session.run(&["kill"]);
        let _ = common::wait_exit(&mut daemon);
        return;
    }
    let lowered = Lowered::new(pid, limit, hard);
    for filter in FILTERS {
        let failed = set(&[
            picture.to_str().unwrap(),
            "--mode",
            "fill",
            "--filter",
            filter,
        ]);
        let stderr = String::from_utf8_lossy(&failed.stderr);
        assert_eq!(failed.status.code(), Some(1), "{filter}: {stderr}");
        assert!(stderr.contains("could not be drawn"), "{filter}: {stderr}");
        assert!(stderr.contains("scootbg query"), "{filter}: {stderr}");
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "{filter}: the daemon died"
        );
    }
    // Lifted before anything is asserted (and on a panic, by the guard),
    // so a failure below leaves a daemon that can still be stopped.
    drop(lowered);

    let after = output(&session);
    assert_eq!(after["draw_failed"], true, "{after}");
    let why = after["draw_error"].as_str().unwrap_or_default();
    assert!(
        why.contains("out of memory") && why.contains("for scaling"),
        "draw_error {why:?} is not the scaler probe's refusal (limit {limit} bytes)"
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
    let drawn = set(&[picture.to_str().unwrap(), "--mode", "fill"]);
    assert!(drawn.status.success(), "{drawn:?}");
    let now = output(&session);
    assert_eq!(now["draw_failed"], false, "{now}");
    assert_eq!(now["draw_error"], Value::Null);
    assert_eq!(now["shows"]["mode"], "fill");

    let killed = session.run(&["kill"]);
    assert!(killed.status.success(), "{killed:?}");
    assert!(common::wait_exit(&mut daemon).success());
}

/// The margin: a limit just above the full draw (the hungriest probe
/// budget, the `wl_shm` buffer, and headroom) still draws, with every
/// filter. Guards against a probe that over-budgets into refusing draws
/// that would have fit. One fresh daemon per filter, so each draw is the
/// first (no earlier buffer still mapped).
#[test]
fn a_limit_just_above_the_full_draw_budget_still_draws() {
    for filter in FILTERS {
        let Some(session) = Session::start_sized("scalmarg", WIDTH, HEIGHT) else {
            return;
        };
        let picture = session.scratch.0.join("blue.png");
        write_png(&picture);
        let mut daemon = session.daemon_logged(&[]);
        session.query_until("configured", |o| {
            o.len() == 1 && o[0]["surface"]["pixels"] == json!({"width": WIDTH, "height": HEIGHT})
        });
        let done = session.run(&["set", "#102030"]);
        assert!(done.status.success(), "{filter}: {done:?}");

        let pid = daemon.id();
        let limit = vm_size(pid) + LANCZOS_BUDGET + SHM_BYTES + HEADROOM;
        let (soft, hard) = address_limits(pid);
        eprintln!("address space: limit {limit}, soft {soft:?}, hard {hard:?}");
        if [soft, hard].into_iter().flatten().any(|cap| cap < limit) {
            eprintln!(
                "skipped -- the address-space limit (soft {soft:?}, hard {hard:?} bytes) is below \
                 the margin draw's limit ({limit} bytes)"
            );
            let _ = session.run(&["kill"]);
            let _ = common::wait_exit(&mut daemon);
            return;
        }
        let lowered = Lowered::new(pid, limit, hard);
        let drawn = session.run(&[
            "set",
            picture.to_str().unwrap(),
            "--mode",
            "fill",
            "--filter",
            filter,
        ]);
        // Lifted before asserting, so a failure still stops the daemon.
        drop(lowered);
        let stderr = String::from_utf8_lossy(&drawn.stderr).into_owned();
        if !drawn.status.success()
            && stderr.contains("cannot start a decoding thread")
            && stderr.contains("Resource temporarily unavailable")
        {
            // Thread-spawn EAGAIN under parallel load, persisting past the
            // daemon's retries: the box is out of threads for the moment,
            // not the draw over budget. Skipped loudly, like the limit
            // skips above; the retry itself is unit-tested in
            // `daemon::worker`.
            eprintln!("skipped -- {filter}: {stderr}");
            let _ = session.run(&["kill"]);
            let _ = common::wait_exit(&mut daemon);
            continue;
        }
        assert!(drawn.status.success(), "{filter}: {stderr}");
        let now = output(&session);
        assert_eq!(now["draw_failed"], false, "{filter}: {now}");
        assert_eq!(now["shows"]["mode"], "fill", "{filter}: {now}");
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "{filter}: the daemon died"
        );

        let killed = session.run(&["kill"]);
        assert!(killed.status.success(), "{filter}: {killed:?}");
        assert!(common::wait_exit(&mut daemon).success());
    }
}
