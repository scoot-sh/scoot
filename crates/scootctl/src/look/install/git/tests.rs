//! The git read path against local repositories (no network): the real
//! hardening arguments and environment, with only the transport widened to
//! `file` through [`Git::for_test`], never the production allowlist.
//!
//! These tests run the system `git`; a machine without it fails them
//! loudly rather than skipping.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::super::tests::{PNG, ScriptedNet, empty_catalog, manifest};
use super::{Git, GitUrl, PROD_PROTOCOLS, install_git_into, parse_git_version, parse_listing};
use crate::look::apply::ProdChecks;

/// `git` for building fixtures, isolated from the machine's own config.
fn fixture_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        // Nothing inherited: a GIT_DIR or GIT_INDEX_FILE from an outer git
        // (a hook-driven test run) must not aim fixture commits at it.
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.org")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.org")
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "protocol.file.allow=always",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A committed repository at `root/repo` holding a look named `name` plus
/// `files`, returning its `file://` URL.
fn repo(root: &Path, name: &str, files: &[(&str, &[u8])]) -> String {
    let dir = root.join("repo");
    std::fs::create_dir_all(&dir).expect("a repo dir");
    fixture_git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("look.toml"), manifest(name)).expect("a manifest");
    for (rel, bytes) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a subdir");
        std::fs::write(path, bytes).expect("a file");
    }
    commit(&dir);
    format!("file://{}", dir.display())
}

fn commit(dir: &Path) {
    fixture_git(dir, &["add", "-A"]);
    fixture_git(dir, &["commit", "-q", "-m", "look"]);
}

fn test_git() -> Git {
    Git::for_test(
        PathBuf::from("git"),
        &["file"],
        64 * 1024 * 1024,
        Duration::from_secs(60),
    )
}

/// What `install_git_into` makes of `url` into fresh scratch dirs; returns
/// the result and the scratch base so callers can assert it was cleaned.
fn install(
    git: &Git,
    url: &str,
    root: &Path,
    trust: bool,
) -> (Result<super::InstallReport, String>, PathBuf) {
    let base = root.join("tmp");
    std::fs::create_dir_all(&base).expect("a scratch base");
    let result = install_git_into(
        git,
        &GitUrl::for_test(url),
        &base,
        &root.join("installed-looks"),
        &empty_catalog(),
        &ProdChecks::none(),
        trust,
        &ScriptedNet::failing(),
    );
    (result, base)
}

fn assert_clean(base: &Path) {
    let left: Vec<_> = std::fs::read_dir(base)
        .expect("the base lists")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(left.is_empty(), "scratch not removed: {left:?}");
}

#[test]
fn a_clean_repository_installs_and_cleans_up() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let url = repo(
        root.path(),
        "from-git",
        &[
            ("wallpaper.png", PNG),
            ("README.md", b"hi"),
            ("LICENSE", b"MIT"),
        ],
    );
    let (result, base) = install(&test_git(), &url, root.path(), false);
    let report = result.expect("a clean repository installs");
    assert_eq!(report.name, "from-git");
    assert_eq!(report.dir, root.path().join("installed-looks/from-git"));
    assert_eq!(
        std::fs::read(report.dir.join("wallpaper.png")).unwrap(),
        PNG
    );
    assert_eq!(
        std::fs::read(report.dir.join("look.toml")).unwrap(),
        manifest("from-git").as_bytes()
    );
    assert!(!report.dir.join(".git").exists());
    assert_clean(&base);
}

#[test]
fn apps_leaves_follow_the_trust_flag_exactly_as_a_directory_install() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let url = repo(
        root.path(),
        "leafy",
        &[
            ("apps/nvim.lua", b"vim.g.colors_name = \"scoot-look\"\n"),
            ("apps/starship.toml", b"a = 1\n"),
        ],
    );
    let (untrusted, _) = install(&test_git(), &url, root.path(), false);
    let report = untrusted.expect("installs without trust");
    assert_eq!(report.dropped, vec!["apps/nvim.lua", "apps/starship.toml"]);
    assert!(report.kept.is_empty());
    assert!(!report.dir.join("apps/nvim.lua").exists());
    assert!(!report.dir.join("apps/starship.toml").exists());

    let again = tempfile::tempdir().expect("a second tree");
    let (trusted, _) = install(&test_git(), &url, again.path(), true);
    let report = trusted.expect("installs with trust");
    assert!(report.dropped.is_empty());
    assert_eq!(report.kept, vec!["apps/nvim.lua", "apps/starship.toml"]);
    assert_eq!(
        std::fs::read(report.dir.join("apps/nvim.lua")).unwrap(),
        b"vim.g.colors_name = \"scoot-look\"\n"
    );
}

/// Installs from a repo built by `build`, expecting a refusal containing
/// `needle`, nothing installed, scratch removed.
fn refuses_repo(build: impl FnOnce(&Path), needle: &str) {
    let root = tempfile::tempdir().expect("a scratch tree");
    let url = repo(root.path(), "hostile", &[]);
    build(&root.path().join("repo"));
    let (result, base) = install(&test_git(), &url, root.path(), true);
    let error = result.expect_err("the repository must refuse");
    assert!(error.contains(needle), "wanted {needle:?} in: {error}");
    assert!(!root.path().join("installed-looks/hostile").exists());
    assert_clean(&base);
}

#[test]
fn a_symlink_in_the_tree_refuses() {
    refuses_repo(
        |repo| {
            std::os::unix::fs::symlink("/etc/passwd", repo.join("README.md")).unwrap();
            commit(repo);
        },
        "is a symlink",
    );
}

#[test]
fn a_symlinked_wallpaper_and_a_dotdot_link_refuse() {
    refuses_repo(
        |repo| {
            std::os::unix::fs::symlink("../../../etc/shadow", repo.join("wallpaper.png")).unwrap();
            commit(repo);
        },
        "is a symlink",
    );
}

#[test]
fn a_submodule_gitlink_refuses_and_is_never_fetched() {
    refuses_repo(
        |repo| {
            fixture_git(
                repo,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    "160000,1234567890123456789012345678901234567890,vendor",
                ],
            );
            fixture_git(repo, &["commit", "-q", "-m", "sub"]);
        },
        "is a submodule",
    );
}

#[test]
fn a_gitattributes_filter_or_ident_file_refuses_by_name() {
    refuses_repo(
        |repo| {
            std::fs::write(
                repo.join(".gitattributes"),
                "* filter=lfs diff=lfs merge=lfs -text\n* ident\n",
            )
            .unwrap();
            commit(repo);
        },
        ".gitattributes is not a file a look uses",
    );
}

#[test]
fn stray_git_metadata_and_nested_directories_refuse_by_name() {
    refuses_repo(
        |repo| {
            std::fs::write(repo.join(".gitignore"), "target\n").unwrap();
            commit(repo);
        },
        ".gitignore is not a file a look uses",
    );
    refuses_repo(
        |repo| {
            std::fs::create_dir_all(repo.join(".github/workflows")).unwrap();
            std::fs::write(repo.join(".github/workflows/ci.yml"), "on: push\n").unwrap();
            commit(repo);
        },
        ".github/workflows/ci.yml is not a file a look uses",
    );
    refuses_repo(
        |repo| {
            std::fs::create_dir_all(repo.join("fonts/deeper")).unwrap();
            std::fs::write(repo.join("fonts/deeper/x.ttf"), b"x").unwrap();
            commit(repo);
        },
        "is not a file a look uses",
    );
    refuses_repo(
        |repo| {
            std::fs::write(repo.join("payload.so"), b"\x7fELF").unwrap();
            commit(repo);
        },
        "payload.so is not a file a look uses",
    );
}

#[test]
fn a_repository_with_no_look_toml_refuses() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let dir = root.path().join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    fixture_git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("README.md"), "no look here").unwrap();
    commit(&dir);
    let url = format!("file://{}", dir.display());
    let (result, base) = install(&test_git(), &url, root.path(), false);
    assert!(result.unwrap_err().contains("no look.toml"));
    assert_clean(&base);
}

#[test]
fn an_empty_repository_is_a_clear_refusal() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let dir = root.path().join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    fixture_git(&dir, &["init", "-q"]);
    let url = format!("file://{}", dir.display());
    let (result, base) = install(&test_git(), &url, root.path(), false);
    let error = result.unwrap_err();
    assert!(
        error.contains("git failed") || error.contains("no look"),
        "{error}"
    );
    assert_clean(&base);
}

#[test]
fn too_many_files_and_an_oversize_blob_refuse_from_the_listing() {
    refuses_repo(
        |repo| {
            for i in 0..70 {
                std::fs::write(repo.join(format!("LICENSE-{i}")), b"x").unwrap();
            }
            commit(repo);
        },
        "more than 64 files",
    );
    refuses_repo(
        |repo| {
            std::fs::write(repo.join("README.md"), vec![b'a'; 200 * 1024]).unwrap();
            commit(repo);
        },
        "over the 65536 byte cap",
    );
}

#[test]
fn a_file_named_like_git_internals_cannot_smuggle_a_checkout_effect() {
    // `.git` as a path inside a tree is refused by git itself on fetch
    // (fsckObjects); a `.gitmodules` file is refused by name by the gate.
    refuses_repo(
        |repo| {
            std::fs::write(repo.join(".gitmodules"), "[submodule \"x\"]\n").unwrap();
            commit(repo);
        },
        ".gitmodules is not a file a look uses",
    );
}

#[test]
fn raw_blobs_arrive_unconverted() {
    // CRLF and `$Id$` bytes survive untouched: `cat-file blob` runs no
    // conversion, and the clone is bare so no checkout ran one.
    let root = tempfile::tempdir().expect("a scratch tree");
    let body: &[u8] = b"line one\r\n$Id$\r\n";
    let url = repo(root.path(), "raw", &[("README.md", body)]);
    let (result, _) = install(&test_git(), &url, root.path(), false);
    let report = result.expect("installs");
    assert_eq!(std::fs::read(report.dir.join("README.md")).unwrap(), body);
}

#[test]
fn a_name_already_taken_refuses_and_leaves_no_scratch() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let url = repo(root.path(), "twice", &[]);
    let (first, _) = install(&test_git(), &url, root.path(), false);
    first.expect("the first installs");
    let (second, base) = install(&test_git(), &url, root.path(), false);
    assert!(second.unwrap_err().contains("already"));
    assert_clean(&base);
}

#[test]
fn the_clone_directory_is_private_and_unique() {
    let base = tempfile::tempdir().expect("a base");
    let a = super::Scratch::new(base.path()).expect("a scratch");
    let b = super::Scratch::new(base.path()).expect("another scratch");
    assert_ne!(a.path(), b.path());
    let mode = std::fs::metadata(a.path()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    let kept = a.path().to_owned();
    drop(a);
    assert!(!kept.exists());
    drop(b);
    assert_clean(base.path());
}

#[test]
fn a_planted_name_is_never_reused() {
    // The scratch name embeds the pid and the clock; plant a symlink at
    // every candidate it could pick and it must fail, not follow.
    let base = tempfile::tempdir().expect("a base");
    let target = tempfile::tempdir().expect("a target");
    let pid = std::process::id();
    let scratch = super::Scratch::new(base.path()).expect("a scratch");
    let name = scratch
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with(&format!("scoot-look-clone-{pid}-")));
    // Re-creating the same path refuses (AlreadyExists), proving mkdir is
    // exclusive; a symlink there is also AlreadyExists, never followed.
    let planted = base.path().join("planted");
    std::os::unix::fs::symlink(target.path(), &planted).unwrap();
    use std::os::unix::fs::DirBuilderExt as _;
    let error = std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&planted)
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
}

#[test]
fn a_missing_git_is_a_clear_error_not_a_panic() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let git = Git::for_test(
        root.path().join("no-such-git"),
        &["file"],
        1 << 20,
        Duration::from_secs(5),
    );
    let (result, base) = install(&git, "file:///nowhere", root.path(), false);
    let error = result.unwrap_err();
    assert!(error.contains("no `git` on PATH"), "{error}");
    assert_clean(&base);
}

/// An executable stand-in for git, run through the real runner.
fn fake_git(dir: &Path, script: &str) -> PathBuf {
    let path = dir.join("fake-git");
    // Every stand-in answers the version check the runner makes first.
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\ncase \"$1\" in --version) echo 'git version 2.45.0'; exit 0;; esac\n{script}\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn a_hung_clone_is_killed_at_the_deadline() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let bin = fake_git(root.path(), "exec sleep 30");
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_millis(300));
    let started = std::time::Instant::now();
    let (result, base) = install(&git, "file:///x", root.path(), false);
    assert!(result.unwrap_err().contains("did not finish"));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_clean(&base);
}

#[test]
fn a_clone_past_the_size_cap_is_killed() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let bin = fake_git(
        root.path(),
        "head -c 3000000 /dev/zero > \"$PWD/big\"; exec sleep 30",
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(60));
    let started = std::time::Instant::now();
    let (result, base) = install(&git, "file:///x", root.path(), false);
    let error = result.unwrap_err();
    assert!(error.contains("is larger than 1048576 bytes"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_clean(&base);
}

#[test]
fn a_real_git_clone_past_the_size_cap_fails_with_nothing_left() {
    // Incompressible bytes (a seeded xorshift stream), so the pack really
    // is about 3 MiB. Whether the poll loop or the post-exit check sees it
    // first, the refusal is the same.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let blob: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 32) as u8
        })
        .collect();
    let root = tempfile::tempdir().expect("a scratch tree");
    let url = repo(root.path(), "heavy", &[("wallpaper.png", &blob)]);
    let git = Git::for_test(
        PathBuf::from("git"),
        &["file"],
        1 << 20,
        Duration::from_secs(60),
    );
    let started = std::time::Instant::now();
    let (result, base) = install(&git, &url, root.path(), false);
    let error = result.unwrap_err();
    assert!(error.contains("is larger than 1048576 bytes"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(20));
    assert!(!root.path().join("installed-looks/heavy").exists());
    assert_clean(&base);
}

#[test]
fn git_words_reach_the_user_escaped_and_truncated() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let bin = fake_git(
        root.path(),
        "printf 'remote: \\033[31mboom\\007 %s\\n' \"$(head -c 900 /dev/zero | tr '\\0' 'x')\" >&2; exit 128",
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(30));
    let (result, _) = install(&git, "file:///x", root.path(), false);
    let error = result.unwrap_err();
    assert!(
        !error.contains('\x1b') && !error.contains('\x07'),
        "{error:?}"
    );
    assert!(error.contains("boom"), "{error}");
    assert!(error.len() < 500, "{} bytes", error.len());
}

#[test]
fn clone_arguments_are_pinned() {
    let git = Git::for_test(
        PathBuf::from("git"),
        PROD_PROTOCOLS,
        1,
        Duration::from_secs(1),
    );
    let args: Vec<String> = git
        .clone_args("https://example.com/r", Path::new("/scratch/repo"))
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let joined = args.join(" ");
    for want in [
        "--no-replace-objects",
        "-c protocol.allow=never",
        "-c protocol.https.allow=always",
        "-c protocol.ssh.allow=always",
        "-c core.hooksPath=/dev/null",
        "-c core.fsmonitor=false",
        "-c transfer.fsckObjects=true",
        "-c http.followRedirects=false",
        "-c credential.helper=",
        "-c submodule.recurse=false",
        "clone --quiet --bare --depth 1 --single-branch --no-tags --no-recurse-submodules --template= -- https://example.com/r /scratch/repo",
    ] {
        assert!(joined.contains(want), "missing {want:?} in {joined}");
    }
    assert!(!joined.contains("protocol.file"), "{joined}");
    assert!(!joined.contains("protocol.git"), "{joined}");
    // The URL comes after the lone `--`, so it cannot be an option.
    let dashes = args.iter().position(|a| a == "--").unwrap();
    assert_eq!(args[dashes + 1], "https://example.com/r");
    assert_eq!(args.iter().filter(|a| *a == "--").count(), 1);
}

#[test]
fn the_environment_is_an_allowlist_plus_fixed_isolation() {
    let git = Git::for_test(
        PathBuf::from("git"),
        PROD_PROTOCOLS,
        1,
        Duration::from_secs(1),
    );
    let parent = |name: &str| -> Option<OsString> {
        match name {
            "PATH" | "HOME" | "SSH_AUTH_SOCK" | "HTTPS_PROXY" => Some(format!("<{name}>").into()),
            // Hostile or merely inherited: none of these may reach git.
            "GIT_SSH_COMMAND"
            | "GIT_CONFIG_COUNT"
            | "GIT_DIR"
            | "GIT_EXEC_PATH"
            | "GIT_CONFIG_PARAMETERS"
            | "GIT_ASKPASS"
            | "SSH_ASKPASS"
            | "GIT_PROXY_COMMAND" => Some("evil".into()),
            _ => None,
        }
    };
    let env = git.env(parent);
    let get = |name: &str| {
        env.iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.to_string_lossy().into_owned())
    };
    assert_eq!(get("PATH").as_deref(), Some("<PATH>"));
    assert_eq!(get("HOME").as_deref(), Some("<HOME>"));
    assert_eq!(get("SSH_AUTH_SOCK").as_deref(), Some("<SSH_AUTH_SOCK>"));
    assert_eq!(get("HTTPS_PROXY").as_deref(), Some("<HTTPS_PROXY>"));
    assert_eq!(get("GIT_CONFIG_NOSYSTEM").as_deref(), Some("1"));
    assert_eq!(get("GIT_CONFIG_GLOBAL").as_deref(), Some("/dev/null"));
    assert_eq!(get("GIT_TERMINAL_PROMPT").as_deref(), Some("0"));
    assert_eq!(get("GIT_ALLOW_PROTOCOL").as_deref(), Some("https:ssh"));
    let ssh = get("GIT_SSH_COMMAND").unwrap();
    assert!(
        ssh.starts_with("ssh ") && ssh.contains("BatchMode=yes"),
        "{ssh}"
    );
    for name in [
        "GIT_CONFIG_COUNT",
        "GIT_DIR",
        "GIT_EXEC_PATH",
        "GIT_CONFIG_PARAMETERS",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GIT_PROXY_COMMAND",
    ] {
        assert_eq!(get(name), None, "{name} must not pass through");
    }
}

#[test]
fn the_production_transport_allowlist_is_https_and_ssh_only() {
    assert_eq!(PROD_PROTOCOLS, ["https", "ssh"]);
}

#[test]
fn the_listing_gate_refuses_each_unsafe_entry_shape() {
    let oid = "0123456789abcdef0123456789abcdef01234567";
    let line = |mode: &str, kind: &str, size: &str, path: &[u8]| -> Vec<u8> {
        let mut out = format!("{mode} {kind} {oid} {size:>7}\t").into_bytes();
        out.extend_from_slice(path);
        out.push(0);
        out
    };
    let ok = parse_listing(&line("100644", "blob", "12", b"look.toml")).unwrap();
    assert_eq!((ok[0].rel.as_str(), ok[0].size), ("look.toml", 12));
    parse_listing(&line("100755", "blob", "12", b"README.md")).expect("executable files pass");
    for (mode, kind, path, needle) in [
        ("120000", "blob", &b"README.md"[..], "is a symlink"),
        ("160000", "commit", b"vendor", "is a submodule"),
        ("100644", "commit", b"vendor", "is a submodule"),
        ("100664", "blob", b"README.md", "not a regular file"),
        ("040000", "tree", b"fonts", "not a regular file"),
        ("100644", "blob", b"../escape.png", "refused"),
        ("100644", "blob", b"/abs.png", "refused"),
        ("100644", "blob", b"a//b.png", "refused"),
        ("100644", "blob", b"nul\x01.png", "refused"),
    ] {
        let error = parse_listing(&line(mode, kind, "1", path)).unwrap_err();
        assert!(error.contains(needle), "{mode} {kind}: {error}");
    }
    let error = parse_listing(&line("100644", "blob", "1", b"\xff.png")).unwrap_err();
    assert!(error.contains("not valid UTF-8"), "{error}");
    for garbage in [
        &b"no tab here"[..],
        b"100644 blob\tREADME.md",
        b"100644 blob nothex 1\tREADME.md",
    ] {
        let mut record = garbage.to_vec();
        record.push(0);
        assert!(parse_listing(&record).unwrap_err().contains("unreadable"));
    }
    let mut many = Vec::new();
    for i in 0..70 {
        many.extend(line(
            "100644",
            "blob",
            "1",
            format!("LICENSE-{i}").as_bytes(),
        ));
    }
    assert!(
        parse_listing(&many)
            .unwrap_err()
            .contains("more than 64 files")
    );
}

// --- escapes, stderr, process groups, version, caps ---

/// Every dangerous character a repository path or git's stderr can carry.
const HOSTILE: &str = "\u{1b}]0;PWNED\u{7}\u{1b}[2J";

fn assert_inert(text: &str) {
    for c in text.chars() {
        assert!(
            !super::url::is_unsafe_char(c),
            "{c:?} reached the message: {text:?}"
        );
    }
}

#[test]
fn hostile_paths_never_reach_a_message_raw() {
    let oid = "0123456789abcdef0123456789abcdef01234567";
    let line = |mode: &str, kind: &str, path: &str| -> Vec<u8> {
        let mut out = format!("{mode} {kind} {oid}       1\t{path}").into_bytes();
        out.push(0);
        out
    };
    let names = [
        format!("{HOSTILE}.png"),
        "line1\nline2.png".to_owned(),
        "cr\rname.png".to_owned(),
        "rtl\u{202e}gnp.exe".to_owned(),
        "iso\u{2066}x.png".to_owned(),
        "zw\u{200b}x.png".to_owned(),
        "bom\u{feff}x.png".to_owned(),
        "tag\u{e0041}x.png".to_owned(),
        "del\u{7f}x.png".to_owned(),
        "c1\u{85}x.png".to_owned(),
    ];
    for name in &names {
        for (mode, kind) in [
            ("120000", "blob"),
            ("160000", "commit"),
            ("100664", "blob"),
            ("100644", "blob"),
        ] {
            let error = parse_listing(&line(mode, kind, name)).unwrap_err();
            assert_inert(&error);
            assert_eq!(error.lines().count(), 1, "{error:?}");
        }
    }
    // The escape spelling is visible, so the reader sees what was there.
    let error = parse_listing(&line("100644", "blob", "a\u{202e}b.png")).unwrap_err();
    assert!(error.contains("\\u{202e}"), "{error}");
}

#[test]
fn a_real_repository_symlink_with_an_escape_name_prints_inert() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let url = repo(root.path(), "esc", &[]);
    let dir = root.path().join("repo");
    std::os::unix::fs::symlink("/etc/passwd", dir.join(format!("{HOSTILE}.png"))).unwrap();
    commit(&dir);
    let (result, base) = install(&test_git(), &url, root.path(), true);
    let error = result.unwrap_err();
    assert_inert(&error);
    assert!(error.contains("control character"), "{error}");
    assert_clean(&base);
}

#[test]
fn git_stderr_with_escapes_newlines_and_bidi_prints_inert() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let bin = fake_git(
        root.path(),
        r"printf 'remote: \033]0;PWNED\007\033[2J\342\200\256evil\342\201\246x\n' >&2; exit 128",
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(30));
    let (result, _) = install(&git, "file:///x", root.path(), false);
    let error = result.unwrap_err();
    assert_inert(&error);
    assert!(error.contains("PWNED") && error.contains("evil"), "{error}");
    assert_eq!(error.lines().count(), 1, "{error:?}");
}

#[test]
fn the_ssh_host_key_line_survives_gits_three_line_tail() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let bin = fake_git(
        root.path(),
        r"printf 'Host key verification failed.\nfatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nand the repository exists.\n' >&2; exit 128",
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(30));
    let (result, _) = install(&git, "file:///x", root.path(), false);
    let error = result.unwrap_err();
    assert!(error.contains("Host key verification failed."), "{error}");
    assert!(error.contains("and the repository exists."), "{error}");
}

#[test]
fn a_long_stderr_keeps_its_first_and_last_lines_and_stays_bounded() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let bin = fake_git(
        root.path(),
        r#"echo 'Permission denied (publickey).' >&2; i=0; while [ $i -lt 400 ]; do echo "noise line $i" >&2; i=$((i+1)); done; echo 'fatal: the end' >&2; exit 128"#,
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(30));
    let (result, _) = install(&git, "file:///x", root.path(), false);
    let error = result.unwrap_err();
    assert!(error.contains("Permission denied (publickey)."), "{error}");
    assert!(error.contains("fatal: the end"), "{error}");
    assert!(error.len() < 1500, "{} bytes", error.len());
}

/// Whether process `pid` is gone (or only a zombie awaiting a reaper).
fn process_gone(pid: u32) -> bool {
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let stat = String::from_utf8_lossy(&out.stdout);
    let stat = stat.trim();
    stat.is_empty() || stat.starts_with('Z')
}

fn wait_gone(pid: u32) -> bool {
    (0..100).any(|_| {
        if process_gone(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
        false
    })
}

fn read_pid(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .expect("the stand-in wrote its pid")
        .trim()
        .parse()
        .expect("a pid")
}

#[test]
fn a_deadline_kills_the_grandchildren_too() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let pidfile = root.path().join("grandchild.pid");
    let bin = fake_git(
        root.path(),
        &format!("sleep 300 & echo $! > '{}'; wait", pidfile.display()),
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_millis(400));
    let (result, base) = install(&git, "file:///x", root.path(), false);
    assert!(result.unwrap_err().contains("did not finish"));
    assert_clean(&base);
    let pid = read_pid(&pidfile);
    assert!(wait_gone(pid), "grandchild {pid} outlived the deadline");
}

#[test]
fn the_size_cap_kills_the_grandchildren_too() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let pidfile = root.path().join("grandchild.pid");
    let bin = fake_git(
        root.path(),
        &format!(
            r#"sleep 300 & echo $! > '{}'; head -c 3000000 /dev/zero > "$PWD/big"; wait"#,
            pidfile.display()
        ),
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(60));
    let (result, _) = install(&git, "file:///x", root.path(), false);
    assert!(result.unwrap_err().contains("is larger than"));
    let pid = read_pid(&pidfile);
    assert!(wait_gone(pid), "grandchild {pid} outlived the size cap");
}

#[test]
fn a_helper_that_outlives_git_is_killed_with_its_group() {
    // git exits at once, leaving a helper behind in its group.
    let root = tempfile::tempdir().expect("a scratch tree");
    let pidfile = root.path().join("helper.pid");
    let bin = fake_git(
        root.path(),
        &format!(
            "sleep 300 >/dev/null 2>&1 & echo $! > '{}'; exit 128",
            pidfile.display()
        ),
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(30));
    let (result, _) = install(&git, "file:///x", root.path(), false);
    assert!(result.is_err());
    let pid = read_pid(&pidfile);
    assert!(wait_gone(pid), "helper {pid} outlived git");
}

#[test]
fn the_watchdog_stops_git_and_removes_the_directory_when_scoot_dies() {
    use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
    let base = tempfile::tempdir().expect("a base");
    let mut scratch = super::Scratch::new(base.path()).expect("a scratch");
    let dir = scratch.path().to_owned();
    std::fs::write(dir.join("partial"), b"half a clone").unwrap();
    let mut git = Command::new("sleep")
        .arg("300")
        .process_group(0)
        .spawn()
        .expect("a stand-in git");
    scratch.register_group(git.id());
    // What scoot being killed does: the pipe to the watchdog closes with
    // nothing else cleaned up.
    let mut watchdog = scratch.simulate_death().expect("a watchdog ran");
    watchdog.wait().expect("the watchdog finishes");
    let status = git.wait().expect("git is reaped");
    assert_eq!(status.signal(), Some(9), "git was not killed: {status:?}");
    assert!(!dir.exists(), "the watchdog left {}", dir.display());
}

/// The runner, not just `Scratch`, must hand git's group to the watchdog:
/// the unit above registers by hand and so cannot see the call deleted.
/// Git leads its own group (its pid is its group id), so the first id
/// registered is the stand-in's own pid, and the last is 0 (git is gone).
#[test]
fn the_runner_registers_gits_group_with_the_watchdog_then_clears_it() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let pidfile = root.path().join("git.pid");
    let bin = fake_git(
        root.path(),
        &format!("echo $$ > '{}'; exit 128", pidfile.display()),
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(30));
    super::scratch::REGISTERED.with(|log| log.borrow_mut().clear());
    let (result, base) = install(&git, "file:///x", root.path(), false);
    assert!(result.is_err());
    assert_clean(&base);
    let pid = read_pid(&pidfile);
    let registered = super::scratch::REGISTERED.with(|log| log.borrow().clone());
    // Each git run registers its group, then 0; the failing clone is the
    // only run that reaches the stand-in past `--version`.
    assert!(
        registered.contains(&pid),
        "git {pid} was never registered with the watchdog: {registered:?}"
    );
    assert_eq!(registered.last(), Some(&0), "{registered:?}");
    let at = registered.iter().position(|g| *g == pid).unwrap();
    assert!(
        registered[at + 1..].contains(&0),
        "the group was not cleared after git exited: {registered:?}"
    );
}

#[test]
fn a_finished_git_is_not_signalled_by_the_watchdog() {
    use std::os::unix::process::CommandExt as _;
    let base = tempfile::tempdir().expect("a base");
    let mut scratch = super::Scratch::new(base.path()).expect("a scratch");
    // A bystander in its own group stands for a reused group id.
    let mut bystander = Command::new("sleep")
        .arg("300")
        .process_group(0)
        .spawn()
        .expect("a bystander");
    scratch.register_group(bystander.id());
    scratch.register_group(0);
    let mut watchdog = scratch.simulate_death().expect("a watchdog ran");
    watchdog.wait().expect("the watchdog finishes");
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "the bystander died"
    );
    bystander.kill().unwrap();
    bystander.wait().unwrap();
}

#[test]
fn git_versions_parse_and_old_or_unreadable_ones_refuse() {
    for (said, want) in [
        ("git version 2.45.0\n", Some((2, 45))),
        ("git version 2.39.2 (Apple Git-143)\n", Some((2, 39))),
        ("git version 2.43.0.windows.1\n", Some((2, 43))),
        ("git version 2.32.0", Some((2, 32))),
        ("git version 2.9.5", Some((2, 9))),
        ("git version 10.0.1", Some((10, 0))),
        ("git version 2.56.0.rc0.gabcdef", Some((2, 56))),
        ("git version", None),
        ("git version x.y", None),
        ("git version 2", None),
        ("2.45.0", None),
        ("", None),
        ("fake git 2.45.0", None),
    ] {
        assert_eq!(parse_git_version(said), want, "{said:?}");
    }
    let root = tempfile::tempdir().expect("a scratch tree");
    for (said, needle) in [
        ("git version 2.31.1", "too old"),
        ("git version 1.8.3", "too old"),
        ("garbage", "cannot tell which git"),
    ] {
        let bin = root.path().join("old-git");
        std::fs::write(&bin, format!("#!/bin/sh\necho '{said}'\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(5));
        let (result, base) = install(&git, "file:///x", root.path(), false);
        let error = result.unwrap_err();
        assert!(error.contains(needle), "{said}: {error}");
        assert_clean(&base);
    }
}

#[test]
fn a_git_that_finishes_over_the_cap_between_two_polls_is_still_refused() {
    // The stand-in writes a sparse 3 MB file and exits; the poll is slow
    // enough that the loop looks only before it starts and after it ends,
    // so only the check after exit can see the file.
    let root = tempfile::tempdir().expect("a scratch tree");
    let bin = fake_git(
        root.path(),
        r#"dd if=/dev/null of="$PWD/big" bs=1 seek=3000000 2>/dev/null; exit 0"#,
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(60))
        .with_poll(Duration::from_millis(400));
    // One run, not a whole install: later runs would trip the in-loop
    // check on the leftover file and hide a missing post-exit check.
    let mut scratch = super::Scratch::new(root.path()).expect("a scratch");
    let error = git
        .run(
            &mut scratch,
            &[OsString::from("clone")],
            None,
            Duration::from_secs(60),
        )
        .expect_err("a run that left 3 MB behind must refuse");
    assert!(error.contains("is larger than 1048576 bytes"), "{error}");
}

#[test]
fn only_a_missing_head_is_called_an_empty_repository() {
    let root = tempfile::tempdir().expect("a scratch tree");
    let bin = fake_git(
        root.path(),
        r#"case "$*" in *ls-tree*) echo 'fatal: bad object store' >&2; exit 128;; esac; exit 0"#,
    );
    let git = Git::for_test(bin, &["file"], 1 << 20, Duration::from_secs(30));
    let (result, _) = install(&git, "file:///x", root.path(), false);
    let error = result.unwrap_err();
    assert!(error.contains("bad object store"), "{error}");
    assert!(!error.contains("empty repository"), "{error}");
}
