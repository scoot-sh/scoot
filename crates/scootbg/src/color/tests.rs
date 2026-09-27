use super::{Color, ColorError};

#[test]
fn rrggbb_parses_in_either_case() {
    let expected = Color {
        r: 0xc0,
        g: 0x30,
        b: 0x20,
    };
    assert_eq!(Color::parse("#c03020"), Ok(expected));
    assert_eq!(Color::parse("#C03020"), Ok(expected));
    assert_eq!(Color::parse("#c0302F").map(|c| c.b), Ok(0x2f));
    assert_eq!(Color::parse("#000000"), Ok(Color { r: 0, g: 0, b: 0 }));
    assert_eq!(
        Color::parse("#ffffff"),
        Ok(Color {
            r: 255,
            g: 255,
            b: 255
        })
    );
}

#[test]
fn anything_not_starting_with_a_hash_is_not_a_color() {
    for text in [
        "",
        "c03020",
        "~/Pictures/hills.jpg",
        "./#draft.png",
        " #c03020",
        "red",
    ] {
        assert_eq!(Color::parse(text), Err(ColorError::NotAColor), "{text:?}");
    }
}

#[test]
fn malformed_colors_are_refused() {
    for text in [
        "#",
        "#fff",              // shorthand: not yet
        "#c03020ff",         // alpha: wallpapers are opaque
        "#c0302",            // five digits
        "#c030200",          // seven
        "#c0302g",           // not hex
        "#c03020 ",          // trailing whitespace
        "#c03020\n",         // a newline
        "# c03020",          // inner space
        "##c03020",          // two hashes
        "#+c0302",           // a sign
        "#c0\u{e9}20",       // non-ASCII: six bytes, not six digits
        "#\u{ff10}\u{ff11}", // fullwidth digits, six bytes
    ] {
        assert_eq!(Color::parse(text), Err(ColorError::Malformed), "{text:?}");
    }
}

#[test]
fn display_is_lowercase_rrggbb_and_round_trips() {
    for text in ["#c03020", "#000000", "#ffffff", "#0a0b0c"] {
        let color = Color::parse(text).unwrap();
        assert_eq!(color.to_string(), text);
    }
    assert_eq!(Color::parse("#ABCDEF").unwrap().to_string(), "#abcdef");
    let json = serde_json::to_string(&Color::parse("#C03020").unwrap()).unwrap();
    assert_eq!(json, "\"#c03020\"");
}

/// `v * 0x01010101` is exactly `v / 255` of `u32::MAX` for every 8-bit
/// value, and a compositor converting back to 8 bits with rounding (the
/// usual `(x * 255 + 2^31) >> 32`, or taking the top byte) recovers `v`.
#[test]
fn single_pixel_channels_scale_exactly() {
    for v in 0..=255u8 {
        let color = Color { r: v, g: v, b: v };
        let [r, g, b, a] = color.single_pixel();
        assert_eq!(a, u32::MAX);
        assert_eq!((r, g, b), (r, r, r));
        // Exact: v/255 of u32::MAX, with no remainder.
        assert_eq!(u64::from(r) * 255, u64::from(v) * u64::from(u32::MAX));
        // Back to 8 bits, both ways a compositor might.
        assert_eq!((r >> 24) as u8, v, "top byte");
        let rounded = (u64::from(r) * 255 + (1 << 31)) >> 32;
        assert_eq!(rounded, u64::from(v), "rounded");
        // And as a float fraction, the way Smithay stores it.
        let fraction = f64::from(r) / f64::from(u32::MAX);
        assert_eq!((fraction * 255.0).round() as u8, v);
    }
    let [r, g, b, _] = Color::parse("#c03020").unwrap().single_pixel();
    assert_eq!((r, g, b), (0xc0c0_c0c0, 0x3030_3030, 0x2020_2020));
}

#[test]
fn xrgb8888_is_little_endian_bgrx() {
    let pixel = Color::parse("#c03020").unwrap().xrgb8888();
    assert_eq!(pixel, [0x20, 0x30, 0xc0, 0xff]);
    assert_eq!(u32::from_le_bytes(pixel), 0xffc0_3020);
}

#[test]
fn errors_say_what_to_do() {
    let not = ColorError::NotAColor.to_string();
    assert!(not.contains("starts with '#'"), "{not}");
    assert!(not.contains("#rrggbb"), "{not}");
    let bad = ColorError::Malformed.to_string();
    assert!(bad.contains("#rrggbb"), "{bad}");
}
