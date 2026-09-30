//! The snapshot scenes of path and image icons: a bar with an icon and
//! text, and one with an icon alone, through the render path at scale 1 and
//! at a fractional scale, in gray tokens so the images are one sample a
//! pixel. The shapes are the ones a user would give: a Material "home"
//! (straight edges, sharp angles) and a circle drawn with arcs, where the
//! anti-aliasing and the fitting to the em show.

use std::sync::Arc;

use ab_glyph::{FontArc, FontVec};
use rustix::event::PollFlags;

use super::{Image, check};
use crate::color::Color;
use crate::density::Scale;
use crate::icon::path::{Vector, ViewBox};
use crate::icon::{Art, Icon};
use crate::layout::Section;
use crate::modules::{Class, Module, OutputView, Placed, Sources, Update, View};
use crate::outputs::{Frame, Size};
use crate::paint::Canvas;
use crate::render::{Member, Record, Scene, Style, paint};
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

/// Every snapshot a test here compares (see `tests::NAMES`).
pub const NAMES: &[&str] = &[
    "icon-path-1x",
    "icon-path-1.5x",
    "icon-path-only-1.25x",
    // Compared only in a build with the feature, but the files are in the
    // tree in every build.
    "icon-image-1x",
    "icon-image-1.5x",
];

const HOME: &str = "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z";
const RING: &str = "M12 2a10 10 0 1 0 0 20a10 10 0 1 0 0-20zm0 4a6 6 0 1 1 0 12a6 6 0 1 1 0-12z";

const fn gray(v: u8) -> Color {
    Color { r: v, g: v, b: v }
}

fn vector(d: &str) -> Icon {
    Icon::Art(Art::Vector(Arc::new(
        Vector::parse(d, ViewBox::default()).unwrap(),
    )))
}

/// A module showing fixed text and an icon.
struct Iconic {
    text: &'static str,
    icon: Icon,
    class: Class,
}

impl Module for Iconic {
    fn sources<'fd>(&'fd self, _: &mut Sources<'_, 'fd>) {}

    fn on_ready(&mut self, _: usize, _: PollFlags) -> Update {
        Update::Unchanged
    }

    fn view(&self, _: &OutputView<'_>, view: &mut View) {
        use std::fmt::Write as _;
        let _ = view.text_mut().write_str(self.text);
        view.show_icon(&self.icon);
        view.set_class(self.class);
    }
}

/// A `width`x20 bar of the given modules (all on the left), at `scale`.
fn bar(width: u32, scale: Scale, modules: Vec<Iconic>) -> Image {
    let placed: Vec<Placed> = modules
        .into_iter()
        .map(|module| Placed {
            id: "label",
            module: Box::new(module),
            revision: 0,
        })
        .collect();
    let style = Style {
        theme: Theme {
            background: gray(0),
            foreground: gray(0xff),
            accent: gray(0xc0),
            dim: gray(0x80),
            urgent: gray(0xe0),
        },
        font_size: 14,
        padding: 4,
        spacing: 6,
        separator: 0,
        radius: 0,
        opacity: u8::MAX,
    };
    let frame = Frame {
        size: Size { width, height: 20 },
        scale,
    };
    let (w, h) = scale.buffer(frame.size).unwrap();
    let mut text = Text::new(FontArc::new(
        FontVec::try_from_vec(testfont::build()).unwrap(),
    ));
    let members: Vec<Member> = (0..placed.len())
        .map(|module| Member {
            module,
            section: Section::Left,
            margin: 0,
        })
        .collect();
    let mut scene = Scene::with_members(&members);
    let output = OutputView { name: None };
    scene.update(
        &placed,
        &output,
        Some(&text),
        &style,
        scale,
        Size {
            width: w,
            height: h,
        },
    );
    let mut pixels = vec![0xaa; w as usize * h as usize * 4];
    let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
    let mut record = Record::new(placed.len());
    paint(
        &mut canvas,
        &mut record,
        &scene,
        &placed,
        Some(&mut text),
        &style,
        frame,
        &output,
    );
    Image::from_xrgb(&pixels, w, h, true)
}

fn home_and_ring() -> Vec<Iconic> {
    vec![
        Iconic {
            text: "12",
            icon: vector(HOME),
            class: Class::Normal,
        },
        Iconic {
            text: "9",
            icon: vector(RING),
            class: Class::Warn,
        },
    ]
}

#[test]
fn a_path_icon_at_1x() {
    check(
        "icon-path-1x",
        "a 90x20 bar at scale 1: a home path icon and text, a ring (arcs, even-odd hole by winding) in the warn token and text",
        &bar(90, Scale::Integer(1), home_and_ring()),
    );
}

/// 1.5: the em is 21 device pixels, so the 24-unit viewbox scales by 7/8
/// and the edges land between pixels.
#[test]
fn a_path_icon_at_1_5x() {
    check(
        "icon-path-1.5x",
        "the same bar at scale 1.5 (fractional, 135x30 device pixels)",
        &bar(90, Scale::Fractional(180), home_and_ring()),
    );
}

/// An icon with no text, at 1.25 (an em of 17.5, rounded to 18).
#[test]
fn a_path_icon_alone_at_1_25x() {
    let alone = vec![Iconic {
        text: "",
        icon: vector(HOME),
        class: Class::Muted,
    }];
    check(
        "icon-path-only-1.25x",
        "a 30x20 bar at scale 1.25: a home path icon alone, in the dim token, no gap after it",
        &bar(30, Scale::Fractional(150), alone),
    );
}

#[cfg(feature = "icon-image")]
mod image {
    use super::*;
    use crate::icon::image::decode;

    /// A 40x40 gray-and-alpha PNG: a soft-edged disc, whose alpha falls
    /// off over its last pixels, on a transparent ground.
    fn disc() -> Vec<u8> {
        let mut data = Vec::new();
        for y in 0..40u32 {
            for x in 0..40u32 {
                let (dx, dy) = (x as f32 + 0.5 - 20.0, y as f32 + 0.5 - 20.0);
                let d = dx.hypot(dy);
                let alpha = ((18.0 - d) / 2.0).clamp(0.0, 1.0);
                // Brighter toward the middle.
                let value = (255.0 - d * 6.0).clamp(90.0, 255.0);
                data.extend_from_slice(&[value as u8, (alpha * 255.0).round() as u8]);
            }
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, 40, 40);
        encoder.set_color(png::ColorType::GrayscaleAlpha);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&data)
            .unwrap();
        out
    }

    fn picture() -> Vec<Iconic> {
        let image = decode(&disc()).unwrap();
        vec![Iconic {
            text: "12",
            icon: Icon::Art(Art::Image(Arc::new(image))),
            class: Class::Normal,
        }]
    }

    #[test]
    fn an_image_icon_at_1x() {
        check(
            "icon-image-1x",
            "a 50x20 bar at scale 1: a 40x40 soft disc PNG scaled to the 14-pixel em, then text",
            &bar(50, Scale::Integer(1), picture()),
        );
    }

    #[test]
    fn an_image_icon_at_1_5x() {
        check(
            "icon-image-1.5x",
            "the same bar at scale 1.5: the PNG scaled to 21 pixels, not the 14-pixel one stretched",
            &bar(50, Scale::Fractional(180), picture()),
        );
    }
}
