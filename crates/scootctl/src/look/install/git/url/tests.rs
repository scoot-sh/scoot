//! The PATH / GIT-URL split and every URL shape that refuses.

use super::{GitUrl, Source, classify};

fn git(word: &str) -> GitUrl {
    match classify(word) {
        Ok(Source::Git(url)) => url,
        other => panic!("{word:?} should be a git URL, got {other:?}"),
    }
}

fn refused(word: &str) -> String {
    match classify(word) {
        Err(error) => {
            assert!(error.starts_with("refused:"), "{word:?}: {error}");
            error
        }
        Ok(source) => panic!("{word:?} should refuse, got {source:?}"),
    }
}

#[test]
fn https_and_ssh_urls_are_git() {
    for word in [
        "https://github.com/someone/look",
        "https://github.com/someone/look.git",
        "https://git.example.org:8443/a/b/c.git",
        "https://127.0.0.1/repo",
        "ssh://git@github.com/someone/look.git",
        "ssh://git@host.example:2222/~me/look",
        "ssh://host/look",
        "ssh://deploy-user_1.x@host/look",
    ] {
        assert_eq!(git(word).as_str(), word);
    }
}

#[test]
fn paths_stay_paths() {
    for word in [
        "handed-over",
        "./handed-over",
        "../up/handed-over.tar",
        "/abs/path/look",
        "~/looks/mine",
        "./a:b",
        "./a://b",
        "../a://b.git",
        "/abs/a://b",
        "/https://example.com/r",
        "./https://example.com/r",
        "/",
        "./",
        "dir/with:colon/after-slash",
        "./-dash",
        "look.tar",
        "",
    ] {
        assert_eq!(classify(word), Ok(Source::Path), "{word:?}");
    }
}

#[test]
fn schemes_other_than_https_and_ssh_refuse_and_never_become_paths() {
    for word in [
        "http://example.com/r",
        "git://example.com/r",
        "file:///tmp/r",
        "file://localhost/tmp/r",
        "ftp://example.com/r",
        "ftps://example.com/r",
        "rsync://example.com/r",
        "git+ssh://example.com/r",
        "ssh+git://example.com/r",
        "ext::sh -c touch% /tmp/pwned",
        "HTTPS://example.com/r",
        "Https://example.com/r",
        "SSH://example.com/r",
        "://example.com/r",
        "ext://example.com/r",
        "fd://3",
    ] {
        refused(word);
    }
    assert!(refused("http://example.com/r").contains("only https:// and ssh://"));
}

#[test]
fn scp_like_and_transport_helper_shapes_refuse() {
    for word in [
        "github.com:someone/look",
        "git@github.com:someone/look.git",
        "user@host:/abs/path",
        "host:path",
        "ext::sh -c 'id'",
        "fd::3",
        "ext::git-remote-ext",
        "a:b",
        "C:\\look",
        "::",
        "x::y/z",
    ] {
        let error = refused(word);
        assert!(error.contains("shorthand"), "{word:?}: {error}");
    }
}

#[test]
fn a_leading_hyphen_refuses_even_though_the_cli_also_does() {
    for word in [
        "-oProxyCommand=id",
        "--upload-pack=touch /tmp/x",
        "-u",
        "-https://example.com/r",
    ] {
        refused(word);
    }
}

#[test]
fn ssh_hosts_and_users_cannot_be_ssh_options() {
    for word in [
        "ssh://-oProxyCommand=id/r",
        "ssh://-oProxyCommand=id@host/r",
        "ssh://-o@host/r",
        "ssh://git@-oProxyCommand=id/r",
        "ssh://git@-host/r",
        "https://-host/r",
        "ssh://.host/r",
        "ssh://-oProxyCommand=touch%20/tmp/x@h/r",
    ] {
        refused(word);
    }
}

#[test]
fn control_characters_whitespace_backslash_and_non_ascii_refuse() {
    for word in [
        "https://example.com/a b",
        "https://example.com/a\tb",
        "https://example.com/a\nb",
        "https://example.com/a\rb",
        "https://example.com/a\0b",
        "https://example.com/a\x1b[31mb",
        "https://example.com/a\\b",
        "https://exa mple.com/r",
        "https://example.com/r\u{85}",
        "https://exämple.com/r",
        "https://example.com/löök",
        "ssh://gi t@host/r",
        "ssh://host/\u{202e}r",
    ] {
        refused(word);
    }
}

#[test]
fn query_fragment_empty_host_and_empty_path_refuse() {
    for word in [
        "https://example.com/r?x=1",
        "https://example.com/r#main",
        "https://example.com?x/r",
        "https://example.com#frag/r",
        "https:///r",
        "ssh:///r",
        "https://",
        "ssh://",
        "https://example.com",
        "https://example.com/",
        "ssh://git@host",
        "ssh://git@host/",
        "https://@/r",
        "ssh://@host/r",
        "ssh://git@/r",
    ] {
        refused(word);
    }
}

#[test]
fn credentials_refuse_and_never_echo() {
    for word in [
        "https://user@example.com/r",
        "https://user:hunter2@example.com/r",
        "https://:hunter2@example.com/r",
        "ssh://git:hunter2@example.com/r",
        "ssh://:hunter2@example.com/r",
    ] {
        let error = refused(word);
        assert!(!error.contains("hunter2"), "{word:?} echoed: {error}");
    }
}

#[test]
fn ports_must_be_numbers_in_range() {
    git("https://example.com:1/r");
    git("https://example.com:65535/r");
    for word in [
        "https://example.com:/r",
        "https://example.com:0/r",
        "https://example.com:65536/r",
        "https://example.com:999999/r",
        "https://example.com:+80/r",
        "https://example.com:8o/r",
        "https://example.com:-1/r",
        "https://example.com:80:81/r",
        "https://[::1]/r",
        "https://[::1]:443/r",
        "ssh://git@[fe80::1]/r",
    ] {
        refused(word);
    }
}

#[test]
fn an_over_long_url_refuses() {
    let long = format!("https://example.com/{}", "a".repeat(2100));
    assert!(refused(&long).contains("2048"));
    let just_fits = format!("https://example.com/{}", "a".repeat(2048 - 20));
    git(&just_fits);
}

#[test]
fn refusals_echo_only_a_bounded_printable_prefix() {
    let hostile = format!("{}://example.com/r", "\x1b[31m".repeat(50));
    let error = refused(&hostile);
    assert!(!error.contains('\x1b'), "{error:?}");
    assert!(error.len() < 400, "{} bytes", error.len());
}

#[test]
fn display_drops_the_ssh_user_name() {
    assert_eq!(
        git("ssh://git@github.com/a/b").display(),
        "ssh://github.com/a/b"
    );
    assert_eq!(
        git("https://github.com/a/b").display(),
        "https://github.com/a/b"
    );
}
