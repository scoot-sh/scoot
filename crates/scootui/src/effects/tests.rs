use super::*;

const DARK: Color = Color {
    r: 30,
    g: 30,
    b: 46,
};
const LIGHT: Color = Color {
    r: 239,
    g: 241,
    b: 245,
};

#[test]
fn the_values_parse_and_nothing_else_does() {
    for (name, style) in [
        ("none", ShadowStyle::None),
        ("soft", ShadowStyle::Soft),
        ("deep", ShadowStyle::Deep),
        ("glow", ShadowStyle::Glow),
    ] {
        assert_eq!(ShadowStyle::parse(name), Some(style));
    }
    assert_eq!(FillStyle::parse("solid"), Some(FillStyle::Solid));
    assert_eq!(FillStyle::parse("gradient"), Some(FillStyle::Gradient));
    for bad in ["", "Soft", "huge", "rainbow", " soft"] {
        assert_eq!(ShadowStyle::parse(bad), None, "{bad:?}");
        assert_eq!(FillStyle::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn only_none_has_no_spec() {
    assert!(ShadowStyle::None.spec().is_none());
    for style in [ShadowStyle::Soft, ShadowStyle::Deep, ShadowStyle::Glow] {
        let spec = style.spec().expect("a spec");
        assert!(spec.blur > 0 && spec.alpha > 0);
    }
    // A glow sits centered under the surface; the others drop.
    assert_eq!(ShadowStyle::Glow.spec().map(|s| s.offset_y), Some(0));
}

#[test]
fn a_solid_fill_is_one_color_and_a_gradient_keeps_the_light_on_top() {
    assert_eq!(FillStyle::Solid.stops(DARK), (DARK, DARK));
    let (top, bottom) = FillStyle::Gradient.stops(DARK);
    assert_eq!(bottom, DARK);
    assert!(top.luminance() > DARK.luminance());
    let (top, bottom) = FillStyle::Gradient.stops(LIGHT);
    assert_eq!(top, LIGHT);
    assert!(bottom.luminance() < LIGHT.luminance());
}
