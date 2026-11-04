//! A themed tray icon the allocator refuses is a missing icon, never an
//! abort: the file's read and the decoder's output and pixel buffers are
//! sized by a PNG an item's `IconThemePath` can point the bar at, and they
//! run on the bus turn. The refusal and its harness are scootui's
//! (`scootui::refusal`); without the fallible allocations each of these
//! ends in `memory allocation of N bytes failed` and `SIGABRT`.

use scootui::refusal::in_refusing_child;

use super::{decode, read_from_file};

/// A `side × side` 8-bit gray PNG.
fn gray_png(side: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, side, side);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![200; (side * side) as usize])
            .unwrap();
    }
    out
}

#[test]
fn a_decode_the_allocator_refuses_is_no_icon() {
    in_refusing_child(
        concat!(
            module_path!(),
            "::a_decode_the_allocator_refuses_is_no_icon"
        ),
        |refusing| {
            let big = gray_png(512);
            let small = gray_png(16);
            // Decodes when memory is not refused: the refusal is the only
            // reason the icon below is missing.
            assert!(decode("big", &big).is_some(), "the fixture did not decode");
            // The pixel buffer is 1 MiB; the decoder's output, 256 KiB, passes.
            refusing.from(512 * 1024);
            assert!(decode("big", &big).is_none());
            assert!(decode("small", &small).is_some(), "the control failed");
        },
    );
}

#[test]
fn a_read_the_allocator_refuses_is_no_icon() {
    in_refusing_child(
        concat!(module_path!(), "::a_read_the_allocator_refuses_is_no_icon"),
        |refusing| {
            let dir =
                std::env::temp_dir().join(format!("scootbar-theme-refused-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            // A sparse 4 MiB file: within the 8 MiB cap, and nothing is read
            // once the block to read it into is refused.
            let big = dir.join("big.png");
            std::fs::File::create(&big)
                .unwrap()
                .set_len(4 * 1024 * 1024)
                .unwrap();
            let small = dir.join("small.png");
            std::fs::write(&small, gray_png(16)).unwrap();
            refusing.from(1024 * 1024);
            let refused = read_from_file("big", std::fs::File::open(&big).unwrap());
            let control = read_from_file("small", std::fs::File::open(&small).unwrap());
            let _ = std::fs::remove_dir_all(&dir);
            assert!(refused.is_none());
            assert!(control.is_some(), "the control failed");
        },
    );
}
