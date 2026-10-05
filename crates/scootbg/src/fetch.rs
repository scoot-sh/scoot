//! Wallpapers from a link: `https://...` (or `http://...`) images,
//! downloaded once and cached.
//!
//! A remote image is keyed by its URL: the cache,
//! `$XDG_CACHE_HOME/scootbg/` (fallback `~/.cache/scootbg/`), holds one
//! file per URL, named for the SHA-256 of the URL. A request whose file is
//! there shows it; otherwise the worker thread (never the event loop)
//! downloads it by spawning `curl` — no TLS or HTTP stack is linked into
//! this binary for it — verifies the expected `sha256` when one was given,
//! checks the bytes start as a PNG, JPEG or WebP, and only then renames
//! the temporary file into place. Anything else (no `curl`, no network, an
//! HTTP error, a file past the size cap, a hash mismatch, HTML) is an
//! error naming the URL, and nothing is cached.
//!
//! **No retries, no polling.** A download runs once per worker job, and a
//! job runs only on demand: a `set`, any `apply-config` (even an unchanged
//! reload), or a render for an output that is (re)configured or
//! re-targeted while the choice is live. Offline at startup, the
//! background shows and one line says why; the next start, `set` or
//! `apply-config` tries again.
//!
//! **Two instances racing** on one cache entry both download to their own
//! temporary file; the atomic rename settles it (last wins, both valid:
//! same URL, and a pinned hash verifies identical bytes). A temporary file
//! is never renamed until the download is complete and checked, so the
//! cache never holds a partial file; stale temporaries of dead processes
//! are swept when a fetch starts.
//!
//! `file://` URLs are refused (use the path), as is any other scheme:
//! fetching local files over HTTP machinery would only add failure modes.
//! `http` fetches (curl follows redirects, capped, `https` included), but
//! anyone can read or rewrite them on the way: prefer `https`, and pin
//! `sha256` so a changed byte fails loudly instead of landing on screen.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

#[cfg(test)]
mod tests;

/// The longest URL taken, in bytes. Past this, a link is not a wallpaper
/// reference but a mistake (signed URLs are long; this still holds several
/// kilobytes of query string).
pub const MAX_URL: usize = 4096;

/// The largest download kept, in bytes (32 MiB). A 4K JPEG is a few MB; the
/// decoder refuses past 2^28 pixels anyway, so past this is either a
/// mistake or hostile, and it never reaches the disk.
pub const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// How long one download may take, all in. Past this the worker's job
/// fails (a newer `set` still supersedes it meanwhile). Generous on
/// purpose: the loop never waits on it, and a slow link still lands.
pub const MAX_TIME_SECS: u64 = 60;

/// How long the connection may take to establish.
pub const CONNECT_TIMEOUT_SECS: u64 = 10;

/// Redirects followed per download. Enough for a shortener or a CDN hop;
/// a loop fails instead of spinning.
pub const MAX_REDIRECTS: u32 = 5;

/// Temporary files older than this, by modification time, are swept when a
/// fetch starts: their downloader is long dead (a live one renames or
/// removes its own within [`MAX_TIME_SECS`]).
pub const STALE_AFTER_SECS: u64 = 7 * 24 * 3600;

/// Whether `text` is a remote image: an `http://` or `https://` URL.
pub fn is_url(text: &str) -> bool {
    text.starts_with("http://") || text.starts_with("https://")
}

/// A remote image, as a request names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetch {
    pub url: String,
    pub sha256: Option<[u8; 32]>,
}

/// Why a fetch failed. Each becomes the request's error (a `set` refusal,
/// a render's `draw_error`); the daemon keeps running and the background
/// stays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// Nowhere to keep the cache: neither `XDG_CACHE_HOME` nor `HOME` is
    /// an absolute path.
    NoCacheDir,
    /// The URL is longer than [`MAX_URL`].
    TooLong(usize),
    /// `curl` could not be started, or did not finish cleanly as a
    /// process (killed, I/O on its pipes).
    Spawn(String),
    /// `curl` ran and failed: its exit code and the bounded tail of its
    /// stderr.
    Failed {
        url: String,
        code: Option<i32>,
        detail: String,
    },
    /// The download passed [`MAX_BYTES`].
    TooLarge,
    /// The bytes are not what `sha256` pins.
    Mismatch { expected: String, actual: String },
    /// The bytes do not start as a PNG, JPEG or WebP (an error page, say):
    /// never cached, so a fixed upstream is picked up on the next request.
    NotAnImage { url: String, bytes: u64 },
    /// The cache directory cannot be made or used.
    Cache { path: PathBuf, error: String },
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCacheDir => write!(
                f,
                "neither XDG_CACHE_HOME nor HOME is an absolute path, so there is nowhere \
                 to cache a downloaded wallpaper (set one to fetch a URL)"
            ),
            Self::TooLong(len) => {
                write!(f, "the URL is {len} bytes, longer than the {MAX_URL} taken")
            }
            Self::Spawn(error) => write!(f, "cannot run `curl` ({CURL}): {error}"),
            Self::Failed { url, code, detail } => {
                let code = code.map_or("killed".to_owned(), |code| code.to_string());
                if detail.is_empty() {
                    write!(f, "cannot fetch {url:?}: `curl` exited {code}")
                } else {
                    write!(f, "cannot fetch {url:?}: `curl` exited {code}: {detail}")
                }
            }
            Self::TooLarge => write!(
                f,
                "the download is larger than the {MAX_BYTES} bytes kept; \
                 link a smaller image"
            ),
            Self::Mismatch { expected, actual } => write!(
                f,
                "the download's sha256 is {actual}, not the pinned {expected}; \
                 nothing was cached"
            ),
            Self::NotAnImage { url, bytes } => write!(
                f,
                "{url:?} downloaded {bytes} bytes that do not start as a PNG, JPEG or WebP \
                 (an error page, say); nothing was cached"
            ),
            Self::Cache { path, error } => write!(
                f,
                "cannot use the wallpaper cache {}: {error}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for FetchError {}

/// The program spawned to download: on `PATH`, looked up at spawn time so
/// a Nix profile or a `PATH` change applies without restarting the daemon.
pub const CURL: &str = "curl";

/// Parses a `sha256` value: 64 hex digits, either case.
pub fn parse_sha256(text: &str) -> Result<[u8; 32], String> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "the sha256 {text:?} is not 64 hex digits (as `sha256sum` prints)"
        ));
    }
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let high = hex(pair[0]);
        let low = hex(pair[1]);
        *slot = high << 4 | low;
    }
    Ok(out)
}

fn hex(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        b'A'..=b'F' => digit - b'A' + 10,
        _ => 0,
    }
}

/// The cache directory: `$XDG_CACHE_HOME/scootbg/`, or
/// `~/.cache/scootbg/` when `XDG_CACHE_HOME` is unset, empty or relative.
/// Pure (no I/O): making it is [`ensure`]'s.
pub fn dir() -> Result<PathBuf, FetchError> {
    dir_from(
        std::env::var_os("XDG_CACHE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// [`dir`], from given values: unset and empty mean the same, as in
/// libwayland, and a relative directory is refused (the daemon's working
/// directory is `/`).
fn dir_from(xdg_cache_home: Option<&OsStr>, home: Option<&OsStr>) -> Result<PathBuf, FetchError> {
    let base = xdg_cache_home
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| {
            home.filter(|home| !home.is_empty())
                .map(PathBuf::from)
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".cache"))
        })
        .ok_or(FetchError::NoCacheDir)?;
    Ok(base.join("scootbg"))
}

/// The cache file for `url` in `dir`: the hex SHA-256 of the URL.
/// Extensionless: the bytes are told apart by content, not by name, as
/// local files are.
pub fn cached_path(dir: &Path, url: &str) -> PathBuf {
    dir.join(crate::sha256::hex(url.as_bytes()))
}

/// Downloads `fetch` into the cache unless it is already there (see
/// [`ensure`]), resolving the directory first. What the worker thread
/// calls: one function from URL to a file to decode.
pub fn ensure_cached(fetch: &Fetch) -> Result<PathBuf, FetchError> {
    let dir = dir()?;
    ensure(&dir, fetch)
}

/// Downloads `fetch` into the cache in `dir` unless its file is already
/// there (and, with a pinned hash, still matches), and returns the file to
/// decode. Creates the directory. Runs on the worker thread: it spawns and
/// waits for `curl`, hashes and reads back megabytes, all off the loop.
pub fn ensure(dir: &Path, fetch: &Fetch) -> Result<PathBuf, FetchError> {
    if fetch.url.len() > MAX_URL {
        return Err(FetchError::TooLong(fetch.url.len()));
    }
    std::fs::create_dir_all(dir).map_err(|error| FetchError::Cache {
        path: dir.to_path_buf(),
        error: error.to_string(),
    })?;
    sweep(dir);
    let cached = cached_path(dir, &fetch.url);
    if let Some(path) = hit(&cached, fetch)? {
        return Ok(path);
    }
    let temp = temp_path(dir, &cached);
    let result = download(fetch, &temp).and_then(|()| check(&temp, fetch));
    match result {
        Ok(()) => {
            std::fs::rename(&temp, &cached).map_err(|error| FetchError::Cache {
                path: cached.clone(),
                error: error.to_string(),
            })?;
            Ok(cached)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// The cached file, when it can be shown as is: present, and matching the
/// pinned hash when there is one. A present file that fails the pin is
/// removed (corrupt or stale), so the caller downloads it again; one that
/// fails again is a [`FetchError::Mismatch`], not a loop. Unpinned hits
/// cost one `stat`, not a read: the bytes are decoded straight after.
fn hit(cached: &Path, fetch: &Fetch) -> Result<Option<PathBuf>, FetchError> {
    let meta = match std::fs::metadata(cached) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(FetchError::Cache {
                path: cached.to_path_buf(),
                error: error.to_string(),
            });
        }
    };
    if !meta.is_file() {
        // Something else under our name: try to replace it with a download
        // (the rename says why if it cannot).
        return Ok(None);
    }
    match fetch.sha256 {
        None => Ok(Some(cached.to_path_buf())),
        Some(expected) => {
            let bytes = std::fs::read(cached).map_err(|error| FetchError::Cache {
                path: cached.to_path_buf(),
                error: error.to_string(),
            })?;
            if crate::sha256::digest(&bytes) == expected {
                Ok(Some(cached.to_path_buf()))
            } else {
                let _ = std::fs::remove_file(cached);
                Ok(None)
            }
        }
    }
}

/// This download's temporary file: beside the cache file, with this
/// process in the name, so two instances never share one.
fn temp_path(dir: &Path, cached: &Path) -> PathBuf {
    let stem = cached
        .file_name()
        .map_or_else(|| OsString::from("wallpaper"), OsString::from);
    let mut name = stem;
    name.push(format!(".part-{}", std::process::id()));
    dir.join(name)
}

/// Removes this directory's stale temporary files (see [`STALE_AFTER_SECS`]).
/// Best effort: any failure leaves them for the next fetch.
fn sweep(dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let stale = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(STALE_AFTER_SECS))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.contains(".part-") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| modified < stale);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Runs `curl` for `fetch` into `temp`. Nothing is read back here: the
/// exit status and the checks in [`check`] decide.
fn download(fetch: &Fetch, temp: &Path) -> Result<(), FetchError> {
    let output = Command::new(CURL)
        .arg("--fail")
        .arg("--silent")
        .arg("--location")
        .arg("--max-redirs")
        .arg(MAX_REDIRECTS.to_string())
        .arg("--proto")
        .arg("http,https")
        .arg("--proto-redir")
        .arg("http,https")
        .arg("--connect-timeout")
        .arg(CONNECT_TIMEOUT_SECS.to_string())
        .arg("--max-time")
        .arg(MAX_TIME_SECS.to_string())
        .arg("--max-filesize")
        .arg(MAX_BYTES.to_string())
        .arg("--output")
        .arg(temp)
        .arg("--")
        .arg(&fetch.url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| FetchError::Spawn(error.to_string()))?;
    if output.status.success() {
        return Ok(());
    }
    // `--fail` writes nothing on an HTTP error, but a killed or capped
    // transfer may leave bytes: `ensure` removes them (nothing is ever
    // cached from a failed download).
    let detail = tail(&output.stderr);
    // Exit 63 is the cap (`--max-filesize`): name the limit, not curl's
    // number.
    if output.status.code() == Some(63) {
        return Err(FetchError::TooLarge);
    }
    Err(exit(&fetch.url, output.status.code(), &detail))
}

/// `curl`'s failure in one line: the exit code named where it helps, then
/// the bounded tail of its stderr.
fn exit(url: &str, code: Option<i32>, detail: &str) -> FetchError {
    let hint = match code {
        Some(6) => "nothing resolves that host",
        Some(7) => "nothing answers there",
        Some(28) => "nothing answered in time",
        Some(60) => "the server's TLS certificate does not verify",
        _ => "",
    };
    let detail = detail.trim();
    let detail = match (detail.is_empty(), hint.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!("({hint})"),
        (false, true) => detail.to_owned(),
        (false, false) => format!("{detail} ({hint})"),
    };
    FetchError::Failed {
        url: url.to_owned(),
        code,
        detail,
    }
}

/// The last line of `stderr`, bounded: curl's errors are one line, and a
/// proxy's page must not land whole in the log.
fn tail(stderr: &[u8]) -> String {
    const MAX_DETAIL: usize = 500;
    let text = String::from_utf8_lossy(stderr);
    let line = text.lines().last().unwrap_or_default().trim();
    if line.len() <= MAX_DETAIL {
        line.to_owned()
    } else {
        // Byte 500 may sit inside a multibyte char; slicing there panics,
        // and the release profile aborts the daemon on panic. Back off to
        // the boundary (at most three bytes back: UTF-8's longest char).
        let mut end = MAX_DETAIL;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", line[..end].trim_end())
    }
}

/// Checks a fresh download: the pinned hash when there is one, then the
/// magic (see the module docs). Reads the file twice at most (once here,
/// once to decode); the cache is for what passed.
fn check(temp: &Path, fetch: &Fetch) -> Result<(), FetchError> {
    let bytes = std::fs::read(temp).map_err(|error| FetchError::Cache {
        path: temp.to_path_buf(),
        error: error.to_string(),
    })?;
    if let Some(expected) = fetch.sha256 {
        let actual = crate::sha256::digest(&bytes);
        if actual != expected {
            return Err(FetchError::Mismatch {
                expected: crate::sha256::hex_bytes(expected.as_slice()),
                actual: crate::sha256::hex_bytes(actual.as_slice()),
            });
        }
    }
    if !is_image(&bytes) {
        return Err(FetchError::NotAnImage {
            url: fetch.url.clone(),
            bytes: bytes.len() as u64,
        });
    }
    Ok(())
}

/// Whether `bytes` start as a PNG, JPEG or WebP: the signatures
/// `image::decode` reads, before it reads them.
fn is_image(bytes: &[u8]) -> bool {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    bytes.starts_with(PNG)
        || bytes.starts_with(b"\xff\xd8\xff")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()))
}
