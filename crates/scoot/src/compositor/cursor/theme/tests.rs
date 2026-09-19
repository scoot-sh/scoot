//! Tests for cursor-theme loading.
//!
//! Deliberately hermetic: none of these depends on a cursor theme being
//! installed on the machine running the suite. A test that loaded "whatever
//! Adwaita is here" would assert different pixels on a developer's laptop, in
//! CI and in a bare container -- and would silently pass by doing nothing at
//! all on the last of those, which is exactly the environment scoot targets.
//!
//! So the file-reading half is left to the integration evidence (a real
//! toolkit under a real scoot, recorded in
//! `docs/backlog/resolved/foot-protocol-warnings-done.md`), and what is
//! tested here is the half that can be silently wrong: [`decode`]'s choice of
//! size, its hotspot, and whether the pixel bytes survive in the order the
//! render path expects. Those are exercised against xcursor files this module
//! builds itself, byte by byte, to the format the parser documents.

use super::*;

/// Builds one xcursor file containing `images`, each `(nominal_size, width,
/// height, xhot, yhot, fill_byte)`.
///
/// Written out by hand rather than with a helper crate: the format is the
/// thing under test, and encoding it here means a change in what the parser
/// expects shows up as a failure rather than as two libraries agreeing with
/// each other about the wrong thing. Layout per
/// `xcursor-0.3.11/src/parser.rs`: a 16-byte header, then one 12-byte table
/// entry per image, then each image's own 36-byte chunk header and pixels.
fn xcursor_file(images: &[(u32, u32, u32, u32, u32, u8)]) -> Vec<u8> {
    const IMAGE_TYPE: u32 = 0xfffd_0002;
    let header_len = 16u32;
    let toc_len = 12u32 * images.len() as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"Xcur");
    out.extend_from_slice(&header_len.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // file version
    out.extend_from_slice(&(images.len() as u32).to_le_bytes());

    // Where each image chunk will start, so the table can point at it.
    let mut position = header_len + toc_len;
    for (size, width, height, ..) in images {
        out.extend_from_slice(&IMAGE_TYPE.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&position.to_le_bytes());
        position += 36 + 4 * width * height;
    }

    for (size, width, height, xhot, yhot, fill) in images {
        out.extend_from_slice(&36u32.to_le_bytes()); // chunk header size
        out.extend_from_slice(&IMAGE_TYPE.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes()); // image version
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&xhot.to_le_bytes());
        out.extend_from_slice(&yhot.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // delay
        out.extend(std::iter::repeat_n(*fill, (4 * width * height) as usize));
    }
    out
}

/// The nominal size, width/height, hotspot and fill of the images in
/// [`xcursor_file`], as `decode` should read them back.
fn image(size: u32, hot: u32, fill: u8) -> (u32, u32, u32, u32, u32, u8) {
    (size, size, size, hot, hot, fill)
}

#[test]
fn a_single_image_decodes_with_its_own_hotspot() {
    let file = xcursor_file(&[image(24, 3, 0xAB)]);
    let themed = decode(&file, 24).expect("the file decodes");
    assert_eq!(themed.hotspot, (3, 3).into());
}

#[test]
fn the_nearest_nominal_size_wins() {
    // A real theme file carries several sizes. Picking the wrong one is not a
    // crash and not a visual bug anyone would report precisely -- the cursor
    // is merely the wrong size -- so it is worth pinning.
    let file = xcursor_file(&[image(16, 1, 0x10), image(24, 2, 0x20), image(48, 4, 0x40)]);
    for (asked, expected_hotspot) in [
        (16, 1),
        (18, 1),
        (23, 2),
        (24, 2),
        (30, 2),
        (48, 4),
        (99, 4),
    ] {
        let themed = decode(&file, asked).expect("the file decodes");
        assert_eq!(
            themed.hotspot,
            (expected_hotspot, expected_hotspot).into(),
            "asking for {asked} picked the wrong image"
        );
    }
}

#[test]
fn nearest_prefers_the_first_frame_of_an_animation() {
    // An animated cursor stores several frames under the *same* nominal size.
    // scoot does not drive cursor animation (see `nearest`'s doc), so the
    // first frame is what gets drawn -- not the last, which is what a naive
    // "find the matching size" would land on.
    let file = xcursor_file(&[image(24, 1, 0x11), image(24, 7, 0x77)]);
    let themed = decode(&file, 24).expect("the file decodes");
    assert_eq!(themed.hotspot, (1, 1).into(), "the second frame was picked");
}

#[test]
fn a_zero_sized_image_is_not_chosen() {
    // `parse_img` already refuses a zero dimension, so such a file fails to
    // parse entirely rather than yielding a degenerate image. Pinned because
    // `nearest` also filters on it, and a reader should not have to wonder
    // which of the two is load-bearing.
    let file = xcursor_file(&[(24, 0, 0, 0, 0, 0)]);
    assert!(decode(&file, 24).is_none());
}

#[test]
fn rubbish_bytes_decode_to_nothing_rather_than_panicking() {
    // A theme file on the user's own disk is not adversarial, but it is not
    // this project's either: it can be truncated by a half-finished package
    // install or simply not be a cursor at all. None of that may take the
    // compositor down.
    for bytes in [
        &b""[..],
        &b"Xcur"[..],
        &b"not a cursor file at all"[..],
        &xcursor_file(&[image(24, 3, 0xAB)])[..8],
    ] {
        assert!(decode(bytes, 24).is_none(), "decoded {bytes:?}");
    }
}

#[test]
fn a_theme_that_does_not_exist_loads_nothing_and_falls_back() {
    // The ordinary state in a container, and the state every other test in
    // this crate runs in: no theme, so every named shape comes from
    // `shapes.rs`. `image` must answer `None` rather than, say, panicking on
    // an empty theme.
    let mut theme = Theme::load(Some("scoot-test-no-such-theme"), 16);
    assert!(!theme.is_loaded());
    assert!(theme.image(CursorIcon::Default).is_none());
    assert!(theme.image(CursorIcon::Text).is_none());
}

#[test]
fn an_empty_configured_name_is_treated_as_unset() {
    // `config.rs` filters an empty `cursor_theme` out before this is reached,
    // but `Theme::load` is public within the crate and the same empty string
    // can arrive from `$XCURSOR_THEME`. Either way it must fall through to
    // the default name rather than search for a theme called "".
    let theme = Theme::load(Some(""), 16);
    assert_ne!(theme.name(), "");
}

#[test]
fn the_configured_size_is_reported_for_export() {
    // `compositor::run` puts this in `XCURSOR_SIZE` for children, so a client
    // loading its own theme matches the compositor's.
    assert_eq!(
        Theme::load(Some("scoot-test-no-such-theme"), 32).size(),
        32
    );
    // Never zero, whatever it is handed: a zero nominal size would make
    // `nearest` prefer the smallest image in every file.
    assert_eq!(Theme::load(Some("scoot-test-no-such-theme"), 0).size(), 1);
}
