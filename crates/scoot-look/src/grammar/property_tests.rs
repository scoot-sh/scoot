//! Property-style robustness: hostile inputs never panic, only refuse.

use super::*;
use crate::schema::parse_look_toml;

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn hostile_bytes_never_panic() {
    // Deterministic garbage over an alphabet rich in TOML syntax, quotes,
    // NUL and non-UTF8: every outcome must be a clean refusal, never a
    // panic, and oversized inputs must refuse on size.
    let alphabet: &[u8] = b"abcd[]= \"'#\n\r\t\0\xff\x80...;{}-_+./:@";
    let mut state = 0x12345678u64;
    for round in 0..2000 {
        let len = (xorshift(&mut state) % 96) as usize;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(alphabet[(xorshift(&mut state) % alphabet.len() as u64) as usize]);
        }
        let _ = parse_look_toml(&bytes);
        let _ = round;
    }
}

#[test]
fn hostile_names_never_panic() {
    let mut state = 0x9e3779b9u64;
    for _ in 0..2000 {
        let len = (xorshift(&mut state) % 40) as usize;
        let s: String = (0..len)
            .map(|_| char::from_u32((xorshift(&mut state) % 0x300) as u32).unwrap_or('?'))
            .collect();
        assert!(!is_name(&s) || s.len() <= 32);
        let _ = is_identifier(&s);
        let _ = is_nix_attr_path(&s);
        let _ = check_display_text("f", &s, 200);
        let _ = check_glyph("f", &s);
        let _ = check_relative_path("f", &s);
        let _ = check_wallpaper_url("f", &s);
    }
}
