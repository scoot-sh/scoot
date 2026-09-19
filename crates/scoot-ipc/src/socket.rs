//! Where the control socket lives.

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

/// Overrides the socket path. Shells set it for the processes they spawn.
pub const SOCKET_ENV: &str = "SCOOT_SOCKET";

const SOCKET_NAME: &str = "scoot.sock";

pub fn socket_path() -> Option<PathBuf> {
    resolve(env::var_os(SOCKET_ENV), env::var_os("XDG_RUNTIME_DIR"))
}

fn resolve(explicit: Option<OsString>, runtime_dir: Option<OsString>) -> Option<PathBuf> {
    let non_empty = |value: &OsString| !value.is_empty();
    explicit.filter(non_empty).map(PathBuf::from).or_else(|| {
        runtime_dir
            .filter(non_empty)
            .map(|dir| PathBuf::from(dir).join(SOCKET_NAME))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_path_wins() {
        let path = resolve(Some("/tmp/a.sock".into()), Some("/run/user/1000".into()));
        assert_eq!(path, Some(PathBuf::from("/tmp/a.sock")));
    }

    #[test]
    fn falls_back_to_the_runtime_dir() {
        let path = resolve(None, Some("/run/user/1000".into()));
        assert_eq!(path, Some(PathBuf::from("/run/user/1000/scoot.sock")));
    }

    #[test]
    fn empty_values_count_as_unset() {
        assert_eq!(resolve(Some("".into()), Some("".into())), None);
    }
}
