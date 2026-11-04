//! The PulseAudio auth cookie: the user's `~/.config/pulse/cookie`, read
//! once per connect on the bar's event-loop thread.
//!
//! The file is the user's own, but a FIFO or device left at that path must
//! not freeze the bar, so it is opened `O_NONBLOCK` and its opened fd must
//! be a regular file. The cookie is exactly [`COOKIE_LEN`] bytes, so one
//! byte more is read than that, into a stack buffer: a longer file is
//! refused without being read whole, and nothing is allocated. A symlink
//! is followed (dotfile managers link this file), which is safe because
//! the check is on what was opened, not on the name.

use std::io::Read;
use std::path::{Path, PathBuf};

use rustix::fs::{FileType, Mode, OFlags, fstat, open};

/// The size of a PulseAudio cookie.
pub const COOKIE_LEN: usize = 256;

#[cfg(test)]
mod tests;

/// The cookie `AUTH` sends: the user's 256 bytes, or zeros when no cookie
/// file is readable (pipewire-pulse takes any cookie from the same user;
/// a real server refuses a wrong one with an error, and the module waits).
pub fn cookie() -> [u8; COOKIE_LEN] {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")));
    base.and_then(|base| read_cookie(&base.join("pulse").join("cookie")))
        .unwrap_or([0; COOKIE_LEN])
}

/// `path`'s bytes when it is a regular file of exactly [`COOKIE_LEN`]
/// bytes; `None` for anything else, a failure included.
fn read_cookie(path: &Path) -> Option<[u8; COOKIE_LEN]> {
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOCTTY,
        Mode::empty(),
    )
    .ok()?;
    if FileType::from_raw_mode(fstat(&fd).ok()?.st_mode) != FileType::RegularFile {
        return None;
    }
    // One byte past the cookie, to tell exactly 256 from more.
    let mut buf = [0u8; COOKIE_LEN + 1];
    let mut file = std::fs::File::from(fd);
    let mut len = 0;
    while len < buf.len() {
        match file.read(&mut buf[len..]) {
            Ok(0) => break,
            Ok(n) => len += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    (len == COOKIE_LEN).then(|| {
        let mut cookie = [0; COOKIE_LEN];
        cookie.copy_from_slice(&buf[..COOKIE_LEN]);
        cookie
    })
}
