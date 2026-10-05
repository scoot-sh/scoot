//! [`super`], without the network: URL shapes, hashes, paths, magic,
//! the cache hit, and one local HTTP server for the download itself.

use super::*;
use std::io::Write as _;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[test]
fn the_tail_never_splits_a_char() {
    // 499 ASCII bytes, then a two-byte `é` straddling the 500-byte cut:
    // slicing there by bytes panics, and the release profile aborts the
    // daemon on panic.
    let mut line = "a".repeat(499);
    line.push('é');
    line.push_str(&"b".repeat(100));
    let stderr = format!("curl: (6) noise\n{line}\n");
    let detail = tail(stderr.as_bytes());
    assert!(detail.ends_with("..."), "{detail:?}");
    assert!(detail.len() <= 500 + 3, "{detail:?}");
    // A four-byte char straddling the same cut.
    let mut line = "a".repeat(499);
    line.push('🦀');
    line.push_str(&"b".repeat(100));
    let detail = tail(line.as_bytes());
    assert!(detail.ends_with("..."), "{detail:?}");
    assert!(detail.len() <= 500 + 3, "{detail:?}");
    // Short lines still pass through whole, without the ellipsis.
    assert_eq!(
        tail(b"curl: (6) nothing resolves that host"),
        "curl: (6) nothing resolves that host"
    );
    assert_eq!(tail(b""), "");
}

#[test]
fn urls_are_http_and_https() {
    for url in [
        "http://example.com/a.png",
        "https://example.com/a.png",
        "https://127.0.0.1:1/a.png",
    ] {
        assert!(is_url(url), "{url}");
    }
    for not in [
        "",
        "/home/me/a.png",
        "a.png",
        "file:///home/me/a.png",
        "ftp://example.com/a.png",
        "https:/example.com/a.png",
        "HTTP://example.com/a.png",
    ] {
        assert!(!is_url(not), "{not}");
    }
}

#[test]
fn sha256_parses_hex_either_case() {
    let lower = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    let upper = "9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08";
    assert_eq!(parse_sha256(lower).unwrap(), crate::sha256::digest(b"test"));
    assert_eq!(parse_sha256(upper), parse_sha256(lower));
}

#[test]
fn sha256_refuses_anything_else() {
    for bad in [
        "",
        "9f86",
        "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a0",
        "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a0zz",
        "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a0 ",
    ] {
        let Err(message) = parse_sha256(bad) else {
            panic!("{bad:?} parsed");
        };
        assert!(message.contains("64 hex digits"), "{message}");
    }
}

#[test]
fn the_cache_dir_prefers_xdg_cache_home() {
    let xdg: &OsStr = "/tmp/xdg".as_ref();
    let home: &OsStr = "/home/me".as_ref();
    assert_eq!(
        dir_from(Some(xdg), Some(home)).as_deref(),
        Ok(Path::new("/tmp/xdg/scootbg"))
    );
    // Unset, empty and relative XDG_CACHE_HOME all fall back to HOME.
    assert_eq!(
        dir_from(None, Some(home)).as_deref(),
        Ok(Path::new("/home/me/.cache/scootbg"))
    );
    assert_eq!(
        dir_from(Some("".as_ref()), Some(home)).as_deref(),
        Ok(Path::new("/home/me/.cache/scootbg"))
    );
    assert_eq!(
        dir_from(Some("relative".as_ref()), Some(home)).as_deref(),
        Ok(Path::new("/home/me/.cache/scootbg"))
    );
    // Nowhere to keep anything: no absolute directory anywhere.
    assert_eq!(dir_from(None, None), Err(FetchError::NoCacheDir));
    assert_eq!(
        dir_from(Some("relative".as_ref()), None),
        Err(FetchError::NoCacheDir)
    );
    assert_eq!(
        dir_from(None, Some("relative".as_ref())),
        Err(FetchError::NoCacheDir)
    );
}

#[test]
fn the_cache_path_is_keyed_by_the_url() {
    let dir = Path::new("/cache");
    let first = cached_path(dir, "https://example.com/a.png");
    assert_eq!(first, cached_path(dir, "https://example.com/a.png"));
    assert_ne!(first, cached_path(dir, "https://example.com/b.png"));
    assert_ne!(first, cached_path(dir, "http://example.com/a.png"));
    let name = first.file_name().unwrap().to_str().unwrap();
    assert_eq!(name.len(), 64, "{name}");
    assert!(name.bytes().all(|b| b.is_ascii_hexdigit()), "{name}");
}

#[test]
fn the_magic_is_png_jpeg_or_webp() {
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), b"rest"].concat();
    let jpeg = [b"\xff\xd8\xff".as_slice(), b"rest"].concat();
    let webp = [b"RIFF....WEBP".as_slice(), b"rest"].concat();
    assert!(is_image(&png));
    assert!(is_image(&jpeg));
    assert!(is_image(&webp));
    for not in [
        vec![],
        b"<".to_vec(),
        b"<html><body>nope".to_vec(),
        b"RIFF....NOTW".to_vec(),
        b"RIFF".to_vec(),
        b"\x89PNG\r\n".to_vec(),
        b"\xff\xd8".to_vec(),
        b"GIF89a...".to_vec(),
    ] {
        assert!(!is_image(&not), "{not:?}");
    }
}

#[test]
fn a_long_url_is_refused_before_any_io() {
    let dir = std::env::temp_dir().join("scootbg-fetch-long-url");
    let _ = std::fs::remove_dir_all(&dir);
    let fetch = Fetch {
        url: format!("https://example.com/{}", "a".repeat(MAX_URL)),
        sha256: None,
    };
    let Err(FetchError::TooLong(len)) = ensure(&dir, &fetch) else {
        panic!("a {MAX_URL}-plus URL was taken");
    };
    assert!(len > MAX_URL, "{len}");
    assert!(!dir.exists(), "nothing was made for a refused URL");
}

/// A directory for one test, removed first: no two tests share one.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scootbg-fetch-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_hit_without_a_pin_needs_only_the_file() {
    let dir = scratch("hit-plain");
    let fetch = Fetch {
        url: "https://example.com/a.png".to_owned(),
        sha256: None,
    };
    let cached = cached_path(&dir, &fetch.url);
    std::fs::write(&cached, png_bytes()).unwrap();
    assert_eq!(ensure(&dir, &fetch), Ok(cached));
}

#[test]
fn a_hit_checks_the_pin_and_drops_what_fails_it() {
    let dir = scratch("hit-pinned");
    let bytes = png_bytes();
    let good = crate::sha256::digest(&bytes);
    // Unreachable, fast: connection refused on loopback, no network.
    let url = format!("http://127.0.0.1:{}/a.png", closed_port());
    let fetch = Fetch {
        url: url.clone(),
        sha256: Some(good),
    };
    let cached = cached_path(&dir, &url);
    std::fs::write(&cached, &bytes).unwrap();
    assert_eq!(ensure(&dir, &fetch), Ok(cached.clone()));
    // Corrupt it: the hit is removed, and with nothing listening the
    // download fails rather than show it.
    std::fs::write(&cached, b"corrupt").unwrap();
    let Err(FetchError::Failed { .. }) = ensure(&dir, &fetch) else {
        panic!("corrupt bytes passed the pin");
    };
    assert!(!cached.exists(), "corrupt bytes stay for no one");
}

/// A loopback port nothing listens on: connection refused, at once.
fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn check_pins_the_hash_and_refuses_html() {
    let dir = scratch("check");
    let bytes = png_bytes();
    let good = crate::sha256::digest(&bytes);
    let temp = dir.join("temp");
    std::fs::write(&temp, &bytes).unwrap();
    let fetch = Fetch {
        url: "https://example.com/a.png".to_owned(),
        sha256: Some(good),
    };
    assert!(check(&temp, &fetch).is_ok());
    let bad = Fetch {
        sha256: Some([0u8; 32]),
        ..fetch.clone()
    };
    let Err(FetchError::Mismatch { .. }) = check(&temp, &bad) else {
        panic!("a wrong pin passed");
    };
    std::fs::write(&temp, b"<html>nope").unwrap();
    // Unpinned, the magic decides: an error page is never cached, so a
    // fixed link is picked up by the next request.
    let plain = Fetch {
        sha256: None,
        ..fetch.clone()
    };
    let Err(FetchError::NotAnImage { bytes, .. }) = check(&temp, &plain) else {
        panic!("HTML passed");
    };
    assert_eq!(bytes, "<html>nope".len() as u64);
}

/// A 1×1 PNG: the smallest thing `is_image` takes.
fn png_bytes() -> Vec<u8> {
    vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xfe, 0xd4, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}

#[test]
fn the_sweep_keeps_the_fresh_and_takes_the_stale() {
    let dir = scratch("sweep");
    let fresh = dir.join("aaa.part-1");
    let stale = dir.join("bbb.part-2");
    let other = dir.join("not-a-temp");
    std::fs::write(&fresh, b"x").unwrap();
    std::fs::write(&stale, b"x").unwrap();
    std::fs::write(&other, b"x").unwrap();
    let old = SystemTime::now() - Duration::from_secs(STALE_AFTER_SECS + 60);
    filetime(&stale, old);
    sweep(&dir);
    assert!(fresh.exists());
    assert!(!stale.exists());
    assert!(other.exists());
}

/// Sets a file's modification time, through `touch` (no new dependency
/// for one test).
fn filetime(path: &Path, modified: SystemTime) {
    let secs = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let status = Command::new("touch")
        .arg("-d")
        .arg(format!("@{secs}"))
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    assert!(
        status.is_ok_and(|status| status.success()),
        "touch {path:?}"
    );
}

// -- The download, against a local server ---------------------------------

/// One canned HTTP response.
struct Route {
    status: &'static str,
    headers: &'static str,
    body: Vec<u8>,
}

/// Serves `routes` by path on 127.0.0.1, one thread, until the process
/// exits (nextest runs each test in its own process; plain `cargo test`
/// leaves one idle thread per serving test behind, blocked in `accept`).
/// Plain HTTP/1.0 over std only, no dependency for a test fixture.
/// Returns the base URL and the requests served so far.
fn serve(routes: Vec<(&'static str, Route)>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let served = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            served.fetch_add(1, Ordering::Relaxed);
            let mut stream = stream.unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while head.len() < 8192 {
                use std::io::Read as _;
                if stream.read(&mut byte).is_err() || byte[0] == 0 {
                    break;
                }
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let path = String::from_utf8_lossy(&head);
            let path = path.split_whitespace().nth(1).unwrap_or("/");
            let route = routes.iter().find(|(want, _)| *want == path);
            let (status, headers, body) = match route {
                Some((_, route)) => (route.status, route.headers, route.body.clone()),
                None => (
                    "404 Not Found",
                    "Content-Type: text/plain",
                    b"not found".to_vec(),
                ),
            };
            let _ = stream.write_all(
                format!(
                    "HTTP/1.0 {status}\r\n{headers}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.write_all(&body);
        }
    });
    (base, hits)
}

#[test]
fn a_download_lands_once_and_then_is_cached() {
    let bytes = png_bytes();
    let (base, hits) = serve(vec![(
        "/a.png",
        Route {
            status: "200 OK",
            headers: "Content-Type: image/png",
            body: bytes.clone(),
        },
    )]);
    let dir = scratch("download-once");
    let fetch = Fetch {
        url: format!("{base}/a.png"),
        sha256: Some(crate::sha256::digest(&bytes)),
    };
    let cached = ensure(&dir, &fetch).expect("the download");
    assert_eq!(std::fs::read(&cached).unwrap(), bytes);
    assert!(
        !dir.join("a.png").exists(),
        "nothing is cached under the URL's name"
    );
    assert_eq!(hits.load(Ordering::Relaxed), 1, "one download");
    // The same URL again is a cache hit: the server sees nothing.
    assert_eq!(ensure(&dir, &fetch), Ok(cached));
    assert_eq!(hits.load(Ordering::Relaxed), 1, "still one download");
}

#[test]
fn a_redirect_is_followed_and_html_is_never_cached() {
    let bytes = png_bytes();
    let (base, _) = serve(vec![
        (
            "/moved",
            Route {
                status: "302 Found",
                headers: "Location: /a.png",
                body: vec![],
            },
        ),
        (
            "/a.png",
            Route {
                status: "200 OK",
                headers: "Content-Type: image/png",
                body: bytes.clone(),
            },
        ),
        (
            "/page",
            Route {
                status: "200 OK",
                headers: "Content-Type: text/html",
                body: b"<html><body>login walled</body></html>".to_vec(),
            },
        ),
    ]);
    let dir = scratch("redirect-html");
    let moved = Fetch {
        url: format!("{base}/moved"),
        sha256: None,
    };
    let cached = ensure(&dir, &moved).expect("the redirect");
    assert_eq!(std::fs::read(&cached).unwrap(), bytes);
    let page = Fetch {
        url: format!("{base}/page"),
        sha256: None,
    };
    let Err(FetchError::NotAnImage { bytes, .. }) = ensure(&dir, &page) else {
        panic!("HTML was taken");
    };
    assert!(bytes > 0);
    assert!(
        !cached_path(&dir, &page.url).exists(),
        "HTML stays out of the cache"
    );
}

#[test]
fn a_missing_page_is_an_error_and_caches_nothing() {
    let (base, _) = serve(vec![]);
    let dir = scratch("missing-page");
    let fetch = Fetch {
        url: format!("{base}/gone.png"),
        sha256: None,
    };
    let Err(FetchError::Failed { code, .. }) = ensure(&dir, &fetch) else {
        panic!("a 404 downloaded");
    };
    // `--fail`: curl reports the HTTP error as exit 22.
    assert_eq!(code, Some(22), "a 404");
    assert!(!cached_path(&dir, &fetch.url).exists());
}

#[test]
fn nothing_answers_is_one_error() {
    // Nothing listens here (the port was just bound and dropped).
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let dir = scratch("no-server");
    let fetch = Fetch {
        url: format!("http://127.0.0.1:{port}/a.png"),
        sha256: None,
    };
    let Err(FetchError::Failed { .. }) = ensure(&dir, &fetch) else {
        panic!("nothing answered and it worked");
    };
    assert!(!cached_path(&dir, &fetch.url).exists());
}
