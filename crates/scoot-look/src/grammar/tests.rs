//! Tests for the value grammar: every refusal named, and every string
//! token fed a newline, `=`, `[`, a quote and NUL.

use super::*;

#[test]
fn hex_colors() {
    assert!(is_hex_color("#FF9A49"));
    assert!(is_hex_color("#ff9a49"));
    assert!(!is_hex_color("#FFF"));
    assert!(!is_hex_color("#FF9A4977"));
    assert!(!is_hex_color("FF9A49"));
    assert!(!is_hex_color("#GGGGGG"));
    assert!(!is_hex_color(""));
    assert!(is_cursor_accent("none"));
    assert!(is_cursor_accent("#2B3648"));
    assert!(!is_cursor_accent(""));
}

#[test]
fn names() {
    assert!(is_name("moonrise"));
    assert!(is_name("ginger-night"));
    assert!(is_name("a"));
    assert!(!is_name(""));
    assert!(!is_name("Moonrise"));
    assert!(!is_name("moon_rise"));
    assert!(!is_name("moon rise"));
    assert!(!is_name("a".repeat(33).as_str()));
    assert!(is_name("a".repeat(32).as_str()));
}

#[test]
fn identifiers_start_alphanumeric_and_stay_ascii() {
    assert!(is_identifier("DroidSansM Nerd Font Propo"));
    assert!(is_identifier("Mocu"));
    assert!(is_identifier("9pt mono+"));
    assert!(!is_identifier(""));
    assert!(!is_identifier("."));
    assert!(!is_identifier(".."));
    assert!(!is_identifier(".hidden"));
    assert!(!is_identifier(" leading"));
    assert!(!is_identifier("has/slash"));
    assert!(!is_identifier("has\"quote"));
    assert!(!is_identifier("déjà"));
    assert!(!is_identifier("a".repeat(65).as_str()));
}

#[test]
fn nix_attr_paths() {
    assert!(is_nix_attr_path("dejavu_fonts"));
    assert!(is_nix_attr_path("nerd-fonts.droid-sans-mono"));
    assert!(!is_nix_attr_path(""));
    assert!(!is_nix_attr_path("9fonts"));
    assert!(!is_nix_attr_path(".fonts"));
    assert!(!is_nix_attr_path("a..b"));
    assert!(!is_nix_attr_path("a b"));
    assert!(!is_nix_attr_path("a/b"));
    assert!(is_nix_attr_path("a.b.c.d.e.f.g.h"));
    assert!(!is_nix_attr_path("a.b.c.d.e.f.g.h.i"));
}

#[test]
fn display_text_refuses_syntax_characters() {
    for bad in [
        "a=b", "a[b", "a]b", "a\"b", "a\\b", "a\nb", "a\0b", "a\x7fb",
    ] {
        assert!(
            check_display_text("title", bad, 64).is_some(),
            "should refuse {bad:?}"
        );
    }
    assert!(check_display_text("title", "Moonrise", 64).is_none());
    assert!(check_display_text("title", "x".repeat(64).as_str(), 64).is_none());
    assert!(check_display_text("title", "x".repeat(65).as_str(), 64).is_some());
}

#[test]
fn glyphs() {
    assert!(check_glyph("k", "●").is_none());
    assert!(check_glyph("k", "ab12").is_none());
    assert!(check_glyph("k", "").is_some());
    assert!(check_glyph("k", "123456789").is_some());
    assert!(check_glyph("k", "a=b").is_some());
    assert!(check_glyph("k", "a\nb").is_some());
}

#[test]
fn paths() {
    assert!(check_relative_path("wallpaper.image", "wallpaper.png").is_none());
    assert!(check_relative_path("p", "fonts/a.otf").is_none());
    assert!(check_relative_path("p", "/abs.png").is_some());
    assert!(check_relative_path("p", "../evil.png").is_some());
    assert!(check_relative_path("p", "a/../../b").is_some());
    assert!(check_relative_path("p", "a\\b").is_some());
    assert!(check_relative_path("p", "").is_some());
    assert!(check_relative_path("p", "a\0b").is_some());
}

#[test]
fn wallpaper_urls() {
    assert!(check_wallpaper_url("wallpaper.image", "https://example.com/w.png").is_none());
    // IDNs and punycode are fine given the hash pin.
    assert!(check_wallpaper_url("wallpaper.image", "https://xn--nxasmq6b.example/w.png").is_none());
    for bad in [
        "http://example.com/w.png",
        "https://user@example.com/w.png",
        "https://127.0.0.1/w.png",
        "https://[::1]/w.png",
        "https://example.com:8443/w.png",
        "https://localhost/w.png",
        "https://LOCALHOST./w.png",
        "https://foo.local/w.png",
        "https://foo.local./w.png",
        "https://foo.internal/w.png",
        "https://foo.home.arpa/w.png",
        "https://foo.localhost/w.png",
        "https://example/w.png",
        "https://127.1/w.png",
        "https://0x7f.0.0.1/w.png",
        "https://example.com/has space.png",
        "https://example.com/a<b.png",
    ] {
        assert!(
            check_wallpaper_url("wallpaper.image", bad).is_some(),
            "should refuse {bad:?}"
        );
    }
}

#[test]
fn sha256_pins() {
    assert!(is_sha256_hex(&"ab".repeat(32)));
    assert!(!is_sha256_hex("abc"));
    assert!(!is_sha256_hex(&"zz".repeat(32)));
}

#[test]
fn every_string_token_refuses_injection_characters() {
    // A newline, `=`, `[`, a quote and NUL must not survive in any string
    // token: identifiers refuse them outright, display text refuses them by
    // name, paths refuse NUL/controls (newline and NUL included).
    for probe in ["a\nb", "a=b", "a[", "a\"", "a\0b"] {
        assert!(!is_identifier(probe), "identifier accepts {probe:?}");
        assert!(
            check_display_text("title", probe, 200).is_some(),
            "display text accepts {probe:?}"
        );
        if probe != "a=b" && probe != "a[" && probe != "a\"" {
            assert!(
                check_relative_path("p", probe).is_some(),
                "path accepts {probe:?}"
            );
        }
    }
}
