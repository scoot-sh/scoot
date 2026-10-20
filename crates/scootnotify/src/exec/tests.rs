//! The spawner's contract: the bound of 4, and no fd growth.

use super::{ExecRequest, MAX_CHILDREN, Spawner};

/// Serializes the pidfd-creating tests: under `cargo test` every test
/// shares one process, and two floods running side by side would each
/// see the other's live pidfds. (Under nextest each test is its own
/// process and the lock is a no-op.) A panic while held poisons the
/// lock, so the guard recovers instead of cascading.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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

/// How many pidfds this process holds: the flood pin reads this, not
/// the whole table — another test's transient socket or file is not
/// this spawner's leak, and under `cargo test` they share the process.
#[cfg(target_os = "linux")]
fn pidfds_now() -> usize {
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        return 0;
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            std::fs::read_link(entry.path())
                .is_ok_and(|target| target.to_string_lossy().contains("pidfd"))
        })
        .count()
}

#[test]
#[cfg(target_os = "linux")]
fn a_fifth_child_while_four_run_is_refused() {
    let _guard = serial();
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
    spawner.reap();
    assert_eq!(spawner.running(), 0);
}

/// Reverting the bound check from `>=` to `>` lets the fifth spawn
/// succeed, leaving the refusal test failing: the pin is load-bearing.
#[test]
fn quick_children_free_their_slots() {
    let _guard = serial();
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
fn a_default_spawner_opens_one_pidfd_per_child() {
    let _guard = serial();
    // The production shape: `Default` asks for pidfds, so a running
    // rule child costs no wakeups. The flood pin below only checks the
    // count returns to baseline, which passes vacuously with pidfds off
    // (nothing is ever opened) — this pin fails if the default ever
    // stops opening pidfds, which is the load-bearing property.
    let mut spawner = Spawner::default();
    let before = pidfds_now();
    for _ in 0..MAX_CHILDREN {
        spawner
            .spawn(&argv_request(&["sleep", "120"]))
            .expect("slots free");
    }
    assert_eq!(pidfds_now(), before + MAX_CHILDREN, "one pidfd per child");
    // No child lacks a pidfd, so the loop may sleep as long as it likes.
    assert_eq!(spawner.poll_timeout(), None);
    spawner.kill_all();
    spawner.reap();
    assert_eq!(spawner.running(), 0);
    assert_eq!(pidfds_now(), before, "reaping closes every pidfd");
}

#[test]
#[cfg(target_os = "linux")]
fn without_pidfds_a_child_falls_back_to_polling() {
    let _guard = serial();
    // The old-kernel shape, reached only by struct literal (production
    // builds on `Default`, which opens pidfds): the child is reaped on
    // a short timeout instead of a pidfd wake.
    let mut spawner = Spawner {
        live: Vec::new(),
        pidfds: false,
    };
    let before = pidfds_now();
    spawner
        .spawn(&argv_request(&["sleep", "120"]))
        .expect("slot free");
    assert_eq!(pidfds_now(), before, "no pidfd without pidfds");
    assert!(spawner.poll_timeout().is_some());
    spawner.kill_all();
    spawner.reap();
    assert_eq!(spawner.running(), 0);
    assert_eq!(spawner.poll_timeout(), None);
    assert_eq!(pidfds_now(), before);
}

#[test]
#[cfg(target_os = "linux")]
fn a_flood_leaves_fds_flat() {
    let _guard = serial();
    let mut spawner = Spawner::default();
    let before = pidfds_now();
    for _ in 0..MAX_CHILDREN {
        let _ = spawner.spawn(&argv_request(&["sleep", "120"]));
    }
    let error = spawner.spawn(&request("true")).unwrap_err();
    assert!(error.contains("4"), "{error}");
    spawner.kill_all();
    spawner.reap();
    // Every child is collected, and every pidfd closed with it: an
    // exact count, since the lock keeps the other floods out and no
    // other test in the process opens pidfds.
    assert_eq!(spawner.running(), 0);
    assert_eq!(pidfds_now(), before, "an exec flood costs pidfds");
    // And the freed slots serve again.
    for _ in 0..MAX_CHILDREN {
        spawner
            .spawn(&argv_request(&["sleep", "120"]))
            .expect("slots freed");
    }
    spawner.kill_all();
    spawner.reap();
    assert_eq!(spawner.running(), 0);
    assert_eq!(pidfds_now(), before, "a second flood costs pidfds");
}

#[test]
#[cfg(target_os = "linux")]
fn the_environment_carries_the_notification() {
    let _guard = serial();
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
    // The hook exits at once: wait for its content, briefly. The
    // shell creates the redirect before `env` writes a byte, so
    // readiness is the marker line, not the file's existence.
    let start = std::time::Instant::now();
    let text = loop {
        if let Ok(text) = std::fs::read_to_string(&out)
            && text.contains("SCOOTNOTIFY_APP=")
        {
            break text;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "the hook never wrote its env"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
        spawner.reap();
    };
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
