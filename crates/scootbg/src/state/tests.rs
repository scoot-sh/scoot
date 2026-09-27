use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::format::{self, Pick};
use super::{DEFAULT_PROFILE, MAX_PROFILE, Profile, ProfileError, Saved, dir, load};
use crate::color::Color;
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::wallpaper::{Image, Wallpaper};

/// A scratch directory for one test, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sbg-state-{}-{tag}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn profile_names_are_plain_file_names() {
    for good in [
        "default",
        "scoot",
        "scoot-nested",
        "a",
        "A.b_c-9",
        "a.b",
        "x.",
    ] {
        assert_eq!(Profile::parse(good).map(|p| p.0), Ok(good.to_owned()));
    }
    let longest = "p".repeat(MAX_PROFILE);
    assert!(Profile::parse(&longest).is_ok());
    assert_eq!(
        Profile::parse(&format!("{longest}p")),
        Err(ProfileError::TooLong)
    );
    assert_eq!(Profile::parse(""), Err(ProfileError::Empty));
    for (bad, ch) in [
        ("a/b", '/'),
        ("/abs", '/'),
        ("../x", '/'),
        ("a b", ' '),
        ("a\0b", '\0'),
        ("a\nb", '\n'),
        ("é", 'é'),
        ("a\\b", '\\'),
        ("~", '~'),
    ] {
        assert_eq!(Profile::parse(bad), Err(ProfileError::Byte(ch)), "{bad:?}");
    }
    for bad in [".", "..", ".hidden", "a..b", "..."] {
        assert_eq!(Profile::parse(bad), Err(ProfileError::Dots), "{bad:?}");
    }
    assert_eq!(Profile::default().as_str(), DEFAULT_PROFILE);
    // Every refusal says what a name may be.
    for error in [
        ProfileError::Empty,
        ProfileError::TooLong,
        ProfileError::Byte('/'),
        ProfileError::Dots,
    ] {
        assert!(error.to_string().contains("1 to 64"), "{error}");
    }
}

#[test]
fn the_directory_follows_the_xdg_spec() {
    let os = |s: &'static str| Some(OsStr::new(s));
    assert_eq!(
        dir(os("/state"), os("/home/me")),
        Some(PathBuf::from("/state/scootbg"))
    );
    // Unset, empty or relative: the spec says to ignore it.
    for state_home in [None, os(""), os("relative/state")] {
        assert_eq!(
            dir(state_home, os("/home/me")),
            Some(PathBuf::from("/home/me/.local/state/scootbg")),
            "{state_home:?}"
        );
    }
    assert_eq!(dir(None, None), None);
    assert_eq!(dir(os(""), os("")), None);
    assert_eq!(dir(None, os("home")), None, "a relative HOME is no home");
}

#[test]
fn loading_tells_missing_from_unreadable() {
    let scratch = Scratch::new("load");
    let file = scratch.0.join("default");
    assert!(load(&file).unwrap().is_none(), "no file is no state");
    std::fs::create_dir(&file).unwrap();
    assert!(load(&file).is_err(), "a directory is not a state file");
    std::fs::remove_dir(&file).unwrap();
    std::fs::write(&file, "scootbg-state 1\nall color #010203\n").unwrap();
    let parsed = load(&file).unwrap().unwrap();
    assert_eq!(
        parsed.record.all,
        Some(Pick::Color(Color { r: 1, g: 2, b: 3 }))
    );
    // Over the limit: read only as far as needed to know.
    std::fs::write(&file, vec![b'\n'; format::MAX_BYTES + 4096]).unwrap();
    let parsed = load(&file).unwrap().unwrap();
    assert_eq!(parsed.warnings.len(), 1, "{:?}", parsed.warnings);
}

fn image(path: &str, serial: u64) -> Option<Wallpaper> {
    Some(Wallpaper::Image(Arc::new(Image {
        path: path.to_owned(),
        look: Look {
            mode: Mode::Fill,
            fill: Color { r: 0, g: 0, b: 0 },
            filter: Filter::Lanczos3,
        },
        serial,
    })))
}

fn color(v: u8) -> Option<Wallpaper> {
    Some(Wallpaper::Color(Color { r: v, g: v, b: v }))
}

fn lines(saved: &mut Saved) -> Vec<String> {
    saved.text().lines().skip(2).map(str::to_owned).collect()
}

/// What the file holds follows `set`'s rules: a named choice is kept by
/// name, connected or not, until a choice for every output replaces it.
#[test]
fn saved_choices_follow_sets_rules() {
    let mut saved = Saved::nowhere();
    // As a restore puts them: an output not plugged in now among them.
    saved.choices.set(None, color(1), 1);
    saved.choices.set(Some("GONE-1"), color(2), 2);
    saved.record(Some("DP-1"), &image("/a b.jpg", 3), 3);
    assert_eq!(
        lines(&mut saved),
        [
            "all color #010101",
            "output GONE-1 color #020202",
            "output DP-1 image /a%20b.jpg fill #000000 lanczos3",
        ]
    );
    saved.record(Some("DP-1"), &None, 4);
    assert_eq!(lines(&mut saved)[2], "output DP-1 clear");
    // An image landing late, older than a choice made since, changes
    // nothing: newest wins, as on screen.
    saved.record(Some("DP-1"), &color(9), 3);
    assert_eq!(lines(&mut saved)[2], "output DP-1 clear");
    // Every output: every named entry goes, as `set` without --output says.
    saved.record(None, &color(5), 5);
    assert_eq!(lines(&mut saved), ["all color #050505"]);
}

#[test]
fn the_fingerprint_and_profile_are_kept_as_read() {
    let mut saved = Saved::new(
        Profile::parse("scoot").unwrap(),
        None,
        Some("abc123".to_owned()),
    );
    assert_eq!(
        saved.text(),
        "scootbg-state 1\nprofile scoot\nfingerprint abc123\n"
    );
}

/// A record writes the file (in the background), and `flush` waits for it.
#[test]
fn recording_writes_the_file() {
    let scratch = Scratch::new("record");
    let file = scratch.0.join("nested/dir/scoot");
    let mut saved = Saved::new(Profile::parse("scoot").unwrap(), Some(file.clone()), None);
    saved.record(None, &color(7), 1);
    assert!(saved.flush(Duration::from_secs(10)));
    let parsed = load(&file).unwrap().unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    assert_eq!(
        parsed.record.all,
        Some(Pick::Color(Color { r: 7, g: 7, b: 7 }))
    );
    assert_eq!(parsed.record.profile.as_deref(), Some("scoot"));
    no_temp_files(file.parent().unwrap());
}

fn no_temp_files(dir: &Path) {
    let names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        names.iter().all(|n| !n.to_string_lossy().ends_with(".tmp")),
        "{names:?}"
    );
}

/// Past the limit, the least recently *set* entry goes, not the least
/// recently added: setting an old name again makes it newest. The table
/// forgets what the file drops, and a restore keeps the order.
#[test]
fn the_least_recently_set_entries_are_dropped_first() {
    use super::format::MAX_OUTPUTS;
    let mut saved = Saved::nowhere();
    let mut generation = 0;
    let mut set = |saved: &mut Saved, name: &str, v: u8| {
        generation += 1;
        saved.record(Some(name), &color(v), generation);
        let _ = saved.text();
    };
    for i in 0..MAX_OUTPUTS {
        set(&mut saved, &format!("O-{i}"), 1);
    }
    // O-0 is the oldest name, but set again now.
    set(&mut saved, "O-0", 2);
    set(&mut saved, "NEW-1", 3);
    set(&mut saved, "NEW-2", 4);
    let names: Vec<String> = saved.choices.named().map(|(n, ..)| n.to_owned()).collect();
    assert_eq!(
        names.len(),
        MAX_OUTPUTS,
        "the table forgot what the file dropped"
    );
    assert!(!names.contains(&"O-1".to_owned()) && !names.contains(&"O-2".to_owned()));
    assert!(names.contains(&"O-0".to_owned()), "re-set: recent");
    // The file lists them oldest first, and reads back whole.
    let text = saved.text();
    let parsed = super::format::decode(text.as_bytes());
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let order: Vec<&str> = parsed
        .record
        .named
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(order.first(), Some(&"O-3"));
    assert_eq!(&order[MAX_OUTPUTS - 3..], ["O-0", "NEW-1", "NEW-2"]);
}

/// Only `apply-config` sets the fingerprint: it is written with the
/// section's choices in one save, and every later `set` keeps it.
#[test]
fn an_applied_section_records_its_fingerprint() {
    let scratch = Scratch::new("applied");
    let file = scratch.0.join("scoot");
    let mut saved = Saved::new(
        Profile::parse("scoot").unwrap(),
        Some(file.clone()),
        Some("old".to_owned()),
    );
    assert_eq!(saved.fingerprint(), Some("old"));
    assert_eq!(saved.profile().as_str(), "scoot");
    saved.choices.set(None, color(1), 1);
    saved.choices.set(Some("DP-2"), color(2), 1);
    saved.applied_config("f00d".to_owned());
    assert_eq!(saved.fingerprint(), Some("f00d"));
    assert!(saved.flush(Duration::from_secs(10)));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "scootbg-state 1\nprofile scoot\nfingerprint f00d\nall color #010101\n\
         output DP-2 color #020202\n"
    );
    // A `set` after it keeps the fingerprint.
    saved.record(None, &color(3), 2);
    assert!(saved.flush(Duration::from_secs(10)));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "scootbg-state 1\nprofile scoot\nfingerprint f00d\nall color #030303\n"
    );
    // With nowhere to save, it is still remembered for the daemon's life.
    let mut nowhere = Saved::nowhere();
    nowhere.applied_config("beef".to_owned());
    assert_eq!(nowhere.fingerprint(), Some("beef"));
}

/// A profile adopted at a generation records nothing older: an image
/// `set` sent before the adoption, landing after it, is not this
/// profile's, even where nothing newer covers its output.
#[test]
fn nothing_older_than_an_adoption_is_recorded() {
    let mut saved = Saved::nowhere();
    saved.adopted_at(10);
    saved.record(None, &color(1), 9);
    saved.record(Some("DP-1"), &color(1), 9);
    assert!(saved.choices.every().is_none());
    assert_eq!(saved.choices.named_len(), 0);
    saved.record(Some("DP-1"), &color(2), 10);
    saved.record(None, &color(3), 11);
    assert_eq!(saved.choices.every(), Some(&color(3)));
}
