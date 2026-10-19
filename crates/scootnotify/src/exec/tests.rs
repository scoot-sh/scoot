//! The spawner's contract: the bound of 4, and no fd growth.

use super::{ExecRequest, MAX_CHILDREN, Spawner};

fn request(program: &str) -> ExecRequest {
    argv_request(&[program])
}

fn argv_request(argv: &[&str]) -> ExecRequest {
    ExecRequest {
        argv: argv.iter().map(|word| (*word).to_owned()).collect(),
        id: 1,
        app_name: "a".to_owned(),
        summary: "s".to_owned(),
        body: "b".to_owned(),
        urgency: "normal",
        category: String::new(),
        desktop_entry: String::new(),
    }
}

/// How many fds this process holds: the flood-fds-flat pin reads it.
#[cfg(target_os = "linux")]
fn fds_now() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[test]
fn a_fifth_child_while_four_run_is_refused() {
    let mut spawner = Spawner::default();
    for _ in 0..MAX_CHILDREN {
        spawner
            .spawn(&argv_request(&["sleep", "120"]))
            .expect("slots free");
    }
    assert_eq!(spawner.running(), MAX_CHILDREN);
    let error = spawner.spawn(&request("true")).unwrap_err();
    assert!(error.contains("4"), "{error}");
    spawner.kill_all();
    assert_eq!(spawner.running(), 0);
}

/// Reverting the bound check from `>=` to `>` lets the fifth spawn
/// succeed, leaving the refusal test failing: the pin is load-bearing.
#[test]
fn quick_children_free_their_slots() {
    let mut spawner = Spawner::default();
    for _ in 0..8 {
        spawner.spawn(&request("true")).expect("true runs");
        // Each `true` exits at once: wait for the exit (fail-fast: a
        // child that never exits fails the test, never hangs the suite),
        // so the next spawn's opening reap frees the slot.
        let start = std::time::Instant::now();
        while spawner.running() > 0 {
            spawner.reap();
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "a `true` that never exits"
            );
            std::thread::yield_now();
        }
    }
    spawner.reap();
    assert_eq!(spawner.running(), 0);
}

#[test]
fn a_missing_program_is_an_error_not_a_panic() {
    let mut spawner = Spawner::default();
    let error = spawner
        .spawn(&request("/nonexistent-scootnotify-hook-7f3a"))
        .unwrap_err();
    assert!(error.contains("cannot run"), "{error}");
    assert_eq!(spawner.running(), 0);
}

#[test]
fn an_empty_argv_is_refused() {
    let mut spawner = Spawner::default();
    let mut empty = request("true");
    empty.argv.clear();
    assert!(spawner.spawn(&empty).is_err());
    assert_eq!(spawner.running(), 0);
}

#[test]
#[cfg(target_os = "linux")]
fn a_flood_leaves_fds_flat() {
    let mut spawner = Spawner::default();
    let before = fds_now();
    for _ in 0..MAX_CHILDREN {
        let _ = spawner.spawn(&argv_request(&["sleep", "120"]));
    }
    let error = spawner.spawn(&request("true")).unwrap_err();
    assert!(error.contains("4"), "{error}");
    spawner.kill_all();
    spawner.reap();
    // Deterministic half: every child is collected.
    assert_eq!(spawner.running(), 0);
    // The fd half tolerates two strays: under `cargo test` every test
    // shares one process, and another test may hold a transient fd
    // across this measurement (under nextest each test is its own
    // process and the count is exact). A real leak keeps all four
    // pidfds, which this still catches.
    let after = fds_now();
    assert!(
        after <= before + 2,
        "an exec flood costs fds: {before} -> {after}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn the_environment_carries_the_notification() {
    // A hook that writes its environment to a file, then the test reads
    // the file back: the only reader of the env shape.
    let dir = std::env::temp_dir().join(format!("scootnotify-exec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    let out = dir.join("env");
    let mut hook = request("sh");
    hook.argv = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        format!("env > {}", out.display()),
    ];
    hook.app_name = "Mail".to_owned();
    hook.summary = "New mail".to_owned();
    hook.body = "hello".to_owned();
    hook.urgency = "critical";
    let mut spawner = Spawner::default();
    spawner.spawn(&hook).expect("sh runs");
    // The hook exits at once; wait for the file, briefly.
    let start = std::time::Instant::now();
    while out.metadata().is_err() && start.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(10));
        spawner.reap();
    }
    let text = std::fs::read_to_string(&out).expect("the hook wrote its env");
    for want in [
        "SCOOTNOTIFY_APP=Mail",
        "SCOOTNOTIFY_SUMMARY=New mail",
        "SCOOTNOTIFY_BODY=hello",
        "SCOOTNOTIFY_URGENCY=critical",
    ] {
        assert!(text.contains(want), "{want} in:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
