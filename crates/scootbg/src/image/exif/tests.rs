use super::orientation;
use crate::image::orientation::Orientation;

/// A minimal EXIF block: TIFF header, IFD0 with `entries` (tag, type,
/// count, value), then a zero next-IFD offset.
pub fn block(big_endian: bool, entries: &[(u16, u16, u32, u16)]) -> Vec<u8> {
    let u16b = |v: u16| {
        if big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let u32b = |v: u32| {
        if big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let mut out = Vec::new();
    out.extend_from_slice(if big_endian { b"MM" } else { b"II" });
    out.extend_from_slice(&u16b(42));
    out.extend_from_slice(&u32b(8));
    out.extend_from_slice(&u16b(entries.len() as u16));
    for &(tag, kind, count, value) in entries {
        out.extend_from_slice(&u16b(tag));
        out.extend_from_slice(&u16b(kind));
        out.extend_from_slice(&u32b(count));
        out.extend_from_slice(&u16b(value));
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&u32b(0));
    out
}

fn with(value: u16) -> [(u16, u16, u32, u16); 3] {
    // Among other tags, as cameras write them: Make, Orientation,
    // XResolution (the latter two with made-up values).
    [(0x010f, 2, 4, 0), (0x0112, 3, 1, value), (0x011a, 5, 1, 0)]
}

#[test]
fn all_eight_orientations_in_both_byte_orders() {
    for big_endian in [false, true] {
        for value in 1..=8 {
            let exif = block(big_endian, &with(value));
            assert_eq!(
                orientation(&exif).value(),
                value as u8,
                "{big_endian} {value}"
            );
            // With the APP1 marker's prefix, as some WebP writers keep it.
            let mut prefixed = b"Exif\0\0".to_vec();
            prefixed.extend_from_slice(&exif);
            assert_eq!(orientation(&prefixed).value(), value as u8);
        }
    }
}

#[test]
fn no_tag_or_a_bad_value_is_normal() {
    let none = block(false, &[(0x010f, 2, 4, 0)]);
    assert_eq!(orientation(&none), Orientation::NORMAL);
    for value in [0, 9, 0xffff] {
        assert_eq!(orientation(&block(true, &with(value))), Orientation::NORMAL);
    }
    // The wrong type, or a count other than 1: not trusted.
    let long = block(false, &[(0x0112, 4, 1, 6)]);
    assert_eq!(orientation(&long), Orientation::NORMAL);
    let many = block(false, &[(0x0112, 3, 2, 6)]);
    assert_eq!(orientation(&many), Orientation::NORMAL);
    assert_eq!(orientation(&block(false, &[])), Orientation::NORMAL);
}

#[test]
fn every_truncation_is_normal_and_never_panics() {
    let exif = block(true, &with(6));
    for len in 0..exif.len() {
        let got = orientation(&exif[..len]);
        // Cut before the orientation's value: nothing to read.
        if len < 8 + 2 + 12 + 10 {
            assert_eq!(got, Orientation::NORMAL, "cut at {len}");
        }
    }
    assert_eq!(orientation(&exif).value(), 6);
}

#[test]
fn malicious_blocks_are_normal() {
    let cases: Vec<Vec<u8>> = vec![
        vec![],
        b"II".to_vec(),
        b"XX*\0\x08\0\0\0".to_vec(),
        // Wrong magic.
        b"II\x2b\0\x08\0\0\0\x01\0".to_vec(),
        // IFD0 offset past the end, and at u32::MAX (an overflow on 32-bit).
        b"II*\0\xff\0\0\0".to_vec(),
        b"II*\0\xff\xff\xff\xff".to_vec(),
        b"MM\0*\xff\xff\xff\xf0".to_vec(),
        // A count of 65,535 entries with none present.
        b"II*\0\x08\0\0\0\xff\xff".to_vec(),
        // IFD0 pointing at the header itself.
        b"II*\0\0\0\0\0".to_vec(),
    ];
    for case in cases {
        assert_eq!(orientation(&case), Orientation::NORMAL, "{case:?}");
    }
    // Random bytes behind a valid header, many lengths: never a panic.
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    for len in 0..512 {
        let mut bytes = b"MM\0*\0\0\0\x08".to_vec();
        for _ in 0..len {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            bytes.push(state as u8);
        }
        let _ = orientation(&bytes);
    }
}
