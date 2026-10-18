use std::path::{Path, PathBuf};

use ab_glyph::Font as _;

use super::{Error, FileError, Held, MAX_FONT, WELL_KNOWN, find, load};
use crate::testfont;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!(
            "scootbar-font-{}-{nanos}-{tag}",
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

#[test]
fn a_font_anyone_can_write_is_read_into_the_heap() {
    let scratch = Scratch::new("read");
    let path = scratch.file("seven.ttf", &testfont::build());
    let font = load(&path).unwrap();
    assert_eq!(font.path, path);
    // Written just now, so writable: the bytes are the bar's own copy, and
    // a later `cp` over the file cannot take the bar down, whatever mount
    // the temp dir is on.
    assert_eq!(font.held, Held::Read);
    assert_ne!(font.face.glyph_id('3').0, 0);
    // Truncated in place under the loaded font: nothing happens to it.
    std::fs::write(&path, b"").unwrap();
    let id = font.face.glyph_id('8');
    assert!(font.face.outline(id).is_some());
}

#[test]
fn what_is_not_a_usable_font_says_why() {
    let scratch = Scratch::new("bad");
    let missing = scratch.0.join("missing.ttf");
    assert!(matches!(load(&missing), Err(FileError::Io(_))));
    assert!(matches!(load(&scratch.0), Err(FileError::NotRegular)));
    let empty = scratch.file("empty.ttf", b"");
    assert!(matches!(load(&empty), Err(FileError::Empty)));
    let junk = scratch.file("junk.ttf", b"this is not a font");
    assert!(matches!(load(&junk), Err(FileError::NotAFont)));
    // A device is refused before any read (it would never end).
    assert!(matches!(
        load(Path::new("/dev/zero")),
        Err(FileError::NotRegular)
    ));
    // A FIFO with no writer: opened without blocking, then refused.
    let fifo = scratch.0.join("fifo");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .unwrap();
    assert!(matches!(load(&fifo), Err(FileError::NotRegular)));
    // Past the cap, found by its size alone (a sparse file costs nothing).
    let big = scratch.0.join("big.ttf");
    let file = std::fs::File::create(&big).unwrap();
    file.set_len(MAX_FONT + 1).unwrap();
    assert!(matches!(load(&big), Err(FileError::TooLarge(n)) if n == MAX_FONT + 1));
}

#[test]
fn a_given_font_that_fails_is_an_error_naming_it() {
    let scratch = Scratch::new("given");
    let junk = scratch.file("junk.ttf", b"nope");
    let error = find(Some(&junk)).unwrap_err();
    assert!(matches!(error, Error::Given { .. }));
    let message = error.to_string();
    assert!(
        message.contains("junk.ttf") && message.contains("not a TrueType"),
        "{message}"
    );
    let good = scratch.file("seven.ttf", &testfont::build());
    assert_eq!(find(Some(&good)).unwrap().path, good);
}

#[test]
fn with_none_found_the_refusal_says_how_to_give_one() {
    let message = Error::NoneFound { tried: Vec::new() }.to_string();
    assert!(message.contains("--font PATH"), "{message}");
    let message = Error::NoneFound {
        tried: vec![(PathBuf::from("/x/DejaVuSans.ttf"), FileError::NotAFont)],
    }
    .to_string();
    assert!(
        message.contains("/x/DejaVuSans.ttf: not a TrueType"),
        "{message}"
    );
}

/// Without `--font`, the first well-known file that loads, if any is on
/// this machine; the list itself is absolute paths only.
#[test]
fn the_well_known_list_is_absolute_and_tried_in_order() {
    assert!(WELL_KNOWN.iter().all(|p| p.starts_with('/')));
    match find(None) {
        Ok(font) => {
            let first = WELL_KNOWN
                .iter()
                .find(|p| load(Path::new(p)).is_ok())
                .unwrap();
            assert_eq!(font.path, Path::new(first));
        }
        Err(error) => {
            assert!(WELL_KNOWN.iter().all(|p| load(Path::new(p)).is_err()));
            assert!(error.to_string().contains("--font"));
        }
    }
}

#[test]
fn a_fallback_that_fails_is_an_error_naming_it() {
    let scratch = Scratch::new("fallback");
    let seven = scratch.file("seven.ttf", &testfont::build());
    let symbols = scratch.file("symbols.ttf", &testfont::build_symbols(['x']));
    let junk = scratch.file("junk.ttf", b"nope");
    // Both load: the chain is the primary and its fallbacks.
    assert!(super::text(Some(&seven), std::slice::from_ref(&symbols)).is_ok());
    // A bad fallback is a refusal naming it, not a silent gap.
    let error = super::text(Some(&seven), &[symbols, junk.clone()])
        .err()
        .unwrap();
    assert!(matches!(&error, Error::Fallback { path, .. } if *path == junk));
    let message = error.to_string();
    assert!(
        message.contains("fallback font") && message.contains("junk.ttf"),
        "{message}"
    );
    let missing = scratch.0.join("absent.ttf");
    assert!(super::text(Some(&seven), &[missing]).is_err());
}
