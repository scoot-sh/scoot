use std::sync::Arc;

use super::{Image, Wallpaper};
use crate::color::Color;
use crate::image::render::Look;
use crate::image::{Filter, Mode};

fn image(path: &str, serial: u64) -> Wallpaper {
    Wallpaper::Image(Arc::new(Image {
        path: path.into(),
        look: Look {
            mode: Mode::Fill,
            fill: Color { r: 0, g: 0, b: 0 },
            filter: Filter::Lanczos3,
        },
        animate: true,
        serial,
        fetch: None,
    }))
}

#[test]
fn images_are_the_same_only_from_the_same_request() {
    assert_eq!(image("/a.png", 3), image("/a.png", 3));
    // The same file set again is a new request: read again.
    assert_ne!(image("/a.png", 3), image("/a.png", 4));
    let red = Wallpaper::Color(Color { r: 255, g: 0, b: 0 });
    assert_eq!(red, Wallpaper::Color(Color { r: 255, g: 0, b: 0 }));
    assert_ne!(red, image("/a.png", 3));
    assert!(red.image().is_none());
    assert_eq!(image("/b.png", 9).image().unwrap().path, "/b.png");
}
