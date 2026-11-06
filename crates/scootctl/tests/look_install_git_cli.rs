//! `scoot look install GIT-URL` through the real `run`, with no network
//! and no real git: a missing `git`, every refused URL shape, and a stand-in
//! `git` that records the exact argument vector and environment it was
//! given, then fails, so the test also sees the temp directory removed and
//! nothing installed.
//!
//! The environment (HOME, XDG, PATH, TMPDIR) is process-global, so the
//! test re-execs itself as a child that is the only test in its process
//! (the pattern of `look_help_docs.rs`).

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use scootctl::look::parse_look;

fn run(args: &[&str]) -> Result<(), String> {
    let look = parse_look(args.iter().map(|arg| (*arg).to_owned()))
        .unwrap_or_else(|error| panic!("`scoot look {}` does not parse: {error}", args.join(" ")));
    scootctl::look::run(&look)
}

fn refusal(args: &[&str]) -> String {
    run(args).expect_err(&format!("`scoot look {}` must refuse", args.join(" ")))
}

#[test]
fn install_from_git_is_gated_end_to_end() {
    if std::env::var_os("SCOOT_INSTALL_GIT_CHILD").is_some() {
        return child_main();
    }
    let root = tempfile::tempdir().expect("a scratch root");
    let exe = std::env::current_exe().expect("the test binary");
    let out = std::process::Command::new(exe)
        .arg("install_from_git_is_gated_end_to_end")
        .arg("--exact")
        .arg("--nocapture")
        .env("SCOOT_INSTALL_GIT_CHILD", "1")
        .env("SCOOT_INSTALL_GIT_ROOT", root.path())
        .output()
        .expect("the child runs");
    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn child_main() {
    let root = PathBuf::from(std::env::var_os("SCOOT_INSTALL_GIT_ROOT").expect("a root"));
    let home = root.join("home");
    let tmp = root.join("tmp");
    let empty = root.join("empty-bin");
    let fake = root.join("fake-bin");
    for dir in [&home, &tmp, &empty, &fake] {
        std::fs::create_dir_all(dir).expect("a scratch dir");
    }
    // SAFETY: the child is the only test in its process (re-exec).
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", home.join("data"));
        std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
        std::env::set_var("XDG_CACHE_HOME", home.join("cache"));
        std::env::set_var("TMPDIR", &tmp);
        std::env::set_var("SCOOT_LOOKS_DIR", root.join("no-shipped-looks"));
        // Hostile inherited git state that must not reach git.
        std::env::set_var("GIT_SSH_COMMAND", "touch /tmp/scoot-test-pwned");
        std::env::set_var("GIT_CONFIG_COUNT", "1");
    }

    // 1. Every refused shape fails before git is even looked for.
    // SAFETY: as above.
    unsafe { std::env::set_var("PATH", &empty) };
    for (word, needle) in [
        ("http://example.com/a/b", "only https:// and ssh://"),
        ("git://example.com/a/b", "only https:// and ssh://"),
        ("file:///tmp/a", "only https:// and ssh://"),
        ("ssh+git://example.com/a", "only https:// and ssh://"),
        ("git@example.com:a/b.git", "shorthand"),
        ("example.com:a/b", "shorthand"),
        ("ext::sh -c id", "shorthand"),
        ("https://user:pw@example.com/a/b", "credentials"),
        ("https://example.com/a b", "refused"),
        ("ssh://-oProxyCommand=id/a", "refused"),
    ] {
        let error = refusal(&["install", word]);
        assert!(error.contains(needle), "{word:?}: {error}");
        assert!(!error.contains("no `git`"), "{word:?} reached git: {error}");
    }

    // 2. A valid URL with no git on PATH is a clear error.
    let error = refusal(&["install", "https://example.com/a/b"]);
    assert!(
        error.contains("no `git` on PATH") && error.contains("scoot look install DIRECTORY"),
        "{error}"
    );

    // 3. A recording stand-in git: the real argv and environment.
    let record = root.join("record");
    let script = fake.join("git");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{r}.argv'\nenv > '{r}.env'\necho 'fatal: nope' >&2\nexit 128\n",
            r = record.display()
        ),
    )
    .expect("a stand-in git");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("exec bit");
    // SAFETY: as above. The stand-in needs `env` and `printf`.
    unsafe { std::env::set_var("PATH", format!("{}:/usr/bin:/bin", fake.display())) };
    let error = refusal(&["install", "ssh://git@example.com/a/b.git"]);
    assert!(
        error.contains("git failed") && error.contains("fatal: nope"),
        "{error}"
    );
    assert!(
        !error.contains("git@"),
        "the ssh user name is not echoed: {error}"
    );

    let argv = std::fs::read_to_string(format!("{}.argv", record.display())).expect("argv");
    let args: Vec<&str> = argv.lines().collect();
    let dashes = args.iter().position(|a| *a == "--").expect("a `--`");
    assert_eq!(args[dashes + 1], "ssh://git@example.com/a/b.git");
    assert!(
        args[dashes + 2].starts_with(tmp.to_str().expect("UTF-8"))
            && args[dashes + 2].ends_with("/repo"),
        "{args:?}"
    );
    for want in [
        "clone",
        "--bare",
        "--depth",
        "1",
        "--no-tags",
        "--no-recurse-submodules",
    ] {
        assert!(args.contains(&want), "missing {want}: {args:?}");
    }
    let env = std::fs::read_to_string(format!("{}.env", record.display())).expect("env");
    let has = |line: &str| env.lines().any(|l| l == line);
    assert!(has("GIT_ALLOW_PROTOCOL=https:ssh"), "{env}");
    assert!(has("GIT_CONFIG_NOSYSTEM=1"), "{env}");
    assert!(has("GIT_CONFIG_GLOBAL=/dev/null"), "{env}");
    assert!(has("GIT_TERMINAL_PROMPT=0"), "{env}");
    assert!(
        !env.contains("scoot-test-pwned") && !env.contains("GIT_CONFIG_COUNT"),
        "inherited git state leaked: {env}"
    );

    // 4. Nothing is left behind: no temp dir, no installed look.
    assert_empty(&tmp);
    assert!(
        !home.join("data/scoot/installed-looks").exists()
            || is_empty(&home.join("data/scoot/installed-looks"))
    );
}

fn is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).map_or(true, |mut d| d.next().is_none())
}

fn assert_empty(dir: &Path) {
    let left: Vec<_> = std::fs::read_dir(dir)
        .expect("lists")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(left.is_empty(), "left in {}: {left:?}", dir.display());
}
