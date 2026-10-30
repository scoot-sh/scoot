//! An open panel at rest asks the compositor for nothing: after the first
//! frames, and after a keystroke, no `wl_surface.commit` while nobody
//! types. Read off the client's own `WAYLAND_DEBUG` log, so what is
//! counted is what went on the wire. (The effects' own idle checks are in
//! `effects.rs`.)

#[path = "harness.rs"]
mod harness;

use std::time::Duration;

use harness::{Panel, desktop, rgb, session_with_apps};

/// How many `wl_surface` requests of `name` (e.g. `commit`) were logged.
fn requests(log: &str, name: &str) -> usize {
    log.lines()
        .filter(|line| line.contains("-> wl_surface@") && line.contains(&format!(".{name}(")))
        .count()
}

#[test]
fn an_idle_panel_commits_nothing_before_or_after_a_keystroke() {
    let bodies = [
        ("alpha.desktop", desktop("Alpha", "/usr/bin/alpha")),
        ("beta.desktop", desktop("Beta", "/usr/bin/beta")),
    ];
    let refs: Vec<(&str, &str)> = bodies.iter().map(|(f, b)| (*f, b.as_str())).collect();
    let Some((session, data_home)) = session_with_apps("idle-default", &refs) else {
        return;
    };
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[("WAYLAND_DEBUG", "1")])
    else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(1500));
    let opened = requests(&panel.stderr(), "commit");
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(requests(&panel.stderr(), "commit"), opened, "idle commits");
    // A keystroke redraws, and then it is quiet again: a buffer release
    // used to mark the panel dirty, so one keystroke kept it committing
    // for as long as it stayed open.
    panel.type_text("a");
    std::thread::sleep(Duration::from_millis(1500));
    let typed = requests(&panel.stderr(), "commit");
    std::thread::sleep(Duration::from_secs(3));
    let end = requests(&panel.stderr(), "commit");
    eprintln!(
        "idle: {opened} commits to open, {} for a keystroke, {} in the 3 s after",
        typed - opened,
        end - typed
    );
    assert_eq!(end, typed, "commits after a keystroke settled");
    assert!(
        typed - opened <= 2,
        "a keystroke is one draw: {}",
        typed - opened
    );
    assert_eq!(requests(&panel.stderr(), "frame"), 0, "no frame callbacks");
    panel.key("Escape");
    panel.wait_exit();
}

/// The CPU ticks (user plus system, 1/100 s) the process has used.
fn cpu_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    // After the `)` that ends the command name: state is field 3, utime 14
    // and stime 15.
    let rest = stat.rsplit(')').next().unwrap_or("");
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let field = |n: usize| {
        fields
            .get(n - 3)
            .and_then(|f| f.parse::<u64>().ok())
            .unwrap_or(0)
    };
    field(14) + field(15)
}

/// What an open panel costs the CPU while nobody touches it, then after one
/// keystroke: `SCOOTLAUNCH_TEST_BIN=<main build> cargo test -p scootlaunch
/// --test idle bench_idle_cpu -- --ignored --nocapture` for the before
/// side.
#[test]
#[ignore = "a benchmark; run it with --ignored --nocapture"]
fn bench_idle_cpu() {
    let bodies = [
        ("alpha.desktop", desktop("Alpha", "/usr/bin/alpha")),
        ("beta.desktop", desktop("Beta", "/usr/bin/beta")),
    ];
    let refs: Vec<(&str, &str)> = bodies.iter().map(|(f, b)| (*f, b.as_str())).collect();
    let Some((session, data_home)) = session_with_apps("idle-cpu", &refs) else {
        return;
    };
    let Some(mut panel) = Panel::start_on(session, data_home, &[], &[]) else {
        return;
    };
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(1000));
    let pid = panel.child.id();
    let start = cpu_ticks(pid);
    std::thread::sleep(Duration::from_secs(5));
    let idle = cpu_ticks(pid) - start;
    panel.type_text("a");
    std::thread::sleep(Duration::from_millis(1000));
    let typed = cpu_ticks(pid);
    std::thread::sleep(Duration::from_secs(5));
    let after = cpu_ticks(pid) - typed;
    println!(
        "bench_idle_cpu: {idle} ticks over 5 s idle; {after} ticks over 5 s after one keystroke (100 ticks = 1 s of CPU)"
    );
    panel.key("Escape");
    panel.wait_exit();
}
