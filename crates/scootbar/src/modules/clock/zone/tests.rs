use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::{LOCALTIME, Spec, ZONEINFO, load, read_capped, stamp};
use crate::modules::clock::tzif::MAX_FILE;

const NEW_YORK: &[u8] = include_bytes!("../fixtures/America_New_York.slim.tzif");
const LONDON: &[u8] = include_bytes!("../fixtures/Europe_London.slim.tzif");

/// A scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!(
            "scootbar-zone-{}-{nanos}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn resolve(tz: Option<&str>, tzdir: Option<&str>) -> Spec {
    Spec::resolve(tz.map(OsStr::new), tzdir.map(OsStr::new))
}

fn file(path: &str, fallback: Option<&str>) -> Spec {
    Spec::File {
        path: PathBuf::from(path),
        fallback: fallback.map(|f| f.as_bytes().to_vec()),
    }
}

#[test]
fn tz_is_read_as_glibc_reads_it() {
    assert_eq!(resolve(None, None), file(LOCALTIME, None));
    assert_eq!(resolve(None, Some("/x")), file(LOCALTIME, None));
    assert_eq!(resolve(Some(""), None), Spec::Utc);
    assert_eq!(resolve(Some(":"), None), Spec::Utc);
    assert_eq!(
        resolve(Some("Europe/London"), None),
        file(&format!("{ZONEINFO}/Europe/London"), Some("Europe/London"))
    );
    assert_eq!(
        resolve(Some(":Europe/London"), Some("/etc/zoneinfo")),
        file("/etc/zoneinfo/Europe/London", Some("Europe/London"))
    );
    // An empty TZDIR is as good as unset.
    assert_eq!(
        resolve(Some("UTC"), Some("")),
        file(&format!("{ZONEINFO}/UTC"), Some("UTC"))
    );
    assert_eq!(
        resolve(Some(":/etc/localtime"), Some("/x")),
        file("/etc/localtime", Some("/etc/localtime"))
    );
    assert_eq!(
        resolve(Some("EST5EDT,M3.2.0,M11.1.0"), Some("/z")),
        file("/z/EST5EDT,M3.2.0,M11.1.0", Some("EST5EDT,M3.2.0,M11.1.0"))
    );
}

#[test]
fn a_zone_file_is_read_and_a_posix_string_stands_in_for_a_missing_one() {
    let scratch = Scratch::new("load");
    let path = scratch.file("zone", NEW_YORK);
    let loaded = load(&file(path.to_str().unwrap(), None));
    assert!(loaded.problem.is_none());
    assert_eq!(loaded.tz.at(1_790_651_220).abbr.as_str(), "EDT");

    // No such file: the value as a POSIX string.
    let dir = scratch.0.to_str().unwrap();
    let loaded = load(&resolve(Some("IST-5:30"), Some(dir)));
    assert!(loaded.problem.is_none());
    assert_eq!(loaded.tz.at(0).offset, 5 * 3600 + 1800);

    // Neither: UTC, and a reason.
    let loaded = load(&resolve(Some("Nowhere/Special"), Some(dir)));
    assert_eq!(loaded.tz.at(0).offset, 0);
    let problem = loaded.problem.unwrap();
    assert!(
        problem.contains("Nowhere/Special") && problem.contains("UTC"),
        "{problem}"
    );

    // A file that is not a zone, when the value is no POSIX string.
    let junk = scratch.file("junk", &[b'x'; 64]);
    let loaded = load(&file(junk.to_str().unwrap(), None));
    assert_eq!(loaded.tz.at(0).offset, 0);
    assert!(loaded.problem.unwrap().contains("bad magic"));

    assert!(load(&Spec::Utc).problem.is_none());
}

#[test]
fn only_a_regular_file_of_a_bounded_size_is_read() {
    let scratch = Scratch::new("cap");
    // A directory.
    assert!(
        read_capped(&scratch.0)
            .unwrap_err()
            .contains("not a regular file")
    );
    // A device that never ends, and one that never answers: neither is
    // read, and neither blocks (`O_NONBLOCK`, checked before any read).
    assert!(read_capped(Path::new("/dev/zero")).is_err());
    assert!(read_capped(Path::new("/dev/null")).is_err());
    // A FIFO with no writer: opened without blocking, then refused.
    let fifo = scratch.0.join("fifo");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .unwrap();
    assert!(
        read_capped(&fifo)
            .unwrap_err()
            .contains("not a regular file")
    );
    // Over the cap.
    let big = scratch.file("big", &vec![0u8; MAX_FILE + 1]);
    assert!(read_capped(&big).unwrap_err().contains("larger than"));
    let exact = scratch.file("exact", &vec![0u8; MAX_FILE]);
    assert_eq!(read_capped(&exact).unwrap().len(), MAX_FILE);
    // Missing.
    assert!(read_capped(&scratch.0.join("missing")).is_err());
}

/// The stamp follows a symlink, and changes when it is re-pointed or the
/// file is rewritten, which is what makes the clock read the zone again.
#[test]
fn the_stamp_changes_with_the_zone() {
    let scratch = Scratch::new("stamp");
    let new_york = scratch.file("ny", NEW_YORK);
    let london = scratch.file("london", LONDON);
    let link = scratch.0.join("localtime");
    std::os::unix::fs::symlink(&new_york, &link).unwrap();
    let first = stamp(&link).unwrap();
    assert_eq!(stamp(&link), Some(first));
    // Re-pointed atomically, as `timedatectl` does.
    let next = scratch.0.join("localtime.new");
    std::os::unix::fs::symlink(&london, &next).unwrap();
    std::fs::rename(&next, &link).unwrap();
    let second = stamp(&link).unwrap();
    assert_ne!(first, second);
    // Gone.
    std::fs::remove_file(&link).unwrap();
    assert_eq!(stamp(&link), None);
}
