//! Wallpapers from a link, end to end: `set URL` against a real
//! `scoot --headless`, served by a loopback HTTP server in this test (no
//! network beyond loopback; `curl` fetches, exactly as in production),
//! checked by real pixels and by `query`.
//!
//! The daemon's cache is a scratch directory per test (`XDG_CACHE_HOME`),
//! so no test downloads into another's cache, or the user's.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::Session;

const RED: [u8; 3] = [255, 0, 0];

/// A solid PNG of one color, encoded in-test.
fn png(color: [u8; 3], width: u32, height: u32) -> Vec<u8> {
    let data: Vec<u8> = vec![color; (width * height) as usize]
        .iter()
        .flatten()
        .copied()
        .collect();
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&data).unwrap();
    writer.finish().unwrap();
    out
}

/// Serves `body` at `/w.png` on 127.0.0.1 (404 elsewhere) until the
/// process exits; returns the URL and the requests served so far.
fn serve(body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/w.png", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let served = Arc::clone(&hits);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            served.fetch_add(1, Ordering::Relaxed);
            let mut stream = stream.unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while head.len() < 8192 {
                if stream.read(&mut byte).is_err() {
                    break;
                }
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let path = String::from_utf8_lossy(&head);
            let path = path.split_whitespace().nth(1).unwrap_or("/");
            let (status, kind, body) = if path == "/w.png" {
                ("200 OK", "image/png", body.clone())
            } else {
                ("404 Not Found", "text/plain", b"not found".to_vec())
            };
            let _ = stream.write_all(
                format!(
                    "HTTP/1.0 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.write_all(&body);
        }
    });
    (url, hits)
}

/// A loopback port nothing listens on: refused at once, no network.
fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// `scootbg ARGS`, asserting it succeeds silently.
fn ok(session: &Session, args: &[&str]) {
    let out = session.run(args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "{args:?} printed {:?}", out.stdout);
}

/// A URL is downloaded once, cached in this test's directory, and shown:
/// exact pixels (`center` copies unscaled), and `query` naming both the
/// cached file and the link.
#[test]
fn a_url_is_downloaded_once_and_shown() {
    let Some(session) = Session::start_with("download-once", 1, "") else {
        return;
    };
    let cache = session.scratch.0.join("cache");
    let mut daemon = session.daemon_logged(&[(
        "XDG_CACHE_HOME",
        cache.to_str().unwrap(),
    )]);
    let id = session.scoot_ipc(r#"{"type":"outputs"}"#)["outputs"][0]["id"]
        .as_u64()
        .unwrap();
    let (url, hits) = serve(png(RED, 64, 64));
    ok(&session, &["set", &url, "--mode", "center"]);
    assert_eq!(hits.load(Ordering::Relaxed), 1, "one download");
    // Exact pixels: unscaled red in the middle, the fill around it.
    let shot = session.scoot_screenshot(id);
    assert_eq!((shot.width, shot.height), (1600, 1000));
    assert_eq!(shot.at(800, 500), RED, "the download, unscaled");
    assert_eq!(shot.at(0, 0), [0, 0, 0], "the fill around it");
    // `query` names the cached file and the link it came from.
    let shows = session.query()["outputs"][0]["shows"].clone();
    assert_eq!(shows["mode"], "center");
    assert_eq!(shows["url"], url.as_str(), "{shows}");
    let cached = shows["image"].as_str().unwrap();
    assert!(
        std::path::Path::new(cached).starts_with(&cache),
        "{shows} is cached in this test's directory"
    );
    assert!(
        std::fs::metadata(cached).is_ok_and(|meta| meta.is_file()),
        "the cached file is there"
    );
    // Again: a cache hit, the server sees nothing.
    ok(&session, &["set", &url, "--mode", "center"]);
    assert_eq!(hits.load(Ordering::Relaxed), 1, "still one download");
    assert!(session.run(&["kill"]).status.success());
    let _ = daemon.wait();
}

/// Nothing listening is one error naming the URL, and the background
/// stays: exit 1, every output as it was.
#[test]
fn a_failed_download_keeps_the_background_and_says_why() {
    let Some(session) = Session::start_with("download-fails", 1, "") else {
        return;
    };
    let cache = session.scratch.0.join("cache");
    let mut daemon = session.daemon_logged(&[(
        "XDG_CACHE_HOME",
        cache.to_str().unwrap(),
    )]);
    let id = session.scoot_ipc(r#"{"type":"outputs"}"#)["outputs"][0]["id"]
        .as_u64()
        .unwrap();
    let url = format!("http://127.0.0.1:{}/w.png", closed_port());
    let out = session.run(&["set", &url]);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{url}: {stderr}");
    // One error line, naming the URL: the request it could not show, and
    // the fetch that failed, are one line.
    assert!(stderr.contains(&url), "{stderr}");
    assert_eq!(stderr.matches("cannot fetch").count(), 1, "{stderr}");
    // The compositor's own background, not pieces of an image.
    let background = session.scoot_screenshot(id);
    assert!(
        background.colors().len() <= 2,
        "one background, not image parts: {:?}",
        background.colors()
    );
    // A failed download caches nothing (the directories themselves are
    // made before the attempt, so they may exist, empty: the daemon
    // caches under `$XDG_CACHE_HOME/scootbg/`).
    let store = cache.join("scootbg");
    let entries: Vec<String> = store
        .exists()
        .then(|| {
            std::fs::read_dir(&store)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(entries.is_empty(), "a failed download caches: {entries:?}");
    assert!(session.run(&["kill"]).status.success());
    let _ = daemon.wait();
}

/// A download at startup (the section `apply-config` starts the daemon
/// from) that fails says why once on the daemon's stderr: the render's
/// one line, not one per output.
#[test]
fn a_startup_download_failure_is_said_once() {
    let Some(session) = Session::start_with("download-startup", 1, "") else {
        return;
    };
    // No daemon runs: `apply-config` starts one from the section. Its
    // stderr is a file, not a pipe: the detached daemon inherits it, and
    // a pipe would never see EOF while the daemon lives (scoot's own
    // `apply-config` runs the same way, into its log).
    let cache = session.scratch.0.join("cache");
    let url = format!("http://127.0.0.1:{}/w.png", closed_port());
    let json = format!(r#"{{"image":"{url}"}}"#);
    let err_file = session.scratch.0.join("apply-config.stderr");
    let err = std::fs::File::create(&err_file).unwrap();
    let status = session
        .scootbg()
        .env("XDG_CACHE_HOME", &cache)
        .args(["apply-config", "--profile", "download-startup", &json])
        .stdout(std::process::Stdio::null())
        .stderr(err)
        .status()
        .unwrap();
    let stderr = std::fs::read_to_string(&err_file).unwrap();
    assert_eq!(status.code(), Some(1), "{url}: {stderr}");
    assert!(
        stderr.contains(&url),
        "the reason names the URL: {stderr}"
    );
    assert_eq!(
        stderr.matches("cannot show").count(),
        1,
        "the download's one line, not one per output: {stderr}"
    );
    assert!(session.run(&["kill"]).status.success());
}
