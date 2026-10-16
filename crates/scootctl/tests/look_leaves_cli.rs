//! The real leaf backends against fake daemons: `CliLeaves` resolves
//! `makoctl`, `busctl` and `systemctl` on `PATH`, passes each its exact
//! arguments, and reports exit codes honestly. Fakes record their argv and
//! exit scripted codes, so this proves the commands a swap runs without
//! touching the session bus or a real daemon.
//!
//! `PATH` is pointed at the fake bin dir for the body only (the same
//! `set_var`/`restore` shape as `scoot-look`'s `non_nix` gate).

use scootctl::look::leaves::{CliLeaves, Leaves};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct FakeBin {
    dir: PathBuf,
    out: PathBuf,
}

impl FakeBin {
    /// A bin dir with fakes for `names`: each appends its argv (one line)
    /// to `$out/argv-<name>` and exits the code in `$out/exit-<name>`
    /// (0 when the file is missing).
    fn new(tag: &str, names: &[&str]) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "scoot-leaves-cli-{tag}-{}-{id}",
            std::process::id()
        ));
        let dir = root.join("bin");
        let out = root.join("out");
        std::fs::create_dir_all(&dir).expect("a bin dir");
        std::fs::create_dir_all(&out).expect("an out dir");
        for name in names {
            let out_sh = out.display().to_string();
            std::fs::write(
                dir.join(name),
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{out_sh}/argv-{name}\"\ncode=\"$(cat \"{out_sh}/exit-{name}\" 2>/dev/null || echo 0)\"\nexit \"$code\"\n"
                ),
            )
            .expect("a fake");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mut perms = std::fs::metadata(dir.join(name))
                    .expect("fake metadata")
                    .permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(dir.join(name), perms).expect("fake perms");
            }
        }
        Self { dir, out }
    }

    fn argv(&self, name: &str) -> String {
        std::fs::read_to_string(self.out.join(format!("argv-{name}"))).unwrap_or_default()
    }

    fn fail(&self, name: &str) {
        std::fs::write(self.out.join(format!("exit-{name}")), "1").expect("an exit code");
    }
}

/// Prepends the fake bin dir to `PATH` for the body (real tools behind it
/// still resolve, but the fakes win).
fn with_fake_path<T>(fake: &FakeBin, f: impl FnOnce() -> T) -> T {
    with_path(fake, true, f)
}

/// Sets `PATH` to exactly the fake bin dir for the body, so tools the
/// fake omits (e.g. `busctl`) resolve to nothing.
fn with_exact_path<T>(fake: &FakeBin, f: impl FnOnce() -> T) -> T {
    with_path(fake, false, f)
}

/// One binary, several threads under `cargo test` (each test its own
/// process under nextest): `PATH` is process-global, so the three tests
/// serialize here rather than racing each other's `set_var`. Module-level
/// (a generic fn would mint one lock per instantiation and serialize
/// nothing).
static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_path<T>(fake: &FakeBin, keep_real: bool, f: impl FnOnce() -> T) -> T {
    let _guard = PATH_LOCK.lock().expect("the PATH lock");
    // Read inside the lock: the suffix must be the pre-body PATH, not
    // another test's body PATH observed mid-flight.
    let old = std::env::var_os("PATH");
    let mut path = fake.dir.display().to_string();
    if keep_real && let Some(real) = &old {
        path.push(':');
        path.push_str(&real.to_string_lossy());
    }
    // `set_var` is `unsafe` (it can race another thread's `getenv`); the
    // body runs alone here and restores the value before returning.
    unsafe { std::env::set_var("PATH", &path) };
    let result = f();
    if let Some(path) = old {
        unsafe { std::env::set_var("PATH", path) };
    }
    result
}

#[test]
fn leaf_commands_reach_the_daemons_with_exact_args() {
    let fake = FakeBin::new("ok", &["makoctl", "busctl", "systemctl"]);
    with_fake_path(&fake, || {
        let leaves = CliLeaves::new();
        leaves.mako_reload().expect("mako reloads");
        leaves.fcitx5_reload().expect("fcitx5 reloads");
        leaves
            .unit_try_restart("scoot-osd")
            .expect("the OSD unit restarts");
        leaves
            .unit_try_restart("scoot-nightlight")
            .expect("the night-light unit restarts");
    });
    // mako goes over the bus with auto-start off (a bare `makoctl reload`
    // would D-Bus-activate a mako nobody started); `makoctl` itself is
    // never invoked while busctl exists.
    assert!(fake.argv("makoctl").trim().is_empty());
    let busctl_argv = fake.argv("busctl");
    let mut busctl = busctl_argv.lines();
    assert_eq!(
        busctl.next().unwrap_or_default(),
        "--user --auto-start=no call org.freedesktop.Notifications /fr/emersion/Mako fr.emersion.Mako Reload"
    );
    assert_eq!(
        busctl.next().unwrap_or_default(),
        "--user --auto-start=no call org.fcitx.Fcitx5 /controller org.fcitx.Fcitx.Controller1 ReloadAddonConfig s classicui"
    );
    let systemctl = fake.argv("systemctl");
    let mut lines = systemctl.lines();
    assert_eq!(
        lines.next().unwrap_or_default(),
        "--user try-restart scoot-osd"
    );
    assert_eq!(
        lines.next().unwrap_or_default(),
        "--user try-restart scoot-nightlight"
    );
}

#[test]
fn mako_falls_back_to_makoctl_without_busctl() {
    // No busctl on PATH at all (exact PATH, nothing behind it): the
    // reload goes through `makoctl reload` as before.
    let fake = FakeBin::new("mako-fallback", &["makoctl"]);
    with_exact_path(&fake, || {
        let leaves = CliLeaves::new();
        leaves.mako_reload().expect("mako reloads");
    });
    assert_eq!(fake.argv("makoctl").trim(), "reload");
}

#[test]
fn failing_daemons_are_named_errors() {
    let fake = FakeBin::new("failing", &["makoctl", "busctl", "systemctl"]);
    fake.fail("makoctl");
    fake.fail("busctl");
    fake.fail("systemctl");
    with_fake_path(&fake, || {
        let leaves = CliLeaves::new();
        let error = leaves.mako_reload().expect_err("mako failure surfaces");
        assert!(error.contains("Reload"), "names the call: {error}");
        // The failure never falls through to `makoctl` (which would
        // D-Bus-activate mako): busctl present means makoctl stays out.
        assert!(fake.argv("makoctl").trim().is_empty());
        let error = leaves.fcitx5_reload().expect_err("fcitx5 failure surfaces");
        assert!(error.contains("fcitx5"), "names the daemon: {error}");
        let error = leaves
            .unit_try_restart("scoot-osd")
            .expect_err("unit failure surfaces");
        assert!(error.contains("scoot-osd"), "names the unit: {error}");
    });
}

/// The process table the signal tests read: our own uid's `hx` twice
/// (one under our own pid, which must never be signaled) and `btop`
/// once, another uid's `hx` and `btop` (never touched), plus a header
/// and a garbage line (both skipped). Pids sit where no real process
/// can be (above any pid_max), so the kills land on `ESRCH` and stay
/// silent: the counts prove the selection, never a side effect.
fn stub_ps_table(bin: &std::path::Path, uid: u32, other: u32) {
    let own = std::process::id();
    let stub = bin.join("ps");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \\\n  '    PID   UID COMMAND' \\\n  'not a process line' \\\n  \"2000000001 {uid} hx\" \\\n  \"2000000002 {uid} hx\" \\\n  \"{own} {uid} hx\" \\\n  \"2000000003 {uid} btop\" \\\n  \"2000000004 {uid} foot\" \\\n  \"2000000005 {other} hx\" \\\n  \"2000000006 {other} btop\" \\\n  \"2000000007 {uid} /nix/store/abc-helix-25.07.1/bin/hx\" \\\n  \"2000000008 {uid} hx-something\"\n"
        ),
    )
    .expect("a stub ps");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("an executable stub");
    }
}

fn own_uid() -> u32 {
    let out = std::process::Command::new("id")
        .arg("-u")
        .output()
        .expect("id -u runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("a numeric uid")
}

#[test]
fn editor_signals_reach_only_the_users_own_processes() {
    let uid = own_uid();
    let dir = std::env::temp_dir().join(format!(
        "scoot-leaves-ps-{}-{}",
        std::process::id(),
        "signals"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a stub dir");
    stub_ps_table(&dir, uid, uid + 1);
    let fake = FakeBin::new("signals-ps", &[]);
    // The stub ps beside (not inside) the fake bin dir: reuse the
    // fake's PATH plumbing by pointing PATH at both.
    let _ = &fake;
    let old = std::env::var_os("PATH");
    {
        let _guard = PATH_LOCK.lock().expect("the PATH lock");
        let mut path = dir.display().to_string();
        if let Some(real) = &old {
            path.push(':');
            path.push_str(&real.to_string_lossy());
        }
        unsafe { std::env::set_var("PATH", &path) };
        let leaves = CliLeaves::new();
        // Two own bare `hx` (our own pid excluded) plus one by store
        // path (the macOS `comm` shape), one own `btop`: the other
        // uid's pair, `foot` and `hx-something` are never counted. The
        // kills hit pids no process holds (`ESRCH`), so nothing is
        // side-effected.
        assert_eq!(leaves.helix_signal().expect("hx signaled"), 3);
        assert_eq!(leaves.btop_signal().expect("btop signaled"), 1);
        if let Some(path) = old.clone() {
            unsafe { std::env::set_var("PATH", path) };
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_running_editor_is_a_count_of_zero() {
    let uid = own_uid();
    let dir = std::env::temp_dir().join(format!(
        "scoot-leaves-ps-{}-{}",
        std::process::id(),
        "empty"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a stub dir");
    let stub = dir.join("ps");
    std::fs::write(
        &stub,
        format!("#!/bin/sh\nprintf '%s\\n' '    PID   UID COMMAND' '2000000007 {uid} foot'\n"),
    )
    .expect("a stub ps");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("an executable stub");
    }
    let old = std::env::var_os("PATH");
    {
        let _guard = PATH_LOCK.lock().expect("the PATH lock");
        let mut path = dir.display().to_string();
        if let Some(real) = &old {
            path.push(':');
            path.push_str(&real.to_string_lossy());
        }
        unsafe { std::env::set_var("PATH", &path) };
        let leaves = CliLeaves::new();
        assert_eq!(leaves.helix_signal().expect("hx counted"), 0);
        assert_eq!(leaves.btop_signal().expect("btop counted"), 0);
        if let Some(path) = old.clone() {
            unsafe { std::env::set_var("PATH", path) };
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_ps_is_a_named_error() {
    // An exact PATH with no `ps`: the step reports, never refuses the
    // swap (the caller turns this into a note).
    let fake = FakeBin::new("signals-nops", &[]);
    with_exact_path(&fake, || {
        let leaves = CliLeaves::new();
        let error = leaves.helix_signal().expect_err("no ps surfaces");
        assert!(error.contains("`ps`"), "names the tool: {error}");
        let error = leaves.btop_signal().expect_err("no ps surfaces");
        assert!(error.contains("`ps`"), "names the tool: {error}");
    });
}

/// Real delivery, hermetically: copies of `sleep` named `hx` and `btop`
/// run as our own children, and a stub `ps` lists only their pids — so
/// the real `ps` output parse, the uid match and the real `kill` run
/// with nothing else in reach. Both sleepers must die by their signal
/// (`SIGUSR1` for Helix, `SIGUSR2` for btop), proving the signal lands.
#[test]
fn editor_signals_are_really_delivered() {
    use std::process::{Command, Stdio};
    let uid = own_uid();
    let dir = std::env::temp_dir().join(format!(
        "scoot-leaves-ps-{}-{}",
        std::process::id(),
        "deliver"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a stub dir");
    // A `sleep` by another name: a copy (never a symlink, which some
    // platforms resolve for `comm`) run under the leaf's process name.
    // Multicall platforms (NixOS coreutils dispatches on `argv[0]`)
    // cannot host one: the probe below skips there instead of failing.
    let sleep = ["/bin/sleep", "/usr/bin/sleep"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file())
        .or_else(|| {
            Command::new("sh")
                .args(["-c", "command -v sleep"])
                .output()
                .ok()
                .and_then(|out| {
                    String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .next()
                        .map(|line| std::path::PathBuf::from(line.trim()))
                })
                .filter(|path| path.is_file())
        });
    let Some(sleep) = sleep else {
        eprintln!("skipped -- no sleep to host a named sleeper");
        return;
    };
    // The capability probe: a copy under a probe name must report that
    // name as its comm, or no renamed sleeper can prove delivery here.
    let probe = dir.join("scoot-probe-sleeper");
    std::fs::copy(&sleep, &probe).expect("a probe copy");
    let mut probe_child = Command::new(&probe)
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("a probe sleeper");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let seen = Command::new("ps")
        .args(["-o", "comm=", "-p", &probe_child.id().to_string()])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    probe_child.kill().ok();
    let _ = probe_child.wait();
    if seen.rsplit('/').next().unwrap_or_default() != "scoot-probe-sleeper" {
        eprintln!("skipped -- renamed copies do not keep their comm here ({seen:?})");
        return;
    }
    let mut children = Vec::new();
    for name in ["hx", "btop"] {
        let dest = dir.join(name);
        std::fs::copy(&sleep, &dest).expect("a named sleeper");
        let child = Command::new(&dest)
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a sleeper");
        children.push((name, child));
    }
    // The stub lists only the two sleepers (plus a header and garbage,
    // both skipped): no ambient process is in reach.
    let table = format!(
        "#!/bin/sh\nprintf '%s\\n' 'PID UID COMMAND' 'junk' '{} {} hx' '{} {} btop'\n",
        children[0].1.id(),
        uid,
        children[1].1.id(),
        uid,
    );
    let stub = dir.join("ps");
    std::fs::write(&stub, table).expect("a stub ps");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("an executable stub");
    }
    let old = std::env::var_os("PATH");
    let (hx_count, btop_count) = {
        let _guard = PATH_LOCK.lock().expect("the PATH lock");
        let mut path = dir.display().to_string();
        if let Some(real) = &old {
            path.push(':');
            path.push_str(&real.to_string_lossy());
        }
        unsafe { std::env::set_var("PATH", &path) };
        let leaves = CliLeaves::new();
        let counts = (
            leaves.helix_signal().expect("hx signaled"),
            leaves.btop_signal().expect("btop signaled"),
        );
        if let Some(path) = old.clone() {
            unsafe { std::env::set_var("PATH", path) };
        }
        counts
    };
    assert_eq!((hx_count, btop_count), (1, 1));
    // Both sleepers died by their signal (never an exit code): the
    // delivery is real, through the real `kill`.
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        // SIGUSR1/SIGUSR2: 30/31 on macOS, 10/12 on Linux.
        #[cfg(target_os = "macos")]
        let want = [30, 31];
        #[cfg(not(target_os = "macos"))]
        let want = [10, 12];
        for ((name, mut child), want) in children.into_iter().zip(want) {
            let status = child.wait().expect("a reaped sleeper");
            assert_eq!(status.signal(), Some(want), "{name} died by its signal");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A process table past the pipe buffer (over 64 KiB of `hx` lines):
/// the reader drains as it goes, so a busy box never deadlocks the
/// child on a full pipe. Every pid is unreachable (`ESRCH`), so the
/// count proves the parse, never a side effect.
#[test]
fn a_table_past_the_pipe_buffer_still_reads() {
    let uid = own_uid();
    let dir = std::env::temp_dir().join(format!(
        "scoot-leaves-ps-{}-{}",
        std::process::id(),
        "bigtable"
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a stub dir");
    // Two thousand own-`hx` lines: ~70 KiB, past the 64 KiB pipe.
    // Pids sit where no process can be, so each kill is an `ESRCH`
    // attempt and the suite side-effects nothing.
    let stub = dir.join("ps");
    let mut table = String::from("#!/bin/sh\nprintf '%s\\n' \\\n");
    for i in 0..2000 {
        use std::fmt::Write as _;
        let cont = if i + 1 == 2000 { "" } else { " \\" };
        write!(table, "  '{} {} hx'{}", 2000000000 + i, uid, cont).expect("a table line");
        table.push('\n');
    }
    std::fs::write(&stub, table).expect("a stub ps");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("an executable stub");
    }
    let old = std::env::var_os("PATH");
    let count = {
        let _guard = PATH_LOCK.lock().expect("the PATH lock");
        let mut path = dir.display().to_string();
        if let Some(real) = &old {
            path.push(':');
            path.push_str(&real.to_string_lossy());
        }
        unsafe { std::env::set_var("PATH", &path) };
        let leaves = CliLeaves::new();
        let count = leaves.helix_signal().expect("a big table reads");
        if let Some(path) = old.clone() {
            unsafe { std::env::set_var("PATH", path) };
        }
        count
    };
    assert_eq!(count, 2000);
    std::fs::remove_dir_all(&dir).ok();
}
