use std::fs;
use std::path::PathBuf;

use super::*;
use crate::outputs::Outputs;

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("scootbg-lockscreen-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Real, distinct output ids (they are only made by `Outputs::add`).
fn ids(count: usize) -> Vec<OutputId> {
    let mut outputs: Outputs<()> = Outputs::default();
    (0..count).map(|n| outputs.add(n as u32, |_| ())).collect()
}

fn index(dir: &Path) -> String {
    fs::read_to_string(dir.join("index")).unwrap_or_default()
}

const IMAGE: Source = Source::Image(1, (10, 10));

#[test]
fn a_picture_is_a_file_and_a_line_of_the_index() {
    let dir = scratch("put").join("pics");
    let [a, b] = ids(2)[..] else { unreachable!() };
    let mut lock = Lockscreen::open(&dir);
    lock.put(a, "DP-1", IMAGE, b"aaa");
    lock.put(
        b,
        "HDMI-A-1",
        Source::Color(Color { r: 1, g: 2, b: 3 }),
        b"bbb",
    );
    assert_eq!(index(&dir), "", "nothing is listed before the flush");
    lock.flush();
    assert_eq!(
        index(&dir),
        format!(
            "DP-1\t{0}/output-{1}.png\nHDMI-A-1\t{0}/output-{2}.png\n",
            dir.display(),
            a.number(),
            b.number()
        )
    );
    assert_eq!(
        fs::read(dir.join(format!("output-{}.png", a.number()))).unwrap(),
        b"aaa"
    );
    assert_eq!(
        fs::read(dir.join(format!("output-{}.png", b.number()))).unwrap(),
        b"bbb"
    );
    fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}

#[test]
fn a_change_for_one_output_leaves_the_other_alone() {
    let dir = scratch("one").join("pics");
    let [a, b] = ids(2)[..] else { unreachable!() };
    let mut lock = Lockscreen::open(&dir);
    lock.put(a, "A", IMAGE, b"a1");
    lock.put(b, "B", IMAGE, b"b1");
    lock.flush();
    let before = fs::metadata(dir.join(format!("output-{}.png", b.number()))).unwrap();
    lock.put(a, "A", Source::Image(2, (10, 10)), b"a2");
    lock.flush();
    assert_eq!(
        fs::read(dir.join(format!("output-{}.png", a.number()))).unwrap(),
        b"a2"
    );
    assert_eq!(
        fs::read(dir.join(format!("output-{}.png", b.number()))).unwrap(),
        b"b1"
    );
    let after = fs::metadata(dir.join(format!("output-{}.png", b.number()))).unwrap();
    assert_eq!(
        before.modified().unwrap(),
        after.modified().unwrap(),
        "B was not rewritten"
    );
    assert!(lock.has(a, "A", Source::Image(2, (10, 10))));
    assert!(!lock.has(a, "A", IMAGE), "the old source is gone");
    assert!(lock.has(b, "B", IMAGE));
    fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}

#[test]
fn an_unplugged_output_loses_its_file_and_its_line() {
    let dir = scratch("unplug").join("pics");
    let [a, b] = ids(2)[..] else { unreachable!() };
    let mut lock = Lockscreen::open(&dir);
    lock.put(a, "A", IMAGE, b"a");
    lock.put(b, "B", IMAGE, b"b");
    lock.flush();
    lock.retain(|id| id == b);
    lock.flush();
    assert!(!dir.join(format!("output-{}.png", a.number())).exists());
    assert!(dir.join(format!("output-{}.png", b.number())).exists());
    assert!(!index(&dir).contains("A\t"));
    assert!(index(&dir).starts_with("B\t"));
    assert!(!lock.knows(a));
    lock.retain(|_| false);
    lock.flush();
    assert_eq!(index(&dir), "", "an empty index, not a stale one");
    fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}

#[test]
fn a_name_a_locker_could_not_take_whole_is_left_out_and_never_a_path() {
    let dir = scratch("names").join("pics");
    let ids = ids(5);
    let mut lock = Lockscreen::open(&dir);
    for (id, name) in ids
        .iter()
        .zip(["a:b", "tab\tname", "new\nline", "", "../../escape"])
    {
        lock.put(*id, name, IMAGE, b"x");
    }
    lock.flush();
    let listed = index(&dir);
    // Only the last is listable (it has nothing a locker splits on); its
    // file is still named by the id, never by the text.
    assert_eq!(
        listed,
        format!(
            "../../escape\t{}/output-{}.png\n",
            dir.display(),
            ids[4].number()
        )
    );
    for id in &ids[..4] {
        assert!(!dir.join(format!("output-{}.png", id.number())).exists());
        assert!(lock.knows(*id), "remembered, not retried");
    }
    let mut files: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(
        files,
        ["index", format!("output-{}.png", ids[4].number()).as_str()]
    );
    assert!(!dir.parent().unwrap().join("escape").exists());
    fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}

#[test]
fn opening_wipes_what_a_dead_daemon_left_and_closing_removes_the_directory() {
    let dir = scratch("wipe").join("pics");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("output-9.png"), b"stale").unwrap();
    fs::write(dir.join("index"), "X\tstale\n").unwrap();
    let mut lock = Lockscreen::open(&dir);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    assert!(lock.enabled());
    lock.close();
    assert!(!dir.exists());
    assert!(!lock.enabled(), "nothing is written after a close");
    let [a] = ids(1)[..] else { unreachable!() };
    lock.put(a, "A", IMAGE, b"a");
    lock.flush();
    assert!(!dir.exists());
    fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}

#[test]
fn the_directory_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("mode").join("pics");
    let [a] = ids(1)[..] else { unreachable!() };
    let mut lock = Lockscreen::open(&dir);
    lock.put(a, "A", IMAGE, b"a");
    lock.flush();
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let file = dir.join(format!("output-{}.png", a.number()));
    assert_eq!(
        fs::metadata(file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}

#[test]
fn a_directory_that_cannot_be_made_disables_it_without_a_panic() {
    let parent = scratch("blocked");
    let blocker = parent.join("file");
    fs::write(&blocker, b"").unwrap();
    // A path under a regular file cannot be made (root included).
    let mut lock = Lockscreen::open(&blocker.join("pics"));
    assert!(!lock.enabled());
    let [a] = ids(1)[..] else { unreachable!() };
    lock.put(a, "A", IMAGE, b"a");
    lock.flush();
    lock.remove(a);
    lock.close();
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn a_write_that_fails_unlists_the_output_and_is_not_retried() {
    let dir = scratch("fail").join("pics");
    let [a] = ids(1)[..] else { unreachable!() };
    let mut lock = Lockscreen::open(&dir);
    // The directory vanishes under a running daemon (someone cleaned the
    // runtime dir).
    fs::remove_dir_all(&dir).unwrap();
    lock.put(a, "A", IMAGE, b"a");
    assert!(lock.has(a, "A", IMAGE), "the failure is remembered");
    lock.flush();
    assert!(!dir.exists());
    fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}

#[test]
fn a_withheld_picture_removes_the_old_one_and_is_not_asked_for_again() {
    let dir = scratch("withhold").join("pics");
    let [a] = ids(1)[..] else { unreachable!() };
    let mut lock = Lockscreen::open(&dir);
    lock.put(a, "A", IMAGE, b"old");
    lock.flush();
    let next = Source::Image(2, (10, 10));
    lock.withhold(a, "A", next);
    lock.flush();
    assert!(!dir.join(format!("output-{}.png", a.number())).exists());
    assert_eq!(index(&dir), "", "no line for it");
    assert!(lock.has(a, "A", next), "remembered, so not made again");
    fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}
