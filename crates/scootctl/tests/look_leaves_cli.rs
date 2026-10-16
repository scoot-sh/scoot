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
