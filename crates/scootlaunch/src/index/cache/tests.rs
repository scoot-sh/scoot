//! Tests for the cache: round-trips, key mismatches, corruption,
//! and the size cap. Filesystem cases run in a `TempDir` per test.

use super::*;
use crate::index::Env;

fn test_env(root: &Path) -> Env {
    Env {
        xdg_data_home: None,
        xdg_data_dirs: Vec::new(),
        xdg_current_desktop: Vec::new(),
        xdg_cache_home: Some(root.join("cache")),
        xdg_state_home: None,
        locale_raw: String::new(),
        locale: None,
        path_dirs: Vec::new(),
    }
}

fn test_entry(id: &str) -> super::super::Entry {
    super::super::Entry {
        id: id.to_string(),
        name: "Name".to_string(),
        generic_name: Some("Generic".to_string()),
        comment: None,
        keywords: vec!["one".to_string(), "two".to_string()],
        icon: Some("icon".to_string()),
        exec: vec!["foo".to_string(), "bar baz".to_string()],
        terminal: false,
        dbus_activatable: false,
        actions: vec![super::super::Action {
            id: "act".to_string(),
            name: "Act".to_string(),
            exec: vec!["foo".to_string(), "--act".to_string()],
        }],
        path: PathBuf::from("/x/a.desktop"),
        hidden: None,
    }
}

fn test_key() -> CacheKey {
    CacheKey::new(Vec::new(), "de_DE.UTF-8", &["scoot".to_string()])
}

#[test]
fn round_trip() {
    let root = tempfile::tempdir().unwrap();
    let env = test_env(root.path());
    let key = test_key();
    let mut hidden = test_entry("b.desktop");
    hidden.hidden = Some(crate::index::filter::HiddenReason::TryExec);
    hidden.terminal = true;
    hidden.dbus_activatable = true;
    let entries = vec![test_entry("a.desktop"), hidden];
    assert!(store(&env, &key, &entries));
    let loaded = load(&env, &key).unwrap();
    assert_eq!(loaded, entries);
}

#[test]
fn empty_index_round_trips() {
    let root = tempfile::tempdir().unwrap();
    let env = test_env(root.path());
    let key = test_key();
    assert!(store(&env, &key, &[]));
    assert_eq!(load(&env, &key).unwrap(), Vec::new());
}

#[test]
fn key_mismatch_rescans() {
    let root = tempfile::tempdir().unwrap();
    let env = test_env(root.path());
    let key = test_key();
    assert!(store(&env, &key, &[test_entry("a.desktop")]));
    let mut other = key.clone();
    other.locale = "fr_FR.UTF-8".to_string();
    assert_eq!(load(&env, &other), None);
    let mut other = key.clone();
    other.current_desktop = "GNOME".to_string();
    assert_eq!(load(&env, &other), None);
    // The stored cache still loads under its own key.
    assert!(load(&env, &key).is_some());
}

#[test]
fn corruption_is_none_never_an_error() {
    let root = tempfile::tempdir().unwrap();
    let env = test_env(root.path());
    let key = test_key();
    assert!(store(&env, &key, &[test_entry("a.desktop")]));
    let path = cache_path(&env).unwrap();
    for broken in [
        Vec::new(),
        b"short".to_vec(),
        b"SLIX0001".to_vec(),
        {
            let mut bytes = std::fs::read(&path).unwrap();
            bytes.truncate(bytes.len() / 2);
            bytes
        },
        {
            let mut bytes = std::fs::read(&path).unwrap();
            bytes.extend_from_slice(b"trailing junk");
            bytes
        },
        {
            let mut bytes = std::fs::read(&path).unwrap();
            bytes[0] = b'Y';
            bytes
        },
        {
            let mut bytes = std::fs::read(&path).unwrap();
            bytes[8] = 0xFF;
            bytes
        },
    ] {
        std::fs::write(&path, &broken).unwrap();
        assert_eq!(load(&env, &key), None, "for {broken:?}");
    }
}

#[test]
fn version_bump_rescans() {
    // A cache whose version field is not ours loads as None, the same
    // path a future format change takes.
    let root = tempfile::tempdir().unwrap();
    let env = test_env(root.path());
    let key = test_key();
    assert!(store(&env, &key, &[test_entry("a.desktop")]));
    let path = cache_path(&env).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[8] = VERSION as u8 + 1;
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(load(&env, &key), None);
}

#[test]
fn oversize_index_is_not_written() {
    let root = tempfile::tempdir().unwrap();
    let env = test_env(root.path());
    let key = test_key();
    let big = "x".repeat(4096);
    let mut entries = Vec::new();
    for index in 0..300 {
        let mut entry = test_entry(&format!("{index}.desktop"));
        entry.keywords = vec![big.clone(); 8];
        entries.push(entry);
    }
    assert!(!store(&env, &key, &entries));
    assert_eq!(load(&env, &key), None);
}

#[test]
fn unwritable_cache_dir_still_builds() {
    // No cache home at all: no cache, never an error.
    let env = Env {
        xdg_cache_home: None,
        ..test_env(Path::new("/nonexistent"))
    };
    assert_eq!(cache_path(&env), None);
    assert!(!store(&env, &test_key(), &[]));
    assert_eq!(load(&env, &test_key()), None);
}

#[test]
fn key_sorts_by_path() {
    let key = CacheKey::new(
        vec![
            super::super::dirs::KeyEntry::File {
                path: PathBuf::from("/b"),
                dev: 1,
                ino: 2,
                mtime: SystemTime::UNIX_EPOCH,
            },
            super::super::dirs::KeyEntry::Dir {
                path: PathBuf::from("/a"),
                dev: 1,
                ino: 1,
                mtime: SystemTime::UNIX_EPOCH,
            },
        ],
        "",
        &[],
    );
    assert_eq!(key_path(&key.entries[0]).as_os_str(), "/a");
}
