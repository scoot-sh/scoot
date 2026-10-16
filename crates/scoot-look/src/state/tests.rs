//! The session-state record, link, lock, seed and resolution: order,
//! grammar, atomicity, repair and isolation.
//!
//! Scratch dirs are `TempDir` guards, so a run leaves no
//! `/tmp/scoot-look-state-*` dirs behind (a failed assert still drops the
//! guard and cleans the dir).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;

fn scratch(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("scoot-look-state-{tag}-"))
        .tempdir()
        .expect("a scratch dir")
}

fn session(value: &str) -> Option<&OsStr> {
    Some(OsStr::new(value))
}

fn record(name: &str, rendered: &str, generation: u64) -> Record {
    Record {
        name: name.to_owned(),
        rendered: rendered.to_owned(),
        generation,
        chosen_by: Some("apply".to_owned()),
        declared_default: Some("ginger-night".to_owned()),
        toolkit: None,
        dconf_wrote: BTreeMap::new(),
        dconf_orig: BTreeMap::new(),
    }
}

fn write_json(dir: &Path, body: &str) {
    std::fs::write(dir.join(RECORD_FILE), body).expect("a record");
}

// The resolution rule, shared by scoot and scootbar: explicit,
// then the session var, then the XDG state home, then the home fallback.
// Empty counts as unset. (scootbg takes no look state: it renders what the
// compositor hands it.)

#[test]
fn an_explicit_dir_wins_over_every_env_var() {
    assert_eq!(
        resolve_state_dir(
            Some(Path::new("/explicit")),
            session("/env"),
            session("/xdg"),
            session("/home"),
        ),
        Some(PathBuf::from("/explicit"))
    );
}

#[test]
fn the_env_wins_over_the_home_fallback_in_order() {
    assert_eq!(
        resolve_state_dir(None, session("/env"), session("/xdg"), session("/home")),
        Some(PathBuf::from("/env"))
    );
    assert_eq!(
        resolve_state_dir(None, None, session("/xdg"), session("/home")),
        Some(PathBuf::from("/xdg/scoot"))
    );
    assert_eq!(
        resolve_state_dir(None, None, None, session("/home")),
        Some(PathBuf::from("/home/.local/state/scoot"))
    );
    assert_eq!(resolve_state_dir(None, None, None, None), None);
}

#[test]
fn empty_env_vars_fall_through_like_unset_ones() {
    assert_eq!(
        resolve_state_dir(None, session(""), session(""), session("")),
        None,
        "empty vars must not resolve to the current directory"
    );
    assert_eq!(
        resolve_state_dir(None, session(""), session("/xdg"), session("/home")),
        Some(PathBuf::from("/xdg/scoot"))
    );
}

// The private session decision: headless is private by default, any backend
// opts in with --private-look, and --tty/--nested otherwise keep the user
// state so their swaps survive a restart.

#[test]
fn headless_is_private_nested_and_tty_are_not_unless_asked() {
    assert!(
        wants_private_session(false, false, false),
        "headless defaults to private"
    );
    assert!(
        !wants_private_session(true, false, false),
        "nested keeps the user state"
    );
    assert!(
        !wants_private_session(false, true, false),
        "tty keeps the user state"
    );
    for (nested, tty) in [(false, false), (true, false), (false, true)] {
        assert!(
            wants_private_session(nested, tty, true),
            "private-look opts every backend in (nested={nested}, tty={tty})"
        );
    }
}

#[test]
fn a_socket_name_makes_a_private_dir_name() {
    assert_eq!(
        private_dir_name(OsStr::new("wayland-1")).as_deref(),
        Ok("scoot-look-wayland-1")
    );
}

#[test]
fn a_display_name_outside_the_socket_grammar_is_refused_not_sanitized() {
    // A sanitized name could collide two displays onto one dir; refuse.
    for bad in ["", "../x", "wayland-1\n", "a=b", "way land", "way/land"] {
        assert!(
            private_dir_name(OsStr::new(bad)).is_err(),
            "{bad:?} must not name a state dir"
        );
    }
    let long = "w".repeat(65);
    assert!(
        private_dir_name(OsStr::new(&long)).is_err(),
        "a 65-byte display must not name a state dir"
    );
}

// The record: a missing file is "no choice yet", anything malformed names
// its reason, and nothing hangs or panics.

#[test]
fn a_missing_record_is_missing_not_corrupt() {
    let _scratch = scratch("missing");
    let dir = _scratch.path().to_path_buf();
    assert!(matches!(read_record(&dir), RecordRead::Missing));
}

#[test]
fn a_valid_record_reads_back() {
    let _scratch = scratch("roundtrip");
    let dir = _scratch.path().to_path_buf();
    let want = record("moonrise", "/cache/rendered/moonrise-9f2c", 7);
    write_record(&dir, &want).expect("atomic write");
    let RecordRead::Present(got) = read_record(&dir) else {
        panic!("a written record reads back");
    };
    assert_eq!(got, want);
}

#[test]
fn malformed_records_name_their_reason() {
    let _scratch = scratch("malformed");
    let dir = _scratch.path().to_path_buf();
    for (tag, body) in [
        ("not json", "{oops"),
        ("missing name", r#"{"rendered":"/r","generation":1}"#),
        ("missing rendered", r#"{"name":"moonrise","generation":1}"#),
        (
            "missing generation",
            r#"{"name":"moonrise","rendered":"/r"}"#,
        ),
        (
            "uppercase name",
            r#"{"name":"Moonrise","rendered":"/r","generation":1}"#,
        ),
        (
            "slash name",
            r#"{"name":"a/b","rendered":"/r","generation":1}"#,
        ),
        (
            "empty name",
            r#"{"name":"","rendered":"/r","generation":1}"#,
        ),
        (
            "long name",
            r#"{"name":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","rendered":"/r","generation":1}"#,
        ),
        (
            "empty rendered",
            r#"{"name":"moonrise","rendered":"","generation":1}"#,
        ),
        (
            "newline rendered",
            "{\"name\":\"moonrise\",\"rendered\":\"/r\\n/etc\",\"generation\":1}",
        ),
        (
            "string generation",
            r#"{"name":"moonrise","rendered":"/r","generation":"1"}"#,
        ),
        (
            "bad declared default",
            r#"{"name":"moonrise","rendered":"/r","generation":1,"declared-default":"Nope"}"#,
        ),
        ("a list", r#"[1,2]"#),
    ] {
        write_json(&dir, body);
        let RecordRead::Corrupt(reason) = read_record(&dir) else {
            panic!("{tag} must be corrupt, not readable");
        };
        assert!(
            reason.contains(RECORD_FILE),
            "{tag}: the reason names the file: {reason}"
        );
    }
}

#[test]
fn minimal_and_maximal_records_both_read() {
    let _scratch = scratch("shapes");
    let dir = _scratch.path().to_path_buf();
    write_json(
        &dir,
        r#"{"name":"moonrise","rendered":"/r","generation":0}"#,
    );
    assert!(
        matches!(read_record(&dir), RecordRead::Present(_)),
        "generation 0 with no optional keys reads"
    );
    write_json(
        &dir,
        r#"{"name":"moonrise","rendered":"/r","generation":1,"chosen-by":"sync","declared-default":null}"#,
    );
    assert!(
        matches!(read_record(&dir), RecordRead::Present(_)),
        "explicit nulls read"
    );
}

fn read_on_thread(dir: PathBuf) -> RecordRead {
    // A FIFO with no writer blocks a plain read forever: read on a thread
    // with a timeout, so the failure is a failed test, never a wedge.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done_tx.send(read_record(&dir));
    });
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("reading the record must not block: over 5 s")
}

#[test]
fn a_fifo_record_refuses_without_blocking() {
    let _scratch = scratch("fifo");
    let dir = _scratch.path().to_path_buf();
    make_fifo(&dir.join(RECORD_FILE));
    let RecordRead::Corrupt(reason) = read_on_thread(dir) else {
        panic!("a FIFO is not a record");
    };
    assert!(
        reason.contains("regular"),
        "the reason names the refusal: {reason}"
    );
}

#[test]
fn an_oversize_record_refuses_without_reading_it_whole() {
    let _scratch = scratch("oversize");
    let dir = _scratch.path().to_path_buf();
    let big = format!(
        r#"{{"name":"moonrise","rendered":"/r","generation":1,"chosen-by":"{}"}}"#,
        "x".repeat(70 * 1024)
    );
    assert!(big.len() as u64 > MAX_FILE, "the test file is past the cap");
    write_json(&dir, &big);
    let RecordRead::Corrupt(reason) = read_record(&dir) else {
        panic!("an oversize record is refused");
    };
    assert!(
        reason.contains("larger"),
        "the reason names the refusal: {reason}"
    );
}

#[test]
fn a_directory_record_is_corrupt() {
    let _scratch = scratch("dir-record");
    let dir = _scratch.path().to_path_buf();
    std::fs::create_dir(dir.join(RECORD_FILE)).expect("a directory at the record path");
    let RecordRead::Corrupt(_) = read_on_thread(dir) else {
        panic!("a directory is not a record");
    };
}

fn make_fifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo failed");
}

// Atomicity: a reader racing a swap sees the old or the new record, never a
// half-written one (every read parses, and names one of the two looks).

#[test]
fn a_reader_racing_a_swap_sees_only_whole_records() {
    let _scratch = scratch("race");
    let dir = _scratch.path().to_path_buf();
    let first = record("moonrise", "/cache/a", 1);
    let second = record("ginger-night", "/cache/b", 2);
    write_record(&dir, &first).expect("the first write");
    let path = dir.clone();
    let (writer_first, writer_second) = (first.clone(), second.clone());
    let writer = std::thread::spawn(move || {
        for _ in 0..200 {
            write_record(&path, &writer_first).expect("swap to first");
            write_record(&path, &writer_second).expect("swap to second");
        }
    });
    for _ in 0..200 {
        match read_record(&dir) {
            RecordRead::Present(got) => {
                assert!(got == first || got == second, "a torn record: {got:?}");
            }
            other => panic!("a concurrent read must see a whole record, not {other:?}"),
        }
    }
    writer.join().expect("the writer");
}

#[test]
fn write_atomic_refuses_a_name_that_escapes_the_dir() {
    let _scratch = scratch("escape");
    let dir = _scratch.path().to_path_buf();
    for bad in ["", "../x", "a/b", ".", ".."] {
        assert!(
            write_atomic(&dir, bad, b"{}").is_err(),
            "{bad:?} must not write"
        );
    }
    assert!(
        std::fs::read_dir(&dir).expect("readable").next().is_none(),
        "no refusal may leave a file behind"
    );
}

// The link repair: the link is derived from the record, so a crash between
// the two renames heals by repointing; without a valid record the link is
// left alone.

#[test]
fn a_matching_link_is_consistent() {
    let _scratch = scratch("consistent");
    let dir = _scratch.path().to_path_buf();
    write_record(&dir, &record("moonrise", "/cache/a", 3)).expect("a record");
    symlink("/cache/a", dir.join(LINK_NAME)).expect("a matching link");
    assert_eq!(ensure_link(&dir), Ok(LinkRepair::Consistent));
}

#[test]
fn a_disagreeing_link_heals_from_the_record() {
    let _scratch = scratch("heal");
    let dir = _scratch.path().to_path_buf();
    write_record(&dir, &record("moonrise", "/cache/new", 4)).expect("a record");
    symlink("/cache/old", dir.join(LINK_NAME)).expect("a stale link");
    let repaired = ensure_link(&dir).expect("repair works");
    assert!(
        matches!(repaired, LinkRepair::Healed(_)),
        "a stale link heals: {repaired:?}"
    );
    assert_eq!(
        std::fs::read_link(dir.join(LINK_NAME)).expect("a link"),
        PathBuf::from("/cache/new")
    );
    assert_eq!(ensure_link(&dir), Ok(LinkRepair::Consistent));
}

#[test]
fn a_missing_link_is_recreated_from_the_record() {
    let _scratch = scratch("recreate");
    let dir = _scratch.path().to_path_buf();
    write_record(&dir, &record("moonrise", "/cache/a", 1)).expect("a record");
    let repaired = ensure_link(&dir).expect("repair works");
    assert!(matches!(repaired, LinkRepair::Healed(_)), "{repaired:?}");
    assert_eq!(
        std::fs::read_link(dir.join(LINK_NAME)).expect("a link"),
        PathBuf::from("/cache/a")
    );
}

#[test]
fn without_a_valid_record_the_link_is_left_alone() {
    for tag in ["missing", "corrupt"] {
        let _scratch = scratch(tag);
        let dir = _scratch.path().to_path_buf();
        if tag == "corrupt" {
            write_json(&dir, "{oops");
        }
        symlink("/cache/live", dir.join(LINK_NAME)).expect("a live link");
        let repaired = ensure_link(&dir).expect("repair never fails startup");
        let LinkRepair::Skipped(note) = repaired else {
            panic!("{tag}: without a record there is nothing to repair from");
        };
        assert!(
            note.contains(RECORD_FILE),
            "{tag}: the note names the record: {note}"
        );
        assert_eq!(
            std::fs::read_link(dir.join(LINK_NAME)).expect("untouched"),
            PathBuf::from("/cache/live"),
            "{tag}: the link is untouched"
        );
    }
}

// The lock: two writers serialize, a waiter past its bound refuses, and a
// non-regular lock path refuses rather than locking something else.

#[test]
fn two_writers_serialize_and_a_waiter_past_its_bound_refuses() {
    let _scratch = scratch("lock");
    let dir = _scratch.path().to_path_buf();
    let _first = acquire_lock(&dir, Duration::from_secs(5)).expect("the first lock");
    assert!(
        acquire_lock(&dir, Duration::from_millis(100)).is_err(),
        "a held lock refuses a bounded waiter"
    );
    drop(_first);
    let _second = acquire_lock(&dir, Duration::from_secs(5)).expect("the lock after release");
}

#[test]
fn a_waiter_inside_its_bound_blocks_until_release() {
    let _scratch = scratch("lock-wait");
    let dir = _scratch.path().to_path_buf();
    let dir_clone = dir.clone();
    let holder = std::thread::spawn(move || {
        let _held = acquire_lock(&dir_clone, Duration::from_secs(5)).expect("the holder");
        std::thread::sleep(Duration::from_millis(300));
    });
    // Give the holder time to take the lock first.
    std::thread::sleep(Duration::from_millis(100));
    let _waiter = acquire_lock(&dir, Duration::from_secs(5)).expect("the waiter after release");
    holder.join().expect("the holder");
}

#[test]
fn a_symlink_or_fifo_lock_refuses() {
    let _scratch = scratch("lock-evil");
    let dir = _scratch.path().to_path_buf();
    let real = dir.join("real.lock");
    std::fs::write(&real, b"").expect("a target");
    symlink(&real, dir.join(LOCK_FILE)).expect("a symlink lock");
    assert!(
        acquire_lock(&dir, Duration::from_millis(50)).is_err(),
        "a symlink must not become the lock"
    );
    std::fs::remove_file(dir.join(LOCK_FILE)).expect("remove the symlink");
    make_fifo(&dir.join(LOCK_FILE));
    assert!(
        acquire_lock(&dir, Duration::from_millis(50)).is_err(),
        "a FIFO must not become the lock"
    );
}

#[test]
fn a_lock_needs_its_parent_dir() {
    assert!(
        acquire_lock(
            Path::new("/nonexistent-scoot-look-state-dir"),
            Duration::from_millis(50)
        )
        .is_err(),
        "a missing state dir cannot be locked"
    );
}

// The seed: the private dir starts as a copy of the user's choice, and the
// user dir is never written (not even to repair it).

fn user_state(tag: &str) -> (tempfile::TempDir, PathBuf) {
    let guard = scratch(tag);
    let user = guard.path().to_path_buf();
    let rendered = user.join("rendered-moonrise");
    std::fs::create_dir(&rendered).expect("a rendered dir");
    std::fs::write(rendered.join("scoot.toml"), "[appearance]\n").expect("a fragment");
    write_record(&user, &record("moonrise", "/cache/moonrise-9f2c", 7)).expect("a record");
    symlink("/cache/moonrise-9f2c", user.join(LINK_NAME)).expect("a link");
    // Snapshot the user dir: the seed must not touch it.
    (guard, rendered)
}

fn user_entries(user: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(user)
        .expect("readable")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    entries.sort();
    entries
}

#[test]
fn the_seed_copies_the_record_and_the_link_leaving_the_user_state_alone() {
    let (_user_guard, _rendered) = user_state("seed");
    let user = _user_guard.path().to_path_buf();
    let before = user_entries(&user);
    let user_record = std::fs::read(user.join(RECORD_FILE)).expect("the user record");
    // Beside the scratch, not inside the user dir, so the untouched-user
    // assertion below is exact.
    let _private_guard = scratch("seed-private");
    let private = _private_guard.path().to_path_buf();
    let notes = seed_private(&user, &private).expect("seeding works");
    assert!(notes.is_empty(), "a clean seed is quiet: {notes:?}");
    assert_eq!(
        std::fs::read(private.join(RECORD_FILE)).expect("the seeded record"),
        user_record,
        "the record copies byte for byte"
    );
    assert_eq!(
        std::fs::read_link(private.join(LINK_NAME)).expect("the seeded link"),
        PathBuf::from("/cache/moonrise-9f2c")
    );
    assert_eq!(user_entries(&user), before, "the user dir is untouched");
}

#[test]
fn an_absent_user_state_seeds_an_empty_private_dir_with_notes() {
    let _user_scratch = scratch("seed-absent");
    let user = _user_scratch.path().to_path_buf();
    let _private_scratch = scratch("seed-absent-private");
    let private = _private_scratch.path().to_path_buf();
    let notes = seed_private(&user, &private).expect("seeding works");
    assert_eq!(notes.len(), 2, "record and link both noted: {notes:?}");
    assert!(
        !private.join(RECORD_FILE).exists()
            && std::fs::symlink_metadata(private.join(LINK_NAME)).is_err(),
        "nothing is invented"
    );
}

#[test]
fn a_dangling_user_link_replicates_as_a_dangling_link() {
    let _user_scratch = scratch("seed-dangling");
    let user = _user_scratch.path().to_path_buf();
    write_record(&user, &record("moonrise", "/cache/gone", 1)).expect("a record");
    symlink("/cache/gone", user.join(LINK_NAME)).expect("a dangling link");
    let _private_scratch = scratch("seed-dangling-private");
    let private = _private_scratch.path().to_path_buf();
    seed_private(&user, &private).expect("seeding works");
    assert_eq!(
        std::fs::read_link(private.join(LINK_NAME)).expect("replicated"),
        PathBuf::from("/cache/gone"),
        "the private session sees the same dangling link, not a repair of the user's"
    );
}

#[test]
fn an_unreadable_user_record_seeds_no_record_but_still_seeds_the_link() {
    let _user_scratch = scratch("seed-fifo");
    let user = _user_scratch.path().to_path_buf();
    make_fifo(&user.join(RECORD_FILE));
    symlink("/cache/a", user.join(LINK_NAME)).expect("a link");
    let _private_scratch = scratch("seed-fifo-private");
    let private = _private_scratch.path().to_path_buf();
    let notes = seed_private(&user, &private).expect("seeding works");
    assert_eq!(notes.len(), 1, "the FIFO is noted: {notes:?}");
    assert!(
        !private.join(RECORD_FILE).exists(),
        "no record is seeded from a FIFO"
    );
    assert_eq!(
        std::fs::read_link(private.join(LINK_NAME)).expect("the link still seeds"),
        PathBuf::from("/cache/a")
    );
}

// Review findings on PR #616: each test below fails before its fix and
// passes after. They run on the dev VM (Linux) under both runners.

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path)
        .expect("statable")
        .permissions()
        .mode()
        & 0o777
}

// Finding 1 (blocking): a stale private dir leaks into a new session. A
// previous session under the same display name leaves a record and a link
// behind; with an empty user state the seed must leave neither, not note
// and keep them.
#[test]
fn a_stale_private_state_does_not_survive_seeding() {
    let _user_scratch = scratch("stale-user");
    let user = _user_scratch.path().to_path_buf();
    let _private_scratch = scratch("stale-private");
    let private = _private_scratch.path().to_path_buf();
    write_json(
        &private,
        r#"{"name":"stale","rendered":"/tmp/stale","generation":3}"#,
    );
    symlink("/tmp/stale", private.join(LINK_NAME)).expect("a stale link");
    let notes = seed_private(&user, &private).expect("seeding works");
    assert_eq!(notes.len(), 2, "record and link both noted: {notes:?}");
    assert!(
        !private.join(RECORD_FILE).exists(),
        "the stale record is gone: the private dir is exactly the seed"
    );
    assert!(
        std::fs::symlink_metadata(private.join(LINK_NAME)).is_err(),
        "the stale link is gone: the private dir is exactly the seed"
    );
}

// Finding 2: the private dir is owner-only, and re-seeding a dir a previous
// build left world-readable seals it. The pre-existing 0755 below pins the
// failure whatever the test runner's umask is.
#[test]
fn the_private_dir_is_owner_only_even_when_reused() {
    let (_user_guard, _rendered) = user_state("perm-user");
    let user = _user_guard.path().to_path_buf();
    let _private_scratch = scratch("perm-private");
    let private = _private_scratch.path().to_path_buf();
    write_json(
        &private,
        r#"{"name":"stale","rendered":"/tmp/stale","generation":3}"#,
    );
    symlink("/tmp/stale", private.join(LINK_NAME)).expect("a stale link");
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755))
        .expect("yesterday's umask-dependent mode");
    seed_private(&user, &private).expect("seeding works");
    assert_eq!(mode_of(&private), 0o700, "the private dir is owner-only");
    assert_eq!(
        mode_of(&private.join(RECORD_FILE)),
        0o600,
        "the seeded record is owner-only"
    );
    assert_eq!(
        std::fs::read_link(private.join(LINK_NAME)).expect("the seeded link"),
        PathBuf::from("/cache/moonrise-9f2c"),
        "the seed still lands on the user's choice"
    );
}

// Finding 4: `rendered` must be an absolute path with no `..` component
// (control characters stay refused).
#[test]
fn a_non_absolute_or_parent_rendered_path_is_corrupt() {
    let _scratch = scratch("rendered-path");
    let dir = _scratch.path().to_path_buf();
    for (tag, rendered) in [
        ("relative", "cache/moonrise"),
        ("dot-relative", "./moonrise"),
        ("parent", "/cache/../etc"),
        ("bare parent", ".."),
        ("root parent", "/.."),
    ] {
        write_json(
            &dir,
            &format!(r#"{{"name":"moonrise","rendered":"{rendered}","generation":1}}"#),
        );
        let RecordRead::Corrupt(reason) = read_record(&dir) else {
            panic!("{tag} `{rendered}` must be corrupt, not readable");
        };
        assert!(
            reason.contains(RECORD_FILE),
            "{tag}: the reason names the file: {reason}"
        );
    }
}

// Finding 5: `write_atomic` must not follow a planted symlink at its temp
// path. Before the fix the temp name is a fixed `.look.json.tmp` written
// with `std::fs::write`, so the victim below receives the record bytes.
#[test]
fn write_atomic_does_not_follow_a_planted_tmp_symlink() {
    let _scratch = scratch("tmp-evil");
    let dir = _scratch.path().to_path_buf();
    let victim = dir.join("victim");
    std::fs::write(&victim, b"untouched").expect("a victim");
    symlink(&victim, dir.join(".look.json.tmp")).expect("a planted tmp symlink");
    let want = record("moonrise", "/cache/a", 1);
    write_record(&dir, &want).expect("the write still lands");
    assert_eq!(
        std::fs::read(&victim).expect("readable"),
        b"untouched",
        "the planted symlink is never followed"
    );
    let RecordRead::Present(got) = read_record(&dir) else {
        panic!("the record lands despite the planted symlink");
    };
    assert_eq!(got, want);
}

// Finding 6: a relative user link copied literally dangles in the private
// dir. The seed resolves it against the user dir, so the private link is
// absolute and names the same directory.
#[test]
fn a_relative_user_link_seeds_as_an_absolute_target() {
    let _user_scratch = scratch("seed-rel");
    let user = _user_scratch.path().to_path_buf();
    let rendered = user.join("rendered-moonrise");
    std::fs::create_dir(&rendered).expect("a rendered dir");
    write_record(&user, &record("moonrise", "/cache/moonrise-9f2c", 1)).expect("a record");
    symlink("rendered-moonrise", user.join(LINK_NAME)).expect("a relative link");
    let _private_scratch = scratch("seed-rel-private");
    let private = _private_scratch.path().to_path_buf();
    seed_private(&user, &private).expect("seeding works");
    let seeded = std::fs::read_link(private.join(LINK_NAME)).expect("the seeded link");
    assert!(
        seeded.is_absolute(),
        "the seeded link is absolute, not copied literally: {seeded:?}"
    );
    assert_eq!(seeded, rendered, "it names the same rendered directory");
}
