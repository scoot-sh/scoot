//! The hints contract: every known key, type confusion ignored, the
//! 64-hint cap, and the value clamp.

use scoot_dbus::proto::Reader;

use super::super::testutils::{Val, hints_bytes};
use super::{Hints, MAX_HINTS, Urgency, read};

fn read_of(bytes: &[u8]) -> Hints {
    read(&mut Reader::le(bytes)).unwrap()
}

#[test]
fn every_known_key_reads() {
    let bytes = hints_bytes(&[
        ("urgency", Val::U8(2)),
        ("category", Val::Str("transfer.complete")),
        ("desktop-entry", Val::Str("firefox")),
        ("image-path", Val::Str("/icon.png")),
        ("transient", Val::Bool(true)),
        ("resident", Val::Bool(true)),
        ("value", Val::I32(73)),
        ("suppress-sound", Val::Str("x")),
        ("sound-file", Val::Str("/s.wav")),
        ("x", Val::I32(1)),
        ("action-icons", Val::Bool(true)),
        ("x-vendor-thing", Val::Str("v")),
    ]);
    let hints = read_of(&bytes);
    assert_eq!(hints.urgency, Urgency::Critical);
    assert_eq!(hints.category, "transfer.complete");
    assert_eq!(hints.desktop_entry, "firefox");
    assert_eq!(hints.image_path, "/icon.png");
    assert!(hints.transient && hints.resident);
    assert_eq!(hints.value, Some(73));
}

#[test]
fn image_data_reads_inline() {
    let pixels = vec![9u8; 2 * 2 * 4];
    let bytes = hints_bytes(&[(
        "image-data",
        Val::Image {
            width: 2,
            height: 2,
            rowstride: 8,
            alpha: true,
            bits: 8,
            channels: 4,
            pixels: &pixels,
        },
    )]);
    let hints = read_of(&bytes);
    let image = hints.image_data.unwrap();
    assert_eq!((image.width, image.height), (2, 2));
    assert_eq!(image.pixels, vec![9u8; 16]);
}

#[test]
fn type_confusion_keeps_the_default() {
    // Every key carrying a stranger's type.
    let bytes = hints_bytes(&[
        ("urgency", Val::Str("1")),
        ("urgency", Val::I32(2)),
        ("urgency", Val::U32(2)),
        ("category", Val::I32(1)),
        ("transient", Val::Str("1")),
        ("resident", Val::U32(1)),
        ("value", Val::Str("1")),
        ("value", Val::Bool(true)),
        ("image-path", Val::I32(1)),
        ("image-data", Val::Str("x")),
    ]);
    assert_eq!(read_of(&bytes), Hints::default());
}

#[test]
fn urgency_bytes_map() {
    for (byte, urgency) in [
        (0, Urgency::Low),
        (1, Urgency::Normal),
        (2, Urgency::Critical),
        (9, Urgency::Normal),
    ] {
        let bytes = hints_bytes(&[("urgency", Val::U8(byte))]);
        assert_eq!(read_of(&bytes).urgency, urgency);
    }
}

#[test]
fn the_value_clamps() {
    for (value, want) in [
        (Val::I32(-5), 0),
        (Val::I32(101), 100),
        (Val::U32(u32::MAX), 100),
        (Val::U8(255), 100),
    ] {
        let bytes = hints_bytes(&[("value", value)]);
        assert_eq!(read_of(&bytes).value, Some(want));
    }
}

#[test]
fn long_strings_cut_never_refuse() {
    let long = "é".repeat(200);
    let bytes = hints_bytes(&[
        ("category", Val::Str(&long)),
        ("desktop-entry", Val::Str(&long)),
    ]);
    let hints = read_of(&bytes);
    assert!(hints.category.len() <= super::MAX_SHORT_HINT);
    assert!(hints.desktop_entry.len() <= super::MAX_SHORT_HINT);
    // Cut at a char boundary, never mid-`é`.
    assert!(core::str::from_utf8(hints.category.as_bytes()).is_ok());
}

#[test]
fn past_64_hints_the_tail_skips_unread() {
    // 80 unknown hints: none stored, all walked without complaint.
    let entries: Vec<(String, Val<'_>)> = (0..80)
        .map(|index| (format!("x-k{index}"), Val::U8(2)))
        .collect();
    let refs: Vec<(&str, Val<'_>)> = entries
        .iter()
        .map(|(key, value)| (key.as_str(), *value))
        .collect();
    assert_eq!(read_of(&hints_bytes(&refs)), Hints::default());
    // 80 `urgency` hints: read in order, so the first (low) wins over
    // the 65th (critical) the cap never reaches.
    let entries: Vec<(&str, Val<'_>)> = (0..80).map(|_| ("urgency", Val::U8(0))).collect();
    let mut tail = entries.clone();
    tail[64] = ("urgency", Val::U8(2));
    assert_eq!(read_of(&hints_bytes(&tail)).urgency, Urgency::Low);
    let _ = MAX_HINTS;
}
