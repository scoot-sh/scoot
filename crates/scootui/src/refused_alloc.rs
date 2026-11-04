//! What a block the allocator refuses does to the bar: a named error or a
//! blank draw, never an abort. This is the situation `LimitAS=`,
//! `ulimit -v` and strict overcommit make real. Each test runs in a child
//! process ([`crate::refusal`]), builds its fixtures, then has the
//! allocator refuse every block from a size smaller than the one under
//! test. Each also runs a small control after the refusal, so a size that
//! starves everything cannot pass for the fix.
//!
//! Without the fallible allocation each of these ends in `memory
//! allocation of N bytes failed` and `SIGABRT`.

use crate::refusal::in_refusing_child;

macro_rules! refused {
    ($name:ident, |$refusing:ident| $body:block) => {
        #[test]
        fn $name() {
            in_refusing_child(
                concat!(module_path!(), "::", stringify!($name)),
                |$refusing| $body,
            );
        }
    };
}

/// A PNG of `side × side` 8-bit gray, every pixel `v`.
#[cfg(feature = "png")]
fn gray_png(side: u32, v: u8) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, side, side);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![v; (side * side) as usize])
            .unwrap();
    }
    out
}

// A 1024 × 1024 gray PNG is 4 MiB of premultiplied pixels (1 MiB decoded
// first); the cap leaves 2 MiB.
#[cfg(feature = "png")]
refused!(
    an_image_icon_the_allocator_refuses_is_an_error,
    |refusing| {
        use crate::icon::image::{Error, decode};
        let big = gray_png(1024, 200);
        let small = gray_png(16, 200);
        refusing.from(2 * 1024 * 1024);
        match decode(&big) {
            Err(Error::NoMemory(bytes)) => assert_eq!(bytes, 4 * 1024 * 1024),
            other => panic!("{other:?}"),
        }
        assert!(decode(&small).is_ok(), "the control did not decode");
        // The error says what happened.
        assert!(
            Error::NoMemory(4096)
                .to_string()
                .contains("out of memory: cannot allocate 4096 bytes")
        );
    }
);

// A sparse 40 MiB file: the read block is the largest the bar owns, and a
// refusal costs no read.
refused!(
    a_font_the_allocator_refuses_is_an_error_not_an_abort,
    |refusing| {
        use crate::font::{FileError, load};
        let dir = std::env::temp_dir().join(format!("scootui-oom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = dir.join("big.ttf");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(40 * 1024 * 1024)
            .unwrap();
        let small = dir.join("small.ttf");
        std::fs::write(&small, crate::testfont::build()).unwrap();
        refusing.from(1024 * 1024);
        let result = load(&big);
        let control = load(&small);
        let _ = std::fs::remove_dir_all(&dir);
        match result {
            Err(FileError::NoMemory(size)) => assert_eq!(size, 40 * 1024 * 1024),
            Err(other) => panic!("{other}"),
            Ok(_) => panic!("loaded"),
        }
        assert!(control.is_ok(), "the control font did not load");
        assert!(
            FileError::NoMemory(7)
                .to_string()
                .contains("out of memory: cannot allocate 7 bytes")
        );
    }
);

// 512 × 512 is the biggest icon the cache draws: a 256 KiB mask, and 1 MiB
// of coverage for the rasterizer. The first refusal draws nothing at all
// (and is asked for again), the second a blank mask.
refused!(
    an_icon_bitmap_the_allocator_refuses_draws_nothing,
    |refusing| {
        use crate::icon::path::{Vector, ViewBox};
        use crate::icon::{Art, Cache};
        let art = Art::Vector(std::sync::Arc::new(
            Vector::parse("M0 0H24V24H0z", ViewBox::default()).unwrap(),
        ));
        let mut cache = Cache::default();
        // Warm: the cache's entry list, so only the big blocks are new.
        assert!(cache.get(&art, 16).is_some());
        refusing.from(200 * 1024);
        assert!(cache.get(&art, 512).is_none(), "the arena was refused");
        assert_eq!(cache.cached().0, 1, "no entry for the refused one");
        assert!(cache.get(&art, 16).is_some(), "a hit survives the refusal");
        assert!(cache.get(&art, 20).is_some(), "the control did not draw");
    }
);

refused!(
    a_rasterizer_buffer_the_allocator_refuses_leaves_a_blank_mask,
    |refusing| {
        use crate::icon::path::{Vector, ViewBox};
        use crate::icon::raster::Rasterizer;
        let vector = Vector::parse("M0 0H24V24H0z", ViewBox::default()).unwrap();
        let mut rasterizer = Rasterizer::default();
        let mut out = vec![0xaau8; 512 * 512];
        let mut small = vec![0xaau8; 16 * 16];
        refusing.from(512 * 1024);
        // 514 × 512 `f32`s, 1 MiB.
        rasterizer.fill(&vector, 512, &mut out);
        assert!(out.iter().all(|&c| c == 0), "not a blank mask");
        rasterizer.fill(&vector, 16, &mut small);
        assert!(small.iter().all(|&c| c >= 254), "the control did not fill");
    }
);

// 1024 × 1024 into 512: the horizontal pass holds 512 × 1024 × 4 `f32`s,
// 8 MiB.
#[cfg(any(feature = "png", feature = "tray"))]
refused!(
    a_resampling_buffer_the_allocator_refuses_draws_transparent,
    |refusing| {
        use crate::icon::sample::scale_into;
        let source = vec![0xffu8; 1024 * 1024 * 4];
        let mut out = vec![0xaau8; 512 * 512 * 4];
        let small_source = vec![0xffu8; 4 * 4 * 4];
        let mut small = vec![0xaau8; 4 * 4 * 4];
        refusing.from(2 * 1024 * 1024);
        scale_into(&source, 1024, 1024, 512, &mut out);
        assert!(out.iter().all(|&b| b == 0), "not transparent");
        scale_into(&small_source, 4, 4, 4, &mut small);
        assert!(small.iter().all(|&b| b == 0xff), "the control did not draw");
    }
);

#[cfg(feature = "tray")]
refused!(a_tray_pixmap_the_allocator_refuses_is_no_icon, |refusing| {
    use crate::icon::tray::TrayIcon;
    let big = vec![0xffu8; 512 * 512 * 4];
    let small = vec![0xffu8; 16 * 16 * 4];
    refusing.from(512 * 1024);
    assert!(TrayIcon::take(1, 512, 512, &big).is_none());
    assert!(TrayIcon::take(2, 16, 16, &small).is_some());
});

// A glyph at 1024 pixels is some 300 KB of coverage, more than the
// allocator's own slack.
refused!(a_glyph_the_allocator_refuses_is_not_drawn, |refusing| {
    use crate::color::Color;
    use crate::paint::{Canvas, Span};
    use crate::text::Text;
    let mut text = Text::new(ab_glyph::FontArc::new(
        ab_glyph::FontVec::try_from_vec(crate::testfont::build()).unwrap(),
    ));
    let (width, height) = (600u32, 700u32);
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, width, height).unwrap();
    let fg = Color {
        r: 255,
        g: 255,
        b: 255,
    };
    let baseline = text.metrics(1024.0).baseline(height);
    refusing.from(128 * 1024);
    text.draw(
        &mut canvas,
        None,
        "8",
        1024.0,
        10,
        baseline,
        fg,
        Span { x: 0, width },
    );
    assert_eq!(text.cached(), (0, 0), "the refused glyph was cached");
    // The control: a small glyph still draws after the refusal.
    let baseline = text.metrics(20.0).baseline(height);
    text.draw(
        &mut canvas,
        None,
        "8",
        20.0,
        10,
        baseline,
        fg,
        Span { x: 0, width },
    );
    assert_eq!(text.cached().0, 1);
});
