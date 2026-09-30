//! A module's icon keys: `icon`, `icon-path` with `icon-viewbox`, and
//! `icon-image`; what each takes, that at most one is set, and that every
//! refusal names its key.

use super::tests::read;
use crate::icon::path::ViewBox;
use crate::icon::{Art, Icon};

fn err(text: &str) -> String {
    read(text).unwrap_err().to_string()
}

#[test]
fn a_path_icon_is_parsed_with_its_viewbox() {
    let config = read("[clock]\nicon-path = \"M0 0h10v10z\"\n").unwrap();
    let Some(Icon::Art(Art::Vector(vector))) = config.modules.clock.icon else {
        panic!("{:?}", config.modules.clock.icon)
    };
    assert_eq!(vector.view(), ViewBox::default());
    assert_eq!(vector.segs().len(), 4);
    let config =
        read("[clock]\nicon-path = \"M0 0h10v10z\"\nicon-viewbox = \"0 0 512 448\"\n").unwrap();
    let Some(Icon::Art(Art::Vector(vector))) = config.modules.clock.icon else {
        panic!()
    };
    assert_eq!(vector.view().width, 512.0);
    assert_eq!(vector.view().height, 448.0);
    // Two reads of the same text are equal configs (a reload compares).
    let a = read("[clock]\nicon-path = \"M0 0h10v10z\"\n").unwrap();
    let b = read("[clock]\nicon-path = \"M0 0h10v10z\"\n").unwrap();
    assert_eq!(a, b);
}

#[test]
fn a_bad_path_or_viewbox_is_refused_naming_its_key() {
    for bad in ["", "L1 1", "M0 0 L", "M0 0 L1e999 0", "M0 0 X", "M1 1"] {
        let error = err(&format!("[clock]\nicon-path = \"{bad}\"\n"));
        assert!(
            error.contains("clock.icon-path") && error.contains("at byte"),
            "{bad:?}: {error}"
        );
    }
    // Longer than the bound, and not a string at all.
    let long = format!("M0 0 {}", "l1 1 ".repeat(4000));
    let error = err(&format!("[clock]\nicon-path = \"{long}\"\n"));
    assert!(error.contains("clock.icon-path"), "{error}");
    let error = err("[clock]\nicon-path = 12\n");
    assert!(error.contains("icon-path"), "{error}");
    for bad in ["", "0 0 24", "0 0 0 24", "x", "0 0 24 24 24"] {
        let error = err(&format!(
            "[clock]\nicon-path = \"M0 0h1v1z\"\nicon-viewbox = \"{bad}\"\n"
        ));
        assert!(error.contains("clock.icon-viewbox"), "{bad:?}: {error}");
    }
    // A viewbox for nothing.
    let error = err("[clock]\nicon-viewbox = \"0 0 24 24\"\n");
    assert!(
        error.contains("clock.icon-viewbox") && error.contains("clock.icon-path"),
        "{error}"
    );
}

#[test]
fn a_clock_shows_one_icon() {
    let error = err("[clock]\nicon = \"x\"\nicon-path = \"M0 0h1v1z\"\n");
    assert!(
        error.contains("clock.icon-path") && error.contains("clock.icon"),
        "{error}"
    );
    #[cfg(feature = "icon-image")]
    {
        let error = err("[clock]\nicon = \"x\"\nicon-image = \"/x.png\"\n");
        assert!(error.contains("clock.icon-image"), "{error}");
        let error = err("[clock]\nicon-path = \"M0 0h1v1z\"\nicon-image = \"/x.png\"\n");
        assert!(
            error.contains("clock.icon-image") && error.contains("clock.icon-path"),
            "{error}"
        );
    }
}

#[cfg(not(feature = "icon-image"))]
#[test]
fn without_the_feature_the_image_key_is_unknown() {
    let error = err("[clock]\nicon-image = \"/x.png\"\n");
    assert!(error.contains("icon-image"), "{error}");
}

#[cfg(feature = "icon-image")]
mod image {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![200u8; (width * height * 4) as usize])
            .unwrap();
        out
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "scootbar-config-image-{}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_image_icon_is_decoded_when_the_config_is_read() {
        let dir = dir("good");
        let file = dir.join("icon.png");
        std::fs::write(&file, png(8, 8)).unwrap();
        let config = read(&format!("[clock]\nicon-image = \"{}\"\n", file.display())).unwrap();
        let Some(Icon::Art(Art::Image(_))) = config.modules.clock.icon else {
            panic!("{:?}", config.modules.clock.icon)
        };
        // Decoded at load: the file can go and the config still has it.
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(config.modules.clock.icon.is_some());
    }

    #[test]
    fn a_missing_relative_or_hostile_image_is_refused_naming_the_key() {
        let dir = dir("bad");
        let missing = err(&format!(
            "[clock]\nicon-image = \"{}/nope.png\"\n",
            dir.display()
        ));
        assert!(
            missing.contains("clock.icon-image") && missing.contains("nope.png"),
            "{missing}"
        );
        let relative = err("[clock]\nicon-image = \"icon.png\"\n");
        assert!(
            relative.contains("clock.icon-image") && relative.contains("absolute"),
            "{relative}"
        );
        let junk = dir.join("junk.png");
        std::fs::write(&junk, b"not a png").unwrap();
        let error = err(&format!("[clock]\nicon-image = \"{}\"\n", junk.display()));
        assert!(
            error.contains("clock.icon-image") && error.contains("not a usable PNG"),
            "{error}"
        );
        let huge = dir.join("huge.png");
        std::fs::write(&huge, png(2000, 2)).unwrap();
        let error = err(&format!("[clock]\nicon-image = \"{}\"\n", huge.display()));
        assert!(
            error.contains("clock.icon-image") && error.contains("2000 x 2"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
