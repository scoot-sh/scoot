//! No retained decoded copy: after an image is up and settled, the daemon
//! holds its output-sized shared buffers (the floor) and nothing anonymous
//! beyond small change.
//!
//! From `docs/scootbg/backlog/image-retention.md` (the five-desktop
//! benchmark's bimodal scootbg: 14.7 vs 2.1 MB PSS): the 12 MB that came
//! and went was never a decoded copy kept by mistake. Traced on a real
//! `--tty` session, the fat cohort shows the wallpaper (two output-sized
//! `wl_shm` pools, shared with the compositor) and the lean cohort shows
//! the compositor's own background (the image file was unreadable —
//! `Permission denied` — so nothing was ever drawn). The decoded source,
//! the crops and the scaled temporaries are all dropped on the worker
//! thread before the buffers are allocated (`image::render::render_each`).
//! This test pins that: past the settle, anonymous memory stays small,
//! whatever decoded megabytes the image needed transiently.

mod common;

use std::time::{Duration, Instant};

use common::{PATIENCE, Session, wait_exit};

/// Anonymous PSS the settled daemon may hold, in kB: heap fragments and
/// allocator change, not image data. Measured 0.5–0.8 MB (debug) with
/// moonrise fill on two outputs; a retained decoded copy (moonrise's
/// 4000×2604×3 ≈ 30 MB, or a scaled 2560×1600×3 ≈ 12 MB) fails this by an
/// order of magnitude either way.
const ANON_KB_MAX: u64 = 2048;

/// `scootbg ARGS`, asserting it succeeds silently.
fn ok(session: &Session, args: &[&str]) {
    let out = session.run(args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// What `query` shows on every output.
fn shows(session: &Session) -> Vec<serde_json::Value> {
    session.query()["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["shows"].clone())
        .collect()
}

/// The daemon's `Pss_Anon`, in kB (`smaps_rollup`, no per-map classifier —
/// the benchmark's own `smaps.sh` credits each block's PSS to the *next*
/// block's class, which misfiles whole shared buffers). `None` where
/// `/proc` cannot be read (not Linux) or the daemon already exited (its
/// `/proc/PID` is gone with it): the caller tells the two apart.
fn anon_kb(pid: u32) -> Option<u64> {
    let rollup = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    rollup
        .lines()
        .find_map(|line| line.strip_prefix("Pss_Anon:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

#[test]
fn no_retained_copy_after_image_settles() {
    let Some(session) = Session::start("retention") else {
        return;
    };
    let mut daemon = session.daemon();
    let image = session.runtime_dir().join("moonrise.png");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/assets/wallpapers/moonrise.png"
        ),
        &image,
    )
    .unwrap();
    ok(
        &session,
        &["set", image.to_str().unwrap(), "--mode", "fill"],
    );
    // `set` answers once both outputs show the image; the decode thread is
    // over by then (its result landed before the draw).
    assert!(
        shows(&session)
            .iter()
            .all(|s| s["mode"] == "fill" && s["image"].as_str().is_some()),
        "both outputs show the image: {:?}",
        shows(&session)
    );
    let pid = daemon.id();
    // Settle: anonymous pages only fall as threads and arenas drain, so
    // poll for the bound rather than sleeping a fixed time.
    let deadline = Instant::now() + PATIENCE;
    let settled = loop {
        let Some(kb) = anon_kb(pid) else {
            // No reading: either there is no `/proc` (not Linux — a
            // polite skip) or the daemon already exited (its
            // `/proc/PID` went with it — a crash masked as a skip).
            // A daemon that just served `set` plus `query` has no
            // reason to be gone, so an exited one fails the test.
            if let Some(status) = daemon.try_wait().expect("cannot poll the daemon") {
                panic!("the daemon exited ({status}) before its memory settled");
            }
            eprintln!("skipped -- no /proc/PID/smaps_rollup (not Linux?)");
            assert!(session.run(&["kill"]).status.success());
            assert!(wait_exit(&mut daemon).success());
            return;
        };
        if kb < ANON_KB_MAX || Instant::now() >= deadline {
            break kb;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        settled < ANON_KB_MAX,
        "settled anonymous PSS {settled} kB >= {ANON_KB_MAX} kB: the daemon keeps a decoded copy"
    );
    assert!(session.run(&["kill"]).status.success());
    assert!(wait_exit(&mut daemon).success());
}
