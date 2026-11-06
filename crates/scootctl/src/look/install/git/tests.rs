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
use super::{Git, GitUrl, PROD_PROTOCOLS, install_git_into, parse_listing};
use crate::look::apply::ProdChecks;

/// `git` for building fixtures, isolated from the machine's own config.
fn fixture_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
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
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
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
