use super::{Color, ColorError};

#[test]
fn rrggbb_parses_in_either_case() {
    let expected = Color {
        r: 0xc0,
        g: 0x30,
        b: 0x2f,
    };
    assert_eq!(Color::parse("#c0302f"), Ok(expected));
    assert_eq!(Color::parse("#C0302F"), Ok(expected));
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
fn anything_else_is_refused() {
    for text in [
        "",
        "#",
        "c03020",
        "#c0302",
        "#c030201",
        "#c0302g",
        " #c03020",
        "#c03020 ",
        "#fff",
        "#c03020ff",
        "#+03020",
        // Six bytes, but not six ASCII hex digits.
        "#c0302é",
        "##c0302",
    ] {
        assert_eq!(Color::parse(text), Err(ColorError), "{text:?}");
    }
}

#[test]
fn a_pixel_is_opaque_xrgb_little_endian_in_memory() {
    let color = Color {
        r: 0x12,
        g: 0x34,
        b: 0x56,
    };
    assert_eq!(color.xrgb8888(), 0xff12_3456);
    assert_eq!(color.xrgb8888().to_le_bytes(), [0x56, 0x34, 0x12, 0xff]);
}

#[test]
fn it_prints_as_it_parses() {
    let color = Color::parse("#1E1E2E").unwrap();
    assert_eq!(color.to_string(), "#1e1e2e");
    assert_eq!(Color::parse(&color.to_string()), Ok(color));
}
