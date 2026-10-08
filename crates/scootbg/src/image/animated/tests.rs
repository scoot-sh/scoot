use std::io::Cursor;

use super::{FALLBACK_DELAY_MS, MAX_ANIMATED_BYTES, decode_animated, normalize_delay};
use crate::color::Color;
use crate::image::samples;

const FILL: Color = Color { r: 0, g: 0, b: 0 };

fn animated(bytes: &[u8]) -> Option<super::Animated> {
    decode_animated(&mut Cursor::new(bytes), FILL).unwrap()
}

#[test]
fn delays_clamp_tiny_and_huge() {
    assert_eq!(normalize_delay(0), FALLBACK_DELAY_MS);
    assert_eq!(normalize_delay(10), FALLBACK_DELAY_MS);
    assert_eq!(normalize_delay(19), FALLBACK_DELAY_MS);
    assert_eq!(normalize_delay(20), 20);
    assert_eq!(normalize_delay(100), 100);
    assert_eq!(normalize_delay(3_600_000), 3_600_000);
    assert_eq!(normalize_delay(3_600_001), 3_600_000);
    assert_eq!(normalize_delay(u64::MAX), 3_600_000);
}

#[test]
fn static_images_are_not_animations() {
    // JPEG, static PNG and static WebP: None, drawn the static way.
    let png = crate::image::samples::png(
        2,
        1,
        png::ColorType::Rgb,
        png::BitDepth::Eight,
        &[10, 20, 30, 40, 50, 60],
        None,
    );
    assert!(animated(&png).is_none());
    let webp = crate::image::samples::webp(2, 1, &[10, 20, 30, 40, 50, 60], false, None);
    assert!(animated(&webp).is_none());
    assert!(animated(crate::image::samples::QUADRANTS_JPEG).is_none());
    // A single-frame GIF is not an animation either.
    assert!(animated(&samples::gif_single_frame()).is_none());
}

#[test]
fn gif_two_frames_decode_with_delays() {
    let bytes = samples::gif_two_frame();
    let anim = animated(&bytes).expect("two-frame GIF is animated");
    assert_eq!((anim.width, anim.height), (2, 1));
    assert_eq!(anim.frames.len(), 2);
    assert_eq!(anim.frames[0].delay_ms, 100);
    assert_eq!(anim.frames[1].delay_ms, 100);
    // First frame: red, green. Second: green, red.
    assert_eq!(&anim.frames[0].rgb, &[255, 0, 0, 0, 255, 0]);
    assert_eq!(&anim.frames[1].rgb, &[0, 255, 0, 255, 0, 0]);
}

#[test]
fn gif_zero_delay_becomes_the_fallback() {
    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, 1, 1, &[0, 0, 0, 255, 255, 255]).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for index in [0, 1] {
            let frame = gif::Frame {
                width: 1,
                height: 1,
                delay: 0,
                buffer: std::borrow::Cow::Borrowed(if index == 0 { &[0] } else { &[1] }),
                ..Default::default()
            };
            encoder.write_frame(&frame).unwrap();
        }
    }
    let anim = animated(&out).expect("animated");
    assert_eq!(anim.frames[0].delay_ms, FALLBACK_DELAY_MS);
    assert_eq!(anim.frames[1].delay_ms, FALLBACK_DELAY_MS);
}

#[test]
fn apng_two_frames_decode() {
    // 2×1, two full frames: red/green, then green/red, 100 ms each.
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_animated(2, 0).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.set_frame_delay(1, 10).unwrap();
        writer.write_image_data(&[255, 0, 0, 0, 255, 0]).unwrap();
        writer.set_frame_delay(1, 10).unwrap();
        writer.write_image_data(&[0, 255, 0, 255, 0, 0]).unwrap();
        writer.finish().unwrap();
    }
    let anim = animated(&out).expect("two-frame APNG is animated");
    assert_eq!((anim.width, anim.height), (2, 1));
    assert_eq!(anim.frames.len(), 2);
    assert_eq!(anim.frames[0].delay_ms, 100);
    assert_eq!(&anim.frames[0].rgb, &[255, 0, 0, 0, 255, 0]);
    assert_eq!(&anim.frames[1].rgb, &[0, 255, 0, 255, 0, 0]);
}

#[test]
fn caps_are_checked() {
    // One 1080p frame is 6.2 MB; 64 MiB holds ten, not eleven.
    let per_1080p = 1920usize * 1080 * 3;
    let fits = MAX_ANIMATED_BYTES / per_1080p;
    assert!((10..=11).contains(&fits), "{fits}");
    // A 4K frame is 24.9 MB; two fit, three do not.
    let per_4k = 3840usize * 2160 * 3;
    assert_eq!(MAX_ANIMATED_BYTES / per_4k, 2);
}

/// An animation check opens the file once, not once to check it and
/// again to draw it: the daemon decodes once (the `Opens` counter in
/// `tests/restore.rs` pins this end to end). Both ways through: an
/// animated GIF (checked, then drawn from the check) and a static PNG
/// (checked, then drawn the static way through the same open).
#[test]
fn an_animation_check_opens_the_file_once() {
    use rustix::fs::inotify::{CreateFlags, ReadFlags, WatchFlags, Reader, add_watch, init};

    fn checked_opens(file: &std::path::Path, run: impl FnOnce()) -> usize {
        let watch = init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK).unwrap();
        // Closes watched too, though not counted: inotify merges an event
        // into the one before it when they are identical and the first is
        // unread, so two opens back to back would read as one; with each
        // open's close between them, no two in a row are alike.
        add_watch(&watch, file, WatchFlags::OPEN | WatchFlags::CLOSE_NOWRITE).unwrap();
        run();
        let mut buf = [std::mem::MaybeUninit::uninit(); 4096];
        let mut reader = Reader::new(&watch, &mut buf);
        let mut opens = 0;
        while let Ok(event) = reader.next() {
            if event.events().contains(ReadFlags::OPEN) {
                opens += 1;
            }
        }
        opens
    }

    let dir = std::env::temp_dir().join(format!("sbg-anim-once-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Animated: checked, and drawn from the check.
    let gif = dir.join("two.gif");
    std::fs::write(&gif, samples::gif_two_frame()).unwrap();
    let opens = checked_opens(&gif, || {
        let checked = super::decode_checked(&gif, FILL).unwrap();
        assert!(matches!(checked, super::Checked::Animated(_)));
    });
    assert_eq!(opens, 1, "one open to check and draw the animation");
    // Static: checked, then drawn the static way through the same open.
    let png = dir.join("still.png");
    std::fs::write(
        &png,
        samples::png(
            2,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[10, 20, 30, 40, 50, 60],
            None,
        ),
    )
    .unwrap();
    let opens = checked_opens(&png, || {
        let checked = super::decode_checked(&png, FILL).unwrap();
        assert!(matches!(checked, super::Checked::Static(_)));
    });
    assert_eq!(opens, 1, "one open to check and draw the still");
    std::fs::remove_dir_all(&dir).unwrap();
}
