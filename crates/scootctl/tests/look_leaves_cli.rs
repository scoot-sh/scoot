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
    /// A bin dir with `makoctl`, `busctl` and `systemctl` fakes: each
    /// appends its argv (one line) to `$out/argv-<name>` and exits the
    /// code in `$out/exit-<name>` (0 when the file is missing).
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "scoot-leaves-cli-{tag}-{}-{id}",
            std::process::id()
        ));
        let dir = root.join("bin");
        let out = root.join("out");
        std::fs::create_dir_all(&dir).expect("a bin dir");
        std::fs::create_dir_all(&out).expect("an out dir");
        for name in ["makoctl", "busctl", "systemctl"] {
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

fn with_fake_path<T>(fake: &FakeBin, f: impl FnOnce() -> T) -> T {
    let old = std::env::var_os("PATH");
    let path = format!(
        "{}{}",
        fake.dir.display(),
        old.as_ref()
            .map(|p| format!(":{}", p.to_string_lossy()))
            .unwrap_or_default()
    );
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
    let fake = FakeBin::new("ok");
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
    assert_eq!(fake.argv("makoctl").trim(), "reload");
    assert_eq!(
        fake.argv("busctl").trim(),
        "--user call org.fcitx.Fcitx5 /controller org.fcitx.Fcitx.Controller1 ReloadAddonConfig s classicui"
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
fn failing_daemons_are_named_errors() {
    let fake = FakeBin::new("failing");
    fake.fail("makoctl");
    fake.fail("busctl");
    fake.fail("systemctl");
    with_fake_path(&fake, || {
        let leaves = CliLeaves::new();
        let error = leaves.mako_reload().expect_err("mako failure surfaces");
        assert!(error.contains("makoctl"), "names the command: {error}");
        let error = leaves.fcitx5_reload().expect_err("fcitx5 failure surfaces");
        assert!(error.contains("fcitx5"), "names the daemon: {error}");
        let error = leaves
            .unit_try_restart("scoot-osd")
            .expect_err("unit failure surfaces");
        assert!(error.contains("scoot-osd"), "names the unit: {error}");
    });
}
