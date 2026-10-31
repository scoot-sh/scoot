use std::path::{Path, PathBuf};

use super::{Family, bold_sibling, find_family, squash};
use crate::font::{Spec, build};
use crate::testfont;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!(
            "scootui-family-{}-{nanos}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// A font file (the test font's bytes) at `rel`, parents made.
    fn font(&self, rel: &str) -> PathBuf {
        self.file(rel, &testfont::build())
    }

    fn file(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dirs(scratch: &Scratch, rels: &[&str]) -> Vec<PathBuf> {
    rels.iter().map(|rel| scratch.0.join(rel)).collect()
}

#[test]
fn names_squash_to_lowercase_letters_and_digits() {
    assert_eq!(squash("DejaVu Sans"), "dejavusans");
    assert_eq!(squash("Noto_Sans-Regular"), "notosansregular");
    assert_eq!(squash("  "), "");
}

#[test]
fn a_family_finds_its_regular_and_bold_files_and_no_other_style() {
    let s = Scratch::new("styles");
    let regular = s.font("dejavu/DejaVuSans.ttf");
    let bold = s.font("dejavu/DejaVuSans-Bold.ttf");
    s.font("dejavu/DejaVuSans-BoldOblique.ttf");
    s.font("dejavu/DejaVuSans-ExtraLight.ttf");
    let mono = s.font("dejavu/DejaVuSansMono.ttf");
    s.file("dejavu/DejaVuSans.txt", b"not a font");
    s.font("dejavu/DejaVuSans.ttc");
    let dirs = dirs(&s, &["dejavu"]);
    assert_eq!(
        find_family("DejaVu Sans", &dirs, true),
        Some(Family {
            regular,
            bold: Some(bold)
        })
    );
    // The longer family name is its own, not a prefix match.
    let found = find_family("DejaVu Sans Mono", &dirs, true).unwrap();
    assert_eq!(found.regular, mono);
    assert_eq!(found.bold, None);
    // The bold is looked for only when asked.
    assert_eq!(find_family("DejaVu Sans", &dirs, false).unwrap().bold, None);
}

#[test]
fn regular_in_the_name_and_a_nerd_font_family_both_match() {
    let s = Scratch::new("regular");
    let noto = s.font("noto/NotoSans-Regular.ttf");
    let nerd = s.font("nerd/DroidSansMNerdFontPropo-Regular.otf");
    let nerd_bold = s.font("nerd/DroidSansMNerdFontPropo-Bold.otf");
    let dirs = dirs(&s, &["noto", "nerd"]);
    let found = find_family("Noto Sans", &dirs, true).unwrap();
    assert_eq!((found.regular, found.bold), (noto, None));
    let found = find_family("DroidSansM Nerd Font Propo", &dirs, true).unwrap();
    assert_eq!((found.regular, found.bold), (nerd, Some(nerd_bold)));
}

#[test]
fn the_first_directory_with_the_family_wins_and_the_bold_comes_from_it() {
    let s = Scratch::new("order");
    let user = s.font("user/Inter-Regular.ttf");
    s.font("system/Inter-Regular.ttf");
    // A bold only in a later directory is another install's, not a sibling.
    s.font("system/Inter-Bold.ttf");
    let found = find_family("Inter", &dirs(&s, &["user", "system"]), true).unwrap();
    assert_eq!(found.regular, user);
    assert_eq!(found.bold, None);
}

#[test]
fn nested_and_symlinked_directories_are_walked() {
    let s = Scratch::new("nested");
    let deep = s.font("truetype/NerdFonts/Family/Iosevka-Regular.ttf");
    let store = s.0.join("store");
    std::fs::create_dir_all(&store).unwrap();
    let linked = s.font("store/Lato-Regular.ttf");
    std::os::unix::fs::symlink(&store, s.0.join("link")).unwrap();
    std::os::unix::fs::symlink(
        s.0.join("store/Lato-Regular.ttf"),
        s.0.join("truetype/Lato-Regular.ttf"),
    )
    .unwrap();
    let dirs = dirs(&s, &["truetype", "link"]);
    assert_eq!(find_family("Iosevka", &dirs, false).unwrap().regular, deep);
    let found = find_family("Lato", &dirs, false).unwrap();
    assert!(found.regular.ends_with("Lato-Regular.ttf"));
    assert!(linked.is_file());
}

#[test]
fn a_symlink_loop_and_a_missing_directory_end_the_search() {
    let s = Scratch::new("loop");
    s.font("a/Other-Regular.ttf");
    std::os::unix::fs::symlink(s.0.join("a"), s.0.join("a/loop")).unwrap();
    let dirs = dirs(&s, &["missing", "a"]);
    assert_eq!(find_family("NotThere", &dirs, true), None);
    assert_eq!(find_family("", &dirs, true), None);
}

#[test]
fn a_bold_sibling_is_named_from_the_regular_file() {
    let s = Scratch::new("sibling");
    let plain = s.font("DejaVuSans.ttf");
    let bold = s.font("DejaVuSans-Bold.ttf");
    let noto = s.font("NotoSans-Regular.otf");
    let noto_bold = s.font("NotoSans-Bold.otf");
    let alone = s.font("Alone-Regular.ttf");
    assert_eq!(bold_sibling(&plain), Some(bold.clone()));
    assert_eq!(bold_sibling(&noto), Some(noto_bold));
    assert_eq!(bold_sibling(&alone), None);
    // A bold file has no bold sibling of its own.
    assert_eq!(bold_sibling(&bold), None);
    assert_eq!(bold_sibling(Path::new("/nonexistent/Regular.ttf")), None);
    assert_eq!(bold_sibling(Path::new("/")), None);
}

#[test]
fn a_text_has_its_bold_face_beside_a_given_file() {
    let s = Scratch::new("given");
    let regular = s.font("Face-Regular.ttf");
    let spec = Spec {
        given: Some(&regular),
        bold: true,
        ..Spec::default()
    };
    // No sibling yet: regular only.
    assert!(!build(&spec, &[]).unwrap().0.has_bold());
    s.file(
        "Face-Bold.ttf",
        &testfont::build_symbols_advance(['x'], 900),
    );
    let (text, note) = build(&spec, &[]).unwrap();
    assert!(text.has_bold());
    assert_eq!(note, None);
    assert_eq!(
        text.measure_face(None, "x", 50.0, crate::text::Face::Bold),
        45
    );
    // Not asked for: not loaded.
    let spec = Spec {
        bold: false,
        ..spec
    };
    assert!(!build(&spec, &[]).unwrap().0.has_bold());
}

#[test]
fn a_family_picks_its_file_and_an_unknown_one_is_a_note() {
    let s = Scratch::new("spec");
    s.font("Brand-Regular.ttf");
    s.file(
        "Brand-Bold.ttf",
        &testfont::build_symbols_advance(['x'], 900),
    );
    let dirs = dirs(&s, &["."]);
    let spec = Spec {
        family: "Brand",
        bold: true,
        ..Spec::default()
    };
    let (text, note) = build(&spec, &dirs).unwrap();
    assert!(text.has_bold());
    assert_eq!(note, None);
    assert_eq!(text.measure(None, "3", 50.0), 30);
    // A family nobody installed: the default font and a note naming it (or
    // no default font on this machine: the error, not a panic).
    let spec = Spec {
        family: "ZZ No Such Family",
        ..Spec::default()
    };
    match build(&spec, &dirs) {
        Ok((_, note)) => assert!(note.unwrap().contains("ZZ No Such Family")),
        Err(error) => assert!(error.to_string().contains("no font")),
    }
    // A family whose file is not a font: a note too.
    s.file("Broken-Regular.ttf", b"not a font");
    let spec = Spec {
        family: "Broken",
        ..Spec::default()
    };
    match build(&spec, &dirs) {
        Ok((_, note)) => assert!(note.unwrap().contains("Broken")),
        Err(error) => assert!(error.to_string().contains("no font")),
    }
}

#[test]
fn a_given_file_beats_the_family() {
    let s = Scratch::new("beats");
    let given = s.font("Given.ttf");
    s.font("Brand-Regular.ttf");
    let spec = Spec {
        given: Some(&given),
        family: "Brand",
        ..Spec::default()
    };
    let (_, note) = build(&spec, &dirs(&s, &["."])).unwrap();
    assert_eq!(note, None);
}
